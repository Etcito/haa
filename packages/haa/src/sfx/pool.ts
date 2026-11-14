// Main-thread side of the sfx voice pool: one AudioWorkletNode (`haa-sfx-pool`) with one stereo
// output per slot, all slots rendered by one wasm instance on the audio thread. Browser-only.
import { patchBytes } from '../engine.ts';
import type { HaaPatch } from '../types.ts';
import { SFX_POOL_PROCESSOR } from './pool-protocol.ts';
import type { SfxPoolInMsg, SfxPoolOutMsg, SfxPoolProcessorOptions } from './pool-protocol.ts';

export interface SfxPoolOptions {
  module: WebAssembly.Module;
  /** URL of the package worklet bundle (`@etcito/haa/worklet`), which registers the pool processor. */
  processorUrl: string | URL;
  /** Voice slots (stereo outputs of the node). Default 16. */
  slots?: number;
  seed?: number;
}

export interface SfxPool {
  readonly node: AudioWorkletNode;
  readonly slots: number;
  preset(key: string, patch: HaaPatch | string | Uint8Array): void;
  start(slot: number, id: number, key: string, inputs: Record<string, number>, oneShot: boolean, maxSeconds: number): void;
  set(slot: number, id: number, inputs: Record<string, number>): void;
  stop(slot: number, id: number): void;
  onMessage(cb: (msg: SfxPoolOutMsg) => void): () => void;
  dispose(): void;
}

const MODULES = new WeakMap<BaseAudioContext, Map<string, Promise<void>>>();

function ensureModule(ctx: BaseAudioContext, url: string | URL): Promise<void> {
  let byUrl = MODULES.get(ctx);
  if (!byUrl) MODULES.set(ctx, (byUrl = new Map()));
  const key = String(url);
  let pending = byUrl.get(key);
  if (!pending) {
    pending = ctx.audioWorklet.addModule(url);
    byUrl.set(key, pending);
    pending.catch(() => byUrl!.delete(key));
  }
  return pending;
}

export async function createSfxPool(ctx: BaseAudioContext, o: SfxPoolOptions): Promise<SfxPool> {
  await ensureModule(ctx, o.processorUrl);
  const slots = Math.max(1, Math.floor(o.slots ?? 16));
  const processorOptions: SfxPoolProcessorOptions = { module: o.module, slots, seed: o.seed ?? 1 };
  const node = new AudioWorkletNode(ctx, SFX_POOL_PROCESSOR, {
    numberOfInputs: 0,
    numberOfOutputs: slots,
    outputChannelCount: Array.from({ length: slots }, () => 2),
    processorOptions,
  });
  const listeners = new Set<(msg: SfxPoolOutMsg) => void>();
  let disposed = false;
  await new Promise<void>((resolve, reject) => {
    node.onprocessorerror = () => reject(new Error('[haa] sfx pool processor error'));
    node.port.onmessage = (event: MessageEvent<SfxPoolOutMsg>) => {
      const msg = event.data;
      if (msg?.t === 'ready') resolve();
      for (const cb of listeners) {
        try {
          cb(msg);
        } catch {
          // A listener's failure never reaches the port.
        }
      }
    };
  });
  const post = (msg: SfxPoolInMsg): void => {
    if (disposed) return;
    try {
      node.port.postMessage(msg);
    } catch {
      // A closed port drops the message.
    }
  };
  return {
    node,
    slots,
    preset(key, patch) {
      post({ t: 'preset', key, bytes: patchBytes(patch) });
    },
    start(slot, id, key, inputs, oneShot, maxSeconds) {
      post({ t: 'start', slot, id, key, inputs, oneShot, maxSeconds });
    },
    set(slot, id, inputs) {
      post({ t: 'set', slot, id, inputs });
    },
    stop(slot, id) {
      post({ t: 'stop', slot, id });
    },
    onMessage(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    dispose() {
      if (disposed) return;
      post({ t: 'dispose' });
      disposed = true;
      listeners.clear();
      try {
        node.disconnect();
      } catch {
        // Already disconnected.
      }
    },
  };
}
