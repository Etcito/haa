/**
 * Encode mono PCM to MP3 with lamejs 1.2.1: Mp3Encoder(1, rate, kbps), 1152-sample Int16
 * chunks, then flush, concatenated into one Uint8Array. Deterministic: the same PCM gives
 * the same bytes in every run and process.
 */
export declare function encodeMp3(pcm: Float32Array, rate: 48000, kbps: 128): Uint8Array;
