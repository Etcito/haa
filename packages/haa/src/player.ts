// Browser main-thread player: one AudioWorklet node, its message protocol and the patch,
// input, trace and lifecycle API (ARCHITECTURE §9). Browser-only (Web Audio API); the
// processor itself is worklet.ts and the wasm wrapper it shares is engine.ts.

import { patchBytes } from './engine.ts';
import { PLAYER_PROCESSOR } from './types.ts';
import type {
  HaaPatch,
  LoadResult,
  PatchIssue,
  PlayerInMsg,
  PlayerOutMsg,
  PlayerProcessorOptions,
  TracePacket,
  VoiceInfo,
} from './types.ts';

/** Options for `createPlayer`. */
export interface PlayerOptions {
  /** Compiled `haa.wasm`, instantiated synchronously by the processor. */
  module: WebAssembly.Module;
  /** URL of the bundled worklet processor (`dist/worklet.js`). */
  processorUrl: string | URL;
  /** Initial patch or its UTF-8 JSON bytes. */
  patch: HaaPatch | string | Uint8Array;
  /** Voice seed; 0 is Sona's sound. */
  seed?: number;
  /** Initial published input values. */
  inputs?: Record<string, number | string>;
  /** Destination node; the context destination by default. */
  output?: AudioNode;
  /** Start with control traces on. */
  trace?: boolean;
}

/** A live player bound to one AudioWorklet node (ARCHITECTURE §9). */
export interface HaaPlayer {
  readonly node: AudioWorkletNode;
  readonly info: VoiceInfo;
  readonly issues: PatchIssue[];
  readonly playable: boolean;
  set(name: string, value: number | string): void;
  trigger(name: string): void;
  restart(): void;
  setPatch(patch: HaaPatch | string | Uint8Array, keepState?: boolean): Promise<LoadResult>;
  onTrace(cb: (t: TracePacket) => void): () => void;
  onEnded(cb: () => void): () => void;
  dispose(): void;
}

/** One `addModule` per context, shared by every player created on it. */
const MODULES = new WeakMap<BaseAudioContext, Promise<void>>();

/** The only error the player rejects with (ARCHITECTURE §Errors). */
function unavailable(reason: unknown): Error {
  return new Error(`[haa] worklet unavailable: ${reason instanceof Error ? reason.message : String(reason)}`);
}

/** Load the processor bundle once per context, dropping the cache if it fails. */
function ensureProcessor(ctx: BaseAudioContext, processorUrl: string | URL): Promise<void> {
  let pending = MODULES.get(ctx);
  if (!pending) {
    pending = (async () => {
      if (!ctx.audioWorklet) throw new Error('no audioWorklet');
      await ctx.audioWorklet.addModule(processorUrl);
    })();
    MODULES.set(ctx, pending);
    pending.catch(() => {
      MODULES.delete(ctx);
    });
  }
  return pending;
}

/** Create the node, wait for its first `ready` and connect it; failures throw unwrapped. */
async function startPlayer(ctx: BaseAudioContext, o: PlayerOptions): Promise<HaaPlayer> {
  await ensureProcessor(ctx, o.processorUrl);

  const processorOptions: PlayerProcessorOptions = {
    module: o.module,
    patch: patchBytes(o.patch),
    seed: o.seed ?? 0,
    inputs: o.inputs ?? {},
    trace: o.trace ?? false,
  };
  const node = new AudioWorkletNode(ctx, PLAYER_PROCESSOR, {
    numberOfInputs: 0,
    numberOfOutputs: 1,
    outputChannelCount: [2],
    processorOptions,
  });

  let info: VoiceInfo = { nodes: [], inputs: [], mode: 'timeline', duration: 0, loop: false };
  let issues: PatchIssue[] = [];
  let playable = false;
  let disposed = false;
  let readySettled = false;
  let settleReady: () => void = () => {};
  let failReady: (error: Error) => void = () => {};
  const ready = new Promise<void>((resolve, reject) => {
    settleReady = resolve;
    failReady = reject;
  });
  const traceListeners = new Set<(t: TracePacket) => void>();
  const endedListeners = new Set<() => void>();
  const pending = new Map<number, { resolve: (result: LoadResult) => void; reject: (error: Error) => void }>();
  let nextReq = 1;

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    readySettled = true;
    traceListeners.clear();
    endedListeners.clear();
    for (const waiter of pending.values()) waiter.reject(new Error('[haa] player disposed'));
    pending.clear();
    try {
      node.port.postMessage({ t: 'dispose' } satisfies PlayerInMsg);
    } catch {
      // A closed port drops the message.
    }
    try {
      node.port.onmessage = null;
    } catch {
      // Ignore a port that is already gone.
    }
    node.onprocessorerror = null;
    try {
      node.disconnect();
    } catch {
      // Already disconnected.
    }
  }

  node.port.onmessage = (event: MessageEvent<PlayerOutMsg>) => {
    try {
      const msg = event.data;
      if (!msg || typeof msg !== 'object') return;
      switch (msg.t) {
        case 'ready':
          info = msg.info;
          issues = msg.issues;
          playable = msg.playable;
          if (!readySettled) {
            readySettled = true;
            settleReady();
          }
          break;
        case 'loaded': {
          info = msg.info;
          issues = msg.issues;
          playable = msg.playable;
          const waiter = pending.get(msg.req);
          if (waiter) {
            pending.delete(msg.req);
            waiter.resolve({ playable: msg.playable, issues: msg.issues });
          }
          break;
        }
        case 'trace': {
          const packet: TracePacket = { time: msg.time, seconds: msg.seconds, values: msg.values };
          for (const listener of traceListeners) {
            try {
              listener(packet);
            } catch {
              // A listener's own failure does not stop the other listeners.
            }
          }
          break;
        }
        case 'ended':
          for (const listener of endedListeners) {
            try {
              listener();
            } catch {
              // A listener's own failure does not stop the other listeners.
            }
          }
          break;
        case 'error':
          if (!readySettled) {
            readySettled = true;
            failReady(new Error(msg.message));
          }
          break;
      }
    } catch {
      // Never throw into the message port.
    }
  };

  node.onprocessorerror = () => {
    if (!readySettled) {
      readySettled = true;
      failReady(new Error('processorerror'));
    }
  };

  try {
    await ready;
  } catch (error) {
    dispose();
    throw error;
  }
  try {
    node.connect(o.output ?? ctx.destination);
  } catch (error) {
    dispose();
    throw error;
  }

  return {
    node,
    get info() {
      return info;
    },
    get issues() {
      return issues;
    },
    get playable() {
      return playable;
    },
    set(name: string, value: number | string): void {
      if (disposed) return;
      try {
        node.port.postMessage({ t: 'set', name, value } satisfies PlayerInMsg);
      } catch {
        // A closed port drops the message.
      }
    },
    trigger(name: string): void {
      if (disposed) return;
      try {
        node.port.postMessage({ t: 'trigger', name } satisfies PlayerInMsg);
      } catch {
        // A closed port drops the message.
      }
    },
    restart(): void {
      if (disposed) return;
      try {
        node.port.postMessage({ t: 'restart' } satisfies PlayerInMsg);
      } catch {
        // A closed port drops the message.
      }
    },
    setPatch(patch: HaaPatch | string | Uint8Array, keepState = false): Promise<LoadResult> {
      if (disposed) return Promise.reject(new Error('[haa] player disposed'));
      const req = nextReq++;
      return new Promise<LoadResult>((resolve, reject) => {
        pending.set(req, { resolve, reject });
        try {
          node.port.postMessage({ t: 'patch', req, bytes: patchBytes(patch), keepState } satisfies PlayerInMsg);
        } catch (error) {
          pending.delete(req);
          reject(unavailable(error));
        }
      });
    },
    onTrace(cb: (t: TracePacket) => void): () => void {
      traceListeners.add(cb);
      return () => {
        traceListeners.delete(cb);
      };
    },
    onEnded(cb: () => void): () => void {
      endedListeners.add(cb);
      return () => {
        endedListeners.delete(cb);
      };
    },
    dispose,
  };
}

/**
 * Create a browser player on `ctx`. Rejects only with
 * `Error('[haa] worklet unavailable: <reason>')` (ARCHITECTURE §Errors).
 */
export async function createPlayer(ctx: BaseAudioContext, o: PlayerOptions): Promise<HaaPlayer> {
  try {
    return await startPlayer(ctx, o);
  } catch (error) {
    throw unavailable(error);
  }
}
