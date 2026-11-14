// Shared-memory rings between the render workers (render-worker.ts) and the ring player worklet
// (ring-worklet.ts): the voices are synthesised off the audio thread and the audio thread only
// copies 128 frames per slot per quantum. Realm-neutral declarations.
//
// control (Int32Array over a SharedArrayBuffer):
//   [0] doorbell: the worklet adds 1 every quantum and notifies (the workers wait on it)
//   [1] quanta played; [2] reserved; [3] reserved
//   per slot s, at CTRL_HEAD + s * CTRL_STRIDE: write frame, read frame, starved quanta, epoch
// audio (Float32Array over a SharedArrayBuffer): per slot, `ringFrames` stereo frames interleaved.
// Positions only grow (frames since the slot's last reset); index = position % ringFrames.

export const SFX_RINGS_PROCESSOR = 'haa-sfx-rings';
export const CTRL_HEAD = 4;
export const CTRL_STRIDE = 4;
export const W_POS = 0;
export const R_POS = 1;
export const STARVED = 2;
export const EPOCH = 3;

export interface SfxRingsProcessorOptions {
  control: SharedArrayBuffer;
  audio: SharedArrayBuffer;
  slots: number;
  ringFrames: number;
}

export function ctrlIndex(slot: number, field: number): number {
  return CTRL_HEAD + slot * CTRL_STRIDE + field;
}

export type RenderWorkerIn =
  | { t: 'init'; module: WebAssembly.Module; control: SharedArrayBuffer; audio: SharedArrayBuffer; slots: number; ringFrames: number; lookahead: number; owned: number[]; seed: number; sampleRate: number }
  | { t: 'preset'; key: string; bytes: Uint8Array }
  | { t: 'start'; slot: number; id: number; key: string; inputs: Record<string, number>; oneShot: boolean; maxSeconds: number }
  | { t: 'set'; slot: number; id: number; inputs: Record<string, number> }
  /** Play baked samples (SharedArrayBuffer-backed: shared, not copied) on `slot`, scaled by `gain`. */
  | { t: 'play'; slot: number; id: number; l: Float32Array; r: Float32Array; gain: number }
  | { t: 'stop'; slot: number; id: number }
  | { t: 'dispose' };

export type RenderWorkerOut =
  | { t: 'ready' }
  | { t: 'started'; slot: number; id: number; playable: boolean; loadMs: number }
  | { t: 'ended'; slot: number; id: number }
  | { t: 'error'; message: string }
  /** Every second: share of wall time spent rendering, and frames rendered. */
  | { t: 'load'; busy: number; frames: number };
