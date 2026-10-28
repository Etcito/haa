// Node-only WAV file writer (module offline): creates the parent directories
// and writes the RIFF PCM 16-bit encoding of a render.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { encodeWav } from "../offline/wav.js";
/** Write `r` as a 16-bit PCM WAV at `path` (mono when `right` is missing or null). */
export function writeWav(path, r) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, encodeWav(r.left, r.right ?? null, r.sampleRate));
}
