// The sfx orchestrator (docs/physics-sound.md, "Orchestrator"): decides per sound between live
// synthesis and baked audio, renders the bakes and the reverb impulse responses in a worker, keeps
// them in a byte-budgeted LRU cache (and across sessions in IndexedDB), and governs the audio
// thread's load: lighter spatial chains, a shorter reverb bank and fewer rays before any dropout.
//
// - A one-shot (impact, footstep, break, explosion) whose cell is baked plays an AudioBuffer through
//   its voice's usual spatial chain; an un-baked cell plays live once and its bake is queued.
//   Continuous, input-driven voices (slide, roll, rain per face, swarm clusters) stay live.
// - A cell is the preset at its declared inputs quantised (energy 16 steps, the others 8, discrete
//   inputs exact), in `variations` seeds so repeats still differ. Between two baked energy cells the
//   level is interpolated from their measured RMS.
// - The governor reads the context's underruns (AudioContext.playbackStats, Chrome) and the
//   engine's modelled load, and steps a level 0-3 up at once and down after a calm spell.
import type { HaaInput } from '../types.ts';

export type BakeWorkerIn =
  | { t: 'init'; module: WebAssembly.Module }
  | { t: 'bake'; id: number; presetKey: string; bytes: Uint8Array; inputs: Record<string, number>; seed: number; sampleRate: number; seconds: number }
  | { t: 'ir'; id: number; rt60: number; absorption: number; sampleRate: number };

export type BakeWorkerOut =
  | { t: 'ready' }
  | { t: 'done'; id: number; l: Float32Array<ArrayBuffer>; r: Float32Array<ArrayBuffer>; rms: number; ms: number }
  | { t: 'error'; id: number; message: string };

// --- quantisation (pure) -----------------------------------------------------------------------

/** Steps of each contract input over its declared range; others get `DEFAULT_STEPS`. */
export const INPUT_STEPS: Readonly<Record<string, number>> = { energy: 16, size: 8, hardness: 8, depth: 6, speed: 8 };
export const DEFAULT_STEPS = 8;

/** Position (0..1) of `v` in a declared input's range (log when the input is log). */
function unit(decl: HaaInput, v: number): number {
  if (decl.kind !== 'continuous') return v;
  const { min, max } = decl;
  if (decl.curve === 'log' && min > 0 && max > 0) return Math.log(Math.max(min, v) / min) / Math.log(max / min);
  return max > min ? (v - min) / (max - min) : 0;
}

function fromUnit(decl: HaaInput, u: number): number {
  if (decl.kind !== 'continuous') return u;
  const { min, max } = decl;
  if (decl.curve === 'log' && min > 0 && max > 0) return min * (max / min) ** u;
  return min + (max - min) * u;
}

export interface Quantized {
  /** Declared values at the cell's centre (what the bake renders). */
  inputs: Record<string, number>;
  /** Cell key: input=step pairs. */
  key: string;
  /** Unit position of `energy` within its declared range and the cell's (for the level interpolation). */
  energy?: { u: number; cell: number; steps: number };
}

/** The cell of `declared` values (as the pool receives them) for a preset's input declarations. */
export function quantize(decls: readonly HaaInput[], declared: Record<string, number>, steps: Readonly<Record<string, number>> = INPUT_STEPS): Quantized {
  const inputs: Record<string, number> = {};
  const parts: string[] = [];
  let energy: Quantized['energy'];
  for (const d of decls) {
    if (!(d.name in declared)) continue;
    const v = declared[d.name];
    if (d.kind !== 'continuous') {
      inputs[d.name] = v;
      parts.push(`${d.name}=${v}`);
      continue;
    }
    const n = steps[d.name] ?? DEFAULT_STEPS;
    const u = Math.min(1, Math.max(0, unit(d, v)));
    const cell = Math.round(u * n);
    inputs[d.name] = fromUnit(d, cell / n);
    parts.push(`${d.name}=${cell}`);
    if (d.name === 'energy') energy = { u, cell, steps: n };
  }
  return { inputs, key: parts.join('&'), energy };
}

/** FNV-1a of a patch's bytes: a preset edited is a new set of cells. */
export function hashBytes(b: Uint8Array): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < b.length; i++) {
    h ^= b[i];
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(36);
}

// --- cache (pure) -----------------------------------------------------------------------------

export interface BakeEntry {
  l: Float32Array<ArrayBuffer>;
  r: Float32Array<ArrayBuffer>;
  rms: number;
  /** Built on first play (an AudioBuffer of the context). */
  buffer?: AudioBuffer;
}

/** Bytes an entry holds: its samples, or its AudioBuffer once built (the samples are then dropped). */
export function entryBytes(e: BakeEntry): number {
  return e.buffer ? e.buffer.length * e.buffer.numberOfChannels * 4 : e.l.byteLength + e.r.byteLength;
}

/** A Map in recency order with a byte budget: the least recently used entries go first. */
export class BakeCache {
  private readonly map = new Map<string, BakeEntry>();
  bytes = 0;
  evicted = 0;
  readonly budget: number;
  constructor(budget: number) {
    this.budget = budget;
  }
  get size(): number {
    return this.map.size;
  }
  has(key: string): boolean {
    return this.map.has(key);
  }
  get(key: string): BakeEntry | undefined {
    const e = this.map.get(key);
    if (!e) return undefined;
    this.map.delete(key);
    this.map.set(key, e);
    return e;
  }
  set(key: string, e: BakeEntry): void {
    const old = this.map.get(key);
    if (old) {
      this.bytes -= entryBytes(old);
      this.map.delete(key);
    }
    this.map.set(key, e);
    this.bytes += entryBytes(e);
    this.trim();
  }
  /** Re-account an entry whose size changed (its AudioBuffer was built). */
  touch(key: string, before: number): void {
    const e = this.map.get(key);
    if (!e) return;
    this.bytes += entryBytes(e) - before;
    this.trim();
  }
  private trim(): void {
    for (const [k, e] of this.map) {
      if (this.bytes <= this.budget || this.map.size <= 1) break;
      this.map.delete(k);
      this.bytes -= entryBytes(e);
      this.evicted++;
    }
  }
}

// --- governor (pure) --------------------------------------------------------------------------

export interface GovernorState {
  level: number;
  /** Underrun events seen in total (the context's running count), and new in the last step. */
  underruns: number;
  recent: number;
  /** Since when (s) nothing went wrong, and how long that must last before a step down. */
  calmSince: number;
  calmNeeded: number;
  /** When the last step down happened (s). */
  downAt: number;
}

export const GOVERNOR_START: GovernorState = { level: 0, underruns: 0, recent: 0, calmSince: 0, calmNeeded: 4, downAt: -Infinity };

/**
 * One governor step at time `now` (s): `underruns` is the context's running count, `load` the
 * modelled share of the audio thread. Up one level at once on a new underrun (or a modelled load
 * over 0.85); down one after `calmNeeded` s with neither. A step down that brings underruns back
 * within 10 s doubles `calmNeeded` (up to 60 s), so the level settles where it holds instead of
 * oscillating; a long calm halves it back toward 4 s. Levels 0-3.
 */
export function governorStep(g: GovernorState, now: number, underruns: number, load: number): GovernorState {
  const fresh = Math.max(0, underruns - g.underruns);
  let { level, calmSince, calmNeeded, downAt } = g;
  if (fresh > 0 || load > 0.85) {
    if (fresh > 0 && now - downAt < 10) calmNeeded = Math.min(60, calmNeeded * 2);
    level = Math.min(3, level + 1);
    calmSince = now;
  } else if (now - calmSince > calmNeeded) {
    if (level > 0) {
      level--;
      downAt = now;
    } else calmNeeded = Math.max(4, calmNeeded / 2);
    calmSince = now;
  }
  return { level, underruns, recent: fresh, calmSince, calmNeeded, downAt };
}

/** Spatial chain of a voice ranked `rank` (0 the loudest) at governor `level`: early-reflection taps and HRTF. */
export function chainQuality(level: number, rank: number, extent = 0): { taps: number; hrtf: boolean } {
  // A large source heard from inside (rain all around) is mostly its wide stereo part.
  const wide = extent >= 3;
  if (level <= 0) return rank < 6 && !wide ? { taps: 4, hrtf: true } : { taps: 2, hrtf: !wide };
  if (level === 1) return rank < 4 && !wide ? { taps: 4, hrtf: true } : { taps: 0, hrtf: !wide && rank < 8 };
  if (level === 2) return rank < 2 && !wide ? { taps: 2, hrtf: true } : { taps: 0, hrtf: false };
  return { taps: 0, hrtf: rank < 1 && !wide };
}

/** Longest reverb time the bank may play at governor `level` (s): the longest responses cost the most. */
export function bankCap(level: number): number {
  return [Infinity, 4, 2.8, 2][Math.min(3, Math.max(0, level))];
}

/**
 * Convolvers of the bank allowed to run at governor `level`. Every fed convolver costs the audio
 * thread whatever its length (Chrome, a busy scene over 6 s: 11 fed, 388 underruns; 3, about 100;
 * 1, seven; none, ten), so the bank runs the steps the voices need most.
 */
export function bankSize(level: number): number {
  return [3, 2, 1, 1][Math.min(3, Math.max(0, level))];
}

/** Share of the listener ray budget kept at governor `level`. */
export function rayScale(level: number): number {
  return [1, 1, 0.5, 0.25][Math.min(3, Math.max(0, level))];
}

// --- the orchestrator (browser) ---------------------------------------------------------------

export interface OrchestratorOptions {
  /** A Worker running `@etcito/haa/bake-worker` (created by the host so its bundler resolves the URL). */
  worker: Worker;
  /** Memory budget of the baked cells (bytes). Default 48 MB. */
  budgetBytes?: number;
  /** Seeds per cell. Default 3. */
  variations?: number;
  /** Keep bakes and impulse responses in IndexedDB across sessions. Default true. */
  persist?: boolean;
  /** Engine build identity for the persistent keys (a new engine renders again). */
  version?: string;
}

export interface BakeablePreset {
  name: string;
  bytes: Uint8Array;
  info: { inputs: readonly HaaInput[]; mode: string; duration: number };
  /** Measured synthesis cost (share of real time): the heavy ones are baked first. */
  cost: number;
}

export interface OrchestratorStats {
  level: number;
  underruns: number;
  baked: number;
  live: number;
  cells: number;
  cacheMB: number;
  queued: number;
  bakesDone: number;
  bakeMs: number;
  irs: number;
  prewarmMs: number | null;
}

export interface SfxOrchestrator {
  /** A baked buffer for this one-shot at these declared inputs, with its level correction; null: play live (the bake is queued). */
  oneShot(rec: BakeablePreset, declared: Record<string, number>): { buffer: AudioBuffer; gain: number } | null;
  /** Queue cells to bake ahead of time (idle priority, heaviest presets first). */
  prewarm(items: Array<{ rec: BakeablePreset; declared: Record<string, number> }>): void;
  /** A bank impulse response, rendered in the worker (or read back from IndexedDB). */
  ir(rt60: number, absorption: number): Promise<AudioBuffer | null>;
  /** The governor's step, once per engine update, with the engine's modelled load. */
  tick(now: number, load: number): void;
  readonly level: number;
  quality(rank: number, extent?: number): { taps: number; hrtf: boolean };
  bankCap(): number;
  /** Convolvers the bank may run now. */
  bankSize(): number;
  rayScale(): number;
  /** Count a voice that played live (the engine calls it for one-shots it could not bake). */
  countLive(): void;
  stats(): OrchestratorStats;
  dispose(): void;
}

interface Job {
  id: number;
  msg: BakeWorkerIn;
  priority: number;
  resolve: (r: BakeWorkerOut) => void;
}

/** IndexedDB store of baked samples, or null where IndexedDB is missing (private windows, workers). */
async function openStore(): Promise<IDBDatabase | null> {
  try {
    if (typeof indexedDB === 'undefined') return null;
    return await new Promise<IDBDatabase | null>((resolve) => {
      const req = indexedDB.open('haa-sfx-bakes', 1);
      req.onupgradeneeded = () => req.result.createObjectStore('cells');
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    });
  } catch {
    return null;
  }
}

function idb<T>(db: IDBDatabase, mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T | undefined> {
  return new Promise((resolve) => {
    try {
      const req = f(db.transaction('cells', mode).objectStore('cells'));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(undefined);
    } catch {
      resolve(undefined);
    }
  });
}

export async function createOrchestrator(ctx: BaseAudioContext, module: WebAssembly.Module, o: OrchestratorOptions): Promise<SfxOrchestrator> {
  const worker = o.worker;
  const variations = Math.max(1, o.variations ?? 3);
  const version = o.version ?? '0';
  const cache = new BakeCache(o.budgetBytes ?? 48 * 1024 * 1024);
  const db = o.persist === false ? null : await openStore();
  const stored = new Set<string>(db ? (((await idb(db, 'readonly', (s) => s.getAllKeys())) ?? []) as string[]) : []);
  const queue: Job[] = [];
  const pending = new Map<string, Promise<BakeEntry | null>>();
  const hashes = new WeakMap<Uint8Array, string>();
  let busy: Job | null = null;
  let nextId = 1;
  let gov: GovernorState = { ...GOVERNOR_START };
  const counts = { baked: 0, live: 0, bakesDone: 0, bakeMs: 0, irs: 0 };
  let prewarmStart = -1;
  let prewarmMs: number | null = null;
  const sr = ctx.sampleRate;

  await new Promise<void>((resolve, reject) => {
    worker.onmessage = (e: MessageEvent<BakeWorkerOut>) => {
      if (e.data.t === 'ready') resolve();
    };
    worker.onerror = (e) => reject(new Error(String(e.message)));
    worker.postMessage({ t: 'init', module } satisfies BakeWorkerIn);
  });
  worker.onmessage = (e: MessageEvent<BakeWorkerOut>) => {
    const m = e.data;
    if (m.t === 'ready' || !busy || ('id' in m && m.id !== busy.id)) return;
    const job = busy;
    busy = null;
    if (m.t === 'done') {
      counts.bakesDone++;
      counts.bakeMs += m.ms;
    }
    job.resolve(m);
    pump();
  };

  function pump(): void {
    if (busy || !queue.length) {
      if (!busy && !queue.length && prewarmStart >= 0 && prewarmMs === null) prewarmMs = performance.now() - prewarmStart;
      return;
    }
    queue.sort((a, b) => b.priority - a.priority);
    busy = queue.shift()!;
    worker.postMessage(busy.msg);
  }

  function run(msg: BakeWorkerIn, priority: number): Promise<BakeWorkerOut> {
    return new Promise((resolve) => {
      queue.push({ id: (msg as { id: number }).id, msg, priority, resolve });
      pump();
    });
  }

  function presetKey(rec: BakeablePreset): string {
    let h = hashes.get(rec.bytes);
    if (!h) {
      h = hashBytes(rec.bytes);
      hashes.set(rec.bytes, h);
    }
    return `${rec.name}#${h}`;
  }

  const cellKey = (pk: string, q: string, seed: number): string => `bake:${version}:${sr}:${pk}:${q}:${seed}`;

  /** The entry for a cell: memory, else IndexedDB, else a bake (queued at `priority`). */
  function fetchCell(rec: BakeablePreset, q: Quantized, seed: number, priority: number): Promise<BakeEntry | null> {
    const pk = presetKey(rec);
    const key = cellKey(pk, q.key, seed);
    const have = cache.get(key);
    if (have) return Promise.resolve(have);
    let p = pending.get(key);
    if (p) return p;
    p = (async () => {
      if (db && stored.has(key)) {
        const v = (await idb(db, 'readonly', (s) => s.get(key))) as { l: Float32Array<ArrayBuffer>; r: Float32Array<ArrayBuffer>; rms: number } | undefined;
        if (v) {
          const e: BakeEntry = { l: v.l, r: v.r, rms: v.rms };
          cache.set(key, e);
          return e;
        }
      }
      const seconds = rec.info.mode === 'timeline' ? Math.min(rec.info.duration, 6) : 3;
      const res = await run({ t: 'bake', id: nextId++, presetKey: pk, bytes: rec.bytes, inputs: q.inputs, seed: 1 + seed * 7919, sampleRate: sr, seconds }, priority);
      if (res.t !== 'done') return null;
      const e: BakeEntry = { l: res.l, r: res.r, rms: res.rms };
      cache.set(key, e);
      if (db) {
        void idb(db, 'readwrite', (s) => s.put({ l: e.l, r: e.r, rms: e.rms }, key));
        stored.add(key);
      }
      return e;
    })().finally(() => pending.delete(key));
    pending.set(key, p);
    return p;
  }

  function bufferOf(key: string, e: BakeEntry): AudioBuffer {
    if (!e.buffer) {
      const before = entryBytes(e);
      const b = ctx.createBuffer(2, e.l.length, sr);
      b.copyToChannel(e.l, 0);
      b.copyToChannel(e.r, 1);
      e.buffer = b;
      // The AudioBuffer holds its own copy (IndexedDB keeps the samples across sessions).
      e.l = new Float32Array(0);
      e.r = new Float32Array(0);
      cache.touch(key, before);
    }
    return e.buffer;
  }

  /** RMS of a ready neighbour cell at energy step `cell` (any seed), or null. */
  function rmsAt(rec: BakeablePreset, q: Quantized, cell: number): number | null {
    const pk = presetKey(rec);
    const k = q.key.replace(/(^|&)energy=\d+/, `$1energy=${cell}`);
    for (let s = 0; s < variations; s++) {
      const e = cache.get(cellKey(pk, k, s));
      if (e) return e.rms;
    }
    return null;
  }

  return {
    get level() {
      return gov.level;
    },
    oneShot(rec, declared) {
      const q = quantize(rec.info.inputs, declared);
      const pk = presetKey(rec);
      const ready: number[] = [];
      for (let s = 0; s < variations; s++) if (cache.has(cellKey(pk, q.key, s))) ready.push(s);
      if (!ready.length) {
        // Played live this time; its seeds bake now, ahead of the pre-warm.
        for (let s = 0; s < variations; s++) void fetchCell(rec, q, s, 10 + rec.cost);
        counts.live++;
        return null;
      }
      const seed = ready[Math.floor(Math.random() * ready.length)];
      const key = cellKey(pk, q.key, seed);
      const e = cache.get(key)!;
      // Level between the two energy cells around the true value, from their RMS (in dB).
      let gain = 1;
      if (q.energy && e.rms > 0) {
        const { u, cell, steps } = q.energy;
        const x = u * steps;
        const other = x >= cell ? cell + 1 : cell - 1;
        const r2 = other >= 0 && other <= steps ? rmsAt(rec, q, other) : null;
        if (r2 && r2 > 0) {
          const t = Math.abs(x - cell);
          gain = 10 ** ((t * 20 * Math.log10(r2 / e.rms)) / 20);
        }
      }
      counts.baked++;
      return { buffer: bufferOf(key, e), gain };
    },
    prewarm(items) {
      if (prewarmStart < 0) prewarmStart = performance.now();
      prewarmMs = null;
      for (const { rec, declared } of items) {
        const q = quantize(rec.info.inputs, declared);
        for (let s = 0; s < variations; s++) void fetchCell(rec, q, s, rec.cost - s * 0.001);
      }
      pump();
    },
    async ir(rt60, absorption) {
      const key = `ir:${version}:${sr}:${rt60}:${absorption}`;
      let l: Float32Array<ArrayBuffer>;
      let r: Float32Array<ArrayBuffer>;
      const v = db && stored.has(key) ? ((await idb(db, 'readonly', (s) => s.get(key))) as { l: Float32Array<ArrayBuffer>; r: Float32Array<ArrayBuffer> } | undefined) : undefined;
      if (v) {
        l = v.l;
        r = v.r;
      } else {
        // The bank's responses first and shortest first (the longest takes seconds of work).
        const res = await run({ t: 'ir', id: nextId++, rt60, absorption, sampleRate: sr }, 100 - rt60);
        if (res.t !== 'done') return null;
        l = res.l;
        r = res.r;
        if (db) {
          void idb(db, 'readwrite', (s) => s.put({ l, r }, key));
          stored.add(key);
        }
      }
      const b = ctx.createBuffer(2, l.length, sr);
      b.copyToChannel(l, 0);
      b.copyToChannel(r, 1);
      counts.irs++;
      return b;
    },
    tick(now, load) {
      const ps = (ctx as unknown as { playbackStats?: { underrunEvents: number } }).playbackStats;
      gov = governorStep(gov, now, ps?.underrunEvents ?? gov.underruns, load);
    },
    quality: (rank, extent) => chainQuality(gov.level, rank, extent),
    bankCap: () => bankCap(gov.level),
    bankSize: () => bankSize(gov.level),
    rayScale: () => rayScale(gov.level),
    countLive() {
      counts.live++;
    },
    stats() {
      return {
        level: gov.level, underruns: gov.underruns, baked: counts.baked, live: counts.live, cells: cache.size,
        cacheMB: +(cache.bytes / 1048576).toFixed(1), queued: queue.length + (busy ? 1 : 0), bakesDone: counts.bakesDone,
        bakeMs: Math.round(counts.bakeMs), irs: counts.irs, prewarmMs: prewarmMs === null ? null : Math.round(prewarmMs),
      };
    },
    dispose() {
      worker.terminate();
      db?.close();
    },
  };
}
