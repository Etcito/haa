// The sfx voice pool with its synthesis off the audio thread (docs/physics-sound.md, "Bounded audio
// thread"): render workers (render-worker.ts) synthesise the voices into shared rings, and one
// AudioWorkletNode (`haa-sfx-rings`) only copies them out, one stereo output per slot, as the
// in-worklet pool does. Same `SfxPool` interface, so the engine is unchanged. Needs
// SharedArrayBuffer, i.e. a cross-origin isolated page (COOP same-origin + COEP require-corp or
// credentialless); `createWorkerPool` throws otherwise and the host keeps the in-worklet pool.
import { patchBytes } from '../engine.ts';
import type { HaaPatch } from '../types.ts';
import type { SfxPool } from './pool.ts';
import type { SfxPoolOutMsg } from './pool-protocol.ts';
import { CTRL_HEAD, CTRL_STRIDE, ctrlIndex, SFX_RINGS_PROCESSOR, STARVED } from './ring-protocol.ts';
import type { RenderWorkerIn, RenderWorkerOut, SfxRingsProcessorOptions } from './ring-protocol.ts';

export interface WorkerPoolOptions {
  module: WebAssembly.Module;
  /** `@etcito/haa/worklet` URL (it registers the rings processor too). */
  processorUrl: string | URL;
  /** Workers running `@etcito/haa/render-worker` (the host creates them so its bundler resolves the URL). */
  workers: Worker[];
  slots?: number;
  /** Frames each ring is kept ahead of the audio thread: the latency of an input change, and the worker stall a voice survives. Default 2048 (43 ms at 48 kHz). */
  lookahead?: number;
  seed?: number;
}

export interface WorkerPool extends SfxPool {
  /** True: the voices cost the audio thread a copy, not their synthesis. */
  readonly offThread: true;
  /** Quanta in which an active voice had nothing to play (a late worker), in total. */
  starved(): number;
  /** Share of wall time each worker spent synthesising over the last second. */
  workerLoad(): number[];
  /** Play baked samples on `slot` from its worker (no node is created on the audio thread). */
  play(slot: number, id: number, l: Float32Array, r: Float32Array, gain: number): void;
}

/** True when this page can share memory with its workers and worklet. */
export function canUseWorkerPool(): boolean {
  return typeof SharedArrayBuffer === 'function' && (globalThis as { crossOriginIsolated?: boolean }).crossOriginIsolated === true;
}

export async function createWorkerPool(ctx: BaseAudioContext, o: WorkerPoolOptions): Promise<WorkerPool> {
  if (!canUseWorkerPool()) throw new Error('[haa] worker pool: needs a cross-origin isolated page (SharedArrayBuffer)');
  if (!o.workers.length) throw new Error('[haa] worker pool: no render worker');
  await ctx.audioWorklet.addModule(o.processorUrl);
  const slots = Math.max(1, Math.floor(o.slots ?? 16));
  const lookahead = Math.max(256, Math.round((o.lookahead ?? 2048) / 128) * 128);
  const ringFrames = Math.max(4096, 1 << Math.ceil(Math.log2(lookahead * 4)));
  const control = new SharedArrayBuffer((CTRL_HEAD + slots * CTRL_STRIDE) * 4);
  const audio = new SharedArrayBuffer(slots * ringFrames * 2 * 4);
  const ctl = new Int32Array(control);
  const processorOptions: SfxRingsProcessorOptions = { control, audio, slots, ringFrames };
  const node = new AudioWorkletNode(ctx, SFX_RINGS_PROCESSOR, {
    numberOfInputs: 0,
    numberOfOutputs: slots,
    outputChannelCount: Array.from({ length: slots }, () => 2),
    processorOptions,
  });
  const listeners = new Set<(msg: SfxPoolOutMsg) => void>();
  const loads = o.workers.map(() => 0);
  const ownerOf = (slot: number): Worker => o.workers[slot % o.workers.length];
  let disposed = false;
  await Promise.all(o.workers.map((w, wi) => new Promise<void>((resolve, reject) => {
    w.onmessage = (e: MessageEvent<RenderWorkerOut>) => {
      const m = e.data;
      if (m.t === 'ready') {
        resolve();
        return;
      }
      if (m.t === 'load') {
        loads[wi] = m.busy;
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
    const owned = Array.from({ length: slots }, (_, s) => s).filter((s) => s % o.workers.length === wi);
    w.postMessage({ t: 'init', module: o.module, control, audio, slots, ringFrames, lookahead, owned, seed: o.seed ?? 1, sampleRate: ctx.sampleRate } satisfies RenderWorkerIn);
  })));
  const post = (w: Worker, m: RenderWorkerIn): void => {
    if (!disposed) w.postMessage(m);
  };
  return {
    node,
    slots,
    offThread: true,
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
      let n = 0;
      for (let s = 0; s < slots; s++) n += Atomics.load(ctl, ctrlIndex(s, STARVED));
      return n;
    },
    workerLoad: () => [...loads],
    dispose() {
      if (disposed) return;
      for (const w of o.workers) w.postMessage({ t: 'dispose' } satisfies RenderWorkerIn);
      disposed = true;
      listeners.clear();
      for (const w of o.workers) w.terminate();
      try {
        node.disconnect();
      } catch {
        // Already disconnected.
      }
    },
  };
}
