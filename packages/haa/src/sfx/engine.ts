// The sound-effect engine: materials, presets, a voice pool with a CPU budget, and 3D placement
// from geometry. Game code (or a physics adapter) calls `handle(event)` or `play(...)`; once per
// frame it calls `update(listener)`. Browser-only (Web Audio + AudioWorklet).
import { HaaEngine, patchBytes } from '../engine.ts';
import type { HaaInput, HaaPatch, VoiceInfo } from '../types.ts';
import type { Occlusion, RayCast, RaySegment, Reflection, RoomEstimate, Vec3 } from './acoustics.ts';
import { earlyReflections, estimateRoom, length, occlusion, occlusionLevels, sub, v3 } from './acoustics.ts';
import type { InteractionKind, SfxEvent, SfxInputs, SoundMaterial, SoundMaterialSpec } from './contract.ts';
import { defineMaterial, fallbackGain, fullInputs, isContinuous, resolveInteraction, SFX_INPUTS } from './contract.ts';
import { createSfxPool } from './pool.ts';
import type { SfxPool } from './pool.ts';
import type { SfxPoolOutMsg } from './pool-protocol.ts';
import { REF_DISTANCE, ReverbBus, setListener, VoiceChain } from './spatial3d.ts';

export interface SfxEngineOptions {
  /** An AudioContext, or an OfflineAudioContext for renders and benchmarks. */
  ctx: BaseAudioContext;
  module: WebAssembly.Module;
  /** `@etcito/haa/worklet` URL. */
  processorUrl: string | URL;
  /** Returns a preset by name (e.g. fetch `patches/sfx/<name>.json`), or null if unknown. */
  loadPreset(name: string): Promise<HaaPatch | string | Uint8Array | null>;
  /** Voice slots of the pool. Default 16. */
  slots?: number;
  /** Fraction of the audio thread the voices may use, from each preset's measured cost. Default 0.6. */
  cpuBudget?: number;
  /** Ray cast for acoustics (walls, floors): occlusion, reflections, room. Without it, sounds are only panned. */
  raycast?: RayCast;
  /** Where the mix goes; the context destination by default. */
  output?: AudioNode;
  /** Occlusion strength, 0..2: 0 off, 1 the default (about -10 dB and 1.2 kHz behind a wall). */
  occlusionStrength?: number;
  /** Reverb amount, 0..2: 0 dry, 1 the level estimated from the room. */
  reverbAmount?: number;
}

export interface PresetRecord {
  name: string;
  bytes: Uint8Array;
  info: VoiceInfo;
  /** Fraction of real time one voice of it takes (render measured on this thread, plus its 3D chain). */
  cost: number;
  readsEnergy: boolean;
  playable: boolean;
}

export interface SfxListener {
  position: Vec3;
  forward: Vec3;
  up: Vec3;
}

export interface PlayOptions {
  gain?: number;
  /** Override the priority (default: energy × distance). */
  priority?: number;
  /** Label for the debug log. */
  label?: string;
}

/** A running continuous voice started with `loop`. */
export interface SfxLoop {
  readonly alive: boolean;
  set(inputs: Partial<SfxInputs>): void;
  move(position: Vec3): void;
  setGain(gain: number): void;
  stop(fadeS?: number): void;
}

export interface SfxLogEntry {
  time: number;
  label: string;
  preset: string;
  kind: InteractionKind | 'play' | 'loop';
  inputs: SfxInputs;
  slot: number;
  /** 'started' | 'dropped' | 'culled' | 'stole <preset>' | 'missing' | 'not playable' */
  result: string;
  distance: number;
}

export interface SfxStats {
  slots: number;
  busy: number;
  /** Estimated audio-thread load of the busy voices (fraction of real time). */
  load: number;
  started: number;
  dropped: number;
  stolen: number;
  /** One-shots not started because they would be inaudible (below −60 dB). */
  culled: number;
  /** Patch loads done on the audio thread, and the slowest (ms). */
  loads: number;
  maxLoadMs: number;
  room: RoomEstimate | null;
}

interface Slot {
  index: number;
  chain: VoiceChain;
  busy: boolean;
  id: number;
  preset: string;
  oneShot: boolean;
  priority: number;
  started: number;
  position: Vec3;
  level: number;
  energy: number;
  occ: Occlusion;
  reflections: Reflection[];
  nextGeo: number;
  key: string | null;
  stopping: boolean;
  cost: number;
}

export interface SfxEngine {
  readonly ctx: BaseAudioContext;
  /** Final mix (post limiter): tap it for meters. */
  readonly master: GainNode;
  /** Per-ear analysers on the final mix. */
  readonly analysers: { left: AnalyserNode; right: AnalyserNode };
  readonly materials: Map<string, SoundMaterial>;
  defineMaterial(spec: SoundMaterialSpec): SoundMaterial;
  /** Register or replace a preset's patch (a live graph edit): running loops of it hot-swap. */
  setPreset(name: string, patch: HaaPatch | string | Uint8Array): PresetRecord;
  preset(name: string): Promise<PresetRecord | null>;
  handle(event: SfxEvent): void;
  play(preset: string, position: Vec3, inputs?: Partial<SfxInputs>, o?: PlayOptions): void;
  loop(preset: string, position: Vec3, inputs?: Partial<SfxInputs>, o?: PlayOptions): SfxLoop;
  update(listener: SfxListener): void;
  setRaycast(cast: RayCast | undefined): void;
  /** Occlusion strength, 0..2 (default 1); the running voices follow at their next geometry update. */
  setOcclusionStrength(strength: number): void;
  /** Reverb amount, 0..2 (default 1): scales the wet level estimated from the room. */
  setReverbAmount(amount: number): void;
  stats(): SfxStats;
  /** Rays cast during the last updates (room, reflections, occlusion), for a debug overlay. */
  rays(): RaySegment[];
  onLog(cb: (e: SfxLogEntry) => void): () => void;
  stopAll(): void;
  dispose(): void;
}

const ROOM_EVERY_S = 0.25;
const GEO_EVERY_S = 0.2;
const CONTINUOUS_FADE_S = 0.08;
/** Audio-thread cost of one voice's 3D chain (HRTF panner, 4 taps), measured in Chrome: ≈ 0.6 % of real time. */
const CHAIN_COST = 0.006;
/** A one-shot estimated below this level at the listener (−60 dB) is not started. */
const AUDIBLE = 1e-3;

/** Map a 0..1 contract value onto the range the preset declares for that input. */
export function toDeclared(input: HaaInput | undefined, v: number): number {
  if (!input || input.kind !== 'continuous') return v;
  const { min, max } = input;
  if (input.curve === 'log' && min > 0 && max > 0) return min * (max / min) ** v;
  return min + (max - min) * v;
}

export async function createSfxEngine(o: SfxEngineOptions): Promise<SfxEngine> {
  const ctx = o.ctx;
  const haa = HaaEngine.fromModule(o.module);
  const pool: SfxPool = await createSfxPool(ctx, { module: o.module, processorUrl: o.processorUrl, slots: o.slots ?? 16 });
  const budget = o.cpuBudget ?? 0.6;
  let raycast = o.raycast;
  let occlusionStrength = o.occlusionStrength ?? 1;

  // Mix: voices -> dry + reverb -> master -> limiter -> output, with per-ear analysers.
  const dry = ctx.createGain();
  const mix = ctx.createGain();
  dry.connect(mix);
  const reverb = new ReverbBus(ctx, haa, mix);
  if (o.reverbAmount !== undefined) reverb.setAmount(o.reverbAmount);
  const limiter = ctx.createDynamicsCompressor();
  limiter.threshold.value = -3;
  limiter.knee.value = 0;
  limiter.ratio.value = 20;
  limiter.attack.value = 0.002;
  limiter.release.value = 0.12;
  const master = ctx.createGain();
  mix.connect(limiter);
  limiter.connect(master);
  master.connect(o.output ?? ctx.destination);
  const splitter = ctx.createChannelSplitter(2);
  master.connect(splitter);
  const left = ctx.createAnalyser();
  const right = ctx.createAnalyser();
  left.fftSize = right.fftSize = 2048;
  splitter.connect(left, 0);
  splitter.connect(right, 1);

  const slots: Slot[] = [];
  for (let i = 0; i < pool.slots; i++) {
    const chain = new VoiceChain(ctx, dry, reverb.input);
    slots.push({
      index: i, chain, busy: false, id: 0, preset: '', oneShot: true, priority: 0, started: 0,
      position: v3(0, 0, 0), level: 0, energy: 0, occ: occlusionLevels(0), reflections: [], nextGeo: 0,
      key: null, stopping: false, cost: 0,
    });
  }

  const materials = new Map<string, SoundMaterial>();
  const presets = new Map<string, PresetRecord>();
  const pending = new Map<string, Promise<PresetRecord | null>>();
  const continuous = new Map<string, Slot>();
  const logs = new Set<(e: SfxLogEntry) => void>();
  let listener: SfxListener = { position: v3(0, 1.6, 0), forward: v3(0, 0, -1), up: v3(0, 1, 0) };
  let room: RoomEstimate | null = null;
  let nextRoom = 0;
  let roomRays: RaySegment[] = [];
  const voiceRays = new Map<number, RaySegment[]>();
  let nextId = 1;
  const counters = { started: 0, dropped: 0, stolen: 0, culled: 0, loads: 0, maxLoadMs: 0 };

  pool.onMessage((msg: SfxPoolOutMsg) => {
    if (msg.t === 'ended') {
      const s = slots[msg.slot];
      if (s && s.id === msg.id) release(s);
    } else if (msg.t === 'started' && msg.loadMs > 0) {
      counters.loads++;
      counters.maxLoadMs = Math.max(counters.maxLoadMs, msg.loadMs);
    }
  });

  function release(s: Slot): void {
    s.chain.mute();
    // Disconnect after the fade so idle chains cost nothing on the audio thread; a slot reused
    // meanwhile keeps its connection.
    setTimeout(() => {
      if (!s.busy) s.chain.detach();
    }, 60);
    s.busy = false;
    s.stopping = false;
    if (s.key && continuous.get(s.key) === s) continuous.delete(s.key);
    s.key = null;
    voiceRays.delete(s.index);
  }

  function measure(bytes: Uint8Array): { info: VoiceInfo; cost: number; playable: boolean } {
    const voice = haa.createVoice(ctx.sampleRate, 3);
    try {
      const { playable } = voice.load(bytes);
      const info = voice.info();
      if (!playable) return { info, cost: 0, playable };
      const blocks = Math.round((0.25 * ctx.sampleRate) / 128);
      let ms = 0;
      for (let pass = 0; pass < 2; pass++) {
        voice.restart();
        const t0 = performance.now();
        for (let b = 0; b < blocks; b++) voice.process(128);
        ms = performance.now() - t0;
      }
      return { info, cost: ms / 250, playable };
    } finally {
      voice.free();
    }
  }

  function setPreset(name: string, patch: HaaPatch | string | Uint8Array): PresetRecord {
    const bytes = patchBytes(patch);
    const { info, cost, playable } = measure(bytes);
    const rec: PresetRecord = { name, bytes, info, cost: cost + CHAIN_COST, playable, readsEnergy: info.inputs.some((i) => i.name === 'energy') };
    presets.set(name, rec);
    pool.preset(name, bytes);
    return rec;
  }

  function preset(name: string): Promise<PresetRecord | null> {
    const have = presets.get(name);
    if (have) return Promise.resolve(have);
    let p = pending.get(name);
    if (!p) {
      p = o.loadPreset(name).then(
        (patch) => (patch ? setPreset(name, patch) : null),
        () => null,
      );
      pending.set(name, p);
      void p.then(() => pending.delete(name));
    }
    return p;
  }

  function defineMat(spec: SoundMaterialSpec): SoundMaterial {
    const m = defineMaterial(spec);
    materials.set(m.name, m);
    for (const slot of Object.values(m.sounds)) if (slot) void preset(slot.preset);
    return m;
  }

  function log(e: Omit<SfxLogEntry, 'time'>): void {
    const entry = { ...e, time: ctx.currentTime };
    for (const cb of logs) {
      try {
        cb(entry);
      } catch {
        // A listener's failure is its own.
      }
    }
  }

  const dist = (p: Vec3): number => length(sub(p, listener.position));

  function declaredInputs(rec: PresetRecord, inputs: SfxInputs): Record<string, number> {
    const out: Record<string, number> = {};
    for (const name of SFX_INPUTS) {
      const decl = rec.info.inputs.find((i) => i.name === name);
      if (decl) out[name] = toDeclared(decl, inputs[name]);
    }
    return out;
  }

  function geometry(s: Slot, now: number, immediate: boolean): void {
    const d = dist(s.position);
    if (raycast && (immediate || now >= s.nextGeo)) {
      const rays: RaySegment[] = [];
      s.occ = occlusion(s.position, listener.position, raycast, 0.2, rays);
      s.reflections = earlyReflections(s.position, listener.position, raycast, { max: 4 }, rays);
      voiceRays.set(s.index, rays);
      s.nextGeo = now + GEO_EVERY_S * (0.8 + 0.4 * ((s.index * 0.618) % 1));
      s.chain.setReflections(s.reflections, immediate);
    }
    const occ = occlusionLevels(s.occ.blocked, occlusionStrength);
    if (!s.stopping) s.chain.setLevels(s.level, occ, d, room ? room.wet : 0, immediate);
    // A preset that declares `_dist` gets the distance in metres (a far explosion rumbles more).
    if (presets.get(s.preset)?.info.inputs.some((i) => i.name === '_dist')) pool.set(s.index, s.id, { _dist: d });
  }

  /** Find a slot for a voice of `rec` at `priority`: a free one, else steal a quieter one. */
  function allocate(rec: PresetRecord, priority: number): { slot: Slot | null; stole: string | null } {
    const busy = slots.filter((s) => s.busy);
    let load = busy.reduce((a, s) => a + s.cost, 0);
    const free = slots.filter((s) => !s.busy);
    const fits = (): boolean => load + rec.cost <= budget;
    if (free.length && fits()) {
      // Prefer a slot that already holds this preset: no patch load on the audio thread.
      const same = free.find((s) => s.preset === rec.name);
      return { slot: same ?? free.sort((a, b) => a.started - b.started)[0], stole: null };
    }
    const victims = busy
      .filter((s) => !s.stopping)
      .sort((a, b) => a.priority - b.priority || a.started - b.started);
    let stole: string | null = null;
    let picked: Slot | null = free[0] ?? null;
    for (const v of victims) {
      if (v.priority >= priority) break;
      stole = v.preset;
      load -= v.cost;
      if (!picked) {
        picked = v;
      } else {
        pool.stop(v.index, v.id);
        release(v);
      }
      if (fits()) break;
    }
    if (!picked || !fits()) return { slot: null, stole: null };
    if (picked.busy) {
      pool.stop(picked.index, picked.id);
      release(picked);
    }
    return { slot: picked, stole };
  }

  function start(
    rec: PresetRecord,
    position: Vec3,
    inputs: SfxInputs,
    oneShot: boolean,
    gain: number,
    priority: number | undefined,
    label: string,
    kind: SfxLogEntry['kind'],
    key: string | null,
  ): Slot | null {
    const d = dist(position);
    const prio = priority ?? (inputs.energy + 0.05) / (1 + d / 8) + (oneShot ? 0 : 0.25);
    const base = { label, preset: rec.name, kind, inputs, distance: d };
    if (!rec.playable) {
      log({ ...base, slot: -1, result: 'not playable' });
      return null;
    }
    // Loudness proxy: the energy on its 40 dB axis, the level trim, 1/r beyond the reference distance.
    if (oneShot && fallbackGain(inputs.energy) * gain * Math.min(1, REF_DISTANCE / Math.max(d, 1e-3)) < AUDIBLE) {
      counters.culled++;
      log({ ...base, slot: -1, result: 'culled' });
      return null;
    }
    const { slot, stole } = allocate(rec, prio);
    if (!slot) {
      counters.dropped++;
      log({ ...base, slot: -1, result: 'dropped' });
      return null;
    }
    if (stole) counters.stolen++;
    counters.started++;
    const id = nextId++;
    Object.assign(slot, {
      busy: true, id, preset: rec.name, oneShot, priority: prio, started: ctx.currentTime, position,
      level: gain * (rec.readsEnergy ? 1 : fallbackGain(inputs.energy)), energy: inputs.energy,
      key, stopping: false, cost: rec.cost,
    });
    const maxSeconds = rec.info.mode === 'timeline' ? rec.info.duration : 3;
    slot.chain.attach(pool.node, slot.index);
    slot.chain.setPosition(position, true);
    pool.start(slot.index, id, rec.name, declaredInputs(rec, inputs), oneShot, maxSeconds);
    geometry(slot, ctx.currentTime, true);
    log({ ...base, slot: slot.index, result: stole ? `stole ${stole}` : 'started' });
    return slot;
  }

  function stopSlot(s: Slot, fadeS = CONTINUOUS_FADE_S): void {
    if (!s.busy || s.stopping) return;
    s.stopping = true;
    s.chain.mute(fadeS);
    const id = s.id;
    setTimeout(() => {
      if (s.id === id && s.busy) {
        pool.stop(s.index, id);
        release(s);
      }
    }, fadeS * 1000 + 40);
  }

  function handle(ev: SfxEvent): void {
    const resolved = resolveInteraction(ev.kind, ev.striker, ev.surface, ev.inputs, ev.surfaceInputs);
    const label = `${ev.striker.name} → ${ev.surface.name}`;
    if (isContinuous(ev.kind)) {
      const baseKey = ev.key ?? `${ev.kind}:${label}`;
      if (ev.phase === 'end') {
        for (const [k, s] of continuous) if (k.startsWith(baseKey + '|')) stopSlot(s);
        return;
      }
      for (const r of resolved) {
        const key = `${baseKey}|${r.slot.preset}`;
        const running = continuous.get(key);
        const rec = presets.get(r.slot.preset);
        if (running && running.busy && !running.stopping && rec) {
          running.position = ev.position;
          running.chain.setPosition(ev.position, false);
          running.level = r.gain * (rec.readsEnergy ? 1 : fallbackGain(r.inputs.energy));
          running.energy = r.inputs.energy;
          pool.set(running.index, running.id, declaredInputs(rec, r.inputs));
          continue;
        }
        if (!rec) {
          void preset(r.slot.preset);
          log({ label, preset: r.slot.preset, kind: ev.kind, inputs: r.inputs, slot: -1, result: 'missing', distance: dist(ev.position) });
          continue;
        }
        const s = start(rec, ev.position, r.inputs, false, r.gain, undefined, label, ev.kind, key);
        if (s) continuous.set(key, s);
      }
      return;
    }
    for (const r of resolved) {
      const rec = presets.get(r.slot.preset);
      if (!rec) {
        void preset(r.slot.preset);
        log({ label, preset: r.slot.preset, kind: ev.kind, inputs: r.inputs, slot: -1, result: 'missing', distance: dist(ev.position) });
        continue;
      }
      start(rec, ev.position, r.inputs, true, r.gain, undefined, label, ev.kind, null);
    }
  }

  function play(name: string, position: Vec3, inputs?: Partial<SfxInputs>, opt: PlayOptions = {}): void {
    const full = fullInputs(inputs);
    const rec = presets.get(name);
    if (rec) {
      start(rec, position, full, true, opt.gain ?? 1, opt.priority, opt.label ?? name, 'play', null);
      return;
    }
    void preset(name).then((r) => {
      if (r) start(r, position, full, true, opt.gain ?? 1, opt.priority, opt.label ?? name, 'play', null);
      else log({ label: opt.label ?? name, preset: name, kind: 'play', inputs: full, slot: -1, result: 'missing', distance: dist(position) });
    });
  }

  function loop(name: string, position: Vec3, inputs?: Partial<SfxInputs>, opt: PlayOptions = {}): SfxLoop {
    let slot: Slot | null = null;
    let id = 0;
    let stopped = false;
    let current = fullInputs(inputs);
    let gain = opt.gain ?? 1;
    let pos = position;
    const alive = (): boolean => !!slot && slot.busy && slot.id === id && !slot.stopping;
    void preset(name).then((rec) => {
      if (!rec || stopped) return;
      slot = start(rec, pos, current, false, gain, opt.priority, opt.label ?? name, 'loop', null);
      if (slot) id = slot.id;
    });
    return {
      get alive() {
        return alive();
      },
      set(next) {
        current = fullInputs({ ...current, ...next });
        const rec = presets.get(name);
        if (!alive() || !rec) return;
        slot!.energy = current.energy;
        slot!.level = gain * (rec.readsEnergy ? 1 : fallbackGain(current.energy));
        pool.set(slot!.index, id, declaredInputs(rec, current));
      },
      move(p) {
        pos = p;
        if (!alive()) return;
        slot!.position = p;
        slot!.chain.setPosition(p, false);
      },
      setGain(g) {
        gain = g;
        const rec = presets.get(name);
        if (alive() && rec) slot!.level = gain * (rec.readsEnergy ? 1 : fallbackGain(current.energy));
      },
      stop(fadeS) {
        stopped = true;
        if (alive()) stopSlot(slot!, fadeS);
      },
    };
  }

  function update(l: SfxListener): void {
    listener = l;
    setListener(ctx, l.position, l.forward, l.up);
    const now = ctx.currentTime;
    if (raycast && now >= nextRoom) {
      const rays: RaySegment[] = [];
      room = estimateRoom(l.position, raycast, {}, rays);
      roomRays = rays;
      reverb.setRoom(room);
      nextRoom = now + ROOM_EVERY_S;
    }
    for (const s of slots) {
      if (!s.busy) continue;
      s.priority = (s.energy + 0.05) / (1 + dist(s.position) / 8) + (s.oneShot ? 0 : 0.25);
      geometry(s, now, false);
    }
  }

  return {
    ctx,
    master,
    analysers: { left, right },
    materials,
    defineMaterial: defineMat,
    setPreset,
    preset,
    handle,
    play,
    loop,
    update,
    setRaycast(cast) {
      raycast = cast;
    },
    setOcclusionStrength(strength) {
      occlusionStrength = Number.isFinite(strength) ? Math.min(2, Math.max(0, strength)) : 1;
    },
    setReverbAmount(amount) {
      reverb.setAmount(amount);
    },
    stats() {
      const busy = slots.filter((s) => s.busy);
      return {
        slots: slots.length,
        busy: busy.length,
        load: busy.reduce((a, s) => a + s.cost, 0),
        ...counters,
        room,
      };
    },
    rays() {
      const out = [...roomRays];
      for (const r of voiceRays.values()) out.push(...r);
      return out;
    },
    onLog(cb) {
      logs.add(cb);
      return () => logs.delete(cb);
    },
    stopAll() {
      for (const s of slots) if (s.busy) stopSlot(s, 0.03);
    },
    dispose() {
      pool.dispose();
      for (const s of slots) s.chain.detach();
      master.disconnect();
    },
  };
}
