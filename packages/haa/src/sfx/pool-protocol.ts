// Messages between the sfx voice pool (main thread, pool.ts) and its AudioWorklet processor
// (pool-worklet.ts). One processor runs one wasm instance and `slots` voices, one stereo
// output per slot. Realm-neutral declarations.

export const SFX_POOL_PROCESSOR = 'haa-sfx-pool';

export interface SfxPoolProcessorOptions {
  module: WebAssembly.Module;
  slots: number;
  seed: number;
}

export type SfxPoolInMsg =
  /** Store (or replace) a preset's patch bytes under `key`; slots holding it reload on next start. */
  | { t: 'preset'; key: string; bytes: Uint8Array }
  /** Start `key` on `slot`: set the inputs, restart, play. A one-shot stops after `maxSeconds`. */
  | { t: 'start'; slot: number; id: number; key: string; inputs: Record<string, number>; oneShot: boolean; maxSeconds: number }
  | { t: 'set'; slot: number; id: number; inputs: Record<string, number> }
  | { t: 'stop'; slot: number; id: number }
  | { t: 'dispose' };

export type SfxPoolOutMsg =
  | { t: 'ready'; slots: number }
  /** `loadMs` is the time the audio thread spent loading the patch (0 when the slot already held it). */
  | { t: 'started'; slot: number; id: number; playable: boolean; loadMs: number }
  | { t: 'ended'; slot: number; id: number }
  | { t: 'error'; message: string };
