// Messages between the main-thread speech host (speech-host.ts) and the speech worker (speech-worker.ts,
// `@etcito/haa/speech-worker`). The worker holds tiny-voca in its own haa.wasm instance and renders a patch's
// speech one utterance (part) at a time; the host routes each part to the players that wait for it.

import type { SpeechPlanned, WeightsProgress } from './speech.ts';
import type { SpeechClip } from './types.ts';

/** One utterance of a source's speech, at the rate the stream asked for. */
export type SpeechPart = SpeechClip & { part: number; last: boolean };

/** Main thread -> speech worker. */
export type SpeechWorkerRequest =
  /** First message: the compiled haa.wasm and where the weights come from; the worker starts loading at once. */
  | { t: 'init'; module: WebAssembly.Module; url?: string; cache?: string | false }
  /** Queue a patch's speech (JSON text); its parts then come one by one as `part` replies, in order. */
  | { t: 'stream'; id: number; patch: string; seed: number; rate: number };

/** Speech worker -> main thread. */
export type SpeechWorkerReply =
  | ({ t: 'progress' } & WeightsProgress)
  | { t: 'ready' }
  | { t: 'failed'; stage: 'download' | 'load'; message: string }
  | { t: 'planned'; id: number; sources: SpeechPlanned[] }
  | { t: 'part'; clip: SpeechPart }
  | { t: 'error'; id: number; message: string };
