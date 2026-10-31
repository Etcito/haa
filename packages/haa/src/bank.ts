// Deterministic one-shot bank rendering: variant seeds and the bank renderer of ARCHITECTURE §8.
// Realm-neutral (no DOM, no Node builtins), exported as `@etcito/haa/bank`.
// Rules:
// - `variantSeed` is FNV-1a 32 over the UTF-8 bytes of `${id}#${variant}`;
// - each entry renders with `renderPatch`, seeded from the patch id and the entry's variant;
// - `seconds` defaults to 1, and the sample rate keeps the patch default (`OFFLINE_RATE`);
// - a patch given as a string is JSON.parsed before rendering.

import type { HaaPatch } from './types.ts';
import { encodeUtf8 } from './utf8.ts';
import { renderPatch } from './offline.ts';
import type { RenderOptions, StereoRender } from './offline.ts';

export { analyse } from './analysis.ts';
export type { SoundStats } from './analysis.ts';
export { spectrogram } from './spectrum.ts';
export type { Spectrogram } from './spectrum.ts';

/** FNV-1a 32 offset basis and prime. */
const FNV_OFFSET = 0x811c9dc5;
const FNV_PRIME = 0x01000193;

/** The stable seed of one variant: FNV-1a 32 over the UTF-8 bytes of `${id}#${variant}`. */
export function variantSeed(id: string, variant: number): number {
  let hash = FNV_OFFSET;
  for (const byte of encodeUtf8(`${id}#${variant}`)) {
    hash = Math.imul(hash ^ byte, FNV_PRIME) >>> 0;
  }
  return hash >>> 0;
}

/** One bank entry: which variant to render and the render script it uses. */
export interface BankEntry {
  variant: number;
  seconds?: number;
  inputs?: Record<string, number | string>;
  ramps?: Record<string, [number, number]>;
  triggers?: { name: string; at: number }[];
}

/** One rendered bank entry, with the seed that produced it. */
export interface BankRender {
  id: string;
  variant: number;
  seed: number;
  render: StereoRender;
}

/** Render every entry of a bank in order with the patch id and its variant seed. */
export function renderBank(
  module: WebAssembly.Module,
  patch: HaaPatch | string,
  entries: readonly BankEntry[],
): BankRender[] {
  const parsed: HaaPatch = typeof patch === 'string' ? (JSON.parse(patch) as HaaPatch) : patch;
  return entries.map((entry) => {
    const seed = variantSeed(parsed.id, entry.variant);
    const options: RenderOptions = {
      seconds: entry.seconds ?? 1,
      seed,
      inputs: entry.inputs,
      ramps: entry.ramps,
      triggers: entry.triggers,
    };
    return {
      id: parsed.id,
      variant: entry.variant,
      seed,
      render: renderPatch(module, parsed, options),
    };
  });
}
