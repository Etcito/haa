// Web Audio side of the 3D sound: one fixed node chain per pool slot (occlusion low-pass, level,
// HRTF panner for the direct path, four early-reflection taps each panned from its image source,
// a send to the shared reverb) and the shared reverb, a ConvolverNode whose impulse response the
// HAA engine renders with its own `space` node. Browser-only; no sample loop is written here.
import type { HaaEngine } from '../engine.ts';
import type { Occlusion, Reflection, RoomEstimate, Vec3 } from './acoustics.ts';
import { airCutoff, RT60_RANGE } from './acoustics.ts';

export const TAPS_PER_VOICE = 4;
/** Distance (m) up to which a voice plays at full level; beyond, 1/r (the panner's inverse model). */
export const REF_DISTANCE = 2;
const RAMP_S = 0.03;

interface Tap {
  delay: DelayNode;
  filter: BiquadFilterNode;
  gain: GainNode;
  panner: PannerNode;
}

/** The node chain of one pool slot. */
export class VoiceChain {
  readonly input: GainNode;
  readonly level: GainNode;
  readonly filter: BiquadFilterNode;
  readonly direct: PannerNode;
  readonly send: GainNode;
  readonly taps: Tap[] = [];
  private readonly ctx: BaseAudioContext;

  constructor(ctx: BaseAudioContext, dry: AudioNode, reverbIn: AudioNode) {
    this.ctx = ctx;
    this.input = ctx.createGain();
    this.filter = ctx.createBiquadFilter();
    this.filter.type = 'lowpass';
    this.filter.frequency.value = 20000;
    this.filter.Q.value = 0.5;
    this.level = ctx.createGain();
    this.level.gain.value = 0;
    this.direct = makePanner(ctx, 'HRTF');
    this.send = ctx.createGain();
    this.send.gain.value = 0;
    this.input.connect(this.filter);
    this.filter.connect(this.level);
    this.level.connect(this.direct);
    this.direct.connect(dry);
    this.level.connect(this.send);
    this.send.connect(reverbIn);
    for (let i = 0; i < TAPS_PER_VOICE; i++) {
      const tap: Tap = {
        delay: ctx.createDelay(1),
        filter: ctx.createBiquadFilter(),
        gain: ctx.createGain(),
        panner: makePanner(ctx, 'equalpower'),
      };
      tap.filter.type = 'lowpass';
      tap.gain.gain.value = 0;
      this.level.connect(tap.delay);
      tap.delay.connect(tap.filter);
      tap.filter.connect(tap.gain);
      tap.gain.connect(tap.panner);
      tap.panner.connect(dry);
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

  /** Place the direct sound. */
  setPosition(p: Vec3, immediate: boolean): void {
    setPos(this.ctx, this.direct, p, immediate);
  }

  /** Level, occlusion and air absorption of the direct path, plus the reverb send. */
  setLevels(level: number, occ: Occlusion, distance: number, wet: number, immediate: boolean): void {
    const t = this.ctx.currentTime;
    const cutoff = Math.min(occ.cutoff, airCutoff(distance));
    ramp(this.level.gain, level * occ.gain, t, immediate);
    ramp(this.filter.frequency, cutoff, t, immediate);
    // The reverb is heard through the walls more than the direct sound: half the occlusion loss.
    ramp(this.send.gain, wet * (0.5 + 0.5 * occ.gain), t, immediate);
  }

  setReflections(refl: readonly Reflection[], immediate: boolean): void {
    const t = this.ctx.currentTime;
    this.taps.forEach((tap, i) => {
      const r = refl[i];
      if (!r) {
        ramp(tap.gain.gain, 0, t, immediate);
        return;
      }
      setPos(this.ctx, tap.panner, r.image, immediate);
      ramp(tap.delay.delayTime, Math.min(0.99, r.delay), t, immediate);
      // Absorbent surfaces swallow the highs first: 16 kHz off concrete, ~3 kHz off snow.
      ramp(tap.filter.frequency, 16000 * (1 - r.absorption) ** 2 + 2000, t, immediate);
      ramp(tap.gain.gain, r.gain, t, immediate);
    });
  }

  /** Silence now (a stolen or ended voice). */
  mute(fadeS = 0.01): void {
    const t = this.ctx.currentTime;
    this.level.gain.cancelScheduledValues(t);
    this.level.gain.setTargetAtTime(0, t, fadeS / 3);
  }
}

function makePanner(ctx: BaseAudioContext, model: PanningModelType): PannerNode {
  const p = ctx.createPanner();
  p.panningModel = model;
  p.distanceModel = 'inverse';
  p.refDistance = REF_DISTANCE;
  p.maxDistance = 500;
  p.rolloffFactor = 1;
  // A point source is mono: fold the voice's stereo before the HRTF.
  p.channelCount = 1;
  p.channelCountMode = 'explicit';
  return p;
}

function ramp(param: AudioParam, value: number, t: number, immediate: boolean): void {
  if (!Number.isFinite(value)) return;
  param.cancelScheduledValues(t);
  if (immediate) param.setValueAtTime(value, t);
  else param.setTargetAtTime(value, t, RAMP_S);
}

// Positions are set, never automated: an automated panner or listener parameter sends Chrome's
// PannerNode down its per-sample path, which measured several times the cost of the whole voice.
function setPos(_ctx: BaseAudioContext, p: PannerNode, v: Vec3, _immediate: boolean): void {
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

// --- shared reverb ---------------------------------------------------------------------------

/** RT60 steps the impulse responses are rendered at (s). */
export const RT60_STEPS: readonly number[] = [0.15, 0.25, 0.4, 0.6, 0.85, 1.2, 1.7];

export function rt60Step(rt60: number): number {
  const r = Math.min(RT60_RANGE[1], Math.max(RT60_RANGE[0], rt60));
  let best = RT60_STEPS[0];
  for (const s of RT60_STEPS) if (Math.abs(Math.log(s / r)) < Math.abs(Math.log(best / r))) best = s;
  return best;
}

/**
 * HAA patch whose output is a reverb impulse response: a 4 ms white-noise burst into a `space`
 * node at full wet. `space` nominally reaches RT60 = 0.16 + 2.6·size s, but its damping makes the
 * measured decay (Schroeder T20) about 2/3 of that, so the size is set for 1.5 × the target; the
 * longest tail is about 1.8 s. Damping follows the room absorption.
 */
export function reverbIrPatch(rt60: number, absorption: number): object {
  const duration = Math.min(4, rt60 * 1.3 + 0.15);
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

/** Render `reverbIrPatch` with the engine into an AudioBuffer of the context's rate. */
export function renderReverbIr(ctx: BaseAudioContext, engine: HaaEngine, rt60: number, absorption: number): AudioBuffer {
  const patch = reverbIrPatch(rt60, absorption) as { graph: { duration: number } };
  const voice = engine.createVoice(ctx.sampleRate, 11);
  try {
    const { playable } = voice.load(new TextEncoder().encode(JSON.stringify(patch)));
    const frames = Math.round(patch.graph.duration * ctx.sampleRate);
    const buffer = ctx.createBuffer(2, frames, ctx.sampleRate);
    if (!playable) return buffer;
    const l = buffer.getChannelData(0);
    const r = buffer.getChannelData(1);
    for (let f = 0; f < frames; f += 128) {
      const n = Math.min(128, frames - f);
      voice.process(n);
      l.set(voice.left().subarray(0, n), f);
      r.set(voice.right().subarray(0, n), f);
    }
    return buffer;
  } finally {
    voice.free();
  }
}

/** Two convolvers cross-faded when the room changes; the IRs are cached per (RT60 step, damping step). */
export class ReverbBus {
  readonly input: GainNode;
  readonly output: GainNode;
  private readonly ctx: BaseAudioContext;
  private readonly engine: HaaEngine;
  private readonly convolvers: [ConvolverNode, ConvolverNode];
  private readonly fades: [GainNode, GainNode];
  private active = 0;
  private current = '';
  private readonly cache = new Map<string, AudioBuffer>();
  private amount = 1;
  private wet = 0;

  constructor(ctx: BaseAudioContext, engine: HaaEngine, dest: AudioNode) {
    this.ctx = ctx;
    this.engine = engine;
    this.input = ctx.createGain();
    this.output = ctx.createGain();
    this.output.connect(dest);
    const make = (): [ConvolverNode, GainNode] => {
      const c = ctx.createConvolver();
      c.normalize = true;
      const g = ctx.createGain();
      g.gain.value = 0;
      this.input.connect(c);
      c.connect(g);
      g.connect(this.output);
      return [c, g];
    };
    const [c0, g0] = make();
    const [c1, g1] = make();
    this.convolvers = [c0, c1];
    this.fades = [g0, g1];
  }

  /** Follow a room estimate: switch IR when its step changes, and scale the wet level. */
  setRoom(room: RoomEstimate): void {
    const step = rt60Step(room.rt60);
    const damp = Math.round(Math.min(1, room.absorption) * 4) / 4;
    const key = `${step}:${damp}`;
    const t = this.ctx.currentTime;
    if (key !== this.current) {
      let ir = this.cache.get(key);
      if (!ir) {
        ir = renderReverbIr(this.ctx, this.engine, step, damp);
        this.cache.set(key, ir);
      }
      const prev = this.active;
      const next = 1 - prev;
      this.convolvers[next].buffer = ir;
      this.fades[prev].gain.setTargetAtTime(0, t, 0.08);
      this.fades[next].gain.setTargetAtTime(1, t, 0.08);
      this.active = next;
      this.current = key;
      // The faded-out convolver drops its IR: one convolution runs, not two.
      setTimeout(() => {
        if (this.active !== prev) this.convolvers[prev].buffer = null;
      }, 500);
    }
    this.wet = room.wet;
    this.output.gain.setTargetAtTime(room.wet * this.amount, t, 0.1);
  }

  /** Scale the reverb level of every room: 0 dry, 1 the estimated room, up to 2. */
  setAmount(amount: number): void {
    this.amount = Math.min(2, Math.max(0, Number.isFinite(amount) ? amount : 1));
    this.output.gain.setTargetAtTime(this.wet * this.amount, this.ctx.currentTime, 0.05);
  }
}
