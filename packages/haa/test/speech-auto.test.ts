/**
 * Host test for the player's automatic speech (`createPlayer({ speech })`, speech-host.ts, speech-serve.ts).
 *
 * Runs with Node's native type stripping:
 *   node --test packages/haa/test/speech-auto.test.ts
 * The browser is faked (AudioWorkletNode, an AudioContext, a module Worker that runs the real worker loop in this
 * thread) and so is tiny-voca: the weights and the speech instance are stubs, nothing is downloaded. The player is
 * driven exactly as the README quickstart drives it.
 */
import { afterEach, test } from 'node:test';
import { strict as assert } from 'node:assert';

import { closeSpeech, createPlayer } from '../src/index.ts';
import { patchSpeechClips } from '../src/speech.ts';
import type { Speech, WeightsProgress } from '../src/speech.ts';
import { serveSpeech, type SpeechWorkerScope } from '../src/speech-serve.ts';
import type { PlayerProcessorOptions, SpeechClip } from '../src/types.ts';

const later = (f: () => void): void => {
  setTimeout(f, 0);
};

/** What the fake browser and the fake tiny-voca saw. */
const log = {
  calls: [] as string[],
  workers: [] as string[],
  fetches: 0,
  nodes: [] as { options: PlayerProcessorOptions; posted: { t: string; clips?: SpeechClip[] }[] }[],
};

const SOURCES = ['k-hello', 'k-bye'];

/** A tiny-voca stand-in: two sources of two parts each, rendered in order. */
function fakeSpeech(): Speech {
  const queue: { key: string; part: number; last: boolean }[] = [];
  return {
    load(gguf: Uint8Array) {
      log.calls.push(`load:${gguf.length}`);
      return { phonemizer: true };
    },
    planPatch(patch: unknown, seed?: number, rate?: number) {
      log.calls.push(`planPatch:${typeof patch}:${seed}:${rate}`);
      for (const part of [0, 1]) for (const key of SOURCES) queue.push({ key, part, last: part === 1 });
      return SOURCES.map((key) => ({ key, parts: 0, complete: false }));
    },
    nextPart() {
      const next = queue.shift();
      log.calls.push(next ? `nextPart:${next.key}#${next.part}` : 'nextPart:null');
      return next ? { ...next, rate: 48_000, pcm: new Float32Array(4).fill(next.part + 1) } : null;
    },
  } as unknown as Speech;
}

const fakeWeights = async (o: { onProgress?: (p: WeightsProgress) => void }): Promise<Uint8Array> => {
  log.calls.push('fetchWeights');
  o.onProgress?.({ loaded: 4, total: 8, cached: false });
  o.onProgress?.({ loaded: 8, total: 8, cached: false });
  return new Uint8Array(8);
};

/** A module Worker whose other end is the real worker loop, with the fake tiny-voca. */
class FakeWorker {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((event: { message: string }) => void) | null = null;
  private readonly scope: SpeechWorkerScope;
  constructor(url: string | URL, options: { type?: string }) {
    log.workers.push(`${String(url)}:${options.type}`);
    this.scope = { onmessage: null, postMessage: (data) => later(() => this.onmessage?.({ data })) };
    serveSpeech(this.scope, { fetchWeights: fakeWeights, createSpeech: async () => fakeSpeech() });
  }
  postMessage(data: unknown): void {
    later(() => this.scope.onmessage?.({ data: data as never }));
  }
  terminate(): void {}
}

/** An AudioWorkletNode that is ready at once and records what it is sent. */
class FakeNode {
  readonly port: { onmessage: ((event: { data: unknown }) => void) | null; postMessage(msg: { t: string }): void };
  onprocessorerror: (() => void) | null = null;
  constructor(_ctx: unknown, _name: string, init: { processorOptions: PlayerProcessorOptions }) {
    const entry = { options: init.processorOptions, posted: [] as { t: string; clips?: SpeechClip[] }[] };
    log.nodes.push(entry);
    this.port = { onmessage: null, postMessage: (msg) => entry.posted.push(msg) };
    later(() => this.port.onmessage?.({ data: { t: 'ready', playable: true, issues: [], info: { nodes: [], inputs: [], mode: 'timeline', duration: 2, loop: false } } }));
  }
  connect(): void {}
  disconnect(): void {}
}

const g = globalThis as Record<string, unknown>;
g.Worker = FakeWorker;
g.AudioWorkletNode = FakeNode;
const realFetch = globalThis.fetch;
globalThis.fetch = (async () => {
  log.fetches += 1;
  throw new Error('no network in tests');
}) as typeof fetch;

const ctx = { sampleRate: 48_000, destination: {}, audioWorklet: { addModule: async () => {} } } as unknown as AudioContext;
// The smallest valid wasm module: the fake worklet and the fake speech never instantiate it.
const module = new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));

// As a game gets it: the .haa.json text Studio exported.
const patchOf = (nodes: object[]): string => JSON.stringify({ haa: 1, id: 'joe', title: 'Joe', inputs: [{ name: 'speed', kind: 'continuous', unit: '', min: 0, max: 1, default: 0, smooth: 0.05, curve: 'lin' }], graph: { duration: 2, mode: 'timeline', nodes, connections: [] } });
const SPEAKS = patchOf([{ id: 's1', type: 'speak', x: 0, y: 0, label: 'Speak', params: { text: 'Hello there. Goodbye.' } }]);
const SILENT = patchOf([{ id: 'o1', type: 'oscillator', x: 0, y: 0, label: 'Osc', params: {} }]);

afterEach(() => {
  closeSpeech();
  log.calls = [];
  log.workers = [];
  log.fetches = 0;
  log.nodes = [];
});

test('a patch without a speak node starts no worker and fetches nothing', async () => {
  const player = await createPlayer(ctx, { module, processorUrl: '/worklet.js', patch: SILENT, inputs: { speed: 0.5 }, speech: { workerUrl: '/speech-worker.js' } });
  player.set('speed', 0.8);
  assert.deepEqual(log.workers, []);
  assert.deepEqual(log.calls, []);
  assert.deepEqual(log.nodes[0]!.options.speechClips, []);
  assert.deepEqual(await patchSpeechClips(module, SILENT), []);
  assert.equal(log.fetches, 0);
  player.dispose();
});

test('a speaking patch plans, waits for each part 0, then streams the rest to the player', async () => {
  const progress: number[] = [];
  const player = await createPlayer(ctx, {
    module,
    processorUrl: '/worklet.js',
    patch: SPEAKS,
    seed: 3,
    inputs: { speed: 0.5 },
    speech: { workerUrl: '/speech-worker.js', onProgress: (p) => progress.push(p.loaded) },
  });
  player.set('speed', 0.8);
  assert.deepEqual(log.workers, ['/speech-worker.js:module']);
  assert.deepEqual(progress, [4, 8]);
  // Started with both first utterances (the player never waits for more).
  const node = log.nodes[0]!;
  const initial = node.options.speechClips!.map((c) => `${c.key}#${c.part}`);
  assert.deepEqual(initial.slice(0, 2), ['k-hello#0', 'k-bye#0']);
  // The rest arrives while it plays, once each.
  for (let i = 0; i < 20 && log.calls.at(-1) !== 'nextPart:null'; i++) await new Promise((r) => setTimeout(r, 5));
  await new Promise((r) => setTimeout(r, 5));
  const streamed = node.posted.filter((m) => m.t === 'clips').flatMap((m) => m.clips!.map((c) => `${c.key}#${c.part}`));
  assert.deepEqual([...initial, ...streamed], ['k-hello#0', 'k-bye#0', 'k-hello#1', 'k-bye#1']);
  assert.deepEqual(log.calls, [
    'fetchWeights', 'load:8', 'planPatch:string:3:48000',
    'nextPart:k-hello#0', 'nextPart:k-bye#0', 'nextPart:k-hello#1', 'nextPart:k-bye#1', 'nextPart:null',
  ]);
  assert.equal(log.fetches, 0);
  player.dispose();
});

test.after(() => {
  globalThis.fetch = realFetch;
});
