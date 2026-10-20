// Node-only WAV file writer (module offline): creates the parent directories
// and writes the RIFF PCM 16-bit encoding of a render.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import type { StereoRender } from '../contract.ts';
import { encodeWav } from '../offline/wav.ts';

/** Write `r` as a 16-bit PCM WAV at `path` (mono when `right` is missing or null). */
export function writeWav(
  path: string,
  r: StereoRender | { left: Float32Array; right?: Float32Array | null; sampleRate: number },
): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, encodeWav(r.left, r.right ?? null, r.sampleRate));
}
