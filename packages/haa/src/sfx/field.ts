// The baked acoustic field as the engine's acoustics (docs/physics-sound.md, "Acoustic field"),
// behind `createSfxEngine({ acoustics: { field } })`. Renderer-agnostic: it takes geometry records
// and, optionally, occupancy points (a renderer's depth buffer unprojected, or RAA's SDF samples).
//
// Two levels:
// - coarse (1-2 m cells) over the level, baked once in a worker, nearest to the listener first,
//   re-baked where the static geometry changes (`invalidate`);
// - fine (0.5 m cells) in a box around the listener, re-baked in a second worker from time to time
//   as the player moves, from the static geometry plus the dynamic bodies and the occupancy points.
// The main thread keeps a copy of each (the baked records) and does the lookups and the flood in
// wasm: per source, one trilinear read and one flooded path instead of rays per event.
import type { Reflection, Vec3 } from './acoustics.ts';
import { add, dot, length, normalize, scale, sub, v3, SPEED_OF_SOUND_M_S } from './acoustics.ts';
import type { FieldGrid, FieldPath, FieldSample, SfxGeometry } from './field-wasm.ts';
import { FieldWasm } from './field-wasm.ts';
import type { FieldWorkerIn, FieldWorkerOut } from './field-worker.ts';
import type { DirectPath, VoiceReverb } from './raytrace.ts';

/** What the engine asks of a field (see engine.ts `acoustics.field`). */
export interface SfxFieldProvider {
  /** Every frame: the listener moved. */
  listener(p: Vec3): void;
  /** True when the field has baked cells around `p`. */
  ready(p: Vec3): boolean;
  path(s: Vec3, l: Vec3, extent: number): DirectPath;
  reverb(s: Vec3, l: Vec3, path: DirectPath): VoiceReverb;
  reflections(s: Vec3, l: Vec3): Reflection[];
}

export interface FineOptions {
  /** Cell size (m). Default 0.5. */
  cell?: number;
  /** Half size of the box around the listener (m). Default [6, 2, 6]. */
  half?: [number, number, number];
  /** Re-bake when the listener has moved this far from the box centre (m). Default 2. */
  moveBy?: number;
  /** ... or after this long (s), to see bodies that moved. Default 3. */
  every?: number;
}

export interface AcousticFieldOptions {
  module: WebAssembly.Module;
  /** A new field worker (`@etcito/haa/field-worker`), e.g. Vite's `new FieldWorker()`. */
  worker: () => Worker;
  /** The static level (fixed colliders). */
  geometry: () => SfxGeometry;
  /** The coarse grid over the level. */
  grid: FieldGrid;
  /** The fine level around the listener; omit to use the coarse level only. */
  fine?: FineOptions | false;
  /**
   * The moving bodies the fine level should see (crates, not pebbles), added to the static level
   * (kept from `geometry`, re-read on `invalidate`). Walk only the moving bodies: it runs on the main
   * thread at every fine re-bake.
   */
  movingGeometry?: () => SfxGeometry;
  /** Occupancy points age out after this (s). Default 8. */
  occupancyLife?: number;
  /** Keep the coarse bake in IndexedDB, keyed by a hash of the level and the grid. Default true. */
  persist?: boolean;
}

export interface FieldStats {
  coarse: { baked: number; left: number; bytes: number; bakeMs: number; casts: number; reached: number; restored: number; saved: number };
  fine: { baked: number; left: number; bytes: number; bakeMs: number; rebakes: number } | null;
  /** Last flood in the worker (ms) and the cells it reached. */
  floodMs: number;
  floodReached: number;
  /** Last opening scan in the worker (ms). */
  apertureMs: number;
  lookups: number;
  lookupMs: number;
  /** Longest single main-thread stretch of field work in a frame (ms), and its running mean. */
  mainMaxMs: number;
  mainMeanMs: number;
  occupancy: number;
}

/** An opening is narrower than half the cross-section at both ends and under 16 m² (a door, not a corridor). */
const OPENING_MAX_M2 = 16;
/** Bump when the baked cell format or what a cell holds changes (the persisted bakes' key). */
const FIELD_FORMAT = 5;

/** Energy (linear, relative to the direct sound at 2 m) of a sample's traced diffuse field. */
function energyOf(s: FieldSample): number {
  return s.weight > 0 ? 10 ** (s.energyDb / 10) : 0;
}

/** FNV-1a over the level's records and the grid: the persisted bake's key. */
export function fieldKey(g: SfxGeometry, grid: FieldGrid): string {
  let h = 0x811c9dc5;
  const mix = (a: Float32Array): void => {
    const u = new Uint32Array(a.buffer, a.byteOffset, a.length);
    for (let i = 0; i < u.length; i++) {
      h ^= u[i];
      h = Math.imul(h, 0x01000193) >>> 0;
    }
  };
  mix(g.boxes);
  mix(g.triangles);
  mix(new Float32Array([grid.origin.x, grid.origin.y, grid.origin.z, grid.cell, ...grid.n]));
  return `field:${FIELD_FORMAT}:${g.boxes.length}:${g.triangles.length}:${h.toString(16)}`;
}

/** IndexedDB store of baked fields, or null where IndexedDB is missing (private windows). */
async function openStore(): Promise<IDBDatabase | null> {
  try {
    if (typeof indexedDB === 'undefined') return null;
    return await new Promise<IDBDatabase | null>((resolve) => {
      const req = indexedDB.open('haa-field-bakes', 1);
      req.onupgradeneeded = () => req.result.createObjectStore('fields');
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
      const req = f(db.transaction('fields', mode).objectStore('fields'));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(undefined);
    } catch {
      resolve(undefined);
    }
  });
}

/** Modules whose lookups ran once (the engine compiles a wasm function at its first call). */
const warmed = new WeakSet<WebAssembly.Module>();

interface Level {
  wasm: FieldWasm;
  worker: Worker;
  grid: FieldGrid;
  left: number;
  bakeMs: number;
  casts: number;
  floodCell: string;
  /** A flood asked of the worker and not answered yet. */
  floodPending: boolean;
  baked: number;
  bytes: number;
  /** Cells baked since the last save. */
  unsaved: boolean;
  /** This bake's number: a pooled worker's answers to an earlier bake are dropped. */
  gen: number;
  /** False until the main-thread copy has the grid (a pooled fine level sets it up on its first cells). */
  setUp: boolean;
  pendingGeometry: SfxGeometry | null;
}

/** A field instance and its worker, reused from one fine bake to the next. */
interface Slot {
  wasm: FieldWasm;
  worker: Worker;
}

/** The DirectPath the field returns, with what its reverb needs (the opening). */
interface FieldDirectPath extends DirectPath {
  aperture: number;
  aperturePoint: Vec3;
  atListener: number;
  atSource: number;
}

export class AcousticField implements SfxFieldProvider {
  private readonly o: AcousticFieldOptions;
  private readonly coarse: Level;
  private fine: Level | null = null;
  private pendingFine: Level | null = null;
  private fineCentre: Vec3 | null = null;
  private fineAt = -Infinity;
  private fineRebakes = 0;
  private lastFocus: Vec3 | null = null;
  private listenerPos: Vec3 = v3(0, 0, 0);
  private occupancy: Array<{ p: Float32Array; at: number }> = [];
  private floodMs = 0;
  private floodReached = 0;
  private floodId = 0;
  private lookups = 0;
  private lookupMs = 0;
  private mainMaxMs = 0;
  private readonly apertures = new Map<string, { aperture: number; aperturePoint: Vec3; atListener: number; atSource: number }>();
  private readonly aperturePending = new Set<string>();
  private lastAperture: { aperture: number; aperturePoint: Vec3; atListener: number; atSource: number } | null = null;
  private apertureMs = 0;
  private mainMeanMs = 0;
  private readonly fineOpts: Required<FineOptions> | null;
  /** Called after each batch of baked cells (debug views). */
  onBaked: (() => void) | null = null;
  private key: string;
  private readonly db: Promise<IDBDatabase | null>;
  private restored = 0;
  private saved = 0;
  private gen = 0;
  /** The static level as last read (constructor, `invalidate`). */
  private staticGeometry: SfxGeometry;
  /** Two fine slots: one serves while the other bakes the next box. */
  private readonly finePool: Slot[] = [];

  constructor(o: AcousticFieldOptions) {
    this.o = o;
    this.fineOpts = o.fine === false || !o.fine ? (o.fine === false ? null : { cell: 0.5, half: [6, 2, 6], moveBy: 2, every: 3 }) : { cell: 0.5, half: [6, 2, 6], moveBy: 2, every: 3, ...o.fine };
    const geometry = o.geometry();
    this.staticGeometry = geometry;
    this.key = fieldKey(geometry, o.grid);
    this.db = o.persist === false ? Promise.resolve(null) : openStore();
    // A bake of this level kept from an earlier session: the worker starts from it.
    const restore = this.db.then(async (db) => {
      const v = db ? await idb(db, 'readonly', (st) => st.get(this.key)) : undefined;
      return v instanceof Uint8Array ? v : v instanceof ArrayBuffer ? new Uint8Array(v) : null;
    });
    this.coarse = this.level(o.grid, geometry, v3(0, 0, 0), 200, restore);
    // The fine slots are made now: a new instance, worker and BVH in the frame of a re-bake took up
    // to 1.8 ms of the main thread.
    if (this.fineOpts) for (let k = 0; k < 2; k++) this.finePool.push({ wasm: new FieldWasm(o.module), worker: o.worker() });
  }

  /** Keep the coarse bake (once complete, and again after a re-bake). */
  private async save(): Promise<void> {
    const db = await this.db;
    if (!db) return;
    this.coarse.unsaved = false;
    const bytes = this.coarse.wasm.pack();
    await idb(db, 'readwrite', (st) => st.put(bytes, this.key));
    this.saved = bytes.length;
  }

  private level(grid: FieldGrid, geometry: SfxGeometry, focus: Vec3, slice = 200, restore?: Promise<Uint8Array | null>, slot?: Slot): Level {
    const wasm = slot?.wasm ?? new FieldWasm(this.o.module);
    if (!warmed.has(this.o.module)) {
      warmed.add(this.o.module);
      wasm.warmUp();
    }
    if (!slot) {
      wasm.setScene(geometry, false);
      wasm.setGrid(grid);
    }
    const worker = slot?.worker ?? this.o.worker();
    const lv: Level = {
      wasm, worker, grid, left: grid.n[0] * grid.n[1] * grid.n[2], bakeMs: 0, casts: 0, floodCell: '', floodPending: false, baked: 0, bytes: 0, unsaved: false,
      gen: ++this.gen, setUp: !slot, pendingGeometry: slot ? geometry : null,
    };
    worker.onmessage = (e: MessageEvent<FieldWorkerOut>) => {
      const m = e.data;
      if ('gen' in m && m.gen !== lv.gen) return;
      if (m.t === 'cells') {
        if (!lv.setUp) {
          // The pooled copy takes the new grid here, in the worker's answer, not in the frame that asked.
          const t0 = performance.now();
          wasm.setScene(lv.pendingGeometry!, false);
          wasm.setGrid(grid);
          lv.setUp = true;
          lv.pendingGeometry = null;
          this.main(performance.now() - t0);
        }
        lv.wasm.ingest(m.records);
        lv.left = m.left;
        lv.bakeMs += m.ms;
        lv.casts += m.casts;
        lv.baked = m.baked;
        lv.bytes = m.bytes;
        lv.floodCell = '';
        if (m.records.length > 0) lv.unsaved = true;
        if (lv === this.coarse && lv.left === 0 && lv.unsaved) void this.save();
        this.onBaked?.();
      } else if (m.t === 'dirty') {
        lv.wasm.dirty(m.lo, m.hi);
        lv.floodCell = '';
      } else if (m.t === 'aperture') {
        this.apertures.set(m.key, { aperture: m.aperture, aperturePoint: m.point, atListener: m.atListener, atSource: m.atSource });
        this.aperturePending.delete(m.key);
        this.apertureMs = m.ms;
      } else if (m.t === 'flood') {
        if (!lv.setUp) {
          lv.floodPending = false;
          lv.floodCell = '';
          return;
        }
        // The worker flooded; the main thread only copies the arrays in (about 0.1 ms).
        const t0 = performance.now();
        lv.wasm.floodImport(m.data);
        lv.floodPending = false;
        this.floodMs = m.ms;
        this.floodReached = m.reached;
        this.main(performance.now() - t0);
      }
    };
    const start = (packed: Uint8Array | null): void => {
      if (packed) {
        this.restored = wasm.unpack(packed);
        lv.floodCell = '';
      }
      const init: FieldWorkerIn = { t: 'init', module: this.o.module, geometry, grid, focus: this.lastFocus ?? focus, slice, packed: packed ?? undefined, gen: lv.gen };
      worker.postMessage(init);
    };
    if (restore) void restore.then(start, () => start(null));
    else start(null);
    return lv;
  }

  /** Static geometry changed inside the box (a door, a roof): those coarse cells are re-baked. */
  invalidate(lo: Vec3, hi: Vec3): void {
    const geometry = this.o.geometry();
    this.staticGeometry = geometry;
    this.key = fieldKey(geometry, this.o.grid);
    this.coarse.unsaved = true;
    this.coarse.wasm.setScene(geometry);
    this.coarse.wasm.dirty(lo, hi);
    this.coarse.worker.postMessage({ t: 'geometry', geometry, lo, hi } satisfies FieldWorkerIn);
    this.apertures.clear();
    this.fineAt = -Infinity;
  }

  /**
   * Occupancy points (x, y, z flat, world): what a renderer saw (its depth buffer unprojected), or
   * samples of an SDF. They become small boxes in the next fine bake and age out after `occupancyLife`.
   */
  occupy(points: Float32Array, now = performance.now() / 1000): void {
    this.occupancy.push({ p: points, at: now });
  }

  private occupancyBoxes(now: number): Float32Array {
    const life = this.o.occupancyLife ?? 8;
    this.occupancy = this.occupancy.filter((x) => now - x.at < life);
    const cell = this.fineOpts?.cell ?? 0.5;
    const h = cell / 2;
    // One box per occupied fine cell (deduplicated).
    const seen = new Set<string>();
    const out: number[] = [];
    for (const { p } of this.occupancy) {
      for (let i = 0; i + 2 < p.length; i += 3) {
        const k = `${Math.floor(p[i] / cell)},${Math.floor(p[i + 1] / cell)},${Math.floor(p[i + 2] / cell)}`;
        if (seen.has(k)) continue;
        seen.add(k);
        const [x, y, z] = k.split(',').map((v) => (Number(v) + 0.5) * cell);
        out.push(x, y, z, h, h, h, 0, 0, 0, 1, 0.1, 9999);
      }
    }
    return new Float32Array(out);
  }

  get occupiedCells(): number {
    return this.occupancyBoxes(performance.now() / 1000).length / 12;
  }

  /** Account a stretch of main-thread field work. */
  private main(ms: number): void {
    this.mainMaxMs = Math.max(this.mainMaxMs, ms);
    this.mainMeanMs += (ms - this.mainMeanMs) * 0.05;
  }

  listener(p: Vec3): void {
    const t0 = performance.now();
    this.listenerPos = p;
    const now = performance.now() / 1000;
    if (!this.lastFocus || length(sub(p, this.lastFocus)) > this.o.grid.cell) {
      this.lastFocus = p;
      this.coarse.worker.postMessage({ t: 'focus', p } satisfies FieldWorkerIn);
    }
    const f = this.fineOpts;
    if (f && (!this.fineCentre || length(sub(p, this.fineCentre)) > f.moveBy || now - this.fineAt > f.every)) this.rebakeFine(p, now);
    this.floodIfMoved(this.coarse, p);
    if (this.fine) this.floodIfMoved(this.fine, p);
    this.main(performance.now() - t0);
  }

  private rebakeFine(p: Vec3, now: number): void {
    const f = this.fineOpts!;
    const n: [number, number, number] = [Math.round((2 * f.half[0]) / f.cell), Math.round((2 * f.half[1]) / f.cell), Math.round((2 * f.half[2]) / f.cell)];
    const origin = v3(Math.floor((p.x - f.half[0]) / f.cell) * f.cell, Math.max(0, Math.floor((p.y - 1.6 - 0.25) / f.cell) * f.cell), Math.floor((p.z - f.half[2]) / f.cell) * f.cell);
    const base = this.staticGeometry;
    const moving = this.o.movingGeometry?.();
    const occ = this.occupancyBoxes(now);
    // Only the boxes within reach of the fine box (its probes go 60 m, its lookups' rays a few): a
    // smaller scene to build, on the main thread too.
    const reach = Math.max(...f.half) + 20;
    const keep: number[] = [];
    for (const b of moving ? [base.boxes, moving.boxes] : [base.boxes]) {
      for (let i = 0; i + 11 < b.length; i += 12) {
        const ext = Math.max(Math.abs(b[i + 3]), Math.abs(b[i + 4]), Math.abs(b[i + 5])) * 1.75;
        if (Math.hypot(b[i] - p.x, b[i + 2] - p.z) - ext < reach) for (let k = 0; k < 12; k++) keep.push(b[i + k]);
      }
    }
    const boxes = new Float32Array(keep.length + occ.length);
    boxes.set(keep);
    boxes.set(occ, keep.length);
    let triangles = base.triangles;
    if (moving && moving.triangles.length > 0) {
      triangles = new Float32Array(base.triangles.length + moving.triangles.length);
      triangles.set(base.triangles);
      triangles.set(moving.triangles, base.triangles.length);
    }
    const geometry = { boxes, triangles };
    const grid: FieldGrid = { origin, cell: f.cell, n };
    // The next fine level bakes in the slot the current one does not use; the current one serves
    // until the next has all its cells.
    const slot = this.finePool.find((x) => x.wasm !== this.fine?.wasm)!;
    const next = this.level(grid, geometry, p, 400, undefined, slot);
    this.pendingFine = next;
    const old = this.fine;
    const swap = next.worker.onmessage;
    next.worker.onmessage = (e) => {
      swap?.call(next.worker, e);
      if (this.fine !== next && next.left === 0 && next.setUp) {
        this.fine = next;
        if (this.pendingFine === next) this.pendingFine = null;
        this.fineRebakes++;
      }
    };
    if (!old) this.fine = next;
    this.fineCentre = p;
    this.fineAt = now;
  }

  /** Ask the level's worker for a flood from the listener when it changed cell (one at a time). */
  private floodIfMoved(lv: Level, p: Vec3): void {
    const c = lv.grid.cell;
    const key = `${Math.floor(p.x / c)},${Math.floor(p.y / c)},${Math.floor(p.z / c)}`;
    if (key === lv.floodCell || lv.floodPending) return;
    lv.floodPending = true;
    lv.floodCell = key;
    lv.worker.postMessage({ t: 'flood', id: ++this.floodId, p, max: 40 } satisfies FieldWorkerIn);
  }

  private inFine(p: Vec3): boolean {
    const g = this.fine?.grid;
    if (!g || this.fine!.left > 0) return false;
    const m = g.cell;
    return p.x > g.origin.x + m && p.x < g.origin.x + g.n[0] * g.cell - m && p.y > g.origin.y && p.y < g.origin.y + g.n[1] * g.cell - m && p.z > g.origin.z + m && p.z < g.origin.z + g.n[2] * g.cell - m;
  }

  private sample(p: Vec3): FieldSample {
    const t0 = performance.now();
    const s = this.inFine(p) ? this.fine!.wasm.sample(p) : this.coarse.wasm.sample(p);
    const r = s.weight > 0.3 ? s : this.coarse.wasm.sample(p);
    this.lookups++;
    this.lookupMs += performance.now() - t0;
    return r;
  }

  ready(p: Vec3): boolean {
    return this.coarse.wasm.sample(p).weight > 0.3 || (this.inFine(p) && this.fine!.wasm.sample(p).weight > 0.3);
  }

  path(s: Vec3, l: Vec3, _extent: number): DirectPath {
    const t0 = performance.now();
    const fine = this.inFine(s) && this.inFine(l);
    // The opening between two spaces is measured only when the two probes differ (decay or open
    // share): the scan costs ~40-100 rays along the path.
    const a = this.sample(s);
    const b = this.sample(l);
    const differ = Math.abs(Math.log(Math.max(0.05, a.rt60) / Math.max(0.05, b.rt60))) > Math.log(1.4) || Math.abs(a.escape - b.escape) > 0.15;
    const lv = fine ? this.fine!.wasm : this.coarse.wasm;
    let p: FieldPath = lv.path(s, l, false);
    if (fine && !p.found) p = this.coarse.wasm.path(s, l, false);
    if (differ && p.found) {
      // The opening scan (~100-150 rays) runs in the coarse worker, once per pair of coarse cells
      // (the opening does not move); until it answers, the previous pair's answer or none.
      const c = this.o.grid.cell;
      const key = `${Math.floor(s.x / c)},${Math.floor(s.y / c)},${Math.floor(s.z / c)}|${Math.floor(l.x / c)},${Math.floor(l.y / c)},${Math.floor(l.z / c)}`;
      const ap = this.apertures.get(key);
      if (ap) {
        p = { ...p, ...ap };
        this.lastAperture = ap;
      } else {
        if (!this.aperturePending.has(key)) {
          if (this.apertures.size > 512) this.apertures.clear();
          this.aperturePending.add(key);
          this.coarse.worker.postMessage({ t: 'aperture', key, s, l } satisfies FieldWorkerIn);
        }
        if (this.lastAperture) p = { ...p, ...this.lastAperture };
      }
    }
    this.lookups++;
    const ms = performance.now() - t0;
    this.lookupMs += ms;
    this.main(ms);
    const extra = { aperture: p.aperture, aperturePoint: p.aperturePoint, atListener: p.atListener, atSource: p.atSource };
    if (!p.found) return { visible: 0, excess: 0, via: s, length: length(sub(s, l)), found: false, ...extra } as FieldDirectPath;
    return { visible: p.visible ? 1 : 0, excess: p.visible ? 0 : p.excess, via: p.visible ? s : p.via, length: p.length, found: true, ...extra } as FieldDirectPath;
  }

  /**
   * The diffuse field of this source at this listener from the two cells' traced fields (each cell
   * baked the field a sound made there returns to it, with the per-voice rays' tracer: 48 rays,
   * 6 bounces, next-event estimation back to the centre).
   * - One space: sqrt(E_s·E_l). With one diffuse field per space, a point's own field is k·c² (c its
   *   coupling to the field, the same both ways by reciprocity) and the field from s at l is k·c_s·c_l.
   *   The decay: the two ends' weighted by their energy. Arrival direction and onset: the listener cell's.
   * - Two spaces joined by an opening (the narrowest cross-section on the path under half of the
   *   wider end's and under 16 m²): the source's room radiates through the opening of area A as a
   *   Lambertian source, E_s·A/(4π r²) at the listener r from it on its axis; plus the listener's room
   *   excited through it, E_l·A/(4π r²) from the source. From the opening's side. The decay is the
   *   rooms' own, weighted by their share (coupled rooms: the late slope is the slower room's).
   */
  reverb(s: Vec3, l: Vec3, path: DirectPath): VoiceReverb {
    const t0 = performance.now();
    const a = this.sample(s);
    const b = this.sample(l);
    const ea = energyOf(a);
    const eb = energyOf(b);
    const fp = path as Partial<FieldDirectPath>;
    const area = fp.aperture ?? Infinity;
    // Narrower than half the wider end: a listener standing in the door still hears the room through it.
    const opening = Number.isFinite(area) && area < OPENING_MAX_M2 && area < 0.5 * Math.max(fp.atListener ?? 0, fp.atSource ?? 0);
    let energy: number;
    let rt60: number;
    let direction: Vec3;
    let predelay: number;
    const lnRt = (x: FieldSample): number => Math.log(Math.max(0.05, x.rt60));
    if (!opening) {
      energy = Math.sqrt(ea * eb);
      // The more reverberant end sets the decay (coupled spaces: the late slope is the slower one's).
      const w = ea / Math.max(1e-12, ea + eb);
      rt60 = Math.exp(w * lnRt(a) + (1 - w) * lnRt(b));
      direction = b.dir;
      predelay = b.predelay;
    } else {
      const at = fp.aperturePoint ?? path.via;
      const rl = Math.max(1, length(sub(l, at)));
      const rs = Math.max(1, length(sub(s, at)));
      const fromSource = ea * Math.min(0.5, area / (4 * Math.PI * rl * rl));
      const own = eb * Math.min(0.5, area / (4 * Math.PI * rs * rs));
      energy = fromSource + own;
      const w = fromSource / Math.max(1e-12, energy);
      rt60 = Math.exp(w * lnRt(a) + (1 - w) * lnRt(b));
      direction = add(scale(normalize(sub(at, l)), (0.3 + 0.6 * w) * w), scale(b.dir, 1 - w));
      predelay = w * (a.predelay + Math.max(0, rl - 1) / SPEED_OF_SOUND_M_S) + (1 - w) * b.predelay;
    }
    const dl = length(direction);
    if (dl > 1) direction = scale(direction, 1 / dl);
    this.main(performance.now() - t0);
    return {
      energy,
      rt60,
      direction,
      predelay: Math.min(0.1, Math.max(0.002, predelay)),
      absorption: b.absorption,
      hits: Math.round((a.weight + b.weight) * 100),
    };
  }

  /**
   * Early reflections from the planes stored in the source's cell (the nearest distinct surfaces its
   * 64 probes hit: oblique walls, the faces of the cubes): each mirrors the source; the listener must
   * be on the air side, and two exact rays must see the bounce point (a plane is infinite, its
   * surface is not). The 4 shortest are kept, so the pattern follows the room and the clutter.
   */
  reflections(s: Vec3, l: Vec3): Reflection[] {
    const t0 = performance.now();
    const lv = this.inFine(s) ? this.fine!.wasm : this.coarse.wasm;
    const planes = lv.planes(s);
    const direct = length(sub(l, s));
    const out: Reflection[] = [];
    planes.forEach((pl, j) => {
      const ds = dot(pl.n, s) - pl.offset;
      const dl = dot(pl.n, l) - pl.offset;
      if (ds <= 0.02 || dl <= 0.02) return;
      const image = sub(s, scale(pl.n, 2 * ds));
      const pathLen = length(sub(l, image));
      const delay = (pathLen - direct) / SPEED_OF_SOUND_M_S;
      if (delay < 0.0015) return;
      const t = ds / (ds + dl);
      const point = add(image, scale(sub(l, image), t));
      const off = scale(pl.n, 0.05);
      if (!lv.clear(add(point, off), l) || !lv.clear(add(point, off), s)) return;
      out.push({
        image, point, path: pathLen, delay,
        gain: Math.sqrt(Math.max(0, (1 - pl.absorption) * 0.65)), absorption: pl.absorption, id: j,
        key: `${pl.n.x.toFixed(2)},${pl.n.y.toFixed(2)},${pl.n.z.toFixed(2)},${pl.offset.toFixed(1)}`, weight: 1,
      });
    });
    out.sort((x, y) => x.path - y.path);
    this.main(performance.now() - t0);
    return out.slice(0, 4);
  }

  /** Debug view: per baked air cell of both levels x, y, z, escape, rt60, flooded distance, energy (dB), plane count. */
  debugCells(): { coarse: Float32Array; fine: Float32Array | null } {
    return { coarse: this.coarse.wasm.debug(), fine: this.fine && this.fine.left === 0 ? this.fine.wasm.debug() : null };
  }

  /** Reset the main-thread maximum (measurements). */
  resetMain(): void {
    this.mainMaxMs = 0;
  }

  stats(): FieldStats {
    const st = this.coarse.wasm.stats();
    return {
      coarse: { baked: this.coarse.baked, left: this.coarse.left, bytes: this.coarse.bytes, bakeMs: this.coarse.bakeMs, casts: this.coarse.casts, reached: st.reached, restored: this.restored, saved: this.saved },
      fine: this.fine ? { baked: this.fine.baked, left: this.fine.left, bytes: this.fine.bytes, bakeMs: this.fine.bakeMs, rebakes: this.fineRebakes } : null,
      floodMs: this.floodMs,
      floodReached: this.floodReached,
      apertureMs: this.apertureMs,
      lookups: this.lookups,
      lookupMs: this.lookupMs,
      mainMaxMs: this.mainMaxMs,
      mainMeanMs: this.mainMeanMs,
      occupancy: this.occupancy.length,
    };
  }

  dispose(): void {
    this.coarse.worker.terminate();
    for (const x of this.finePool) x.worker.terminate();
  }
}
