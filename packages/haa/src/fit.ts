// Graph fitter host: drives the haa-fit wasm exports (crates/haa-wasm/src/fit.rs) step by step.
// Realm-neutral (Worker, Node, main thread): no DOM, no Node builtins, text through utf8.ts.
// Every computation (render, loss, CMA-ES, mutations) runs in Rust; this file only copies buffers
// and JSON across linear memory. A fit is stepped so a Worker can report progress and stop:
//
//   const fitter = await createFitter(wasmBytes);
//   fitter.start(pcm, 48000, templates, { budget: 800, seed: 1 });
//   while (!fitter.step(25).done) postMessage(fitter.progress());   // cancel = stop calling step
//   const { best } = fitter.result();
//   fitter.free();

import type { HaaPatch } from './types.ts';
import { decodeUtf8, encodeUtf8 } from './utf8.ts';

/** The fit exports of `haa.wasm`, plus the shared allocation and result exports. */
export interface FitExports {
  memory: WebAssembly.Memory;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_result_len(): number;
  haa_fit_alloc_f32(frames: number): number;
  haa_fit_free_f32(ptr: number, frames: number): void;
  haa_fit_start(pcmPtr: number, frames: number, sampleRate: number, cfgPtr: number, cfgLen: number): number;
  haa_fit_step(n: number): number;
  haa_fit_progress(): number;
  haa_fit_result(): number;
  haa_fit_patch(): number;
  haa_fit_free(): void;
  haa_fit_describe(ptr: number, len: number, maxDims: number): number;
  haa_fit_compare(aPtr: number, aFrames: number, bPtr: number, bFrames: number, sampleRate: number): number;
}

/** Fitter settings; every key is optional (defaults in crates/haa-fit/src/fit.rs). */
export interface FitOptions {
  /** Total renders (default 600). */
  budget?: number;
  /** Search seed (default 1): same target, templates, options and seed give the same result. */
  seed?: number;
  /** Templates that race after screening (default 4). */
  topK?: number;
  /** One-node structure mutations tried on the best patch (default 3). */
  mutations?: number;
  /** Dimensions per optimisation, at most 40 (default 24). */
  maxDims?: number;
  /** Multiplier of the initial search steps (default 0.4). */
  sigma?: number;
  /** Cut the target's leading silence and report it as `offset` (default true). */
  trimLead?: boolean;
  /** Try the measured pitch/level alignment of each template (default true). */
  align?: boolean;
  /** Use the f0 term: 'auto' (pitched targets), true or false. */
  f0?: 'auto' | boolean;
  weights?: { stft?: number; mel?: number; env?: number; f0?: number };
  /** STFT sizes of the loss (default [256, 1024, 4096]). */
  ffts?: number[];
}

export interface LossTerms {
  total: number;
  stft: number;
  mel: number;
  env: number;
  f0: number;
}

export interface FitProgress {
  stage: 'screen' | 'optimize' | 'mutate' | 'polish' | 'done';
  done: boolean;
  evals: number;
  budget: number;
  best: number | null;
  template: string | null;
  mutations: string[] | null;
  job: { template: string; dims: number; used: number; budget: number; restarts: number; sigma: number } | null;
}

export interface FitParamInfo {
  node: string;
  type: string;
  key: string;
  kind: 'param' | 'curve-x' | 'curve-y' | 'input';
  min: number;
  max: number;
  scale: 'lin' | 'log';
  value: number;
  selected?: boolean;
}

export interface ParamDescription {
  id: string;
  total: number;
  selected: number;
  params: FitParamInfo[];
}

export interface SignalSummary {
  seconds: number;
  peak: number;
  rmsDb: number;
  centroidHz: number;
  rmsEnvelopeDb: number[];
  /** Energy-weighted centroid per point; 0 where the point is quieter than −60 dB. */
  centroidEnvelopeHz: number[];
}

export interface FitComparison {
  loss: LossTerms;
  usesF0: boolean;
  target: SignalSummary;
  candidate: SignalSummary;
  rmsEnvelopeGapDb: number;
  centroidEnvelopeGapHz: number;
}

export interface FitTemplateScore {
  index: number;
  id: string;
  playable: boolean;
  evals: number;
  score0: number;
  best: number;
  aligned: string[];
  error?: string;
}

export interface FitTrial {
  stage: string;
  template: string;
  mutations: string[];
  dims: number;
  evals: number;
  restarts: number;
  best: number;
}

export interface FitResult {
  done: boolean;
  evals: number;
  budget: number;
  seed: number;
  sampleRate: number;
  /** Seconds of leading silence cut from the target: place the patch at `offset`. */
  offset: number;
  seconds: number;
  usesF0: boolean;
  targetF0: number;
  best: {
    score: number;
    terms: LossTerms;
    template: number;
    templateId: string;
    mutations: string[];
    aligned: string[];
    params: FitParamInfo[];
    patch: HaaPatch;
  } | null;
  templates: FitTemplateScore[];
  history: [number, number][];
  trials: FitTrial[];
  report: FitComparison | null;
}

/** A stepped fitter over one wasm instance (one fit at a time). */
export interface Fitter {
  /** Starts a fit of a mono target; throws `[haa] fit: <reason>` when it cannot start. */
  start(targetPcm: Float32Array, sampleRate: number, templates: (HaaPatch | string)[], opts?: FitOptions): FitProgress;
  /** Runs at most `n` renders and returns the progress (`done` once the budget is spent). */
  step(n: number): FitProgress;
  progress(): FitProgress | null;
  /** The full result (the best so far while running). */
  result(): FitResult | null;
  /** The best patch as HAA patch JSON text, or null before the first render. */
  patch(): string | null;
  /** The fit-able params of a patch, the first `maxDims` marked selected. */
  describe(patch: HaaPatch | string, maxDims?: number): ParamDescription;
  /** Loss terms and level/brightness summaries of a candidate against a target (mono, one rate). */
  compare(target: Float32Array, candidate: Float32Array, sampleRate: number): FitComparison;
  /** Drops the current fit. */
  free(): void;
  readonly exports: FitExports;
}

/** Compile (if needed) and instantiate `haa.wasm`, then wrap its fit exports. */
export async function createFitter(wasm: BufferSource | WebAssembly.Module): Promise<Fitter> {
  const module = wasm instanceof WebAssembly.Module ? wasm : await WebAssembly.compile(wasm);
  const instance = await WebAssembly.instantiate(module, {});
  return wrapFitter(instance.exports as unknown as FitExports);
}

/** Wrap already-instantiated exports (for hosts that share one instance). */
export function wrapFitter(exports: FitExports): Fitter {
  const resultText = (len: number): string => {
    const ptr = exports.haa_result_ptr();
    return decodeUtf8(new Uint8Array(exports.memory.buffer, ptr, len));
  };
  const lastResult = (): string => resultText(exports.haa_result_len());
  const json = <T>(text: string): T => JSON.parse(text) as T;

  const withBytes = <T>(bytes: Uint8Array, f: (ptr: number, len: number) => T): T => {
    const ptr = exports.haa_alloc(bytes.length);
    try {
      new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
      return f(ptr, bytes.length);
    } finally {
      exports.haa_free(ptr, bytes.length);
    }
  };
  const withPcm = <T>(pcm: Float32Array, f: (ptr: number, frames: number) => T): T => {
    const ptr = exports.haa_fit_alloc_f32(pcm.length);
    try {
      if (pcm.length > 0) new Float32Array(exports.memory.buffer, ptr, pcm.length).set(pcm);
      return f(ptr, pcm.length);
    } finally {
      exports.haa_fit_free_f32(ptr, pcm.length);
    }
  };
  const fail = (): never => {
    const message = json<{ error?: string }>(lastResult()).error ?? 'unknown error';
    throw new Error(`[haa] fit: ${message}`);
  };

  return {
    exports,
    start(targetPcm, sampleRate, templates, opts = {}) {
      const config = { ...opts, templates: templates.map((t) => (typeof t === 'string' ? t : JSON.stringify(t))) };
      const ok = withPcm(targetPcm, (pcmPtr, frames) =>
        withBytes(encodeUtf8(JSON.stringify(config)), (cfgPtr, cfgLen) =>
          exports.haa_fit_start(pcmPtr, frames, sampleRate, cfgPtr, cfgLen)));
      if (ok !== 1) fail();
      return json<FitProgress>(lastResult());
    },
    step(n) {
      const code = exports.haa_fit_step(Math.max(1, Math.floor(n)));
      if (code < 0) throw new Error('[haa] fit: no fit started');
      return json<FitProgress>(lastResult());
    },
    progress() {
      const len = exports.haa_fit_progress();
      return len === 0 ? null : json<FitProgress>(resultText(len));
    },
    result() {
      const len = exports.haa_fit_result();
      return len === 0 ? null : json<FitResult>(resultText(len));
    },
    patch() {
      const len = exports.haa_fit_patch();
      return len === 0 ? null : resultText(len);
    },
    describe(patch, maxDims = 24) {
      const text = typeof patch === 'string' ? patch : JSON.stringify(patch);
      const ok = withBytes(encodeUtf8(text), (ptr, len) => exports.haa_fit_describe(ptr, len, maxDims));
      if (ok !== 1) fail();
      return json<ParamDescription>(lastResult());
    },
    compare(target, candidate, sampleRate) {
      const len = withPcm(target, (aPtr, aFrames) =>
        withPcm(candidate, (bPtr, bFrames) => exports.haa_fit_compare(aPtr, aFrames, bPtr, bFrames, sampleRate)));
      if (len === 0) throw new Error('[haa] fit: nothing to compare');
      return json<FitComparison>(resultText(len));
    },
    free() {
      exports.haa_fit_free();
    },
  };
}

/** One note a render plays, for the spectral judge (seconds, MIDI pitch). */
export interface SpectralNote {
  start: number;
  end: number;
  midi: number;
}

/**
 * The spectral judge's terms (crates/haa-fit/src/spectral.rs), each about 0..1: multi-resolution log-mel,
 * linear log-spectrum, centroid / flatness / roll-off, MFCC, and per note the relative harmonic amplitudes,
 * the harmonic line positions, the attack/decay envelope and the pick transient (`notes`: `[harm, lines, env, attack]`,
 * null = unmeasured).
 */
export interface SpectralTerms {
  total: number;
  mel: number;
  lin: number;
  centroid: number;
  flatness: number;
  rolloff: number;
  mfcc: number;
  harm: number;
  lines: number;
  env: number;
  attack: number;
  notes: [number | null, number | null, number | null, number | null][];
}

interface SpectralExports {
  memory: WebAssembly.Memory;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_fit_alloc_f32(frames: number): number;
  haa_fit_free_f32(ptr: number, frames: number): void;
  haa_spec_target(pcmPtr: number, frames: number, sampleRate: number, notesPtr: number, notesLen: number): number;
  haa_spec_score(pcmPtr: number, frames: number): number;
  haa_spec_free(): void;
}

/** Scores renders against one stem; the stem and its notes are analysed once, in Rust. */
export interface SpectralJudge {
  score(render: Float32Array): SpectralTerms;
  free(): void;
}

/** A spectral judge on its own instance of `haa.wasm` (synchronous: the module is already compiled). */
export function createSpectralJudge(module: WebAssembly.Module, target: Float32Array, sampleRate: number, notes: SpectralNote[]): SpectralJudge {
  const e = new WebAssembly.Instance(module, {}).exports as unknown as SpectralExports;
  const text = (len: number) => decodeUtf8(new Uint8Array(e.memory.buffer, e.haa_result_ptr(), len));
  const withPcm = <T>(pcm: Float32Array, f: (ptr: number, frames: number) => T): T => {
    const ptr = e.haa_fit_alloc_f32(pcm.length);
    try {
      if (pcm.length > 0) new Float32Array(e.memory.buffer, ptr, pcm.length).set(pcm);
      return f(ptr, pcm.length);
    } finally {
      e.haa_fit_free_f32(ptr, pcm.length);
    }
  };
  const bytes = encodeUtf8(JSON.stringify(notes.map((n) => ({ start: n.start, end: n.end, midi: n.midi }))));
  const ok = withPcm(target, (ptr, frames) => {
    const np = e.haa_alloc(bytes.length);
    try {
      new Uint8Array(e.memory.buffer, np, bytes.length).set(bytes);
      return e.haa_spec_target(ptr, frames, sampleRate, np, bytes.length);
    } finally {
      e.haa_free(np, bytes.length);
    }
  });
  if (ok !== 1) throw new Error('[haa] spectral judge: the target could not be analysed');
  return {
    score(render) {
      const len = withPcm(render, (ptr, frames) => e.haa_spec_score(ptr, frames));
      if (len === 0) throw new Error('[haa] spectral judge: no target');
      return JSON.parse(text(len)) as SpectralTerms;
    },
    free() {
      e.haa_spec_free();
    },
  };
}
