// Web Audio side of the 3D sound: one fixed node chain per pool slot and a shared reverb bank.
// Browser-only; no sample loop runs here at audio time (the impulse responses are rendered once by
// the HAA engine's own `space` node). The engine computes every value from the geometry
// (raytrace.ts) and ramps them here; nothing switches in one step.
//
// Chain of a voice:
//   pool slot -> level -> propagation delay (distance / c: arrival time and Doppler)
//     -> air absorption (a high shelf falling with distance) -> trunk
//   trunk -> direct gain (diffraction loss, near field) + 2 diffraction shelves
//        -> panned:  HRTF panner at the apparent position (the source, or the edge it bends around)
//        -> wide:    the voice's own stereo, unpanned (a large source around the listener: rain)
//   trunk -> low-pass (the surfaces' absorption) -> 4 early-reflection taps: delay -> gain ->
//        HRTF panner at the image source
//   trunk -> reverb send -> pre-delay -> low-pass (absorbent rooms are darker) -> stereo pan
//        (where the diffuse field comes from: a hall heard through its door) -> 2 of the bank's
//        convolvers, weighted by the voice's own reverb time.
import type { HaaEngine } from '../engine.ts';
import type { Vec3 } from './acoustics.ts';
import { OCCLUSION_SHELF_HZ } from './acoustics.ts';

/**
 * Early-reflection taps per voice. Every node of a chain runs for every busy voice: with 6 taps
 * (each its own filter), two air filters and a proximity shelf a chain measured 3.2 % of the audio
 * thread (16 voices: half of it); 4 taps sharing one filter keep the strongest echoes for about half.
 */
export const TAPS_PER_VOICE = 4;
/** Distance (m) at which a voice plays at its nominal level; beyond, 1/r (the panner's inverse model). */
export const REF_DISTANCE = 2;
/** Near-field: below REF_DISTANCE the level keeps rising as REF/d down to this distance (+6 dB at 1 m). */
export const NEAR_MIN_DISTANCE = 1;
/** High-shelf frequency of the air absorption. */
export const AIR_SHELF_HZ = 4000;

interface Tap {
  delay: DelayNode;
  gain: GainNode;
  panner: PannerNode;
}

/** Everything the engine sets on a voice chain, in linear gains, dB, seconds and hertz. */
export interface ChainState {
  /** Voice trim (preset level law and slot gain). */
  level: number;
  /** Propagation delay (s). */
  delay: number;
  airShelfDb: number;
  /** Broadband gain of the direct sound: diffraction loss times near-field gain. */
  direct: number;
  /** Diffraction shelves (dB) at OCCLUSION_SHELF_HZ. */
  shelvesDb: readonly [number, number];
  /** Gain of the HRTF-panned part and of the unpanned (wide) part; equal power. */
  panned: number;
  wide: number;
  /** Where the direct sound seems to come from (world). */
  apparent: Vec3;
  /** Reverb send amplitude (diffuse-to-direct at REF_DISTANCE). */
  send: number;
  predelay: number;
  sendHz: number;
  /** -1..1: left/right of the diffuse field's arrival. */
  sendPan: number;
  /** Weight of each convolver of the bank. */
  bank: readonly number[];
  /** Low-pass of the early reflections (the reflecting surfaces' absorption). */
  tapHz: number;
}

export interface TapState {
  image: Vec3;
  delay: number;
  gain: number;
}

/** The node chain of one pool slot. */
export class VoiceChain {
  readonly input: GainNode;
  readonly level: GainNode;
  readonly prop: DelayNode;
  readonly airShelf: BiquadFilterNode;
  readonly occ: GainNode;
  readonly shelves: [BiquadFilterNode, BiquadFilterNode];
  readonly tapFilter: BiquadFilterNode;
  readonly pannedGain: GainNode;
  readonly wideGain: GainNode;
  readonly direct: PannerNode;
  readonly send: GainNode;
  readonly sendDelay: DelayNode;
  readonly sendFilter: BiquadFilterNode;
  readonly sendPan: StereoPannerNode;
  readonly sendTo: GainNode[] = [];
  readonly taps: Tap[] = [];
  private readonly ctx: BaseAudioContext;

  constructor(ctx: BaseAudioContext, dry: AudioNode, bankInputs: readonly AudioNode[]) {
    this.ctx = ctx;
    const gain = (v: number): GainNode => {
      const g = ctx.createGain();
      g.gain.value = v;
      return g;
    };
    const biquad = (type: BiquadFilterType, hz: number, db = 0): BiquadFilterNode => {
      const f = ctx.createBiquadFilter();
      f.type = type;
      f.frequency.value = hz;
      f.gain.value = db;
      if (type === 'lowpass') f.Q.value = 0.5;
      return f;
    };
    this.input = gain(1);
    this.level = gain(0);
    this.prop = ctx.createDelay(1.2);
    this.airShelf = biquad('highshelf', AIR_SHELF_HZ);
    this.occ = gain(1);
    this.shelves = [biquad('highshelf', OCCLUSION_SHELF_HZ[0]), biquad('highshelf', OCCLUSION_SHELF_HZ[1])];
    this.tapFilter = biquad('lowpass', 12000);
    this.pannedGain = gain(1);
    this.wideGain = gain(0);
    this.direct = makePanner(ctx);
    this.send = gain(0);
    this.sendDelay = ctx.createDelay(0.2);
    this.sendFilter = biquad('lowpass', 16000);
    this.sendPan = ctx.createStereoPanner();
    chain(this.input, this.level, this.prop, this.airShelf);
    const trunk = this.airShelf;
    chain(trunk, this.occ, this.shelves[0], this.shelves[1]);
    chain(this.shelves[1], this.pannedGain, this.direct, dry);
    chain(this.shelves[1], this.wideGain, dry);
    chain(trunk, this.send, this.sendDelay, this.sendFilter, this.sendPan);
    for (const input of bankInputs) {
      const g = gain(0);
      this.sendPan.connect(g);
      g.connect(input);
      this.sendTo.push(g);
    }
    trunk.connect(this.tapFilter);
    for (let i = 0; i < TAPS_PER_VOICE; i++) {
      const tap: Tap = { delay: ctx.createDelay(0.5), gain: gain(0), panner: makePanner(ctx) };
      chain(this.tapFilter, tap.delay, tap.gain, tap.panner, dry);
      this.taps.push(tap);
    }
  }

  private source: { node: AudioNode; output: number } | null = null;

  /** Feed this chain from output `output` of `node` (a pool slot); idle chains stay unconnected. */
  attach(node: AudioNode, output: number): void {
    if (this.source) return;
    node.connect(this.input, output, 0);
    this.source = { node, output };
  }

  detach(): void {
    if (!this.source) return;
    try {
      this.source.node.disconnect(this.input, this.source.output, 0);
    } catch {
      // Already disconnected.
    }
    this.source = null;
  }

  /** Last value written to each parameter (changes under a tolerance are skipped). */
  private readonly written = new Map<AudioParam | PannerNode, number>();

  /**
   * Write a value: skipped when it moved less than `tol` (absolute, or relative for `rel`), else a
   * linear ramp over `ramp` seconds that then ends. The engine calls this every frame with values
   * it already glides, so the steps are small; and a parameter with no pending ramp is not
   * automated, which keeps Chrome's biquads, delays and gains off their per-sample paths (writing
   * a never-ending setTarget on every parameter every frame cost 2.4 times the audio graph).
   */
  private write(p: AudioParam, v: number, immediate: boolean, tol: number, rel = false, ramp = 0.025): void {
    if (!Number.isFinite(v)) return;
    const last = this.written.get(p);
    if (!immediate && last !== undefined && Math.abs(v - last) <= (rel ? tol * Math.abs(last) : tol)) return;
    this.written.set(p, v);
    const t = this.ctx.currentTime;
    p.cancelScheduledValues(t);
    if (immediate || last === undefined) {
      p.setValueAtTime(v, t);
      return;
    }
    p.setValueAtTime(last, t);
    p.linearRampToValueAtTime(v, t + ramp);
  }

  private place(p: PannerNode, v: Vec3): void {
    const key = Math.round(v.x * 100) * 73856093 ^ Math.round(v.y * 100) * 19349663 ^ Math.round(v.z * 100) * 83492791;
    if (this.written.get(p) === key) return;
    this.written.set(p, key);
    setPos(p, v);
  }

  /** Apply a state (every frame, from smoothed values); `immediate` jumps (a voice that starts). */
  apply(s: ChainState, immediate: boolean): void {
    const g = (p: AudioParam, v: number): void => this.write(p, v, immediate, 0.0035, true);
    const dB = (p: AudioParam, v: number): void => this.write(p, v, immediate, 0.05);
    const hz = (p: AudioParam, v: number): void => this.write(p, v, immediate, 0.005, true);
    // The level follows the voice trim; a stopping voice (NaN) keeps its fade (`mute`).
    if (Number.isFinite(s.level)) g(this.level.gain, s.level);
    // The delay's slope is the Doppler shift: ramps between frame values give a steady pitch.
    this.write(this.prop.delayTime, Math.min(1.19, Math.max(0, s.delay)), immediate, 2e-5, false, 0.05);
    dB(this.airShelf.gain, s.airShelfDb);
    g(this.occ.gain, s.direct);
    dB(this.shelves[0].gain, s.shelvesDb[0]);
    dB(this.shelves[1].gain, s.shelvesDb[1]);
    g(this.pannedGain.gain, s.panned);
    g(this.wideGain.gain, s.wide);
    this.place(this.direct, s.apparent);
    g(this.send.gain, s.send);
    this.write(this.sendDelay.delayTime, Math.min(0.19, Math.max(0, s.predelay)), immediate, 2e-4, false, 0.05);
    hz(this.sendFilter.frequency, s.sendHz);
    this.write(this.sendPan.pan, Math.max(-1, Math.min(1, s.sendPan)), immediate, 0.005);
    this.sendTo.forEach((node, i) => this.write(node.gain, s.bank[i] ?? 0, immediate, 0.002));
    hz(this.tapFilter.frequency, s.tapHz);
  }

  /** Set early-reflection tap `i` (null fades it out). */
  setTap(i: number, tap: TapState | null, immediate: boolean): void {
    const node = this.taps[i];
    if (!node) return;
    if (!tap) {
      this.write(node.gain.gain, 0, immediate, 0, false, 0.05);
      return;
    }
    this.place(node.panner, tap.image);
    this.write(node.delay.delayTime, Math.min(0.49, Math.max(0, tap.delay)), immediate, 2e-5, false, 0.05);
    this.write(node.gain.gain, tap.gain, immediate, 0.0035, true);
  }

  /** Silence now (a stolen or ended voice). */
  mute(fadeS = 0.02): void {
    const t = this.ctx.currentTime;
    this.written.delete(this.level.gain);
    this.level.gain.cancelScheduledValues(t);
    this.level.gain.setTargetAtTime(0, t, fadeS / 3);
  }
}

function chain(...nodes: AudioNode[]): void {
  for (let i = 0; i + 1 < nodes.length; i++) nodes[i].connect(nodes[i + 1]);
}

function makePanner(ctx: BaseAudioContext): PannerNode {
  const p = ctx.createPanner();
  p.panningModel = 'HRTF';
  p.distanceModel = 'inverse';
  p.refDistance = REF_DISTANCE;
  p.maxDistance = 500;
  p.rolloffFactor = 1;
  // A point source is mono: fold the voice's stereo before the HRTF.
  p.channelCount = 1;
  p.channelCountMode = 'explicit';
  return p;
}

// Positions are set, never automated: an automated panner or listener parameter sends Chrome's
// PannerNode down its per-sample path, which measured several times the cost of the whole voice.
// They are set every frame from smoothed values, so they move in small steps; the HRTF panner
// cross-fades its own filters between positions.
function setPos(p: PannerNode, v: Vec3): void {
  if (p.positionX) {
    p.positionX.value = v.x;
    p.positionY.value = v.y;
    p.positionZ.value = v.z;
  } else {
    (p as unknown as { setPosition(x: number, y: number, z: number): void }).setPosition(v.x, v.y, v.z);
  }
}

/** Point the context listener (the camera): position, forward and up vectors. */
export function setListener(ctx: BaseAudioContext, pos: Vec3, forward: Vec3, up: Vec3): void {
  const l = ctx.listener;
  if (l.positionX) {
    l.positionX.value = pos.x;
    l.positionY.value = pos.y;
    l.positionZ.value = pos.z;
    l.forwardX.value = forward.x;
    l.forwardY.value = forward.y;
    l.forwardZ.value = forward.z;
    l.upX.value = up.x;
    l.upY.value = up.y;
    l.upZ.value = up.z;
  } else {
    const legacy = l as unknown as {
      setPosition(x: number, y: number, z: number): void;
      setOrientation(x: number, y: number, z: number, ux: number, uy: number, uz: number): void;
    };
    legacy.setPosition(pos.x, pos.y, pos.z);
    legacy.setOrientation(forward.x, forward.y, forward.z, up.x, up.y, up.z);
  }
}

// --- reverb bank -----------------------------------------------------------------------------

/**
 * Reverb times (s) of the bank's impulse responses, √2 apart; a voice mixes the two around its own.
 * Two decays a factor 2 apart did not blend into the one between: the longer one set the tail (a
 * 0.86 s shed measured 1.37 s).
 */
export const BANK_RT60: readonly number[] = [0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0];
/** Longest decay the `space` node renders faithfully (s); longer ones are stretched from it. */
const SPACE_MAX_RT60 = 1.6;
const BANK_DAMPING = 0.15;

/**
 * Weights of the bank's convolvers for a reverb time: the two neighbours, equal power (sine law), linear in
 * log(RT60) between them. Continuous: a voice walking from a shed into a hall glides from one IR
 * to the next, it never swaps.
 */
export function bankWeights(rt60: number, steps: readonly number[] = BANK_RT60): number[] {
  const w = steps.map(() => 0);
  const r = Math.min(steps[steps.length - 1], Math.max(steps[0], rt60));
  for (let i = 0; i + 1 < steps.length; i++) {
    if (r <= steps[i + 1]) {
      const x = Math.log(r / steps[i]) / Math.log(steps[i + 1] / steps[i]);
      w[i] = Math.cos((x * Math.PI) / 2);
      w[i + 1] = Math.sin((x * Math.PI) / 2);
      return w;
    }
  }
  w[steps.length - 1] = 1;
  return w;
}

/**
 * HAA patch whose output is a reverb impulse response: a 4 ms white-noise burst into a `space`
 * node at full wet. `space` nominally reaches RT60 = 0.16 + 2.6·size s, but its damping makes the
 * measured decay (Schroeder T20) about 2/3 of that, so the size is set for 1.5 × the target.
 */
export function reverbIrPatch(rt60: number, absorption: number, duration = Math.min(4, rt60 * 1.3 + 0.15)): object {
  const burst = 0.004 / duration;
  return {
    haa: 1,
    id: 'sfx-reverb-ir',
    title: 'Reverb impulse response',
    inputs: [],
    graph: {
      duration,
      loop: false,
      mode: 'timeline',
      nodes: [
        { id: 'env', type: 'curve', params: { amount: 1 }, curve: [[0, 1], [burst, 1], [burst * 1.5, 0], [1, 0]] },
        { id: 'burst', type: 'noise', params: { color: 'white', level: 1, width: 1 } },
        { id: 'vca', type: 'gain', params: { gain: 0 } },
        {
          id: 'room',
          type: 'space',
          params: {
            size: Math.min(1, Math.max(0, (rt60 * 1.5 - 0.16) / 2.6)),
            damping: Math.min(1, Math.max(0, 0.25 + absorption)),
            mix: 1,
            width: 0.85,
            distance: 0,
          },
        },
        { id: 'out', type: 'output', params: { gain: 1 } },
      ],
      connections: [
        { id: 'c-env', from: 'env', to: 'vca', target: 'gain', amount: 1 },
        { id: 'c-burst', from: 'burst', to: 'vca', target: 'audio', amount: 1 },
        { id: 'c-room', from: 'vca', to: 'room', target: 'audio', amount: 1 },
        { id: 'c-out', from: 'room', to: 'out', target: 'audio', amount: 1 },
      ],
    },
  };
}

/**
 * Render an impulse response of `rt60` with the engine into an AudioBuffer of the context's rate,
 * scaled to unit energy (mean of the two channels' sum of squares): a send gain is then the
 * diffuse-to-direct amplitude. Beyond what `space` renders, the decay of a shorter render is slowed
 * by an exponential gain (its noise-like tail keeps its texture). The first 3 ms fade in, so the
 * response starts as a dense field, not a slap.
 */
export function renderReverbIr(ctx: BaseAudioContext, engine: HaaEngine, rt60: number, absorption: number): AudioBuffer {
  const rendered = Math.min(rt60, SPACE_MAX_RT60);
  const duration = Math.min(5, rt60 * 1.25 + 0.15);
  const patch = reverbIrPatch(rendered, absorption, duration);
  const voice = engine.createVoice(ctx.sampleRate, 11);
  const sr = ctx.sampleRate;
  try {
    const { playable } = voice.load(new TextEncoder().encode(JSON.stringify(patch)));
    const frames = Math.round(duration * sr);
    const buffer = ctx.createBuffer(2, frames, sr);
    if (!playable) return buffer;
    const l = buffer.getChannelData(0);
    const r = buffer.getChannelData(1);
    for (let f = 0; f < frames; f += 128) {
      const n = Math.min(128, frames - f);
      voice.process(n);
      l.set(voice.left().subarray(0, n), f);
      r.set(voice.right().subarray(0, n), f);
    }
    // Once per IR, at start-up: slow the decay to the target (measured, since the FDN's slower
    // modes grow with the gain), fade in, unit energy.
    const raw = [l.slice(), r.slice()];
    const fade = Math.round(0.003 * sr);
    const shape = (rate: number): void => {
      for (let i = 0; i < frames; i++) {
        const g = (rate ? Math.exp((rate * i) / sr) : 1) * (i < fade ? i / fade : 1);
        l[i] = raw[0][i] * g;
        r[i] = raw[1][i] * g;
      }
    };
    // Calibrated on the late decay (from 50 ms): the `space` tail bends, its later slope read 20 %
    // longer than its T20 from the onset, and the late slope is what a listener hears as the room.
    let rate = rt60 > rendered ? 6.9078 * (1 / rendered - 1 / rt60) : 0;
    shape(rate);
    for (let pass = 0; pass < 4; pass++) {
      const measured = decayTime(l, r, sr, 0.05);
      if (!(measured > 0)) break;
      if (Math.abs(measured / rt60 - 1) < 0.02) break;
      rate += 6.9078 * (1 / measured - 1 / rt60);
      shape(rate);
    }
    let energy = 0;
    for (let i = 0; i < frames; i++) energy += (l[i] * l[i] + r[i] * r[i]) / 2;
    if (energy > 0) {
      const k = 1 / Math.sqrt(energy);
      for (let i = 0; i < frames; i++) {
        l[i] *= k;
        r[i] *= k;
      }
    }
    return buffer;
  } finally {
    voice.free();
  }
}

/** Schroeder T20 × 3 (s) of a stereo response from `from` seconds: the time of 60 dB of decay from its -5..-25 dB slope. */
export function decayTime(l: Float32Array, r: Float32Array, sr: number, from = 0): number {
  const start = Math.round(from * sr);
  const n = l.length - start;
  const sch = new Float64Array(n);
  let acc = 0;
  for (let i = n - 1; i >= 0; i--) sch[i] = acc += l[start + i] * l[start + i] + r[start + i] * r[start + i];
  const at = (db: number): number => {
    const lim = sch[0] * 10 ** (db / 10);
    for (let i = 0; i < n; i++) if (sch[i] < lim) return i / sr;
    return NaN;
  };
  return (at(-25) - at(-5)) * 3;
}

/**
 * The shared reverb: one convolver per `BANK_RT60` step, always running, each fed by the voices'
 * weighted sends. A voice in a small wooden shed feeds the short ones, a voice in a concrete hall
 * the long ones, both at once when one is heard from the other.
 */
export class ReverbBank {
  readonly inputs: GainNode[] = [];
  readonly output: GainNode;
  private readonly ctx: BaseAudioContext;
  private amount = 1;

  constructor(ctx: BaseAudioContext, engine: HaaEngine, dest: AudioNode, steps: readonly number[] = BANK_RT60) {
    this.ctx = ctx;
    this.output = ctx.createGain();
    this.output.connect(dest);
    for (const rt of steps) {
      const input = ctx.createGain();
      const c = ctx.createConvolver();
      // The IR is rendered at unit energy; the browser's own normalisation (a fixed -58 dB
      // calibration) buried the tail 30 dB and more under the direct sound.
      c.normalize = false;
      c.buffer = renderReverbIr(ctx, engine, rt, BANK_DAMPING);
      input.connect(c);
      c.connect(this.output);
      this.inputs.push(input);
    }
  }

  /** Scale every room: 0 dry, 1 the estimated rooms, 2 exaggerated (+6 dB; RT60 × 1.5 via `stretched`). */
  setAmount(amount: number): void {
    this.amount = Math.min(2, Math.max(0, Number.isFinite(amount) ? amount : 1));
    this.output.gain.setTargetAtTime(this.amount, this.ctx.currentTime, 0.05);
  }

  /** The reverb time a voice of measured `rt60` is played at, after the amount's exaggeration. */
  stretched(rt60: number): number {
    return rt60 * (1 + Math.max(0, this.amount - 1) / 2);
  }
}
