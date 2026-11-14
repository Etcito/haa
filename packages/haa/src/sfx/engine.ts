// The sound-effect engine: materials, presets, a voice pool with a CPU budget, and 3D placement
// from geometry. Game code (or a physics adapter) calls `handle(event)` or `play(...)`; once per
// frame it calls `update(listener)`. Browser-only (Web Audio + AudioWorklet).
import { HaaEngine, patchBytes } from '../engine.ts';
import type { HaaInput, HaaPatch, VoiceInfo } from '../types.ts';
import type { RayCast, RaySegment, Reflection, Vec3 } from './acoustics.ts';
import { add, cross, dot, earlyReflections, length, normalize, scale, SPEED_OF_SOUND_M_S, sub, v3, WET_MAX } from './acoustics.ts';
import type { DirectPath, ListenerTrace, SpaceEstimate, VoiceReverb } from './raytrace.ts';
import { diffractionLevels, directPath, sphereDirections, traceListener, voiceReverb } from './raytrace.ts';
import type { InteractionKind, SfxEvent, SfxInputs, SoundMaterial, SoundMaterialSpec } from './contract.ts';
import { defineMaterial, fallbackGain, fullInputs, isContinuous, resolveInteraction, SFX_INPUTS } from './contract.ts';
import { createSfxPool } from './pool.ts';
import type { SfxPool } from './pool.ts';
import type { SfxPoolOutMsg } from './pool-protocol.ts';
import type { ChainState } from './spatial3d.ts';
import { bankWeights, NEAR_MIN_DISTANCE, REF_DISTANCE, ReverbBank, setListener, TAPS_PER_VOICE, VoiceChain } from './spatial3d.ts';
import type { FaceSource, FaceSourceOptions, SoundFace } from './faces.ts';
import { createFaceSource } from './faces.ts';
import type { SwarmOptions } from './swarm.ts';
import { SwarmTracker } from './swarm.ts';

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
  /** Reverb amount, 0..2: 0 dry, 1 the level estimated from the room, 2 exaggerated. */
  reverbAmount?: number;
  /** Diffuse level relative to the physical estimate (dB): 0 is physical, default `WET_OFFSET_DB` (-7). */
  reverbLevelDb?: number;
  /** Swarms of small impacts (slots with a `cluster` preset): grouping options, or false to give every impact its own voice. */
  swarm?: SwarmOptions | false;
  /** Ray budget of the acoustics (docs/physics-sound.md): more rays, smoother and slower. */
  acoustics?: AcousticsOptions;
}

export interface AcousticsOptions {
  /** Rays traced from the listener. Default 256. */
  traceRays?: number;
  /** Bounces followed per ray. Default 6. */
  bounces?: number;
  /** Every n-th traced ray feeds each voice's diffuse field. Default 2. */
  reverbStride?: number;
  /** Listener trace period (s). Default 0.1. */
  traceEvery?: number;
  /** Geometry period of each voice (s). Default 0.15. */
  voiceEvery?: number;
}

export interface PresetRecord {
  name: string;
  bytes: Uint8Array;
  info: VoiceInfo;
  /** Fraction of real time one voice of it takes (render measured on this thread, plus its 3D chain). */
  cost: number;
  readsEnergy: boolean;
  playable: boolean;
  /**
   * A looping preset that reads `speed` (slide, roll): its RMS level at `SPEED_STEPS`, measured
   * once with the other inputs at their defaults. The engine starts no slide/roll voice that would
   * be inaudible at the listener and stops one that falls silent.
   */
  speedLevels: number[] | null;
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
  /** Radius (m) of a large source (rain, an explosion): it spreads around the listener up close. */
  extent?: number;
  /** The position is already the source's nearest point (a face emitter): the extent only spreads it. */
  nearest?: boolean;
}

/** A running continuous voice started with `loop`. */
export interface SfxLoop {
  readonly alive: boolean;
  set(inputs: Partial<SfxInputs>): void;
  move(position: Vec3): void;
  setGain(gain: number): void;
  /** Extra loss (dB at 250 Hz, 1 kHz, 4 kHz) of the direct sound: a face heard through its own solid. */
  setTransmission(db: readonly [number, number, number]): void;
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
  /** Voices not started because they would be inaudible (below −60 dB): one-shots, and slide/roll contacts. */
  culled: number;
  /** Impacts absorbed by a swarm's cluster voice instead of taking their own. */
  clustered: number;
  /** Patch loads done on the audio thread, and the slowest (ms). */
  loads: number;
  maxLoadMs: number;
  /** The listener's space, from the ray trace. */
  room: SfxRoom | null;
  /** Acoustics of each busy voice as it is heard now (smoothed values). */
  voices: SfxVoiceAcoustics[];
}

export interface SfxVoiceAcoustics {
  slot: number;
  preset: string;
  distance: number;
  /** Visible share of the source (direct path). */
  visible: number;
  /** Extra length of the path around the obstacles (m). */
  excess: number;
  /** Direct-path loss (dB) at 250 Hz, 1 kHz, 4 kHz. */
  directDb: [number, number, number];
  /** Diffuse field energy relative to the direct sound at 2 m (dB), its reverb time and pre-delay. */
  wetDb: number;
  rt60: number;
  predelay: number;
  /** Left/right of the diffuse field (-1..1). */
  pan: number;
  taps: number;
}

/** The listener's space, plus the diffuse level a footstep at the listener's feet gets (`wet`, amplitude at 2 m). */
export interface SfxRoom extends SpaceEstimate {
  wet: number;
  /** Reverb time of that footstep's diffuse field (s). */
  wetRt60: number;
}

/** Smoothed acoustic state of a voice: targets from the geometry, values glided toward them every frame. */
interface Acoustic {
  path: DirectPath | null;
  reverb: VoiceReverb | null;
  visible: number;
  excess: number;
  found: number;
  via: Vec3;
  wetDb: number;
  rtLog: number;
  rdir: Vec3;
  predelay: number;
  sendHz: number;
  taps: Array<{ key: string; refl: Reflection; w: number; target: number } | null>;
  primed: boolean;
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
  /** Radius of a large source (m), 0 for a point. */
  extent: number;
  /** The position is the nearest point of the source (face emitters). */
  nearest: boolean;
  /** Extra direct-path loss (dB per band), its smoothed value, and the fade-in of a continuous voice. */
  tx: [number, number, number];
  txs: [number, number, number];
  fadeIn: number;
  /** Bank weights last sent (which convolvers it feeds). */
  bankW: readonly number[];
  level: number;
  energy: number;
  ac: Acoustic;
  nextGeo: number;
  key: string | null;
  stopping: boolean;
  cost: number;
  /** Slide/roll: the last speed, and since when its estimated level has been inaudible (-1: audible). */
  speed: number;
  quietSince: number;
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
  /** Diffuse level relative to the physical estimate (dB, -30..+6): 0 is physical, default -7. */
  setReverbLevel(db: number): void;
  /**
   * A distributed source on the level's faces (rain): one emitter per exposed face, fed by the
   * face's flux, exposed area, material and body (docs/physics-sound.md, "Rain on faces").
   */
  faceSource(faces: SoundFace[], o: FaceSourceOptions): FaceSource;
  stats(): SfxStats;
  /** Rays cast during the last updates (room, reflections, occlusion), for a debug overlay. */
  rays(): RaySegment[];
  onLog(cb: (e: SfxLogEntry) => void): () => void;
  stopAll(): void;
  dispose(): void;
}

/** Rays from a source that find its reflecting planes. */
const SOURCE_DIRS = sphereDirections(64);
const WET_FLOOR_DB = -90;
// Glide time constants (s) of the per-frame smoothing: the geometry updates every 0.1-0.15 s and
// each value follows it continuously, so nothing jumps when a ray starts or stops hitting.
const TAU_DIRECT = 0.12;
const TAU_VIA = 0.15;
const TAU_WET = 0.2;
const TAU_RT = 0.35;
const TAU_TAP = 0.1;
/**
 * The diffuse field is computed physically (dB re the direct sound at 2 m), then mixed: lowered by
 * `WET_OFFSET_DB`, and above 0 dB it grows at half the rate. A bare wooden shed or a concrete hall
 * is 10 dB wetter than a footstep's direct sound at the feet (critical distance under half a metre):
 * true, and it turned the step into a wash. A constant offset keeps every contrast between places
 * (hall centre, in front of the door, the doorway, outside); the knee only tames the wettest rooms.
 * The former hard cap made every closed room the same.
 */
export const WET_OFFSET_DB = -7;
/** Largest reverb send amplitude (+12 dB re the direct sound at 2 m). */
export const SEND_MAX = 4;
/** Voices fade out over this when stolen (a spare slot takes the new one meanwhile). */
const STEAL_FADE_S = 0.06;
/** Slots kept free for stealing with a fade instead of a cut. */
const SPARE_SLOTS = 2;
/** A continuous voice (slide, roll, loop, swarm) fades in over this. */
const CONTINUOUS_FADE_IN_S = 0.05;

/** Diffuse level as mixed (dB) from the physical one: shifted by `offset` (0 = physical). */
export function mixedWetDb(db: number, offset = WET_OFFSET_DB): number {
  return db + offset;
}
const CONTINUOUS_FADE_S = 0.08;
/** Audio-thread cost of one voice's 3D chain (5 HRTF panners, 6 delays, 4 biquads, the bank sends), measured in Chrome: 14 noise loops render at 0.47 x real time, so about 3 %. */
const CHAIN_COST = 0.03;
/** A one-shot estimated below this level at the listener (−60 dB) is not started. */
const AUDIBLE = 1e-3;
/** A slide/roll voice (its measured RMS, not a proxy) estimated below −70 dB at the listener is not started. */
const CONTACT_AUDIBLE = 10 ** (-70 / 20);
/** A running slide/roll voice estimated 6 dB under `CONTACT_AUDIBLE` for this long (s) is stopped. */
const QUIET_STOP_S = 0.25;
/** Speeds (0..1) at which a looping `speed` preset's level is measured. */
export const SPEED_STEPS: readonly number[] = [0, 0.01, 0.02, 0.035, 0.05, 0.075, 0.1, 0.15, 0.2, 0.3, 0.5, 0.75, 1];

/** Linear interpolation of a `speedLevels` table at `speed`. */
export function levelAtSpeed(levels: readonly number[], speed: number): number {
  const s = Math.min(1, Math.max(0, speed));
  for (let i = 1; i < SPEED_STEPS.length; i++) {
    if (s <= SPEED_STEPS[i]) {
      const a = SPEED_STEPS[i - 1];
      const t = (s - a) / (SPEED_STEPS[i] - a);
      return levels[i - 1] + (levels[i] - levels[i - 1]) * t;
    }
  }
  return levels[levels.length - 1];
}

function clampWetDb(db: number): number {
  return Number.isFinite(db) ? Math.min(6, Math.max(-30, db)) : WET_OFFSET_DB;
}

function newAcoustic(): Acoustic {
  return {
    path: null, reverb: null, visible: 1, excess: 0, found: 1, via: v3(0, 0, 0), wetDb: WET_FLOOR_DB, rtLog: Math.log(0.5),
    rdir: v3(0, 0, 0), predelay: 0, sendHz: 16000, taps: Array.from({ length: TAPS_PER_VOICE }, () => null), primed: false,
  };
}

/** Low-pass of the early reflections from their surfaces' absorption (16 kHz off concrete, ~3 kHz off snow), weighted by their levels. */
function tapCutoff(ac: Acoustic): number {
  let w = 0;
  let a = 0;
  for (const t of ac.taps) {
    if (!t) continue;
    const g = t.refl.gain * t.w;
    w += g;
    a += g * t.refl.absorption;
  }
  const alpha = w > 0 ? a / w : 0.05;
  return 16000 * (1 - alpha) ** 2 + 2000;
}

/** Low-pass of the reverb send: a room of absorbent surfaces is darker (16 kHz off concrete, ~4 kHz off snow). */
function sendCutoff(absorption: number): number {
  return 16000 * (1 - Math.min(1, Math.max(0, absorption))) ** 2 + 1500;
}

/** Map a 0..1 contract value onto the range the preset declares for that input. */
export function toDeclared(input: HaaInput | undefined, v: number): number {
  // An enum receives the index: the contract value is index / (count - 1).
  if (input && input.kind === 'enum') {
    const n = (input as { values?: readonly string[] }).values?.length ?? 1;
    return Math.round(Math.min(1, Math.max(0, v)) * Math.max(0, n - 1));
  }
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
  let wetOffsetDb = clampWetDb(o.reverbLevelDb ?? WET_OFFSET_DB);
  const faceSources = new Set<FaceSource>();
  const ao = o.acoustics ?? {};
  const traceEvery = ao.traceEvery ?? 0.1;
  const voiceEvery = ao.voiceEvery ?? 0.15;

  // Mix: voices -> dry + reverb -> master -> limiter -> output, with per-ear analysers.
  const dry = ctx.createGain();
  const mix = ctx.createGain();
  dry.connect(mix);
  const reverb = new ReverbBank(ctx, haa, mix);
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
    const chain = new VoiceChain(ctx, dry, reverb.inputs);
    slots.push({
      index: i, chain, busy: false, id: 0, preset: '', oneShot: true, priority: 0, started: 0,
      position: v3(0, 0, 0), extent: 0, nearest: false, tx: [0, 0, 0], txs: [0, 0, 0], fadeIn: 1, bankW: [], level: 0, energy: 0, ac: newAcoustic(), nextGeo: 0,
      key: null, stopping: false, cost: 0, speed: 0, quietSince: -1,
    });
  }
  const swarms = o.swarm === false ? null : new SwarmTracker(o.swarm ?? {});
  /** Cluster voice of each swarm. */
  const clusterVoices = new Map<number, { slot: Slot; id: number; forced: Partial<SfxInputs> }>();
  /** Slide/roll keys found inaudible (logged once until they end or start). */
  const quietKeys = new Set<string>();

  // Deferred actions run on the audio clock (each update and handle), with a timer as the fallback
  // for a page that stops calling update (a background tab); each runs once. On the audio clock an
  // OfflineAudioContext render (benchmarks, the measurement harness) releases its voices on time.
  const deferred: Array<{ at: number; run: () => void; done: boolean }> = [];
  function defer(seconds: number, run: () => void): void {
    const job = { at: ctx.currentTime + seconds, run, done: false };
    deferred.push(job);
    setTimeout(() => runJob(job), seconds * 1000 + 40);
  }
  function runJob(job: { run: () => void; done: boolean }): void {
    if (job.done) return;
    job.done = true;
    job.run();
  }
  function runDue(): void {
    const now = ctx.currentTime;
    for (let i = 0; i < deferred.length; ) {
      const job = deferred[i];
      if (job.done) deferred.splice(i, 1);
      else if (job.at <= now) {
        deferred.splice(i, 1);
        runJob(job);
      } else i++;
    }
  }

  const materials = new Map<string, SoundMaterial>();
  const presets = new Map<string, PresetRecord>();
  const pending = new Map<string, Promise<PresetRecord | null>>();
  const continuous = new Map<string, Slot>();
  const logs = new Set<(e: SfxLogEntry) => void>();
  let listener: SfxListener = { position: v3(0, 1.6, 0), forward: v3(0, 0, -1), up: v3(0, 1, 0) };
  let room: SfxRoom | null = null;
  let trace: ListenerTrace | null = null;
  let nextRoom = 0;
  let lastFrame = -1;
  let roomRays: RaySegment[] = [];
  const voiceRays = new Map<number, RaySegment[]>();
  let nextId = 1;
  const counters = { started: 0, dropped: 0, stolen: 0, culled: 0, clustered: 0, loads: 0, maxLoadMs: 0 };

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
    defer(0.06, () => {
      if (!s.busy) s.chain.detach();
    });
    s.busy = false;
    s.stopping = false;
    if (s.key && continuous.get(s.key) === s) continuous.delete(s.key);
    s.key = null;
    voiceRays.delete(s.index);
  }

  function measure(bytes: Uint8Array): { info: VoiceInfo; cost: number; playable: boolean; speedLevels: number[] | null } {
    const voice = haa.createVoice(ctx.sampleRate, 3);
    try {
      const { playable } = voice.load(bytes);
      const info = voice.info();
      if (!playable) return { info, cost: 0, playable, speedLevels: null };
      const blocks = Math.round((0.25 * ctx.sampleRate) / 128);
      let ms = 0;
      for (let pass = 0; pass < 2; pass++) {
        voice.restart();
        const t0 = performance.now();
        for (let b = 0; b < blocks; b++) voice.process(128);
        ms = performance.now() - t0;
      }
      // A slide/roll loop: its level at each speed (0.4 s each, the first 0.1 s skipped).
      let speedLevels: number[] | null = null;
      const speed = info.inputs.find((i) => i.name === 'speed');
      if (info.loop && speed) {
        speedLevels = SPEED_STEPS.map((v) => {
          voice.set('speed', toDeclared(speed, v));
          voice.restart();
          let sum = 0;
          let n = 0;
          const total = Math.round((0.4 * ctx.sampleRate) / 128);
          const skip = Math.round((0.1 * ctx.sampleRate) / 128);
          for (let b = 0; b < total; b++) {
            voice.process(128);
            if (b < skip) continue;
            const l = voice.left();
            const r = voice.right();
            for (let i = 0; i < 128; i++) sum += (l[i] * l[i] + r[i] * r[i]) / 2;
            n += 128;
          }
          return Math.sqrt(sum / Math.max(1, n));
        });
      }
      return { info, cost: ms / 250, playable, speedLevels };
    } finally {
      voice.free();
    }
  }

  function setPreset(name: string, patch: HaaPatch | string | Uint8Array): PresetRecord {
    const bytes = patchBytes(patch);
    const { info, cost, playable, speedLevels } = measure(bytes);
    const rec: PresetRecord = { name, bytes, info, cost: cost + CHAIN_COST, playable, readsEnergy: info.inputs.some((i) => i.name === 'energy'), speedLevels };
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
      if (decl && Number.isFinite(inputs[name])) out[name] = toDeclared(decl, inputs[name]);
    }
    return out;
  }

  function retrace(now: number): void {
    if (!raycast) return;
    const rays: RaySegment[] = [];
    trace = traceListener(listener.position, raycast, { rays: ao.traceRays ?? 256, bounces: ao.bounces ?? 6 }, rays);
    roomRays = rays;
    const feet = voiceReverb(trace, sub(listener.position, v3(0, 1.5, 0)), raycast, { stride: (ao.reverbStride ?? 2) * 2, ref: REF_DISTANCE });
    room = { ...trace.space, wet: feet.energy > 0 ? Math.min(SEND_MAX, 10 ** (mixedWetDb(10 * Math.log10(feet.energy), wetOffsetDb) / 20)) : 0, wetRt60: feet.rt60 };
    nextRoom = now + traceEvery;
  }

  /** Recompute a voice's geometry (direct path, its diffuse field, its echoes) when due. */
  function geometry(s: Slot, now: number, immediate: boolean): void {
    if (!raycast || !(immediate || now >= s.nextGeo)) return;
    if (!trace) retrace(now);
    const rays: RaySegment[] = [];
    const ac = s.ac;
    ac.path = directPath(s.position, listener.position, trace, raycast, { extent: s.extent }, rays);
    ac.reverb = trace ? voiceReverb(trace, s.position, raycast, { stride: ao.reverbStride ?? 2, ref: REF_DISTANCE }) : null;
    const refl = earlyReflections(s.position, listener.position, raycast, {
      max: TAPS_PER_VOICE, directions: SOURCE_DIRS, soft: 0.35, minDelay: 0.0015, scattering: 0.35,
    }, rays);
    // A tap keeps its plane while the plane reflects; a new plane takes a free tap from silence.
    const keep = new Set(refl.map((r) => r.key));
    for (const t of ac.taps) if (t && !keep.has(t.key)) t.target = 0;
    for (const r of refl) {
      const held = ac.taps.find((t) => t && t.key === r.key);
      if (held) {
        held.refl = r;
        held.target = r.weight;
        continue;
      }
      const free = ac.taps.findIndex((t) => !t || (t.target === 0 && t.w < 1e-3));
      if (free >= 0) ac.taps[free] = { key: r.key, refl: r, w: immediate ? r.weight : 0, target: r.weight };
    }
    voiceRays.set(s.index, rays);
    s.nextGeo = now + voiceEvery * (0.8 + 0.4 * ((s.index * 0.618) % 1));
    if (immediate || !ac.primed) prime(ac);
  }

  /** Jump the smoothed values to the targets (a voice that starts). */
  function prime(ac: Acoustic): void {
    const p = ac.path;
    if (p) {
      ac.visible = p.visible;
      ac.excess = p.excess;
      ac.found = p.found ? 1 : 0;
      ac.via = p.via;
    }
    const r = ac.reverb;
    ac.wetDb = r && r.energy > 0 ? Math.max(WET_FLOOR_DB, 10 * Math.log10(r.energy)) : WET_FLOOR_DB;
    if (r) {
      ac.rtLog = Math.log(r.rt60);
      ac.rdir = r.direction;
      ac.predelay = r.predelay;
      ac.sendHz = sendCutoff(r.absorption);
    }
    for (const t of ac.taps) if (t) t.w = t.target;
    ac.primed = true;
  }

  /** Glide a voice's smoothed acoustics by `dt` and write its chain. */
  function applyChain(s: Slot, dt: number, immediate: boolean): void {
    const ac = s.ac;
    const L = listener.position;
    const k = (tau: number): number => (immediate ? 1 : 1 - Math.exp(-dt / tau));
    const p = ac.path;
    if (p) {
      ac.visible += (p.visible - ac.visible) * k(TAU_DIRECT);
      ac.excess += (p.excess - ac.excess) * k(TAU_DIRECT);
      ac.found += ((p.found ? 1 : 0) - ac.found) * k(TAU_DIRECT);
      ac.via = add(ac.via, scale(sub(p.via, ac.via), k(TAU_VIA)));
    }
    const r = ac.reverb;
    const wetTarget = r && r.energy > 0 ? Math.max(WET_FLOOR_DB, 10 * Math.log10(r.energy)) : WET_FLOOR_DB;
    ac.wetDb += (wetTarget - ac.wetDb) * k(TAU_WET);
    if (r) {
      ac.rtLog += (Math.log(r.rt60) - ac.rtLog) * k(TAU_RT);
      ac.rdir = add(ac.rdir, scale(sub(r.direction, ac.rdir), k(TAU_WET)));
      ac.predelay += (r.predelay - ac.predelay) * k(TAU_WET);
      ac.sendHz += (sendCutoff(r.absorption) - ac.sendHz) * k(TAU_WET);
    }
    // Direct sound: the source, or the edge it bends around, at the length of that path.
    const toS = sub(s.position, L);
    const d = Math.max(1e-3, length(toS));
    const shift = s.nearest ? 0 : 0.8 * s.extent;
    const dEff = Math.max(0.3, d - shift);
    const dirS = scale(toS, 1 / d);
    const hidden = Math.min(1, Math.max(0, 1 - ac.visible));
    let dir = dirS;
    let len = dEff;
    if (hidden > 0 && raycast) {
      const toV = sub(ac.via, L);
      const lv = length(toV);
      if (lv > 1e-3) {
        dir = normalize(add(scale(dirS, 1 - hidden), scale(toV, hidden / lv)));
        if (length(dir) < 1e-3) dir = dirS;
        const around = lv + length(sub(s.position, ac.via)) - shift;
        len = dEff + (Math.max(dEff, around) - dEff) * hidden;
      }
    }
    const levels = raycast
      ? diffractionLevels({ visible: ac.visible, excess: ac.excess, found: ac.found > 0.5 }, occlusionStrength)
      : { bandsDb: [0, 0, 0] as [number, number, number] };
    for (let b = 0; b < 3; b++) {
      s.txs[b] += (s.tx[b] - s.txs[b]) * k(TAU_DIRECT);
      levels.bandsDb[b] -= s.txs[b];
    }
    if (!immediate && s.fadeIn < 1) s.fadeIn = Math.min(1, s.fadeIn + dt / CONTINUOUS_FADE_IN_S);
    const near = len < REF_DISTANCE ? REF_DISTANCE / Math.max(len, NEAR_MIN_DISTANCE) : 1;
    const u = s.extent / Math.max(len, 0.01);
    const spread = u >= 1 ? 1 : u * u;
    const send = Math.min(SEND_MAX, 10 ** (mixedWetDb(ac.wetDb, wetOffsetDb) / 20));
    const right = normalize(cross(listener.forward, listener.up));
    const state: ChainState = {
      level: s.stopping ? NaN : s.level * s.fadeIn,
      delay: len / SPEED_OF_SOUND_M_S,
      // Air absorption, exaggerated for a game-sized world: -0.3 dB per metre above 4 kHz (-6 dB at 20 m).
      airShelfDb: -Math.min(24, 0.3 * len),
      direct: near * 10 ** (levels.bandsDb[0] / 20),
      shelvesDb: [levels.bandsDb[1] - levels.bandsDb[0], levels.bandsDb[2] - levels.bandsDb[1]],
      panned: Math.sqrt(1 - spread),
      wide: Math.sqrt(spread) * Math.min(1, REF_DISTANCE / Math.max(len, REF_DISTANCE)),
      apparent: add(L, scale(dir, len)),
      send: ac.wetDb <= WET_FLOOR_DB + 1 ? 0 : send,
      predelay: ac.predelay,
      sendHz: ac.sendHz,
      // Half the lateral component: a field arriving through a door on the left still reaches the
      // right ear (head shadow, room), 5-8 dB lower, not 25.
      sendPan: 0.5 * dot(ac.rdir, right),
      bank: bankWeights(reverb.stretched(Math.exp(ac.rtLog))),
      tapHz: tapCutoff(ac),
    };
    s.chain.apply(state, immediate);
    s.bankW = state.send > 0 ? state.bank : [];
    ac.taps.forEach((t, i) => {
      if (!t) {
        s.chain.setTap(i, null, immediate);
        return;
      }
      t.w += (t.target - t.w) * k(TAU_TAP);
      const path = length(sub(L, t.refl.image));
      s.chain.setTap(i, {
        image: t.refl.image,
        delay: Math.max(0, (path - len) / SPEED_OF_SOUND_M_S),
        gain: t.refl.gain * t.w,
      }, immediate);
      if (t.target === 0 && t.w < 1e-3) ac.taps[i] = null;
    });
    // A preset that declares `_dist` gets the distance in metres (a far explosion rumbles more).
    if (presets.get(s.preset)?.info.inputs.some((i) => i.name === '_dist')) pool.set(s.index, s.id, { _dist: d });
  }

  /** Find a slot for a voice of `rec` at `priority`: a free one, else steal a quieter one. */
  /**
   * Find a slot for a voice of `rec` at `priority`. Two slots stay spare: a stolen voice fades out
   * over 60 ms in its own slot while the new one starts in a spare (a cut was a click). Only when
   * every slot is busy, fading ones included, is the oldest fading voice cut.
   */
  function allocate(rec: PresetRecord, priority: number): { slot: Slot | null; stole: string | null } {
    const busy = slots.filter((s) => s.busy);
    let load = busy.reduce((a, s) => a + (s.stopping ? 0 : s.cost), 0);
    let active = busy.filter((s) => !s.stopping).length;
    const limit = Math.max(1, slots.length - SPARE_SLOTS);
    const fits = (): boolean => load + rec.cost <= budget && active < limit;
    const freeSlot = (): Slot | null => {
      const free = slots.filter((s) => !s.busy);
      if (!free.length) return null;
      // Prefer a slot that already holds this preset: no patch load on the audio thread.
      return free.find((s) => s.preset === rec.name) ?? free.sort((a, b) => a.started - b.started)[0];
    };
    let stole: string | null = null;
    if (!fits()) {
      const victims = busy
        .filter((s) => !s.stopping)
        .sort((a, b) => a.priority - b.priority || a.started - b.started);
      for (const v of victims) {
        if (fits() || v.priority >= priority) break;
        stole = v.preset;
        load -= v.cost;
        active--;
        stopSlot(v, STEAL_FADE_S);
      }
      if (!fits()) return { slot: null, stole: null };
    }
    let slot = freeSlot();
    if (!slot) {
      const fading = slots.filter((s) => s.busy && s.stopping).sort((a, b) => a.started - b.started)[0];
      if (!fading) return { slot: null, stole: null };
      pool.stop(fading.index, fading.id);
      release(fading);
      slot = fading;
    }
    return { slot, stole };
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
    extent?: number,
    nearest?: boolean,
  ): Slot | null {
    const d = dist(position);
    const prio = priority ?? (inputs.energy + 0.05) / (1 + d / 8) + (oneShot ? 0 : 0.25);
    const base = { label, preset: rec.name, kind, inputs, distance: d };
    if (!rec.playable) {
      log({ ...base, slot: -1, result: 'not playable' });
      return null;
    }
    // A slide/roll contact too slow (or too far) to be heard takes no voice.
    if (key && !oneShot && rec.speedLevels && contactLevel(rec, inputs.speed, gain, d) < CONTACT_AUDIBLE) {
      counters.culled++;
      if (!quietKeys.has(key)) log({ ...base, slot: -1, result: 'culled' });
      quietKeys.add(key);
      return null;
    }
    if (key) quietKeys.delete(key);
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
      busy: true, id, preset: rec.name, oneShot, priority: prio, started: ctx.currentTime, position, extent: extent ?? 0, ac: newAcoustic(),
      nearest: !!nearest, tx: [0, 0, 0], txs: [0, 0, 0], fadeIn: oneShot ? 1 : 0,
      level: gain * levelTrim(rec, inputs.energy, kind), energy: inputs.energy,
      key, stopping: false, cost: rec.cost, speed: inputs.speed, quietSince: -1,
    });
    const maxSeconds = rec.info.mode === 'timeline' ? rec.info.duration : 3;
    slot.chain.attach(pool.node, slot.index);
    pool.start(slot.index, id, rec.name, declaredInputs(rec, inputs), oneShot, maxSeconds);
    geometry(slot, ctx.currentTime, true);
    applyChain(slot, 0, true);
    useBank();
    log({ ...base, slot: slot.index, result: stole ? `stole ${stole}` : 'started' });
    return slot;
  }

  function stopSlot(s: Slot, fadeS = CONTINUOUS_FADE_S): void {
    if (!s.busy || s.stopping) return;
    s.stopping = true;
    s.chain.mute(fadeS);
    const id = s.id;
    defer(fadeS + 0.01, () => {
      if (s.id === id && s.busy) {
        pool.stop(s.index, id);
        release(s);
      }
    });
  }

  /**
   * Level trim of a voice. A one-shot (or a game loop) whose preset does not read `energy` follows
   * the energy on its 40 dB axis. A slide/roll voice does not: its preset's `speed` input is its
   * loudness law, and the contact's energy (a pressing power) was scaling it a second time.
   */
  function levelTrim(rec: PresetRecord, energy: number, kind: SfxLogEntry['kind']): number {
    if (rec.readsEnergy || kind === 'slide' || kind === 'roll') return 1;
    return fallbackGain(energy);
  }

  /** Estimated RMS at the listener of a slide/roll voice (no occlusion, which is not known yet). */
  function contactLevel(rec: PresetRecord, speed: number, gain: number, d: number): number {
    if (!rec.speedLevels) return 1;
    return levelAtSpeed(rec.speedLevels, speed) * gain * Math.min(1, REF_DISTANCE / Math.max(d, 1e-3));
  }

  function clusterSlot(m: SoundMaterial, kind: InteractionKind): { preset: string; gain: number; inputs: Partial<SfxInputs> } | null {
    const c = m.sounds[kind]?.cluster;
    if (!c) return null;
    return typeof c === 'string' ? { preset: c, gain: 1, inputs: {} } : { preset: c.preset, gain: c.gain ?? 1, inputs: c.inputs ?? {} };
  }

  /** An impact of a swarm material: its own voice (false is returned), or the swarm's cluster voice. */
  function swarmed(ev: SfxEvent, label: string): boolean {
    if (!swarms || ev.kind !== 'impact') return false;
    const c = clusterSlot(ev.striker, 'impact');
    if (!c) return false;
    const rec = presets.get(c.preset);
    if (!rec) {
      void preset(c.preset);
      return false;
    }
    const inputs = fullInputs({ ...ev.inputs, hardness: ev.surface.hardness, ...c.inputs });
    const now = ctx.currentTime;
    const route = swarms.route(c.preset, ev.position, inputs, now);
    if (route.own) return false;
    counters.clustered++;
    const sw = route.swarm;
    const cin = fullInputs({ ...inputs, ...swarms.inputs(sw, now), ...c.inputs });
    const voice = clusterVoices.get(sw.id);
    if (voice && voice.slot.busy && voice.slot.id === voice.id && !voice.slot.stopping) {
      voice.slot.position = sw.center;
      voice.slot.energy = cin.energy;
      pool.set(voice.slot.index, voice.id, declaredInputs(rec, cin));
      return true;
    }
    const s = start(rec, sw.center, cin, false, c.gain, undefined, `${label} ×swarm`, 'impact', null);
    if (s) clusterVoices.set(sw.id, { slot: s, id: s.id, forced: c.inputs });
    return true;
  }

  function updateSwarms(now: number): void {
    if (!swarms) return;
    for (const sw of swarms.swarms) {
      const voice = clusterVoices.get(sw.id);
      if (!voice) continue;
      const { slot, id } = voice;
      if (!slot.busy || slot.id !== id) {
        clusterVoices.delete(sw.id);
        continue;
      }
      if (swarms.quiet(sw, now)) {
        // The swarm's cluster voice fades out (it faded in): the cluster is never switched on or off in one step.
        stopSlot(slot, 0.2);
        clusterVoices.delete(sw.id);
        continue;
      }
      const rec = presets.get(slot.preset);
      if (rec) pool.set(slot.index, id, declaredInputs(rec, fullInputs({ ...swarms.inputs(sw, now), ...voice.forced })));
    }
    for (const sw of swarms.prune(now)) clusterVoices.delete(sw.id);
  }

  function handle(ev: SfxEvent): void {
    runDue();
    const resolved = resolveInteraction(ev.kind, ev.striker, ev.surface, ev.inputs, ev.surfaceInputs);
    const label = `${ev.striker.name} → ${ev.surface.name}`;
    if (isContinuous(ev.kind)) {
      const baseKey = ev.key ?? `${ev.kind}:${label}`;
      if (ev.phase === 'end') {
        for (const [k, s] of continuous) if (k.startsWith(baseKey + '|')) stopSlot(s);
        for (const k of quietKeys) if (k.startsWith(baseKey + '|')) quietKeys.delete(k);
        return;
      }
      for (const r of resolved) {
        const key = `${baseKey}|${r.slot.preset}`;
        const running = continuous.get(key);
        const rec = presets.get(r.slot.preset);
        if (running && running.busy && !running.stopping && rec) {
          running.position = ev.position;
          running.level = r.gain * levelTrim(rec, r.inputs.energy, ev.kind);
          running.energy = r.inputs.energy;
          running.speed = r.inputs.speed;
          pool.set(running.index, running.id, declaredInputs(rec, r.inputs));
          // Fallen silent (a bottle rocking to rest): release the voice once it stays so.
          const now = ctx.currentTime;
          if (contactLevel(rec, r.inputs.speed, r.gain, dist(ev.position)) < CONTACT_AUDIBLE * 0.5) {
            if (running.quietSince < 0) running.quietSince = now;
            else if (now - running.quietSince >= QUIET_STOP_S) {
              stopSlot(running);
              quietKeys.add(key);
            }
          } else running.quietSince = -1;
          continue;
        }
        if (!rec) {
          void preset(r.slot.preset);
          log({ label, preset: r.slot.preset, kind: ev.kind, inputs: r.inputs, slot: -1, result: 'missing', distance: dist(ev.position) });
          continue;
        }
        const s = start(rec, ev.position, r.inputs, false, r.gain, undefined, label, ev.kind, key, ev.extent);
        if (s) continuous.set(key, s);
      }
      return;
    }
    if (swarmed(ev, label)) return;
    for (const r of resolved) {
      const rec = presets.get(r.slot.preset);
      if (!rec) {
        void preset(r.slot.preset);
        log({ label, preset: r.slot.preset, kind: ev.kind, inputs: r.inputs, slot: -1, result: 'missing', distance: dist(ev.position) });
        continue;
      }
      start(rec, ev.position, r.inputs, true, r.gain, undefined, label, ev.kind, null, ev.extent);
    }
  }

  function play(name: string, position: Vec3, inputs?: Partial<SfxInputs>, opt: PlayOptions = {}): void {
    const full = fullInputs(inputs);
    const rec = presets.get(name);
    if (rec) {
      start(rec, position, full, true, opt.gain ?? 1, opt.priority, opt.label ?? name, 'play', null, opt.extent);
      return;
    }
    void preset(name).then((r) => {
      if (r) start(r, position, full, true, opt.gain ?? 1, opt.priority, opt.label ?? name, 'play', null, opt.extent);
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
    let tx: [number, number, number] = [0, 0, 0];
    const alive = (): boolean => !!slot && slot.busy && slot.id === id && !slot.stopping;
    void preset(name).then((rec) => {
      if (!rec || stopped) return;
      slot = start(rec, pos, current, false, gain, opt.priority, opt.label ?? name, 'loop', null, opt.extent, opt.nearest);
      if (slot) slot.tx = [...tx];
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
      },
      setTransmission(db) {
        tx = [db[0], db[1], db[2]];
        if (alive()) slot!.tx = [...tx];
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
    const dt = lastFrame < 0 ? 0 : Math.min(0.25, Math.max(0, now - lastFrame));
    lastFrame = now;
    runDue();
    updateSwarms(now);
    if (raycast && now >= nextRoom) retrace(now);
    for (const f of faceSources) f.tick(dt);
    for (const s of slots) {
      if (!s.busy) continue;
      s.priority = (s.energy + 0.05) / (1 + dist(s.position) / 8) + (s.oneShot ? 0 : 0.25);
      geometry(s, now, false);
      applyChain(s, dt, false);
    }
    useBank();
  }

  /** Connect the bank's convolvers the busy voices feed; the others leave after their tail. */
  function useBank(): void {
    const n = reverb.inputs.length;
    for (let i = 0; i < n; i++) {
      let active = false;
      for (const s of slots) if (s.busy && (s.bankW[i] ?? 0) > 1e-3) active = true;
      reverb.setActive(i, active);
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
      trace = null;
      nextRoom = 0;
    },
    setOcclusionStrength(strength) {
      occlusionStrength = Number.isFinite(strength) ? Math.min(2, Math.max(0, strength)) : 1;
    },
    setReverbAmount(amount) {
      reverb.setAmount(amount);
    },
    setReverbLevel(db) {
      wetOffsetDb = clampWetDb(db);
    },
    faceSource(faces, fo) {
      const src = createFaceSource({
        raycast: () => raycast,
        listener: () => listener.position,
        loop: (preset, position, inputs, lo) => loop(preset, position, inputs, lo),
      }, faces, fo);
      const stop = src.stop;
      src.stop = (fadeS) => {
        stop(fadeS);
        faceSources.delete(src);
      };
      faceSources.add(src);
      return src;
    },
    stats() {
      const busy = slots.filter((s) => s.busy);
      return {
        slots: slots.length,
        busy: busy.length,
        load: busy.reduce((a, s) => a + s.cost, 0),
        ...counters,
        room,
        voices: busy.map((s) => {
          const ac = s.ac;
          const lv = diffractionLevels({ visible: ac.visible, excess: ac.excess, found: ac.found > 0.5 }, occlusionStrength);
          return {
            slot: s.index, preset: s.preset, distance: dist(s.position), visible: ac.visible, excess: ac.excess,
            directDb: lv.bandsDb, wetDb: ac.wetDb, rt60: Math.exp(ac.rtLog), predelay: ac.predelay,
            pan: 0.5 * dot(ac.rdir, normalize(cross(listener.forward, listener.up))), taps: ac.taps.filter((t) => t && t.w > 0.01).length,
          };
        }),
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
