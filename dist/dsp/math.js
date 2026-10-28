// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe scalar math helpers. Runs unchanged in node, on the main thread and in
// AudioWorkletGlobalScope. Plain Math.* is allowed here: audio is outside SIM SCOPE.
// This file never allocates.
/** Full turn in radians. */
export const TAU = Math.PI * 2;
/** Decibels (amplitude) to linear gain. */
export function dbToGain(db) {
    return Math.pow(10, db / 20);
}
/** Linear gain to decibels; floored at -200 for g <= 1e-10 (and for non-positive input). */
export function gainToDb(g) {
    if (!(g > 1e-10))
        return -200;
    return 20 * Math.log10(g);
}
/** Clamp to [lo, hi]; NaN resolves to lo. */
export function clamp(x, lo, hi) {
    if (x !== x)
        return lo;
    if (x < lo)
        return lo;
    if (x > hi)
        return hi;
    return x;
}
/** Linear interpolation, no clamping. */
export function lerp(a, b, t) {
    return a + (b - a) * t;
}
/** Hermite smoothstep on [e0, e1], clamped to [0, 1]. */
export function smoothstep(e0, e1, x) {
    const t = clamp((x - e0) / (e1 - e0), 0, 1);
    return t * t * (3 - 2 * t);
}
/** x when finite, else fallback. */
export function finiteOr(x, fallback) {
    return Number.isFinite(x) ? x : fallback;
}
/** MIDI note number to Hz (A4 = 69 = 440 Hz). */
export function midiToHz(m) {
    return 440 * Math.pow(2, (m - 69) / 12);
}
/**
 * Tanh via the clamped rational Pade 7/6 approximant.
 * |error| < 2e-3 for |x| <= 5; the input is clamped to [-5, 5] past the accurate range.
 */
export function fastTanh(x) {
    if (x !== x)
        return 0;
    if (x <= -5)
        return -1;
    if (x >= 5)
        return 1;
    const x2 = x * x;
    const num = x * (135135 + x2 * (17325 + x2 * (378 + x2)));
    const den = 135135 + x2 * (62370 + x2 * (3150 + x2 * 28));
    const y = num / den;
    return y < -1 ? -1 : y > 1 ? 1 : y;
}
/**
 * Rescaled cubic soft clipper: x - x^3/3 on [-1, 1] (peaks at 2/3), flat beyond,
 * rescaled by 3/2 so the output range is [-1, 1].
 */
export function softClip(x) {
    if (x >= 1)
        return 1;
    if (x <= -1)
        return -1;
    return 1.5 * (x - (x * x * x) / 3);
}
