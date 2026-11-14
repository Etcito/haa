// The mix thread (docs/physics-sound.md, "Mix thread"): render workers synthesise their voices,
// spatialise them in wasm (crates/haa-mix) and pre-mix them onto MIX_BUSES world-fixed virtual
// speakers; the `haa-sfx-mixbus` worklet sums the workers' buses and the audio thread decodes them
// with MIX_BUSES static HRTF panners. Realm-neutral declarations.
//
// Transport 'sab' (cross-origin isolated page): per worker a ring of `ringFrames` frames x MIX_BUSES
// channels interleaved in `audio`, positions in `control` (as ring-protocol.ts, one entry per
// worker), voice parameters in `params` (slots x MIX_PARAMS f32, written by the main thread), the
// workers' counters in `stats`. Transport 'port' (no SharedArrayBuffer): each worker posts its blocks
// to the worklet through its own MessagePort and the worklet posts its read position back; the
// parameters come in batches from the main thread.

export const MIX_PROCESSOR = 'haa-sfx-mixbus';
export const MIX_BUSES = 16;
/** f32 per voice parameter record (crates/haa-mix `p::LEN`); index 49 carries the priority. */
export const MIX_PARAMS = 50;
export const MIX_PRIORITY = 49;
/** Per worker, Int32 in `stats`: voice-blocks rendered, voice-blocks thinned, voices silenced. */
export const STAT_STRIDE = 4;

/** Index of crates/haa-mix's parameters. */
export const P = {
  LEVEL: 0, DIR: 1, DELAY: 4, LOW: 5, MID: 6, HIGH: 7, WIDE: 8, AIR: 9, SEND: 10, PREDELAY: 11, SEND_HZ: 12,
  RDIR: 13, TAP_HZ: 16, WEIGHTS: 17, TAPS: 28, FOCUS: 48,
} as const;

/** The 16 virtual speakers (crates/haa-mix): a ring of 8 at ear level every 45 degrees (0 toward -z, 2 toward +x), 4 at +45 degrees of elevation and 4 at -45, between the ring's diagonals. */
export const MIX_SPEAKERS: ReadonlyArray<readonly [number, number, number]> = (() => {
  const out: Array<readonly [number, number, number]> = [];
  for (let k = 0; k < 8; k++) out.push([Math.sin((k * Math.PI) / 4), 0, -Math.cos((k * Math.PI) / 4)] as const);
  const e = Math.PI / 4;
  for (const y of [1, -1]) for (let k = 0; k < 4; k++) {
    const a = Math.PI / 4 + (k * Math.PI) / 2;
    out.push([Math.sin(a) * Math.cos(e), y * Math.sin(e), -Math.cos(a) * Math.cos(e)] as const);
  }
  return out;
})();

export interface MixProcessorOptions {
  transport: 'sab' | 'port';
  workers: number;
  ringFrames: number;
  control?: SharedArrayBuffer;
  audio?: SharedArrayBuffer;
}

export type MixWorkerIn =
  | {
    t: 'init'; module: WebAssembly.Module; mixModule: WebAssembly.Module; sampleRate: number; owned: number[]; lookahead: number; seed: number; index: number;
    transport: 'sab' | 'port'; ringFrames: number; control?: SharedArrayBuffer; audio?: SharedArrayBuffer; params?: SharedArrayBuffer; stats?: SharedArrayBuffer; port?: MessagePort;
  }
  | { t: 'preset'; key: string; bytes: Uint8Array }
  | { t: 'start'; slot: number; id: number; key: string; inputs: Record<string, number>; oneShot: boolean; maxSeconds: number }
  | { t: 'set'; slot: number; id: number; inputs: Record<string, number> }
  | { t: 'play'; slot: number; id: number; l: Float32Array; r: Float32Array; gain: number }
  | { t: 'stop'; slot: number; id: number }
  /** Transport 'port': the parameters of some slots (`slots.length` records of MIX_PARAMS). */
  | { t: 'params'; slots: number[]; data: Float32Array }
  | { t: 'dispose' };

export type MixWorkerOut =
  | { t: 'ready' }
  | { t: 'started'; slot: number; id: number; playable: boolean; loadMs: number }
  | { t: 'ended'; slot: number; id: number }
  | { t: 'error'; message: string }
  /** Every second: busy share, and (transport 'port') the counters. */
  | { t: 'load'; busy: number; rendered: number; thinned: number; silenced: number };
