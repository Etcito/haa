import type { AudioRng } from '../dsp/rng.js';
export type ClickMaterial = 'glass' | 'plastic';
export type TickMaterial = 'wood' | 'plastic';
export interface SoftClickOpts {
    material: ClickMaterial;
    /** Frequency ratio on the material's base ping (1 = nominal). */
    pitch: number;
    /** 0..1: the noise-to-ping balance and the high-mode gains. */
    brightness: number;
    level: number;
    /** Start sample in out (default 0). */
    at?: number;
}
export interface ChimeOpts {
    /** Note frequencies (Hz), struck in order. */
    notes: readonly number[];
    /** Seconds between two note onsets. */
    spacing: number;
    /** 0..1: the FM index and the high bell modes. */
    bright: number;
    /** RT60 of the rendered-in reverb tail (s). */
    tail: number;
    /** Fundamental bell t60 (s), default 0.6. */
    decay?: number;
    /** FM layer weight against the modal bell (default 0.2 + 0.2 bright). */
    fm?: number;
    /** FM modulator ratio (default 1.4, the Chowning bell; 3 gives harmonic 2f/4f/5f sidebands
     * with no sub-sideband under the note). */
    fmRatio?: number;
    /** Reverb return (default 0.25). */
    wet?: number;
    /** Strike noise sent into the reverb only (default 0): a soft, diffuse, non-tonal room tail
     * that carries a short note without letting its tone ring on. */
    air?: number;
    /** Default 1. */
    level?: number;
    at?: number;
}
export interface SwellOpts {
    /** Chord note frequencies (Hz). */
    notes: readonly number[];
    seconds: number;
    /** 0..1: the noise share and the partial count. */
    bright: number;
    level: number;
    at?: number;
}
export interface TickOpts {
    material: TickMaterial;
    /** Frequency ratio on the material's base (1 = nominal). */
    pitch: number;
    /** Fractional pitch change across the tick (-0.5..0.5). */
    drift: number;
    level: number;
    at?: number;
}
/**
 * A filtered-noise tick plus a short modal ping.
 * - Noise: 1..4 ms (seeded) of WhiteNoise under an exp decay reaching -60 dB at its end,
 *   band-passed by an Svf (q 1.4) at 2.5..6 kHz (rising with brightness, +/-8 % seeded).
 * - Ping: 'glass' has 4 modes (1, 2.32, 4.25, 6.63) on 1.6..3.2 kHz, t60 60..160 ms;
 *   'plastic' 3 modes (1, 2.11, 3.47) on 0.8..1.6 kHz, t60 20..60 ms. Struck by a 0.4 ms
 *   raised-cosine pulse plus the raw burst; high-mode gains scale with brightness.
 * - Mix: noise x (0.25 + 0.75 b), ping x (1 - 0.45 b), each from unit peak.
 * Length: the fundamental t60 plus 5 ms.
 */
export declare function softClick(out: Float32Array, o: SoftClickOpts, sr: number, rng: AudioRng): number;
/**
 * A wood or plastic roulette tick with pitch drift: a ratchet pawl striking a peg.
 * - Impulse: 0.3..1 ms (seeded) of Hann-windowed WhiteNoise, then a seeded 1.5..4 ms later a
 *   second, 0.3..0.6 weaker strike (the pawl bouncing back onto the peg).
 * - Body: a 4-mode ModalBank, 'wood' (1, 2.6, 4.1 on 900 Hz..2 kHz) or 'plastic'
 *   (1, 2.24, 3.62 on 1.2..2.6 kHz), each plus a low housing mode (0.43 / 0.5), fundamental
 *   t60 12..32 ms (seeded). freqScale glides linearly from 1 to 1 + drift across the tick
 *   (updated every 16 frames; resonator states are kept).
 * - Click: the impulses through a 4 kHz highpass Biquad, 0.5 of the body peak.
 * Length: 1.6 x the fundamental t60 plus 8 ms.
 */
export declare function tick(out: Float32Array, o: TickOpts, sr: number, rng: AudioRng): number;
/**
 * Modal and FM bell notes with a rendered-in short reverb tail. Note i (Hz, clamped to
 * [40, 0.4 sr]) starts i x spacing s after the first.
 * - Modal bell: a 0.08..0.23 ms raised-cosine mallet pulse (shorter when brighter) plus 0.35 x
 *   2..4 ms (seeded) of Hann-windowed WhiteNoise through a OnePole lowpass at 3 + 6 bright kHz
 *   excites 6 modes (1, 2.0, 2.76, 4.07, 5.4, 8.9), each detuned +/-6 cents
 *   (seeded), t60 = decay x (1, 0.95, 0.7, 0.5, 0.38, 0.22), high-mode gains scaled by
 *   0.4 + 0.6 bright. Each mode has a doublet partner 2.5..7 Hz above it (seeded), 0.35 as
 *   loud, t60 x 0.85: the slow beating keeps a held note alive instead of a static sine.
 *   Modes above 6 kHz lose gain as (6 kHz / hz)^2 and t60 as 6 kHz / hz.
 * - FM pair: carrier f, modulator fmRatio x f (default 1.4), index (0.4 + 2.6 bright) x exp
 *   decay (reaching 1/e at the bell t60, so the carrier never thins to a bare sine), capped so
 *   f + (index + 1) fmRatio f stays under min(12 kHz, 0.45 sr); amplitude:
 *   1.5 ms attack then -60 dB at 0.6 x the bell t60: the FM is the bright strike, and the
 *   modal body (octave and inharmonic modes) carries the tail, so its carrier never leaves
 *   a sine on the fundamental.
 * - Dry = modal x 0.7 + FM x fm (default 0.2 + 0.2 bright), each from unit peak. It feeds an
 *   FdnReverb (size 0.12, decay tail, damping 4.5 + 3 bright kHz, predelay 8 ms), so the tail
 *   darkens as it fades; the reverb input also takes air (default 0) x the unit-peak strike
 *   noise. out = dry + wet (default 0.25) x outL.
 * Length: the last onset plus max(decay, tail) plus 20 ms.
 */
export declare function chime(out: Float32Array, o: ChimeOpts, sr: number, rng: AudioRng): number;
/**
 * A noise swell plus a harmonic-stack chord under a rising-then-falling envelope (a raised
 * cosine up to 60 % of `seconds`, then a raised cosine down to 0).
 * - Noise: PinkNoise through an Svf lowpass (q 0.9) whose cutoff rises exponentially from
 *   400 Hz to 5 kHz across the swell, then a 250 Hz OnePole highpass (no sub rumble).
 * - Chord: one AdditiveBank per note of 6 + round(4 bright) partials, amplitude 1/k, ratio
 *   k detuned +/-3 cents (seeded), random phases; f0 wanders slowly by +/-2 cents (seeded
 *   0.1..0.3 Hz), updated per 128-frame block.
 * - Mix: chord x 1 + noise x (0.25 + 0.35 bright), each from unit peak.
 */
export declare function swell(out: Float32Array, o: SwellOpts, sr: number, rng: AudioRng): number;
