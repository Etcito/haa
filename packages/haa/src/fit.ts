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
 * null = unmeasured), and `extra`: the energy the render makes where the stem is silent (`extraDb`: that energy over
 * the stem's, dB).
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
  extra: number;
  extraDb: number;
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

/** What a CTC phoneme recogniser heard: [phone, first frame, end frame (exclusive)]. */
export type HeardPhone = [string, number, number];

/** The diction judge's verdict on a render against its stem. */
export interface PhoneErrorRate {
  reference: HeardPhone[];
  hypothesis: HeardPhone[];
  /** (substitutions + deletions + insertions) / reference phones, length and stress marks ignored */
  per: { per: number; substitutions: number; deletions: number; insertions: number; reference: number };
}

interface PhoneExports {
  memory: WebAssembly.Memory;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_fit_alloc_f32(frames: number): number;
  haa_fit_free_f32(ptr: number, frames: number): void;
  haa_phone_per(refPtr: number, refLen: number, hypPtr: number, hypLen: number, vocabPtr: number, vocabLen: number): number;
  haa_phone_align(logitsPtr: number, logitsLen: number, vocabPtr: number, vocabLen: number, reqPtr: number, reqLen: number): number;
  haa_artic_curves(emaPtr: number, emaLen: number, reqPtr: number, reqLen: number): number;
}

/**
 * Phoneme error rate of a render against its stem from a CTC phoneme recogniser's logits (flat rows of the
 * vocabulary's size, e.g. wav2vec2-xlsr-53-espeak run in onnxruntime) and its `vocab.json` text; decoding and the
 * edit distance run in Rust (haa-fit `phonemes`).
 */
export function phoneErrorRate(module: WebAssembly.Module, vocabJson: string, stemLogits: Float32Array, renderLogits: Float32Array): PhoneErrorRate {
  const e = new WebAssembly.Instance(module, {}).exports as unknown as PhoneExports;
  const put = (x: Float32Array) => {
    const ptr = e.haa_fit_alloc_f32(x.length);
    if (x.length > 0) new Float32Array(e.memory.buffer, ptr, x.length).set(x);
    return ptr;
  };
  const bytes = encodeUtf8(vocabJson);
  const a = put(stemLogits), b = put(renderLogits), v = e.haa_alloc(bytes.length);
  try {
    new Uint8Array(e.memory.buffer, v, bytes.length).set(bytes);
    const len = e.haa_phone_per(a, stemLogits.length, b, renderLogits.length, v, bytes.length);
    if (len === 0) throw new Error('[haa] diction judge: the vocabulary is not a {"token": id} object');
    return JSON.parse(decodeUtf8(new Uint8Array(e.memory.buffer, e.haa_result_ptr(), len))) as PhoneErrorRate;
  } finally {
    e.haa_fit_free_f32(a, stemLogits.length);
    e.haa_fit_free_f32(b, renderLogits.length);
    e.haa_free(v, bytes.length);
  }
}

/** One phone of a line as the audio says it, in seconds. */
export interface AlignedPhone {
  /** what the voice says (IPA) */
  ipa: string;
  /** index of the text's word it belongs to */
  word: number;
  start: number;
  /** up to the next phone's start */
  end: number;
  /** heard as written, heard as another phone (`written` is the text's), or heard where the text has none */
  origin: 'kept' | 'substituted' | 'inserted';
  written?: string;
}

/** A line's words timed on the audio by a CTC phoneme recogniser (see {@link phoneAlign}). */
export interface PhoneAlignment {
  /** IPA per word as the voice will say it (a word the audio drops entirely is left out) */
  words: string[];
  /** the text's words as the G2P read them */
  said: string[];
  phones: AlignedPhone[];
  /** the text's phones the audio does not say */
  dropped: { ipa: string; word: number }[];
  /** what the recogniser heard (greedy), and the text's phones force-aligned as written: [phone, start s, end s] */
  heard: [string, number, number][];
  written: [string, number, number][];
}

/**
 * A line's words timed on the audio: a CTC phoneme recogniser's logits of the line (flat rows of the vocabulary's
 * size, `frame` seconds each, row 0 at `offset` s) and its `vocab.json` text. The words go through the lyrics voice's
 * G2P, are force-aligned (Viterbi) on the posteriors and matched with what the recogniser heard; where the two
 * disagree the audio wins (a phone substituted, dropped or inserted). All in Rust (haa-fit `phonemes::align_lyrics`).
 */
export function phoneAlign(module: WebAssembly.Module, vocabJson: string, logits: Float32Array, words: string, frame = 0.02, offset = 0): PhoneAlignment {
  return phoneCall(module, vocabJson, logits, { words, frame, offset }) as PhoneAlignment;
}

/** A render's phones against the stem's (see {@link renderLead}). */
export interface RenderLead {
  /** the phones force-aligned on the render: [phone, start s, end s] (-1 where none could be placed) */
  written: [string, number, number][];
  /** how much earlier (s) to play the line so the render says its phones when the stem does */
  lead: number;
  /** mean |render - stem| start (s) of the phones the render is heard saying as written, and how many */
  lag: number;
  heard: number;
  /** the mean lag over every phone, one the render is not heard saying counted 0.12 s (lower is better) */
  score: number;
}

/**
 * How much earlier a line must be played: a render of phones the stem says at `phones` ([ipa, start s]) heard by the
 * same recogniser (its logits, `frame` s per row, row 0 at `offset` s), the median lag added to `lead` (the seconds
 * the line is played early so far). In Rust (haa-fit `phonemes::render_lead`).
 */
export function renderLead(module: WebAssembly.Module, vocabJson: string, logits: Float32Array, phones: [string, number][], lead: number, frame = 0.02, offset = 0): RenderLead {
  return phoneCall(module, vocabJson, logits, { phones, lead, frame, offset }) as RenderLead;
}

function phoneCall(module: WebAssembly.Module, vocabJson: string, logits: Float32Array, request: object): unknown {
  const e = new WebAssembly.Instance(module, {}).exports as unknown as PhoneExports;
  const vocab = encodeUtf8(vocabJson);
  const req = encodeUtf8(JSON.stringify(request));
  const a = e.haa_fit_alloc_f32(logits.length);
  if (logits.length > 0) new Float32Array(e.memory.buffer, a, logits.length).set(logits);
  const v = e.haa_alloc(vocab.length), r = e.haa_alloc(req.length);
  try {
    new Uint8Array(e.memory.buffer, v, vocab.length).set(vocab);
    new Uint8Array(e.memory.buffer, r, req.length).set(req);
    const len = e.haa_phone_align(a, logits.length, v, vocab.length, r, req.length);
    if (len === 0) throw new Error('[haa] phone alignment: the vocabulary is not a {"token": id} object or the request has no words');
    return JSON.parse(decodeUtf8(new Uint8Array(e.memory.buffer, e.haa_result_ptr(), len)));
  } finally {
    e.haa_fit_free_f32(a, logits.length);
    e.haa_free(v, vocab.length);
    e.haa_free(r, req.length);
  }
}

/** The chimera `articulate` inputs {@link articCurves} writes, in order. */
export const ARTIC_INPUTS = ["jaw", "tongueHeight", "tongueFront", "lipRound", "lipOpen", "lipSpread", "tipUp", "tipFront"] as const;
export type ArticInput = (typeof ARTIC_INPUTS)[number];

/** What {@link articCurves} asks for one phrase (seconds on the patch clock). */
export interface ArticRequest {
  /** seconds of the EMA's row 0 (the clip's start) */
  offset: number;
  start: number;
  end: number;
  /** the curves' frame (s) */
  hop: number;
  /** amplitude 0..1 per hop frame from `start` (the syllables' sonority) */
  level: number[];
  /** keyframes per syllable per articulator, at most */
  maxPerSyllable: number;
  /** seconds the articulators are played early (the tract lags its targets) */
  lead: number;
}

/** One phrase's articulator curves (see {@link articCurves}). */
export interface ArticCurves {
  /** syllable cuts (s), the phrase's start and end included */
  syllables: number[];
  /** per input, its keyframes [s, value in the node's units] */
  inputs: Record<ArticInput, [number, number][]>;
  /** per input, its value per hop frame from `start` (before the keyframes) */
  frames: Record<ArticInput, number[]>;
  /** how open the mouth is per hop frame (SPARC's z units) */
  opening: number[];
  /** keyframes over every input */
  points: number;
}

/**
 * A phrase's articulator curves with no text, from SPARC's raw EMA of a clip (12 coordinates per 20 ms row: tongue
 * dorsum, blade, tip, lower incisor, upper and lower lip, x and y): SPARC's 10 Hz lowpass, the calibrated map to the
 * chimera `articulate` inputs, the syllables from the sonority, and per input a few keyframes per syllable. All in
 * Rust (haa-fit `articulatory`).
 */
export function articCurves(module: WebAssembly.Module, ema: Float32Array, request: ArticRequest): ArticCurves {
  const e = new WebAssembly.Instance(module, {}).exports as unknown as PhoneExports;
  const req = encodeUtf8(JSON.stringify(request));
  const a = e.haa_fit_alloc_f32(ema.length);
  if (ema.length > 0) new Float32Array(e.memory.buffer, a, ema.length).set(ema);
  const r = e.haa_alloc(req.length);
  try {
    new Uint8Array(e.memory.buffer, r, req.length).set(req);
    const len = e.haa_artic_curves(a, ema.length, r, req.length);
    if (len === 0) throw new Error("[haa] articulator curves: the request needs start < end, hop > 0 and one EMA row");
    return JSON.parse(decodeUtf8(new Uint8Array(e.memory.buffer, e.haa_result_ptr(), len))) as ArticCurves;
  } finally {
    e.haa_fit_free_f32(a, ema.length);
    e.haa_free(r, req.length);
  }
}

/** The singer fit's exports of `haa.wasm`. */
interface SingerExports {
  memory: WebAssembly.Memory;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_result_len(): number;
  haa_fit_alloc_f32(frames: number): number;
  haa_fit_free_f32(ptr: number, frames: number): void;
  haa_singer_start(pcmPtr: number, frames: number, sampleRate: number, reqPtr: number, reqLen: number): number;
  haa_singer_step(n: number): number;
  haa_singer_result(): number;
}

/** What {@link singerFit} fits a voice on (seconds on the stem's clock, which is the patch's). */
export interface SingerRequest {
  /** the voice's chimera params (female, pressure, tension, breath, brightness, presence, tract, glide, ...) */
  base: Record<string, number | string>;
  /** per phrase, its phones timed on the stem ([ipa, start]) and its end: the vowels' nuclei are cut from them */
  phrases?: { phones: [string, number][]; end: number }[];
  /** or the nuclei themselves */
  nuclei?: { ipa: string; start: number; end: number; params?: Record<string, number> }[];
  /** what the phrase curves play per frame (each nucleus takes their values at its middle) */
  timbre?: { hop: number; values: Record<string, ArrayLike<number>> };
  /** evaluations of the joint stage (each renders every nucleus) and per vowel; nuclei per vowel; seed; f0 range */
  jointEvals?: number;
  vowelEvals?: number;
  perVowel?: number;
  seed?: number;
  fLo?: number;
  fHi?: number;
}

/** The singer fit (see {@link singerFit}). */
export interface SingerFit {
  /** each habit input's static value (node units) */
  statics: Record<string, number>;
  /** per vowel (IPA) that keeps its own values, those of `vowelKeys` */
  vowels: Record<string, Record<string, number>>;
  vowelKeys: string[];
  /** mean nucleus loss (rms band dB + 0.3 x rms harmonicity dB): inputs at their defaults, after the joint stage, final */
  loss: { start: number; joint: number; final: number };
  /** per nucleus kept, render minus stem per report band (dB), the inputs at their defaults and fitted */
  nuclei: { ipa: string; start: number; end: number; before: number[]; after: number[] }[];
  reportEdges: number[];
  renders: number;
}

/**
 * The singer's own vowels and timbre: the chimera voice's habit inputs (articulator offsets, velum, the folds' pulse,
 * walls, side chambers, aspiration) fitted by CMA-ES on the stem's vowel nuclei, each rendered by the chimera node in
 * `lyrics` mode on the stem's f0 (haa-fit `singer`): one static value per input, then per vowel quality its own
 * articulator values where they help. Runs to the end; `onProgress(done, total)` between steps of `step` renders.
 * Throws when no nucleus is usable.
 */
export function singerFit(module: WebAssembly.Module, pcm: Float32Array, sampleRate: number, request: SingerRequest, onProgress?: (done: number, total: number) => void, step = 25): SingerFit {
  const e = new WebAssembly.Instance(module, {}).exports as unknown as SingerExports;
  const timbre = request.timbre && { hop: request.timbre.hop, values: Object.fromEntries(Object.entries(request.timbre.values).map(([k, v]) => [k, Array.from(v)])) };
  const req = encodeUtf8(JSON.stringify({ ...request, timbre }));
  const a = e.haa_fit_alloc_f32(pcm.length);
  if (pcm.length > 0) new Float32Array(e.memory.buffer, a, pcm.length).set(pcm);
  const r = e.haa_alloc(req.length);
  const text = (len: number) => decodeUtf8(new Uint8Array(e.memory.buffer, e.haa_result_ptr(), len));
  try {
    new Uint8Array(e.memory.buffer, r, req.length).set(req);
    if (e.haa_singer_start(a, pcm.length, sampleRate, r, req.length) !== 1) {
      throw new Error(`[haa] singer fit: ${(JSON.parse(text(e.haa_result_len())) as { error?: string }).error ?? "bad request"}`);
    }
    for (;;) {
      const running = e.haa_singer_step(step);
      const p = JSON.parse(text(e.haa_result_len())) as { done: number; total: number };
      onProgress?.(p.done, p.total);
      if (running !== 1) break;
    }
    const len = e.haa_singer_result();
    if (len === 0) throw new Error("[haa] singer fit: no result");
    return JSON.parse(text(len)) as SingerFit;
  } finally {
    e.haa_fit_free_f32(a, pcm.length);
    e.haa_free(r, req.length);
  }
}
