// tiny-voca speech host: drives the speech exports of haa.wasm (crates/haa-wasm/src/speech.rs and dizione.rs).
// Realm-neutral (Worker, Node, main thread): no DOM, no Node builtins, text through utf8.ts. Every computation
// (phonemizer, sentence cut, model, voice edits) runs in Rust; this file copies JSON and audio across linear memory
// and fetches the weights.
//
//   const gguf = await fetchTinyVoca({ onProgress: (p) => show(p.loaded / p.total) });  // Cache API when there is one
//   const speech = await createSpeech(wasmBytesOrModule);
//   speech.load(gguf);                                   // the model and, from the same file, the phonemizer
//   const pcm = speech.speak('Hello there. How are you?', { m2: 0.6, warp: 1.05 });   // Float32Array at 24 kHz
//
// The weights are never bundled: the official file of the tiny-voca Hugging Face repo, pinned to a revision.

import type { SpeechClip } from './types.ts';

/** A source `planPatch` queued: its key, the parts already kept and whether they are all of it. */
export interface SpeechPlanned {
  key: string;
  parts: number;
  complete: boolean;
}

/** What one `speechNext` made available. */
export interface SpeechStep {
  key: string;
  index: number;
  last: boolean;
  rate: number;
  frames: number;
  error?: string | null;
}
import { decodeUtf8, encodeUtf8 } from './utf8.ts';

/** The tiny-voca Hugging Face repo and the revision the Rust port was checked against. */
export const TINY_VOCA_REPO = 'n4ze3m/tiny-voca';
export const TINY_VOCA_REVISION = '1e73262226dbf4ef02e9df277e81aaa1db5ce16c';
/** The one-file GGUF (model + phonemizer, 7,988,384 bytes); Hugging Face serves it with CORS. */
export const TINY_VOCA_URL = `https://huggingface.co/${TINY_VOCA_REPO}/resolve/${TINY_VOCA_REVISION}/gguf/tiny-voca-js.gguf`;
export const TINY_VOCA_BYTES = 7_988_384;
/** Cache API bucket the weights are kept in. */
export const TINY_VOCA_CACHE = 'haa-tinyvoca';

/** The speech exports of `haa.wasm`, plus the shared allocation and result exports. */
export interface SpeechExports {
  memory: WebAssembly.Memory;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_result_len(): number;
  haa_tinyvoca_load(ptr: number, len: number): number;
  haa_tinyvoca_info(): number;
  haa_tinyvoca_run(ptr: number, len: number): number;
  haa_dizione_phonemize?(ptr: number, len: number): number;
}

/** The `speak` node's voice params (crates/haa-core/src/nodes/speak.rs); every key is optional. */
export interface SpeakParams {
  /** Blend of the two stock speakers: 1 = M2, 0 = F2 (-0.5 to 1.5). */
  m2?: number;
  /** Tract warp (> 1 = shorter tract), 0.7 to 1.4. */
  warp?: number;
  /** Spectral tilt, dB per octave about 1 kHz. */
  tilt?: number;
  /** 32 band offsets in dB (1/6 octave from 150 Hz) as text, numbers separated by spaces or commas; '' = none. */
  envelope?: string;
  /** Breath (noise branch) level, dB. */
  breath?: number;
  /** H1-H2 change, dB. */
  phonation?: number;
  /** Pitch shift, semitones. */
  shift?: number;
  /** Pitch range, x. */
  range?: number;
  /** Speaking rate, x. */
  rate?: number;
  /** Jitter, x (1 = none). */
  jitter?: number;
}

/** The voice keys of the `speak` node, in catalog order (`gain` is a level, not part of the voice). */
export const SPEAK_VOICE_KEYS = ['m2', 'warp', 'tilt', 'envelope', 'breath', 'phonation', 'shift', 'range', 'rate', 'jitter'] as const;

/** voice.tvv.json v3, the import/export form of the voice params. */
export interface VoiceFileV3 {
  v: 3;
  base: { m2?: number; f2?: number };
  post: {
    warp?: number;
    tilt_db_oct?: number;
    env_offset_db?: number[];
    breath_db?: number;
    phonation?: number;
    f0_shift_st?: number;
    f0_range_x?: number;
    rate_x?: number;
    jitter_x?: number;
  };
  origin?: { kind: 'sample' | 'prior'; seed?: number; attrs?: Record<string, unknown> } | Record<string, unknown>;
}

/** A voice for the stage ops: a stock speaker or voice.tvv.json v3 (absent = M2). */
export type VoiceSpec = 'M2' | 'F2' | VoiceFileV3;

export interface SpeechInfo {
  loaded: boolean;
  phonemizer: boolean;
  sampleRate?: number;
  hop?: number;
  speakers?: string[];
  heads?: [number, number];
  maxTokens?: number;
}

/** One utterance as `speak` cuts the text: tiny-voca token ids, stress (0/1/2) and phone symbols. */
export interface Utterance {
  ids: number[];
  stress: number[];
  symbols: string[];
}

/** How the speak node reads a text's script markup (SCRIPT FORMAT v2: `[key:N]` tags, `*emphasis*`, `|` pauses, `{sounds}`). */
export interface SpeechScript {
  plain: string;
  segments: { text: string; keys: Record<string, number> }[];
  emphasis: { start: number; end: number; level: number }[];
  pauses: { at: number; seconds: number }[];
  sounds: { at: number; event: string | null; phones: { id: number; stress: number; stretch: number; seconds: number | null; voiced: boolean | null }[] }[];
  warnings: string[];
}

/** dizione's result for one text (crates/haa-wasm/src/dizione.rs). */
export interface Phonemized {
  ids: number[];
  stress: number[];
  symbols: string[];
  words: { word: string; raw: string; phones: string[]; stress: number[]; source: string; tokens: [number, number]; span: [number, number] | null }[];
}

type Tokens = { tokens: number[]; stress: number[] };
type Cepstra = { harmonic: number[]; noise: number[] };

/** The staged ops of `haa_tinyvoca_run`: request, result. Audio results are Float32Array at 24 kHz. */
export interface SpeechOps {
  encode: [Tokens & { voice?: VoiceSpec }, { hidden: number[]; durationRaw: number[]; durations: number[] }];
  durations: [{ tokens: number[]; durationRaw: number[]; speed?: number }, { durations: number[] }];
  expand: [{ tokens: number[]; hidden: number[]; durations: number[]; voice?: VoiceSpec }, { logF0: number[]; voiced: number[]; energy: number[]; prosodyHidden: number[]; owner: number[] }];
  decode: [{ prosodyHidden: number[]; logF0: number[]; voiced: number[]; energy: number[] }, Cepstra];
  envelope: [Cepstra & { logF0: number[]; voice?: VoiceSpec }, Cepstra];
  vocode: [Cepstra & { logF0: number[]; cepDelta?: Cepstra; noiseSamples?: number[]; seed?: number }, Float32Array];
  synthesize: [Tokens & { voice?: VoiceSpec; speed?: number; pitchShift?: number; pitchRange?: number; seed?: number; noiseSamples?: number[] }, Float32Array];
  perform: [{ perf: unknown; voice?: VoiceSpec; seed?: number; noiseSamples?: number[] }, Float32Array];
  speak: [{ text: string; params?: SpeakParams; voice?: VoiceSpec; seed?: number; withWarnings?: boolean }, Float32Array];
  script: [{ text: string }, SpeechScript];
  utterances: [{ text: string }, Utterance[]];
  speechPlan: [{ patch: unknown; seed?: number; rate?: number }, SpeechPlanned[]];
  speechNext: [Record<string, never>, SpeechStep | null];
  clipPart: [{ key: string; index: number }, Float32Array];
  voiceToParams: [{ voice: VoiceFileV3 }, Required<SpeakParams>];
  paramsToVoice: [{ params: SpeakParams }, VoiceFileV3];
}

const AUDIO_OPS = new Set<keyof SpeechOps>(['vocode', 'synthesize', 'perform', 'speak', 'clipPart']);

/** tiny-voca over one wasm instance: the model and phonemizer are that instance's. */
export interface Speech {
  /** Loads a tiny-voca GGUF (and its phonemizer when the file has one); throws `[haa] speech: <reason>`. */
  load(gguf: Uint8Array): { phonemizer: boolean; phonemizerError?: string };
  info(): SpeechInfo;
  /**
   * Says `text` (English, with script markup: `[scared:4, question:10]` tags, `*emphasis*`, `|` pauses, `{hmm}`
   * sounds) with a voice: the `speak` node params, a stock speaker or voice.tvv.json v3. Cut per sentence, as the
   * node does. Float32Array at `info().sampleRate` (24 kHz). `onWarnings` gets the script's warnings, if any.
   */
  speak(text: string, voice?: SpeakParams | VoiceSpec, opts?: { seed?: number; onWarnings?: (warnings: string[]) => void }): Float32Array;
  /** How `speak` reads the script markup of `text`, with its warnings (no rendering). */
  script(text: string): SpeechScript;
  /** How `speak` cuts and phonemizes `text`. */
  utterances(text: string): Utterance[];
  /**
   * Queues the speech of every `speak` node of a patch (JSON text or object) for voice seed `seed`, nothing
   * rendered yet, its parts to come at `rate` (the player's sample rate, so the audio thread only copies them;
   * default the model's 24 kHz); returns each source's key. Then call `nextPart` until it returns null: each call renders one
   * utterance (or hands out one already kept), to give to a player (`createPlayer({ speechClips })`,
   * `player.setSpeechClips`), which has no model and plays each part as it arrives.
   */
  planPatch(patch: unknown, seed?: number, rate?: number): SpeechPlanned[];
  /** The next part of the queued speech, or null when every source is complete. */
  nextPart(): (SpeechClip & { part: number; last: boolean; error?: string }) | null;
  /** dizione on one text (no cut), with the words and their token ranges. */
  phonemize(text: string): Phonemized;
  voiceToParams(voice: VoiceFileV3): Required<SpeakParams>;
  paramsToVoice(params: SpeakParams): VoiceFileV3;
  /** One staged op (see `SpeechOps`). */
  run<K extends keyof SpeechOps>(op: K, request: SpeechOps[K][0]): SpeechOps[K][1];
  readonly exports: SpeechExports;
}

/** Compile (if needed) and instantiate `haa.wasm`, then wrap its speech exports. */
export async function createSpeech(wasm: BufferSource | WebAssembly.Module): Promise<Speech> {
  const module = wasm instanceof WebAssembly.Module ? wasm : await WebAssembly.compile(wasm);
  const instance = await WebAssembly.instantiate(module, {});
  return wrapSpeech(instance.exports as unknown as SpeechExports);
}

/** Wrap already-instantiated exports (for hosts that share one instance, e.g. with a patch voice). */
export function wrapSpeech(exports: SpeechExports): Speech {
  const resultBytes = (len: number): Uint8Array => new Uint8Array(exports.memory.buffer, exports.haa_result_ptr(), len).slice();
  const lastText = (): string => decodeUtf8(resultBytes(exports.haa_result_len()));
  const withBytes = <T>(bytes: Uint8Array, f: (ptr: number, len: number) => T): T => {
    const ptr = exports.haa_alloc(bytes.length);
    try {
      new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
      return f(ptr, bytes.length);
    } finally {
      exports.haa_free(ptr, bytes.length);
    }
  };
  const fail = (): never => {
    let message = 'unknown error';
    try {
      message = (JSON.parse(lastText()) as { error?: string }).error ?? message;
    } catch {
      // the result held no JSON
    }
    throw new Error(`[haa] speech: ${message}`);
  };

  const call = (op: string, request: object): Uint8Array => {
    const n = withBytes(encodeUtf8(JSON.stringify({ ...request, op })), (ptr, len) => exports.haa_tinyvoca_run(ptr, len));
    if (n < 0) fail();
    return resultBytes(n);
  };
  const run = <K extends keyof SpeechOps>(op: K, request: SpeechOps[K][0]): SpeechOps[K][1] => {
    const bytes = call(op, request);
    if (AUDIO_OPS.has(op)) return new Float32Array(bytes.buffer, 0, bytes.length >> 2) as SpeechOps[K][1];
    return JSON.parse(decodeUtf8(bytes)) as SpeechOps[K][1];
  };

  return {
    exports,
    load(gguf) {
      const code = withBytes(gguf, (ptr, len) => exports.haa_tinyvoca_load(ptr, len));
      if (code !== 0) fail();
      const text = lastText();
      return text ? (JSON.parse(text) as { phonemizer: boolean; phonemizerError?: string }) : { phonemizer: false };
    },
    info() {
      const n = exports.haa_tinyvoca_info();
      return JSON.parse(decodeUtf8(resultBytes(n))) as SpeechInfo;
    },
    speak(text, voice, opts = {}) {
      const request: SpeechOps['speak'][0] = { text, seed: opts.seed ?? 0 };
      if (typeof voice === 'string' || (voice && 'v' in voice)) request.voice = voice as VoiceSpec;
      else if (voice) request.params = voice;
      if (!opts.onWarnings) return run('speak', request);
      // withWarnings: the result is JSON {audio, warnings}, not raw samples
      const said = JSON.parse(decodeUtf8(call('speak', { ...request, withWarnings: true }))) as { audio: number[]; warnings: string[] };
      if (said.warnings.length) opts.onWarnings(said.warnings);
      return Float32Array.from(said.audio);
    },
    script(text) {
      return run('script', { text });
    },
    utterances(text) {
      return run('utterances', { text });
    },
    planPatch(patch, seed = 0, rate) {
      return run('speechPlan', { patch: typeof patch === 'string' ? patch : JSON.stringify(patch), seed, ...(rate ? { rate } : {}) });
    },
    nextPart() {
      const step = run('speechNext', {});
      if (!step) return null;
      const pcm = run('clipPart', { key: step.key, index: step.index });
      return { key: step.key, rate: step.rate, pcm, part: step.index, last: step.last, ...(step.error ? { error: step.error } : {}) };
    },
    phonemize(text) {
      const call = exports.haa_dizione_phonemize;
      if (!call) throw new Error('[haa] speech: this haa.wasm has no phonemizer (feature dizione)');
      const n = withBytes(encodeUtf8(JSON.stringify({ text })), (ptr, len) => call(ptr, len));
      const value = JSON.parse(decodeUtf8(resultBytes(n))) as Phonemized & { error?: string };
      if (value.error) throw new Error(`[haa] speech: ${value.error}`);
      return value;
    },
    voiceToParams(voice) {
      return run('voiceToParams', { voice });
    },
    paramsToVoice(params) {
      return run('paramsToVoice', { params });
    },
    run,
  };
}

/** Download progress of the weights; `total` is 0 when the server sent no length. */
export interface WeightsProgress {
  loaded: number;
  total: number;
  /** True when the bytes came from the Cache API. */
  cached: boolean;
}

export interface FetchWeightsOptions {
  /** Default TINY_VOCA_URL. */
  url?: string;
  /** Cache API bucket (default TINY_VOCA_CACHE); false = no cache. Ignored where the Cache API is missing. */
  cache?: string | false;
  onProgress?: (progress: WeightsProgress) => void;
  signal?: AbortSignal;
}

interface CacheLike {
  match(url: string): Promise<{ arrayBuffer(): Promise<ArrayBuffer> } | undefined>;
  put(url: string, response: Response): Promise<void>;
  delete(url: string): Promise<boolean>;
}

function cacheStorage(): { open(name: string): Promise<CacheLike> } | null {
  const store = (globalThis as { caches?: { open(name: string): Promise<CacheLike> } }).caches;
  return store && typeof store.open === 'function' ? store : null;
}

async function openCache(name: string | false | undefined): Promise<CacheLike | null> {
  if (name === false) return null;
  const store = cacheStorage();
  if (!store) return null;
  try {
    return await store.open(name ?? TINY_VOCA_CACHE);
  } catch {
    return null; // opaque origin, storage disabled: download every time
  }
}

/**
 * The tiny-voca GGUF: from the Cache API when it holds it, else downloaded (with progress) and cached. The
 * default URL's file must have its known size; a wrong cached copy is dropped and fetched again.
 */
export async function fetchTinyVoca(o: FetchWeightsOptions = {}): Promise<Uint8Array> {
  const url = o.url ?? TINY_VOCA_URL;
  const expected = url === TINY_VOCA_URL ? TINY_VOCA_BYTES : 0;
  const cache = await openCache(o.cache);
  if (cache) {
    try {
      const hit = await cache.match(url);
      if (hit) {
        const bytes = new Uint8Array(await hit.arrayBuffer());
        if (!expected || bytes.length === expected) {
          o.onProgress?.({ loaded: bytes.length, total: bytes.length, cached: true });
          return bytes;
        }
        await cache.delete(url);
      }
    } catch {
      // an unreadable entry: download again
    }
  }
  const response = await fetch(url, { signal: o.signal });
  if (!response.ok) throw new Error(`[haa] tiny-voca weights: HTTP ${response.status} for ${url}`);
  const total = Number(response.headers.get('content-length')) || expected;
  let bytes: Uint8Array;
  if (response.body) {
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let loaded = 0;
    o.onProgress?.({ loaded, total, cached: false });
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      loaded += value.length;
      o.onProgress?.({ loaded, total, cached: false });
    }
    bytes = new Uint8Array(loaded);
    let at = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, at);
      at += chunk.length;
    }
  } else {
    bytes = new Uint8Array(await response.arrayBuffer());
    o.onProgress?.({ loaded: bytes.length, total: bytes.length, cached: false });
  }
  if (expected && bytes.length !== expected) {
    throw new Error(`[haa] tiny-voca weights: got ${bytes.length} bytes, expected ${expected} (${url})`);
  }
  if (cache) {
    try {
      const copy = new Uint8Array(bytes.length);
      copy.set(bytes);
      await cache.put(url, new Response(copy.buffer, { headers: { 'content-type': 'application/octet-stream' } }));
    } catch {
      // quota or private mode: the weights stay for this session only
    }
  }
  return bytes;
}

/** Forgets the cached weights (e.g. after a load error). */
export async function clearTinyVocaCache(o: { url?: string; cache?: string } = {}): Promise<void> {
  const cache = await openCache(o.cache);
  await cache?.delete(o.url ?? TINY_VOCA_URL).catch(() => false);
}

/** Whether a patch (object, JSON text or UTF-8 bytes) has a `speak` node with something to say (text or input). */
export function patchSpeaks(patch: unknown): boolean {
  let value = patch;
  try {
    if (value instanceof Uint8Array) value = decodeUtf8(value);
    if (typeof value === 'string') value = JSON.parse(value);
  } catch {
    return false; // not JSON: the patch cannot play either
  }
  const shaped = value as { graph?: { nodes?: unknown }; nodes?: unknown } | null;
  const nodes = shaped?.graph?.nodes ?? shaped?.nodes; // a HAA patch or Sona project, or a bare Sona graph
  if (!Array.isArray(nodes)) return false;
  return nodes.some((node: { type?: unknown; params?: Record<string, unknown> } | null) => node?.type === 'speak'
    && [node.params?.text, node.params?.input].some((v) => typeof v === 'string' && v.trim() !== ''));
}

/** What a patch's speech depends on besides the seed: its `speak` nodes ('' for a patch without one). */
export function speechSignature(patch: unknown): string {
  if (!patchSpeaks(patch)) return '';
  let value = patch;
  if (value instanceof Uint8Array) value = decodeUtf8(value);
  if (typeof value === 'string') value = JSON.parse(value);
  const shaped = value as { graph?: { nodes?: unknown[] }; nodes?: unknown[] };
  const nodes = (shaped.graph?.nodes ?? shaped.nodes ?? []) as { id?: unknown; type?: unknown; params?: unknown; muted?: unknown }[];
  return JSON.stringify(nodes.filter((n) => n?.type === 'speak').map((n) => [n.id, n.params, n.muted ?? false]));
}

/** The patch as the JSON text `planPatch` takes. */
function patchText(patch: unknown): string {
  if (patch instanceof Uint8Array) return decodeUtf8(patch);
  return typeof patch === 'string' ? patch : JSON.stringify(patch);
}

/**
 * Every part of a patch's speech, rendered now with an already loaded `speech` (`planPatch`, then `nextPart` until
 * null), at `rate` (the render's sample rate; default the model's 24 kHz).
 */
export function collectPatchSpeech(speech: Speech, patch: unknown, seed = 0, rate?: number): SpeechClip[] {
  speech.planPatch(patchText(patch), seed, rate);
  const clips: SpeechClip[] = [];
  for (let next = speech.nextPart(); next; next = speech.nextPart()) {
    clips.push({ key: next.key, rate: next.rate, pcm: next.pcm, part: next.part, last: next.last });
  }
  return clips;
}

/** Options of `patchSpeechClips`: the weight fetch options, or the weights themselves. */
export interface PatchSpeechOptions extends FetchWeightsOptions {
  seed?: number;
  /** The render's sample rate (offline: `RenderOptions.sampleRate`, 48 kHz by default). */
  rate?: number;
  /** The tiny-voca GGUF already in hand (e.g. read from disk in Node); otherwise `fetchTinyVoca(options)`. */
  gguf?: Uint8Array;
}

/**
 * The speech an offline render of `patch` needs (`renderPatch(module, patch, { speechClips })`), realm-neutral (Node,
 * Worker). Empty, with nothing fetched or instantiated, for a patch without a `speak` node; otherwise the weights are
 * fetched (or `o.gguf` is used), loaded into a new haa.wasm instance and every part is rendered.
 */
export async function patchSpeechClips(
  wasm: BufferSource | WebAssembly.Module,
  patch: unknown,
  o: PatchSpeechOptions = {},
): Promise<SpeechClip[]> {
  if (!patchSpeaks(patch)) return [];
  const [gguf, speech] = await Promise.all([o.gguf ?? fetchTinyVoca(o), createSpeech(wasm)]);
  const loaded = speech.load(gguf);
  if (!loaded.phonemizer) throw new Error(`[haa] speech: the weights have no phonemizer: ${loaded.phonemizerError ?? 'unknown'}`);
  return collectPatchSpeech(speech, patch, o.seed ?? 0, o.rate);
}
