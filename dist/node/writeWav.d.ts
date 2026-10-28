import type { StereoRender } from '../contract.js';
/** Write `r` as a 16-bit PCM WAV at `path` (mono when `right` is missing or null). */
export declare function writeWav(path: string, r: StereoRender | {
    left: Float32Array;
    right?: Float32Array | null;
    sampleRate: number;
}): void;
