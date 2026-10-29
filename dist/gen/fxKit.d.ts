import type { AudioRng } from '../dsp/rng.js';
import { type ImpactOpts } from './impactModel.js';
/**
 * Exponential pitch envelope. x = clamp(t / time, 0, 1), s = 1 - (1 - x)^curve (curve > 1
 * moves the change early, < 1 moves it late). Returns from * (to / from)^s when both ends are
 * positive (a constant musical interval), else the linear lerp(from, to, s). time <= 0 -> to.
 */
export declare function pitchEnvelope(t: number, o: {
    from: number;
    to: number;
    time: number;
    curve: number;
}): number;
/**
 * Pink noise through an SVF bandpass whose centre follows pitchEnvelope(f0 -> f1) every
 * sample, clamped to [20, 0.45 sr]. A 6 ms raised-cosine attack, then (1 - x)^1.5 to 0 at
 * `seconds`, all x level. The bandpass output is divided by sqrt(q) so a q change does not
 * move the peak level by more than about 6 dB. With stereoSeed the noise runs on
 * rng.fork(stereoSeed) and leaves rng itself unadvanced; otherwise it draws from rng.
 */
export declare function noiseSweep(out: Float32Array, start: number, o: {
    seconds: number;
    f0: number;
    f1: number;
    q: number;
    curve: number;
    level: number;
    stereoSeed?: number;
}, sr: number, rng: AudioRng): number;
/**
 * An AdditiveBank of `partials` partials with a pitch glide. Ratio of partial k (1-based) is
 * k * sqrt(1 + inharm k^2) * 2^(d_k / 1200), d_k a seeded offset in +/-detuneCents drawn once
 * per call. Amplitude dbToGain(tiltDb log2(k)) (a negative tilt darkens), phase seeded in
 * [0, TAU). Rendered in 128-frame blocks, f0 per block from pitchEnvelope(f0 -> f1, curve
 * 1.5). Envelope: a raised-cosine attack over `attack` s, then exp(-t / decay), to 0 at
 * `seconds` with a 5 ms fade, all x level. The bank's 0.45 sr band limit handles aliasing.
 */
export declare function harmonicStack(out: Float32Array, start: number, o: {
    seconds: number;
    f0: number;
    f1: number;
    partials: number;
    tiltDb: number;
    inharm: number;
    detuneCents: number;
    attack: number;
    decay: number;
    level: number;
}, sr: number, rng: AudioRng): number;
/**
 * A click plus a pitch-dropping sub thump. Click: a raised-cosine pulse 1 ms wide at
 * 0.5 x level. Thump: a phase-accumulated sine gliding f0 -> f1 (pitchEnvelope, time
 * 0.4 x seconds, curve 2) under an exp decay that reaches -60 dB at `seconds`, x level. It is
 * the only allowed bare sine: it sits under a transient.
 */
export declare function subTransient(out: Float32Array, start: number, o: {
    f0: number;
    f1: number;
    seconds: number;
    level: number;
}, sr: number): number;
/**
 * The excitation drives a 3-mode ModalBank. Modes: hz at gain 1, hz x 2.76 at gain 0.25,
 * hz x 5.40 at gain 0.08; each t60 = min(seconds, 6.91 q / (pi hz)); modes above 0.45 sr are
 * skipped by the bank. Excitation 'click' is a 0.5 ms raised-cosine pulse, 'noise' is 4 ms of
 * white noise under an exp decay. The output is scaled so the first-10 ms peak is about
 * `level`, rendered for `seconds` with a 5 ms end fade.
 */
export declare function resonantPing(out: Float32Array, start: number, o: {
    hz: number;
    q: number;
    seconds: number;
    level: number;
    excite: 'click' | 'noise';
}, sr: number, rng: AudioRng): number;
/**
 * Wraps renderImpact: it renders into a scratch buffer of length out.length - start, then
 * adds scratch x level from start. Returns start + used, clamped to out.length.
 */
export declare function modalHit(out: Float32Array, start: number, o: ImpactOpts & {
    level: number;
}, sr: number, rng: AudioRng): number;
/**
 * A Saturator(factor) with set(drive, 0, 1) processing out[from, to) in place. When drive >
 * 1.5 and factor is 1 the audible aliasing is oversampled: 2x is used instead.
 */
export declare function shapeBuffer(out: Float32Array, from: number, to: number, drive: number, factor: 1 | 2 | 4): void;
/**
 * Scales out so max |x| = dbToGain(peakDb). A no-op when the peak is 0 or non-finite or the
 * target is non-positive.
 */
export declare function normalizePeak(out: Float32Array, peakDb: number): void;
