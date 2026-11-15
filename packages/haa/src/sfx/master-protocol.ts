// The sfx master limiter's worklet protocol (crates/haa-core dsp::master_limiter, exported by haa.wasm).

/** Name of the master limiter processor (registered by the package's worklet bundle). */
export const MASTER_PROCESSOR = 'haa-sfx-master';

/** Options of the master limiter node. */
export interface MasterProcessorOptions {
  /** `@etcito/haa/wasm`, compiled. */
  module: WebAssembly.Module;
}

/**
 * What the processor posts about every 100 ms: the input samples whose true peak passed the ceiling
 * since the last report (`over`), the loudest input true peak (dBFS) and the lowest gain (dB, <= 0).
 */
export interface MasterStatsMsg {
  t: 'stats';
  over: number;
  peakDb: number;
  gainDb: number;
}
