/**
 * Encode floating point samples as a RIFF/WAVE PCM 16-bit file.
 *
 * - `right === null` writes a mono file, otherwise a stereo one (L, R, ...).
 * - `length` is the per-channel sample count; it defaults to `left.length`.
 */
export declare function encodeWav(left: Float32Array, right: Float32Array | null, sampleRate: number, length?: number): Uint8Array;
/**
 * Decode a RIFF/WAVE PCM 16-bit file into per-channel floats (sample / 32767).
 *
 * - Unknown chunks are skipped (word-aligned).
 * - Anything else than PCM format 1 at 16 bits, or a truncated file, throws `[haa] bad wav`.
 */
export declare function decodeWav(bytes: Uint8Array): {
    sampleRate: number;
    channels: Float32Array[];
};
