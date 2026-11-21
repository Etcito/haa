// HAA voice host: the multivoice student in a browser or a worker. Its four ONNX graphs (voice, encode, prosody,
// decode) run in the ONNX runtime the caller hands in (onnxruntime-web, WebGPU first; CPU wasm only as a visible
// fallback); everything between them (text and tags, emotion inputs, seeded noise, expand by durations, the graph's
// curve overrides, resampling) runs in haa.wasm (crates/haa-wasm/src/multivoice.rs, haa_core::multivoice). This
// file is glue: it moves tensors between the runtime and the wasm slots.
//
//   const exports = (await WebAssembly.instantiate(haaWasmModule, {})).exports;
//   loadPhonemizer(exports, await fetchTinyVoca());            // the dizione tokenizer data (tiny-voca GGUF)
//   const version = await fetchMultivoiceVersion();             // the current weights (https://haa-voice.vercel.app)
//   const voice = await createMultivoice({ exports, ort, version, weights: await fetchMultivoiceWeights(version) });
//   voice.plan(patchJson, 0, 48000);                            // the patch's multivoice nodes
//   for (let p; (p = await voice.next()); ) player.setSpeechClips([p]);   // one part (sentence) at a time
//
// Weights are never bundled: version.json names the set (`path`), the graphs are fetched at run time and kept in
// the Cache API. A version.json without `io` is the first interface (emotion ids + intensity); `io: 2` takes emotion
// mix vectors of `emotion_dim` and a per-frame decode noise_scale.
import { fetchTinyVoca, type WeightsProgress } from './speech.ts';
import { decodeUtf8, encodeUtf8 } from './utf8.ts';

export const MULTIVOICE_ORIGIN = 'https://haa-voice.vercel.app';
export const MULTIVOICE_CACHE = 'haa-multivoice';
export const MULTIVOICE_GRAPHS = ['voice', 'encode', 'prosody', 'decode'] as const;
export type MultivoiceGraph = (typeof MULTIVOICE_GRAPHS)[number];

/** version.json of a weights site. */
export interface MultivoiceVersion {
  stamp: string;
  /** The folder of the four graphs, relative to the site (`w/<stamp>/`). */
  path: string;
  step?: number;
  time?: string;
  files?: Partial<Record<MultivoiceGraph, { bytes?: number }>>;
  mb?: number;
  /** Absent = 1. */
  io?: number;
  emotion_dim?: number;
}

/** The haa.wasm exports this host uses. */
export interface MultivoiceExports {
  memory: WebAssembly.Memory;
  haa_alloc(len: number): number;
  haa_free(ptr: number, len: number): void;
  haa_result_ptr(): number;
  haa_result_len(): number;
  haa_mv_run(ptr: number, len: number): number;
  haa_mv_put(namePtr: number, nameLen: number, dataPtr: number, dataLen: number): number;
  haa_mv_get(namePtr: number, nameLen: number): number;
  haa_dizione_gguf?(ptr: number, len: number): number;
}

/** The slice of onnxruntime-web this host uses. */
export interface OrtTensorLike {
  readonly data: unknown;
  readonly dims: readonly number[];
}
export interface OrtSessionLike {
  readonly inputNames: readonly string[];
  run(feeds: Record<string, OrtTensorLike>): Promise<Record<string, OrtTensorLike>>;
  release(): Promise<void>;
}
export interface OrtLike {
  Tensor: new (type: 'float32' | 'int64', data: Float32Array | BigInt64Array, dims: readonly number[]) => OrtTensorLike;
  InferenceSession: { create(model: Uint8Array, options?: Record<string, unknown>): Promise<OrtSessionLike> };
}

/** One rendered part (a sentence) of a source, at the rate the plan asked for. */
export interface MultivoicePart {
  key: string;
  part: number;
  last: boolean;
  rate: number;
  pcm: Float32Array;
  /** Why the source stopped here (its part is then silent). */
  error?: string;
  /** Overrides that replaced predicted curves in this part. */
  replaced?: string[];
}

export interface MultivoicePlanned {
  key: string;
  parts: number;
  complete: boolean;
}

export interface MultivoiceTags {
  events: string[];
  eventAliases: Array<[string, string]>;
  emotions: string[];
}

/** Installs the dizione phonemizer (from the tiny-voca one-file GGUF) in the instance; throws when it cannot. */
export function loadPhonemizer(exports: MultivoiceExports, gguf: Uint8Array): void {
  if (!exports.haa_dizione_gguf) throw new Error('[haa] multivoice: this haa.wasm has no phonemizer (feature dizione)');
  const ptr = exports.haa_alloc(gguf.length);
  try {
    new Uint8Array(exports.memory.buffer, ptr, gguf.length).set(gguf);
    if (!exports.haa_dizione_gguf(ptr, gguf.length)) throw new Error('[haa] multivoice: the tokenizer data did not load');
  } finally {
    exports.haa_free(ptr, gguf.length);
  }
}

/** The weights site's current version.json. */
export async function fetchMultivoiceVersion(origin = MULTIVOICE_ORIGIN, signal?: AbortSignal): Promise<MultivoiceVersion> {
  const r = await fetch(`${origin}/version.json?t=${Date.now()}`, { cache: 'no-store', signal });
  if (!r.ok) throw new Error(`[haa] multivoice: version.json: HTTP ${r.status}`);
  const v = (await r.json()) as MultivoiceVersion;
  if (typeof v.path !== 'string' || typeof v.stamp !== 'string') throw new Error('[haa] multivoice: version.json names no weights');
  return v;
}

export interface FetchMultivoiceOptions {
  origin?: string;
  /** Cache API bucket (default MULTIVOICE_CACHE); false = no cache. */
  cache?: string | false;
  /** Progress over the four graphs together. */
  onProgress?: (progress: WeightsProgress) => void;
  signal?: AbortSignal;
}

/** The four graphs of a version (Cache API first; older sets are dropped from the cache once this one is in). */
export async function fetchMultivoiceWeights(version: MultivoiceVersion, o: FetchMultivoiceOptions = {}): Promise<Record<MultivoiceGraph, Uint8Array>> {
  const origin = o.origin ?? MULTIVOICE_ORIGIN;
  const base = new URL(version.path, `${origin}/`).href;
  const sizes = MULTIVOICE_GRAPHS.map((n) => version.files?.[n]?.bytes ?? 0);
  const loaded = MULTIVOICE_GRAPHS.map(() => 0);
  const cached = MULTIVOICE_GRAPHS.map(() => true);
  const total = sizes.reduce((a, b) => a + b, 0);
  const report = () => o.onProgress?.({ loaded: loaded.reduce((a, b) => a + b, 0), total, cached: cached.every(Boolean) });
  const cache = o.cache === false ? false : (o.cache ?? MULTIVOICE_CACHE);
  const out = await Promise.all(MULTIVOICE_GRAPHS.map(async (name, i) => {
    const bytes = await fetchTinyVoca({
      url: `${base}${name}.onnx`,
      cache,
      signal: o.signal,
      onProgress: (p) => {
        loaded[i] = p.loaded;
        cached[i] = p.cached;
        report();
      },
    });
    if (sizes[i] && bytes.length !== sizes[i]) throw new Error(`[haa] multivoice: ${name}.onnx has ${bytes.length} bytes, version.json says ${sizes[i]}`);
    return bytes;
  }));
  if (cache !== false && typeof caches !== 'undefined') {
    try {
      const bucket = await caches.open(cache);
      for (const request of await bucket.keys()) if (!request.url.startsWith(base)) await bucket.delete(request);
    } catch {
      // no cache access: nothing to tidy
    }
  }
  return Object.fromEntries(MULTIVOICE_GRAPHS.map((n, i) => [n, out[i]])) as Record<MultivoiceGraph, Uint8Array>;
}

export interface CreateMultivoiceOptions {
  /** A haa.wasm instance with the phonemizer loaded (loadPhonemizer). */
  exports: MultivoiceExports;
  ort: OrtLike;
  version: MultivoiceVersion;
  weights: Record<MultivoiceGraph, Uint8Array>;
  /** Execution provider: 'webgpu' (default; falls back to 'wasm' when a graph cannot run there) or 'wasm'. */
  ep?: 'webgpu' | 'wasm';
  /** Extra session options (e.g. graphOptimizationLevel). */
  sessionOptions?: Record<string, unknown>;
  /** Warm the graphs up with a short line once loaded (default true). */
  warm?: boolean;
}

export interface Multivoice {
  readonly version: MultivoiceVersion;
  /** The provider the graphs run on, and why it is the CPU when it is. */
  readonly ep: 'webgpu' | 'wasm';
  readonly epNote: string;
  readonly io: number;
  readonly emotionDim: number;
  /** Queues the speech of every `multivoice` node of a patch (JSON) for voice seed `seed`, parts at `rate`. */
  plan(patch: string, seed: number, rate: number): MultivoicePlanned[];
  /** Renders (or hands back, when kept) the next queued part; null when nothing is queued. One call at a time. */
  next(): Promise<MultivoicePart | null>;
  clear(): void;
  tags(): MultivoiceTags;
  release(): Promise<void>;
}

/** Loads the four graphs (WebGPU first) and returns the host; see the file comment. */
export async function createMultivoice(o: CreateMultivoiceOptions): Promise<Multivoice> {
  const e = o.exports;
  const { Tensor } = o.ort;
  let ep: 'webgpu' | 'wasm' = o.ep ?? 'webgpu';
  let epNote = ep === 'wasm' ? 'CPU chosen' : '';
  const with_ = <T>(bytes: Uint8Array, fn: (ptr: number, len: number) => T): T => {
    const ptr = e.haa_alloc(bytes.length);
    try {
      if (bytes.length) new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
      return fn(ptr, bytes.length);
    } finally {
      e.haa_free(ptr, bytes.length);
    }
  };
  const result = (len: number): Uint8Array => new Uint8Array(e.memory.buffer, e.haa_result_ptr(), len).slice();
  const run = <T>(request: Record<string, unknown>): T => {
    const n = with_(encodeUtf8(JSON.stringify(request)), (p, l) => e.haa_mv_run(p, l));
    const text = decodeUtf8(result(n < 0 ? e.haa_result_len() : n));
    if (n < 0) throw new Error(`[haa] multivoice: ${(JSON.parse(text) as { error: string }).error}`);
    return JSON.parse(text) as T;
  };
  const put = (name: string, data: Float32Array | BigInt64Array) => {
    const bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    const nameBytes = encodeUtf8(name);
    with_(nameBytes, (np, nl) => with_(bytes, (dp, dl) => e.haa_mv_put(np, nl, dp, dl)));
  };
  const get = (name: string): Uint8Array => {
    const n = with_(encodeUtf8(name), (p, l) => e.haa_mv_get(p, l));
    if (n < 0) throw new Error(`[haa] multivoice: slot ${name} is empty`);
    return result(n);
  };
  const f32 = (name: string) => { const b = get(name); return new Float32Array(b.buffer, b.byteOffset, b.byteLength / 4); };
  const i64 = (name: string) => { const b = get(name); return new BigInt64Array(b.buffer, b.byteOffset, b.byteLength / 8); };
  const T = (type: 'float32' | 'int64', data: Float32Array | BigInt64Array, dims: number[]) => new Tensor(type, data, dims);

  const sessions = {} as Record<MultivoiceGraph, OrtSessionLike>;
  const options = () => ({ executionProviders: [ep], graphOptimizationLevel: 'all', ...o.sessionOptions });
  for (const name of MULTIVOICE_GRAPHS) {
    try {
      sessions[name] = await o.ort.InferenceSession.create(o.weights[name], options());
    } catch (error) {
      if (ep !== 'webgpu') throw error;
      ep = 'wasm';
      epNote = `WebGPU could not run the model (${error instanceof Error ? error.message : String(error)})`;
      for (const s of Object.values(sessions)) await s.release().catch(() => undefined);
      for (const done of MULTIVOICE_GRAPHS) {
        sessions[done] = await o.ort.InferenceSession.create(o.weights[done], options());
        if (done === name) break;
      }
    }
  }
  // the interface: version.json says it; the encoder's inputs confirm it (io 1 has `intensity`)
  const io = sessions.encode.inputNames.includes('intensity') ? 1 : Math.max(2, o.version.io ?? 2);
  const emotionDim = io >= 2 ? (o.version.emotion_dim ?? 17) : 16;
  run({ op: 'interface', io, emotionDim });
  let rate = 48_000;

  /** The emotion feeds of a stage: io 1 = ids + intensity, io 2 = the mix vectors. */
  const emotionFeeds = (e_: string, i_: string, n: number): Record<string, OrtTensorLike> => io >= 2
    ? { emotion: T('float32', f32(e_), [1, n, emotionDim]) }
    : { emotion: T('int64', i64(e_), [1, n]), intensity: T('float32', f32(i_), [1, n]) };

  async function render(step: { key: string; index: number; last: boolean; t: number }): Promise<MultivoicePart> {
    const t = step.t;
    const V = await sessions.voice.run({ attrs: T('float32', f32('attrs'), [1, 3]), z: T('float32', f32('z'), [1, 8]) });
    const spk = V.spk_emb;
    const quality = T('float32', f32('quality'), [1, 3]);
    const E = await sessions.encode.run({
      input: T('int64', i64('input'), [1, t]),
      input_lengths: T('int64', BigInt64Array.of(BigInt(t)), [1]),
      stress: T('int64', i64('stress'), [1, t]),
      spk_emb: spk,
      quality,
      ...emotionFeeds('emotion', 'intensity', t),
      scales: T('float32', f32('scales'), [2]),
      dp_noise: T('float32', f32('dp_noise'), [1, 2, t]),
    });
    for (const n of ['durations', 'h', 'm_p', 'logs_p']) put(n, E[n].data as Float32Array);
    const { frames } = run<{ frames: number }>({ op: 'expand' });
    let replaced: string[] = [];
    if (frames > 0) {
      const P = await sessions.prosody.run({ h: T('float32', f32('h_f'), [1, 192, frames]), ...emotionFeeds('e_f', 'i_f', frames), spk_emb: spk, quality });
      for (const n of ['log_f0', 'voiced', 'energy']) put(n, P[n].data as Float32Array);
      replaced = run<{ replaced: string[] }>({ op: 'curves' }).replaced;
      run({ op: 'noise' });
      const D = await sessions.decode.run({
        m_p: T('float32', f32('m_f'), [1, 192, frames]),
        logs_p: T('float32', f32('l_f'), [1, 192, frames]),
        log_f0: T('float32', f32('log_f0'), [1, frames]),
        voiced: T('float32', f32('voiced'), [1, frames]),
        energy: T('float32', f32('energy'), [1, frames]),
        ...emotionFeeds('e_f', 'i_f', frames),
        spk_emb: spk,
        quality,
        noise_scale: io >= 2 ? T('float32', f32('noise_scale'), [1, 1, frames]) : T('float32', f32('noise_scale'), [1]),
        z_noise: T('float32', f32('z_noise'), [1, 192, frames]),
        src_noise: T('float32', f32('src_noise'), [1, 1, 256 * frames]),
      });
      put('audio', D.audio.data as Float32Array);
    } else {
      put('audio', new Float32Array(0));
    }
    const done = run<{ key: string; index: number; last: boolean; rate: number }>({ op: 'finish' });
    return { key: done.key, part: done.index, last: done.last, rate: done.rate, pcm: f32('pcm').slice(), replaced };
  }

  type Next = null | { key: string; index: number; last: boolean; kept?: boolean; failed?: boolean; error?: string; rate?: number; t?: number };
  const host: Multivoice = {
    version: o.version,
    get ep() { return ep; },
    get epNote() { return epNote; },
    io,
    emotionDim,
    plan(patch, seed, r) {
      rate = r;
      return run<MultivoicePlanned[]>({ op: 'plan', patch, seed: seed >>> 0, rate: r });
    },
    async next() {
      const s = run<Next>({ op: 'next' });
      if (!s) return null;
      if (s.failed) return { key: s.key, part: s.index, last: true, rate, pcm: new Float32Array(0), error: s.error };
      if (s.kept) {
        const c = run<{ rate: number }>({ op: 'clipPart', key: s.key, index: s.index });
        return { key: s.key, part: s.index, last: s.last, rate: c.rate, pcm: f32('pcm').slice() };
      }
      return render({ key: s.key, index: s.index, last: s.last, t: s.t ?? 0 });
    },
    clear() { run({ op: 'clear' }); },
    tags() { return run<MultivoiceTags>({ op: 'tags' }); },
    async release() { for (const s of Object.values(sessions)) await s.release().catch(() => undefined); },
  };
  if (o.warm !== false) {
    const warm = { haa: 1, id: 'multivoice-warm', title: 'warm', graph: { duration: 1, nodes: [
      { id: 'v', type: 'multivoice', x: 0, y: 0, label: '', params: { text: 'Hello.' } },
      { id: 'o', type: 'output', x: 0, y: 0, label: '', params: {} },
    ], connections: [{ id: 'c', from: 'v', to: 'o', target: 'audio', amount: 1 }] } };
    host.plan(JSON.stringify(warm), 0, rate);
    while (await host.next());
  }
  return host;
}
