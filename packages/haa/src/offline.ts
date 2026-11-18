// Offline renderer (ARCHITECTURE §8): patch JSON to a stereo buffer in a Worker or in Node.
// Mirrors crates/haa-core/src/render.rs and is realm-neutral: no DOM, no Node builtins.
// Rules:
// - `seconds` defaults to the graph duration in timeline mode and to `LIVE_DEFAULT_SECONDS` in live mode;
// - the sample rate defaults to `OFFLINE_RATE` and the seed to 0;
// - inputs are set before the first block;
// - a ramp sets its input to the linear value between `a` and `b` at every block start;
// - a trigger fires at the first block whose start is at or after `at` seconds;
// - the result holds exactly `round(seconds × rate)` frames, silent after `ended`.

import type { HaaPatch, PatchIssue } from './types.ts';
import { BLOCK, LIVE_DEFAULT_SECONDS, OFFLINE_RATE } from './types.ts';
import { HaaEngine, patchBytes } from './engine.ts';
import { decodeUtf8, encodeUtf8 } from './utf8.ts';

export { encodeWav } from './wav.ts';
export * from './types.ts';

/** The offline render settings. Defaults: `OFFLINE_RATE`, seed 0, the patch's own length. */
export interface RenderOptions {
  sampleRate?: number;
  seconds?: number;
  seed?: number;
  inputs?: Record<string, number | string>;
  /** `name: [a, b]`: linear from `a` to `b` over the render, set at every block start. */
  ramps?: Record<string, [number, number]>;
  /** Pulse each trigger at the first block whose start is at or after `at` seconds. */
  triggers?: { name: string; at: number }[];
}

/** The rendered stereo buffer and the loader/validate issues of its patch. */
export interface StereoRender {
  left: Float32Array;
  right: Float32Array;
  sampleRate: number;
  issues: PatchIssue[];
}

/** The reserved inputs §8 places after the header inputs. */
const RESERVED_INPUTS = ['_dist', '_pan', '_doppler'] as const;

/** Clamp the render rate exactly like `Voice::new` (8000..192000, fallback 48000). */
function renderSampleRate(value: number | undefined): number {
  if (value === undefined || !Number.isFinite(value) || value === 0) return OFFLINE_RATE;
  return Math.min(192_000, Math.max(8_000, value));
}

/** Throw the §8 error for a name that the loaded patch does not publish. */
function requireInput(known: Set<string>, name: string): void {
  if (!known.has(name)) throw new Error(`[haa] unknown input ${name}`);
}

/** Render patch data (object, JSON string or UTF-8 bytes) with the shared §8 rules. */
export function renderPatch(
  module: WebAssembly.Module,
  patch: HaaPatch | string | Uint8Array,
  o: RenderOptions = {},
): StereoRender {
  const sampleRate = renderSampleRate(o.sampleRate);
  const rate = sampleRate;
  const engine = HaaEngine.fromModule(module);
  const voice = engine.createVoice(sampleRate, o.seed ?? 0);
  try {
    const { playable, issues } = voice.load(patchBytes(patch));
    if (!playable) {
      const error = issues.find((issue) => issue.severity === 'error') ?? issues[0];
      const code = error?.code ?? 'bad-field';
      const message = error?.message ?? 'the patch cannot play';
      throw new Error(`[haa] patch not playable: ${code} ${message}`);
    }

    const info = voice.info();
    const known = new Set<string>(RESERVED_INPUTS);
    for (const input of info.inputs) known.add(input.name);
    const inputs = Object.entries(o.inputs ?? {});
    const ramps = Object.entries(o.ramps ?? {});
    const triggers = (o.triggers ?? []).map(({ name, at }) => ({ name, at, fired: false }));
    for (const [name] of inputs) requireInput(known, name);
    for (const [name] of ramps) requireInput(known, name);
    for (const { name } of triggers) requireInput(known, name);

    let seconds = o.seconds ?? (info.mode === 'live' ? LIVE_DEFAULT_SECONDS : info.duration);
    seconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
    const total = Math.round(seconds * rate);
    const left = new Float32Array(total);
    const right = new Float32Array(total);

    // Inputs are in place before the first block.
    for (const [name, value] of inputs) {
      if (!voice.set(name, value)) throw new Error(`[haa] unknown input ${name}`);
    }

    let frame = 0;
    while (frame < total) {
      const start = frame / rate;
      for (const [name, [a, b]] of ramps) {
        voice.set(name, a + (b - a) * (frame / total));
      }
      for (const trigger of triggers) {
        if (!trigger.fired && start >= trigger.at) {
          voice.trigger(trigger.name);
          trigger.fired = true;
        }
      }
      const len = Math.min(BLOCK, total - frame);
      const playing = voice.process(len);
      left.set(voice.left().subarray(0, len), frame);
      right.set(voice.right().subarray(0, len), frame);
      frame += len;
      if (!playing) break;
    }

    return { left, right, sampleRate, issues };
  } finally {
    voice.free();
  }
}

/** A `chimera` node in `lyrics` mode: its text, body and rate input, to time its words. */
export interface LyricsTimingRequest {
  /** the node text: one line per phrase, `<seconds> words` */
  text: string;
  language?: 'english' | 'ipa';
  female?: number;
  child?: number;
  /** the rate input over the patch: `[seconds, rate]` points, linear between them (none: rate 1) */
  rate?: [number, number][];
  /** with `seconds`: also the score time per `hop` frame */
  hop?: number;
  seconds?: number;
}

/** One timed line: patch seconds of its start, its end, each word and each sound (-1 where the plan never reaches it). */
export interface LyricsTimedLine {
  start: number;
  end: number;
  words: { word: string; start: number; end: number }[];
  phones: { ipa: string; start: number; end: number }[];
}

/** The timed lines, and with `hop` / `seconds` per frame the playing line's score time (-1: none) and vowel flag (1/0). */
export interface LyricsTiming {
  lines: LyricsTimedLine[];
  tau?: number[];
  vowel?: number[];
}

/** When the node says each word: its own plan (G2P, durations, vowel-stretching rate) run in Rust (`haa_lyrics_timing`). */
export function lyricsTiming(module: WebAssembly.Module, request: LyricsTimingRequest): LyricsTiming {
  const exports = new WebAssembly.Instance(module, {}).exports as unknown as {
    memory: WebAssembly.Memory;
    haa_alloc(len: number): number;
    haa_free(ptr: number, len: number): void;
    haa_result_ptr(): number;
    haa_result_len(): number;
    haa_lyrics_timing(ptr: number, len: number): number;
  };
  const bytes = encodeUtf8(JSON.stringify(request));
  const ptr = exports.haa_alloc(bytes.length);
  new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
  try {
    exports.haa_lyrics_timing(ptr, bytes.length);
    const out = new Uint8Array(exports.memory.buffer, exports.haa_result_ptr(), exports.haa_result_len());
    const json = JSON.parse(decodeUtf8(out)) as LyricsTiming & { error?: string };
    if (json.error || !json.lines) throw new Error(`[haa] lyrics timing: ${json.error ?? 'no lines'}`);
    return json;
  } finally {
    exports.haa_free(ptr, bytes.length);
  }
}

/** A `chimera` lyrics text and, per line, when the audio says each of its phones (as written in the text). */
export interface LyricsFitRequest {
  /** the node text: one line per phrase, `<seconds> words` (the seconds are replaced by the fitted start) */
  text: string;
  language?: 'english' | 'ipa';
  female?: number;
  child?: number;
  /** per text line: [ipa, start s, lead s] of each phone in order (played `lead` s early, default 0), and when the last one ends */
  lines: { phones: ([string, number] | [string, number, number])[]; end: number }[];
  /** points per line's rate input at most (default 120) */
  maxPoints?: number;
}

/** Per line its fitted start and rate input ([seconds, rate] steps), and how far the node's own clock then lands each sound from the audio. */
export interface LyricsFit {
  lines: { start: number; rate: [number, number][] }[];
  /** mean and worst |planned - heard| start of the sounds (s) */
  error: number;
  worst: number;
}

/**
 * The start and rate input that make each `lyrics` line say its sounds when the audio does: the node's plan fitted
 * to the heard phone times and checked with its own clock, in Rust (`haa_lyrics_fit`).
 */
export function lyricsFit(module: WebAssembly.Module, request: LyricsFitRequest): LyricsFit {
  const exports = new WebAssembly.Instance(module, {}).exports as unknown as {
    memory: WebAssembly.Memory;
    haa_alloc(len: number): number;
    haa_free(ptr: number, len: number): void;
    haa_result_ptr(): number;
    haa_result_len(): number;
    haa_lyrics_fit(ptr: number, len: number): number;
  };
  const bytes = encodeUtf8(JSON.stringify(request));
  const ptr = exports.haa_alloc(bytes.length);
  new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
  try {
    exports.haa_lyrics_fit(ptr, bytes.length);
    const out = new Uint8Array(exports.memory.buffer, exports.haa_result_ptr(), exports.haa_result_len());
    const json = JSON.parse(decodeUtf8(out)) as LyricsFit & { error?: number | string };
    if (typeof json.error === 'string' || !json.lines) throw new Error(`[haa] lyrics fit: ${json.error ?? 'no lines'}`);
    return json;
  } finally {
    exports.haa_free(ptr, bytes.length);
  }
}
