// The mix thread's main-thread side (docs/physics-sound.md, "Mix thread"): the render workers
// (mix-worker.ts) synthesise, spatialise in wasm and pre-mix every voice onto 8 virtual-speaker
// buses; the audio thread sums them (`haa-sfx-mixbus`) and decodes them with 8 static HRTF panners
// at the speakers' world directions, which the AudioListener's orientation rotates at once (only
// the listener's translation waits for the lookahead). Its work is fixed: workers x 8 x 128 samples
// summed, 8 panners, the limiter, whatever the number of voices. Same SfxPool interface as the
// other pools; `MixChain` stands for a VoiceChain and writes the voice's parameters for its worker.
import type { Vec3 } from './acoustics.ts';
import { patchBytes } from '../engine.ts';
import type { HaaPatch } from '../types.ts';
import type { SfxPool } from './pool.ts';
import type { SfxPoolOutMsg } from './pool-protocol.ts';
import { CTRL_HEAD, CTRL_STRIDE, ctrlIndex, STARVED } from './ring-protocol.ts';
import { MIX_BUSES, MIX_PARAMS, MIX_PRIORITY, MIX_PROCESSOR, MIX_SPEAKERS, P, STAT_STRIDE } from './mix-protocol.ts';
import type { MixProcessorOptions, MixWorkerIn, MixWorkerOut } from './mix-protocol.ts';
import type { ChainState, TapState } from './spatial3d.ts';
import { REF_DISTANCE, TAPS_PER_VOICE } from './spatial3d.ts';
import { canUseWorkerPool } from './worker-pool.ts';

export interface MixPoolOptions {
  module: WebAssembly.Module;
  /** `@etcito/haa/mix-wasm` (crates/haa-mix), compiled. */
  mixModule: WebAssembly.Module;
  /** `@etcito/haa/worklet` URL (it registers the mix bus processor too). */
  processorUrl: string | URL;
  /** Workers running `@etcito/haa/mix-worker`. */
  workers: Worker[];
  /** Voices at once (engine slots). No audio-thread cost per voice: only worker time. Default 48. */
  slots?: number;
  /** Frames the buses are kept ahead: 2048 (43 ms) with shared memory, 6144 (128 ms) without. */
  lookahead?: number;
  seed?: number;
  /** Force a transport (tests): 'port' even where shared memory works. Default: 'sab' when the page is isolated. */
  transport?: 'sab' | 'port';
}

export interface MixStats {
  transport: 'sab' | 'port';
  /** Voice-blocks rendered, voice-blocks thinned (frozen, the worker behind), voices silenced. */
  rendered: number;
  thinned: number;
  silenced: number;
}

export interface MixPool extends SfxPool {
  readonly offThread: true;
  readonly mixer: true;
  /** The decoded binaural mix (connect it where the voices went). */
  readonly output: GainNode;
  /** The stand-in chain of slot `i`. */
  chain(i: number, listener: () => Vec3, reverbAmount: () => number): MixChain;
  play(slot: number, id: number, l: Float32Array, r: Float32Array, gain: number): void;
  /** Transport 'port': send the parameters written since the last flush (once per engine update). */
  flush(): void;
  starved(): number;
  workerLoad(): number[];
  mixStats(): MixStats;
}

/**
 * A voice's chain in the mix thread: the engine writes the same ChainState and taps as for a
 * VoiceChain, and this turns them into crates/haa-mix's parameters (direction from the listener,
 * delay, band gains with the 1/r loss, air, send, decay weights, taps with their 1/r loss).
 */
export class MixChain {
  readonly cheapTaps = false;
  private readonly p: Float32Array;
  private readonly listener: () => Vec3;
  private readonly amount: () => number;
  private readonly dirty: () => void;
  private delay = 0;

  constructor(p: Float32Array, listener: () => Vec3, amount: () => number, dirty: () => void) {
    this.p = p;
    this.listener = listener;
    this.amount = amount;
    this.dirty = dirty;
  }

  get quality(): { taps: number; hrtf: boolean } {
    return { taps: TAPS_PER_VOICE, hrtf: true };
  }
  attach(_node: AudioNode, _output: number): void {}
  detach(): void {}
  setQuality(_taps: number, _hrtf: boolean): void {}

  apply(s: ChainState, _immediate: boolean): void {
    const p = this.p;
    const L = this.listener();
    const dx = s.apparent.x - L.x;
    const dy = s.apparent.y - L.y;
    const dz = s.apparent.z - L.z;
    const d = Math.max(1e-3, Math.hypot(dx, dy, dz));
    // The PannerNode applied 1/r beyond the reference distance at the apparent position.
    const r = REF_DISTANCE / Math.max(d, REF_DISTANCE);
    if (Number.isFinite(s.level)) p[P.LEVEL] = s.level;
    p[P.DIR] = dx / d;
    p[P.DIR + 1] = dy / d;
    p[P.DIR + 2] = dz / d;
    p[P.DELAY] = s.delay;
    this.delay = s.delay;
    const low = s.direct * s.panned * r;
    const mid = low * 10 ** (s.shelvesDb[0] / 20);
    p[P.LOW] = low;
    p[P.MID] = mid;
    p[P.HIGH] = mid * 10 ** (s.shelvesDb[1] / 20);
    p[P.WIDE] = s.wide * s.direct;
    p[P.AIR] = 10 ** (s.airShelfDb / 20);
    p[P.SEND] = s.send * this.amount();
    p[P.PREDELAY] = s.predelay;
    p[P.SEND_HZ] = s.sendHz;
    const rd = s.rdir ?? { x: 0, y: 0, z: 0 };
    p[P.RDIR] = rd.x;
    p[P.RDIR + 1] = rd.y;
    p[P.RDIR + 2] = rd.z;
    p[P.TAP_HZ] = s.tapHz;
    for (let k = 0; k < 11; k++) p[P.WEIGHTS + k] = s.bank[k] ?? 0;
    p[P.FOCUS] = 1;
    p[MIX_PRIORITY] = s.priority ?? 0;
    this.dirty();
  }

  setTap(i: number, tap: TapState | null, _immediate: boolean): void {
    const b = P.TAPS + i * 5;
    if (i >= 4) return;
    if (!tap) {
      this.p[b + 4] = 0;
      this.dirty();
      return;
    }
    const L = this.listener();
    const dx = tap.image.x - L.x;
    const dy = tap.image.y - L.y;
    const dz = tap.image.z - L.z;
    const d = Math.max(1e-3, Math.hypot(dx, dy, dz));
    this.p[b] = dx / d;
    this.p[b + 1] = dy / d;
    this.p[b + 2] = dz / d;
    // The engine's tap delay follows the direct sound; the mixer reads both from one line.
    this.p[b + 3] = this.delay + tap.delay;
    this.p[b + 4] = tap.gain * (tap.distanceGain ?? REF_DISTANCE / Math.max(d, REF_DISTANCE));
    this.dirty();
  }

  /** Silence now: the mixer smooths the level over 10 ms. */
  mute(_fadeS = 0.02): void {
    this.p[P.LEVEL] = 0;
    this.dirty();
  }
}

export async function createMixPool(ctx: BaseAudioContext, o: MixPoolOptions): Promise<MixPool> {
  if (!o.workers.length) throw new Error('[haa] mix pool: no worker');
  await ctx.audioWorklet.addModule(o.processorUrl);
  const sab = o.transport === 'port' ? false : canUseWorkerPool();
  const transport = sab ? 'sab' as const : 'port' as const;
  const slots = Math.max(1, Math.floor(o.slots ?? 48));
  const W = o.workers.length;
  const lookahead = Math.max(512, Math.round((o.lookahead ?? (sab ? 2048 : 6144)) / 128) * 128);
  const ringFrames = Math.max(8192, 1 << Math.ceil(Math.log2(lookahead * 4)));
  const control = sab ? new SharedArrayBuffer((CTRL_HEAD + W * CTRL_STRIDE) * 4) : undefined;
  const audio = sab ? new SharedArrayBuffer(W * ringFrames * MIX_BUSES * 4) : undefined;
  const paramsBuf = sab ? new SharedArrayBuffer(slots * MIX_PARAMS * 4) : new ArrayBuffer(slots * MIX_PARAMS * 4);
  const statsBuf = sab ? new SharedArrayBuffer(W * STAT_STRIDE * 4) : undefined;
  const params = new Float32Array(paramsBuf);
  const ctl = control ? new Int32Array(control) : null;
  const statsArr = statsBuf ? new Int32Array(statsBuf) : null;
  const processorOptions: MixProcessorOptions = { transport, workers: W, ringFrames, control, audio };
  const node = new AudioWorkletNode(ctx, MIX_PROCESSOR, { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [MIX_BUSES], processorOptions });
  // The fixed decode: 8 HRTF panners at the speakers' directions around the listener (kept at the
  // origin by the engine), rotated by its orientation; no distance loss (the mixer did it).
  const split = ctx.createChannelSplitter(MIX_BUSES);
  node.connect(split);
  const output = ctx.createGain();
  MIX_SPEAKERS.forEach((s, i) => {
    const pan = ctx.createPanner();
    pan.panningModel = 'HRTF';
    pan.distanceModel = 'linear';
    pan.rolloffFactor = 0;
    pan.channelCount = 1;
    pan.channelCountMode = 'explicit';
    if (pan.positionX) {
      pan.positionX.value = s[0];
      pan.positionY.value = s[1];
      pan.positionZ.value = s[2];
    } else (pan as unknown as { setPosition(x: number, y: number, z: number): void }).setPosition(s[0], s[1], s[2]);
    split.connect(pan, i);
    pan.connect(output);
  });
  const listeners = new Set<(msg: SfxPoolOutMsg) => void>();
  const loads = o.workers.map(() => 0);
  const portCounters = o.workers.map(() => ({ rendered: 0, thinned: 0, silenced: 0 }));
  let portStarved = 0;
  node.port.onmessage = (e: MessageEvent<{ t: string; n: number }>) => {
    if (e.data?.t === 'starved') portStarved = e.data.n;
  };
  const ownerOf = (slot: number): Worker => o.workers[slot % W];
  let disposed = false;
  await Promise.all(o.workers.map((w, wi) => new Promise<void>((resolve, reject) => {
    w.onmessage = (e: MessageEvent<MixWorkerOut>) => {
      const m = e.data;
      if (m.t === 'ready') return resolve();
      if (m.t === 'load') {
        loads[wi] = m.busy;
        portCounters[wi] = { rendered: m.rendered, thinned: m.thinned, silenced: m.silenced };
        return;
      }
      for (const cb of listeners) {
        try {
          cb(m);
        } catch {
          // A listener's failure is its own.
        }
      }
    };
    w.onerror = (e) => reject(new Error(String(e.message)));
    const owned = Array.from({ length: slots }, (_, s) => s).filter((s) => s % W === wi);
    const init: MixWorkerIn = {
      t: 'init', module: o.module, mixModule: o.mixModule, sampleRate: ctx.sampleRate, owned, lookahead, seed: o.seed ?? 1, index: wi,
      transport, ringFrames, control, audio, params: sab ? (paramsBuf as SharedArrayBuffer) : undefined, stats: statsBuf,
    };
    if (sab) w.postMessage(init);
    else {
      const ch = new MessageChannel();
      node.port.postMessage({ t: 'port', worker: wi, port: ch.port2 }, [ch.port2]);
      w.postMessage({ ...init, port: ch.port1 }, [ch.port1]);
    }
  })));
  const post = (w: Worker, m: MixWorkerIn): void => {
    if (!disposed) w.postMessage(m);
  };
  const dirtySlots = new Set<number>();
  return {
    node,
    output,
    slots,
    offThread: true,
    mixer: true,
    chain(i, listener, amount) {
      return new MixChain(params.subarray(i * MIX_PARAMS, i * MIX_PARAMS + MIX_PARAMS), listener, amount, () => {
        if (!sab) dirtySlots.add(i);
      });
    },
    flush() {
      if (sab || !dirtySlots.size) return;
      const byWorker = new Map<number, number[]>();
      for (const s of dirtySlots) {
        const wi = s % W;
        if (!byWorker.has(wi)) byWorker.set(wi, []);
        byWorker.get(wi)!.push(s);
      }
      for (const [wi, list] of byWorker) {
        const data = new Float32Array(list.length * MIX_PARAMS);
        list.forEach((s, k) => data.set(params.subarray(s * MIX_PARAMS, s * MIX_PARAMS + MIX_PARAMS), k * MIX_PARAMS));
        post(o.workers[wi], { t: 'params', slots: list, data });
      }
      dirtySlots.clear();
    },
    preset(key: string, patch: HaaPatch | string | Uint8Array) {
      const bytes = patchBytes(patch);
      for (const w of o.workers) post(w, { t: 'preset', key, bytes });
    },
    start(slot, id, key, inputs, oneShot, maxSeconds) {
      post(ownerOf(slot), { t: 'start', slot, id, key, inputs, oneShot, maxSeconds });
    },
    set(slot, id, inputs) {
      post(ownerOf(slot), { t: 'set', slot, id, inputs });
    },
    play(slot, id, l, r, gain) {
      post(ownerOf(slot), { t: 'play', slot, id, l, r, gain });
    },
    stop(slot, id) {
      post(ownerOf(slot), { t: 'stop', slot, id });
    },
    onMessage(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    starved() {
      if (!ctl) return portStarved;
      let n = 0;
      for (let w = 0; w < W; w++) n += Atomics.load(ctl, ctrlIndex(w, STARVED));
      return n;
    },
    workerLoad: () => [...loads],
    mixStats() {
      if (statsArr) {
        let rendered = 0;
        let thinned = 0;
        let silenced = 0;
        for (let w = 0; w < W; w++) {
          rendered += Atomics.load(statsArr, w * STAT_STRIDE);
          thinned += Atomics.load(statsArr, w * STAT_STRIDE + 1);
          silenced += Atomics.load(statsArr, w * STAT_STRIDE + 2);
        }
        return { transport, rendered, thinned, silenced };
      }
      return { transport, ...portCounters.reduce((a, c) => ({ rendered: a.rendered + c.rendered, thinned: a.thinned + c.thinned, silenced: a.silenced + c.silenced }), { rendered: 0, thinned: 0, silenced: 0 }) };
    },
    dispose() {
      if (disposed) return;
      for (const w of o.workers) w.postMessage({ t: 'dispose' } satisfies MixWorkerIn);
      disposed = true;
      listeners.clear();
      for (const w of o.workers) w.terminate();
      try {
        node.disconnect();
        output.disconnect();
      } catch {
        // Already disconnected.
      }
    },
  };
}
