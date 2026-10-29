// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// Strip packing, PCM hashing and runtime alignment for the audio bank. Pure: it only
// imports the shared contract, touches no DOM and allocates only when explicitly asked.
//
// The bake packs each family into one mono strip (variants in order, each followed by
// GAP_SAMPLES zeros), hashes it with FNV-1a and records every variant's start and length.
// The runtime decodes the strip at the context rate and aligns the variants with the same
// rule for every rate, so a 44.1 kHz context and the 48 kHz bake agree.
import { GAP_SAMPLES, ONSET_THRESHOLD } from "../contract.js";
/** The first index of buf with |x| > ONSET_THRESHOLD, or -1 when none. */
function firstOnsetIndex(buf) {
    for (let i = 0; i < buf.length; i++) {
        if (Math.abs(buf[i]) > ONSET_THRESHOLD)
            return i;
    }
    return -1;
}
/**
 * Concatenate the variants, each followed by GAP_SAMPLES zeros, and record where each one
 * starts and how long it is. onset0 is the first strip index above ONSET_THRESHOLD (0 when
 * the strip is silent). A zero-length variant still reserves its gap.
 */
export function packStrip(variants) {
    let total = 0;
    for (let i = 0; i < variants.length; i++)
        total += variants[i].length + GAP_SAMPLES;
    const strip = new Float32Array(total);
    const entries = [];
    let at = 0;
    for (let i = 0; i < variants.length; i++) {
        const v = variants[i];
        entries.push({ start: at, length: v.length });
        strip.set(v, at);
        at += v.length + GAP_SAMPLES;
    }
    const onset = firstOnsetIndex(strip);
    return { strip, entries, onset0: onset < 0 ? 0 : onset };
}
/**
 * FNV-1a 32 (offset 2166136261, prime 16777619) over the strip's little-endian float bytes,
 * as 8 lowercase hex digits. A Uint8Array view of the underlying buffer is used so the value
 * is independent of the platform's typed-array element handling.
 */
export function pcmHash(strip) {
    const bytes = new Uint8Array(strip.buffer, strip.byteOffset, strip.byteLength);
    let h = 0x811c9dc5;
    for (let i = 0; i < bytes.length; i++) {
        h ^= bytes[i];
        h = Math.imul(h, 0x01000193);
    }
    return (h >>> 0).toString(16).padStart(8, '0');
}
/**
 * The shared runtime alignment rule. After decodeAudioData at rate R the strip is compressed
 * or stretched by k = R / 48000; the decoded onset sits at firstDecodedOnset, so every
 * recorded offset is shifted by the difference from where the bake put onset0. Each variant
 * is decoded[round(start * k) + shift, + round(length * k)]. Slices are clamped to the
 * decoded buffer; an empty result is a zero-length Float32Array.
 */
export function alignVariants(decoded, entry, rate) {
    const k = rate / 48000;
    const decodedOnset = firstOnsetIndex(decoded);
    const shift = (decodedOnset < 0 ? 0 : decodedOnset) - Math.round(entry.onset0 * k);
    const n = decoded.length;
    const out = [];
    for (let i = 0; i < entry.variants.length; i++) {
        const spec = entry.variants[i];
        const start = Math.min(n, Math.max(0, Math.round(spec.start * k) + shift));
        const end = Math.min(n, Math.max(start, start + Math.round(spec.length * k)));
        if (end <= start)
            out.push(new Float32Array(0));
        else
            out.push(decoded.slice(start, end));
    }
    return out;
}
