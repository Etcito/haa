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
