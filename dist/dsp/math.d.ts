/** Full turn in radians. */
export declare const TAU: number;
/** Decibels (amplitude) to linear gain. */
export declare function dbToGain(db: number): number;
/** Linear gain to decibels; floored at -200 for g <= 1e-10 (and for non-positive input). */
export declare function gainToDb(g: number): number;
/** Clamp to [lo, hi]; NaN resolves to lo. */
export declare function clamp(x: number, lo: number, hi: number): number;
/** Linear interpolation, no clamping. */
export declare function lerp(a: number, b: number, t: number): number;
/** Hermite smoothstep on [e0, e1], clamped to [0, 1]. */
export declare function smoothstep(e0: number, e1: number, x: number): number;
/** x when finite, else fallback. */
export declare function finiteOr(x: number, fallback: number): number;
/** MIDI note number to Hz (A4 = 69 = 440 Hz). */
export declare function midiToHz(m: number): number;
/**
 * Tanh via the clamped rational Pade 7/6 approximant.
 * |error| < 2e-3 for |x| <= 5; the input is clamped to [-5, 5] past the accurate range.
 */
export declare function fastTanh(x: number): number;
/**
 * Rescaled cubic soft clipper: x - x^3/3 on [-1, 1] (peaks at 2/3), flat beyond,
 * rescaled by 3/2 so the output range is [-1, 1].
 */
export declare function softClip(x: number): number;
