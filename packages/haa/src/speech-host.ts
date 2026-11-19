// Main-thread speech for the browser player (`createPlayer({ speech })`, ARCHITECTURE §9): one shared module
// Worker (`@etcito/haa/speech-worker`) holds tiny-voca and renders a patch's speech one utterance at a time; this
// file starts it, routes the parts it posts to the players that wait for them and reports the weights' progress.
// A patch without a `speak` node never reaches it: nothing is fetched and no worker starts. Plumbing only: the
// worker's wasm does every computation.

import { patchSpeaks, TINY_VOCA_URL, type WeightsProgress } from './speech.ts';
import type { SpeechPart, SpeechWorkerReply, SpeechWorkerRequest } from './speech-protocol.ts';
import type { HaaPatch, SpeechClip } from './types.ts';
import { decodeUtf8 } from './utf8.ts';

export type { SpeechPart } from './speech-protocol.ts';
export type { WeightsProgress } from './speech.ts';

/** The slice of a `Worker` the host uses. */
export interface SpeechWorkerLike {
  postMessage(message: SpeechWorkerRequest): void;
  onmessage: ((event: MessageEvent<SpeechWorkerReply>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate(): void;
}

/** How the player gets its speech (`createPlayer({ speech })`). Every field is optional. */
export interface SpeechOptions {
  /**
   * A new speech worker, e.g. Vite's `new SpeechWorker()` (`import SpeechWorker from '@etcito/haa/speech-worker?worker'`).
   * Wins over `workerUrl`.
   */
  worker?: () => SpeechWorkerLike;
  /**
   * URL of `dist/speech-worker.js` (`@etcito/haa/speech-worker`), started as a module worker. Default: the file next
   * to this bundle (`new URL('./speech-worker.js', import.meta.url)`), which bundlers that pre-bundle dependencies
   * may not serve: pass the URL (e.g. Vite's `@etcito/haa/speech-worker?url`) or `worker`.
   */
  workerUrl?: string | URL;
  /** Where the tiny-voca weights come from (default the pinned Hugging Face file, `TINY_VOCA_URL`). */
  weightsUrl?: string;
  /** Cache API bucket for the weights (default `TINY_VOCA_CACHE`); false = download every time. */
  cache?: string | false;
  /** Weights download progress (8 MB the first time, then from the Cache API). */
  onProgress?: (progress: WeightsProgress) => void;
  /** The speech could not load or render: the patch plays without it. Default: one console warning. */
  onError?: (error: Error) => void;
}

/** What receives speech parts: a player (`HaaPlayer.setSpeechClips`). */
export interface SpeechTarget {
  setSpeechClips(clips: SpeechClip[]): void;
}

/** A patch's speech as it arrives from the worker. */
export interface SpeechFeed {
  /** The source keys the patch's `speak` nodes play. */
  readonly keys: string[];
  /** The parts received and not handed on yet (the player's `speechClips`); empties the buffer. */
  take(): SpeechClip[];
  /** Hands the buffered parts to `target`, then every later part as it arrives. */
  attach(target: SpeechTarget): void;
  /** Resolves once every source is complete. */
  done(): Promise<void>;
  /** Stops following the parts (the player was disposed or got another patch). */
  stop(): void;
}

type Pending = {
  onPart: (part: SpeechPart) => void;
  resolve: (keys: string[], complete: (key: string) => boolean) => void;
  reject: (error: Error) => void;
};

/** One speech worker and its weights, shared by every player that uses the same worker and weights. */
class SpeechHost {
  private readonly worker: SpeechWorkerLike;
  private readonly progress = new Set<(p: WeightsProgress) => void>();
  private readonly pending = new Map<number, Pending>();
  private readonly routes = new Map<string, Set<(part: SpeechPart) => void>>();
  private readonly failures = new Set<(error: Error) => void>();
  private nextId = 1;
  private failed: Error | null = null;
  readonly loaded: Promise<void>;
  private settleLoaded: (error?: Error) => void = () => {};
  private readonly forget: () => void;

  constructor(module: WebAssembly.Module, o: SpeechOptions, forget: () => void) {
    this.forget = forget;
    this.loaded = new Promise<void>((resolve, reject) => {
      this.settleLoaded = (error) => (error ? reject(error) : resolve());
    });
    this.loaded.catch(() => {});
    this.worker = o.worker
      ? o.worker()
      : (new Worker(o.workerUrl ?? new URL('./speech-worker.js', import.meta.url), { type: 'module' }) as unknown as SpeechWorkerLike);
    this.worker.onmessage = (event) => this.onReply(event.data);
    this.worker.onerror = (event) => this.fail(new Error(`[haa] speech worker: ${event?.message || 'failed to start'}`));
    this.worker.postMessage({ t: 'init', module, url: o.weightsUrl, cache: o.cache });
  }

  /** Follow the weights' progress until `remove` is called. */
  onProgress(cb: ((p: WeightsProgress) => void) | undefined): () => void {
    if (!cb) return () => {};
    this.progress.add(cb);
    return () => this.progress.delete(cb);
  }

  /** Queue a patch's speech; `onPart` gets each of its parts (registered before the first one can arrive). */
  stream(patch: string, seed: number, rate: number, onPart: (part: SpeechPart) => void): Promise<{ keys: string[]; complete: (key: string) => boolean; stop(): void }> {
    if (this.failed) return Promise.reject(this.failed);
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, {
        onPart,
        reject,
        resolve: (keys, complete) => resolve({ keys, complete, stop: () => this.unroute(keys, onPart) }),
      });
      this.worker.postMessage({ t: 'stream', id, patch, seed, rate });
    });
  }

  /** Call `cb` if the worker fails from now on; returns the remover. */
  onFailure(cb: (error: Error) => void): () => void {
    if (this.failed) {
      cb(this.failed);
      return () => {};
    }
    this.failures.add(cb);
    return () => this.failures.delete(cb);
  }

  terminate(): void {
    this.fail(new Error('[haa] speech closed'));
  }

  private unroute(keys: string[], onPart: (part: SpeechPart) => void): void {
    for (const key of keys) {
      const set = this.routes.get(key);
      set?.delete(onPart);
      if (set?.size === 0) this.routes.delete(key);
    }
  }

  private onReply(reply: SpeechWorkerReply): void {
    switch (reply.t) {
      case 'progress':
        for (const cb of [...this.progress]) {
          try {
            cb({ loaded: reply.loaded, total: reply.total, cached: reply.cached });
          } catch {
            // a listener's failure does not stop the load
          }
        }
        return;
      case 'ready':
        this.settleLoaded();
        return;
      case 'failed':
        this.fail(new Error(`[haa] speech ${reply.stage}: ${reply.message}`));
        return;
      case 'planned': {
        const p = this.pending.get(reply.id);
        if (!p) return;
        this.pending.delete(reply.id);
        const keys = reply.sources.map((s) => s.key);
        // Routed now, before the worker's next message (its first part) is handled.
        for (const key of keys) {
          let set = this.routes.get(key);
          if (!set) this.routes.set(key, (set = new Set()));
          set.add(p.onPart);
        }
        // A source already complete with no part has nothing to wait for.
        const empty = new Set(reply.sources.filter((s) => s.complete && s.parts === 0).map((s) => s.key));
        p.resolve(keys, (key) => empty.has(key));
        return;
      }
      case 'part':
        for (const cb of [...(this.routes.get(reply.clip.key) ?? [])]) cb(reply.clip);
        return;
      case 'error': {
        const p = this.pending.get(reply.id);
        if (!p) return;
        this.pending.delete(reply.id);
        p.reject(new Error(`[haa] speech: ${reply.message}`));
        return;
      }
    }
  }

  private fail(error: Error): void {
    if (this.failed) return;
    this.failed = error;
    this.forget();
    this.settleLoaded(error);
    for (const p of this.pending.values()) p.reject(error);
    this.pending.clear();
    for (const cb of [...this.failures]) cb(error);
    this.failures.clear();
    this.routes.clear();
    this.progress.clear();
    try {
      this.worker.terminate();
    } catch {
      // already gone
    }
  }
}

/** The shared hosts, per worker and weights; a failed host is dropped so the next player tries again. */
const HOSTS = new Map<string, SpeechHost>();
const factoryIds = new WeakMap<() => SpeechWorkerLike, number>();
let nextFactory = 1;

function hostFor(module: WebAssembly.Module, o: SpeechOptions): SpeechHost {
  let worker: string;
  if (o.worker) {
    let id = factoryIds.get(o.worker);
    if (id === undefined) factoryIds.set(o.worker, (id = nextFactory++));
    worker = `factory:${id}`;
  } else {
    worker = String(o.workerUrl ?? 'default');
  }
  const key = JSON.stringify([worker, o.weightsUrl ?? TINY_VOCA_URL, o.cache ?? null]);
  let host = HOSTS.get(key);
  if (!host) {
    host = new SpeechHost(module, o, () => {
      if (HOSTS.get(key) === host) HOSTS.delete(key);
    });
    HOSTS.set(key, host);
  }
  return host;
}

/** Ends every speech worker (the weights are dropped from memory, not from the Cache API). */
export function closeSpeech(): void {
  for (const host of [...HOSTS.values()]) host.terminate();
  HOSTS.clear();
}

/** The patch as JSON text. */
function patchJson(patch: HaaPatch | string | Uint8Array): string {
  if (patch instanceof Uint8Array) return decodeUtf8(patch);
  return typeof patch === 'string' ? patch : JSON.stringify(patch);
}

/**
 * Starts the speech of `patch` for voice seed `seed`, its parts at `rate` (the player's sample rate), and resolves
 * once every `speak` source has its first part: the player can start, and the rest streams in while it plays.
 * Resolves null, with nothing fetched and no worker started, for a patch without a `speak` node. Rejects when the
 * weights or the worker cannot load (the player then plays without speech).
 */
export async function prepareSpeech(
  module: WebAssembly.Module,
  patch: HaaPatch | string | Uint8Array,
  o: SpeechOptions & { seed?: number; rate: number },
): Promise<SpeechFeed | null> {
  if (!patchSpeaks(patch)) return null;
  const host = hostFor(module, o);
  const dropProgress = host.onProgress(o.onProgress);
  const seen = new Set<string>();
  const firsts = new Set<string>();
  const lasts = new Set<string>();
  let buffer: SpeechClip[] = [];
  let target: SpeechTarget | null = null;
  let stopped = false;
  const waiters = new Set<() => void>();

  const onPart = (part: SpeechPart): void => {
    const id = `${part.key}#${part.part}`;
    if (stopped || seen.has(id)) return; // a repeat (two plays of one text) is handed on once
    seen.add(id);
    if (part.part === 0) firsts.add(part.key);
    if (part.last) lasts.add(part.key);
    if (target) target.setSpeechClips([part]);
    else buffer.push(part);
    for (const check of [...waiters]) check();
  };

  let planned: Awaited<ReturnType<SpeechHost['stream']>>;
  try {
    await host.loaded;
    planned = await host.stream(patchJson(patch), o.seed ?? 0, o.rate, onPart);
  } finally {
    dropProgress();
  }
  const { keys, complete } = planned;

  const waitFor = (enough: (key: string) => boolean): Promise<void> =>
    new Promise<void>((resolve, reject) => {
      let dropFailure = (): void => {};
      const check = (): void => {
        if (!keys.every((key) => complete(key) || enough(key))) return;
        waiters.delete(check);
        dropFailure();
        resolve();
      };
      waiters.add(check);
      dropFailure = host.onFailure((error) => {
        waiters.delete(check);
        reject(error);
      });
      check();
    });

  const feed: SpeechFeed = {
    keys,
    take() {
      const out = buffer;
      buffer = [];
      return out;
    },
    attach(next) {
      target = next;
      if (buffer.length > 0) next.setSpeechClips(feed.take());
    },
    done: () => waitFor((key) => lasts.has(key)),
    stop() {
      stopped = true;
      target = null;
      buffer = [];
      planned.stop();
    },
  };
  try {
    await waitFor((key) => firsts.has(key));
  } catch (error) {
    feed.stop();
    throw error;
  }
  return feed;
}
