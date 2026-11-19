// The speech worker's message loop (bundled as `@etcito/haa/speech-worker`, see speech-worker.ts): it fetches the
// tiny-voca weights (pinned Hugging Face URL, Cache API), loads them into its own haa.wasm instance, then for each
// `stream` request queues the patch's speech (`planPatch`) and renders it part by part (`nextPart`), posting each
// part as soon as it is ready. Everything that computes runs in Rust; this file only moves messages and buffers.
// The scope and the speech factory are parameters so the loop can run outside a worker (tests, custom hosts).

import { createSpeech, fetchTinyVoca, type FetchWeightsOptions, type Speech } from './speech.ts';
import type { SpeechPart, SpeechWorkerReply, SpeechWorkerRequest } from './speech-protocol.ts';

/** The slice of a worker's `self` the loop uses. */
export interface SpeechWorkerScope {
  onmessage: ((event: { data: SpeechWorkerRequest }) => void) | null;
  postMessage(message: SpeechWorkerReply, transfer?: Transferable[]): void;
}

/** What the loop calls to get the weights and the speech instance (defaults: fetchTinyVoca, createSpeech). */
export interface SpeechServeDeps {
  fetchWeights?: (o: FetchWeightsOptions) => Promise<Uint8Array>;
  createSpeech?: (module: WebAssembly.Module) => Promise<Speech>;
}

const reason = (error: unknown): string => (error instanceof Error ? error.message : String(error)).replace(/^\[haa\] /, '');

/** Serve speech requests on `scope` until the worker ends. */
export function serveSpeech(scope: SpeechWorkerScope, deps: SpeechServeDeps = {}): void {
  const fetchWeights = deps.fetchWeights ?? fetchTinyVoca;
  const makeSpeech = deps.createSpeech ?? createSpeech;
  let loading: Promise<Speech> | null = null;
  let pumping = false;

  async function load(init: Extract<SpeechWorkerRequest, { t: 'init' }>): Promise<Speech> {
    let stage: 'download' | 'load' = 'download';
    try {
      const [gguf, speech] = await Promise.all([
        fetchWeights({ url: init.url, cache: init.cache, onProgress: (p) => scope.postMessage({ t: 'progress', ...p }) }),
        makeSpeech(init.module),
      ]);
      stage = 'load';
      const loaded = speech.load(gguf);
      if (!loaded.phonemizer) throw new Error(`the weights have no phonemizer: ${loaded.phonemizerError ?? 'unknown'}`);
      scope.postMessage({ t: 'ready' });
      return speech;
    } catch (error) {
      scope.postMessage({ t: 'failed', stage, message: reason(error) });
      throw error;
    }
  }

  /** Render the queued speech part by part, yielding between parts so new requests get in. */
  async function pump(speech: Speech): Promise<void> {
    if (pumping) return;
    pumping = true;
    try {
      for (;;) {
        const next = speech.nextPart();
        if (!next) break;
        if (next.error) console.warn('[haa] speech:', next.error);
        const clip: SpeechPart = { key: next.key, rate: next.rate, pcm: next.pcm, part: next.part, last: next.last };
        scope.postMessage({ t: 'part', clip }, [clip.pcm.buffer]);
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    } finally {
      pumping = false;
    }
  }

  async function handle(request: SpeechWorkerRequest): Promise<void> {
    if (request.t === 'init') {
      loading ??= load(request);
      await loading.catch(() => undefined);
      return;
    }
    try {
      if (!loading) throw new Error('speech worker: no init message');
      const speech = await loading;
      const sources = speech.planPatch(request.patch, request.seed, request.rate);
      scope.postMessage({ t: 'planned', id: request.id, sources });
      void pump(speech).catch((error) => console.warn('[haa] speech:', reason(error)));
    } catch (error) {
      scope.postMessage({ t: 'error', id: request.id, message: reason(error) });
    }
  }

  scope.onmessage = (event) => {
    void handle(event.data);
  };
}
