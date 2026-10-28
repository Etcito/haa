import { DcBlocker } from "../dsp/filters.js";
import { hashSeed } from "../dsp/rng.js";
import { normalizePeak } from "../gen/fxKit.js";
import { Saturator } from "../dsp/shaper.js";
import { chime, softClick, swell, tick } from "../gen/uiKit.js";
/** Variants per UI family (within FAMILY_VARIANTS_MIN..MAX). */
const VARIANTS = 8;
/** Nominal peak of a rendered variant (dBFS). */
const PEAK_DB = -6;
/** The cosine fade that ends every variant at maxSeconds (s). */
const TAIL_FADE = 0.01;
/** HAA tuning: the roulette tick saturator drive (2x oversampled, symmetric). */
const ROULETTE_DRIVE = 2.5;
/** Draws the jitter record, in field order, from the variant stream. */
function drawJitter(rng) {
    return {
        pitch: 1 + rng.range(-0.015, 0.015),
        bright: rng.range(-0.15, 0.15),
        decay: 1 + rng.range(-0.15, 0.15),
        spacing: 1 + rng.range(-0.1, 0.1),
        transient: 1 + rng.range(-0.2, 0.2),
    };
}
/** A cosine fade over the last TAIL_FADE seconds of out. */
function tailFade(out, sr) {
    const n = Math.min(out.length, Math.max(1, Math.round(TAIL_FADE * sr)));
    const from = out.length - n;
    for (let i = 0; i < n; i++)
        out[from + i] *= 0.5 + 0.5 * Math.cos((Math.PI * (i + 1)) / n);
}
/** HAA tuning: removes the clicks' DC offset over the whole written length (tail included),
 * before the peak normalization, so the normalized variants stay under |DC| 5e-4 (m1 check 2). */
function dcBlock(out) {
    const dc = new DcBlocker();
    for (let i = 0; i < out.length; i++)
        out[i] = dc.process(out[i]);
}
/** The shared UI family shape. render runs on the first round(maxSeconds x sr) frames of the
 * given zeroed out and returns that count. */
function uiFamily(id, maxSeconds, recipe) {
    return {
        id,
        bus: 'ui',
        category: 'ui',
        variants: VARIANTS,
        maxSeconds,
        seed: hashSeed('ui', id),
        render: (buf, sampleRate, rng, _v) => {
            const n = Math.min(buf.length, Math.round(maxSeconds * sampleRate));
            const out = buf.subarray(0, n);
            const j = drawJitter(rng);
            recipe(out, sampleRate, rng, j);
            dcBlock(out);
            normalizePeak(out, PEAK_DB);
            tailFade(out, sampleRate);
            return n;
        },
    };
}
/** A base brightness plus the jitter, kept in 0..1. */
function br(base, j) {
    const b = base + j.bright;
    return b < 0 ? 0 : b > 1 ? 1 : b;
}
/** Notes (Hz) scaled by the pitch jitter. */
function hz(notes, j) {
    return notes.map((f) => f * j.pitch);
}
/** Seconds to samples. */
function at(seconds, sr) {
    return Math.round(seconds * sr);
}
/** Menu move: a quiet plastic tick and a short, mellow E6 'tok' (t60 0.2 s). Its strike noise
 * feeds a damped room tail (air), so the end of the move up to 0.14 s is diffuse air rather
 * than a ringing sine, and ten moves in a row stay soft. */
const move = (out, sr, rng, j) => {
    softClick(out, { material: 'plastic', pitch: j.pitch, brightness: br(0.3, j), level: 0.3 * j.transient }, sr, rng);
    chime(out, { notes: hz([1319], j), spacing: 0, bright: br(0.4, j), tail: 0.5 * j.decay, decay: 0.2 * j.decay, fm: 0.1, wet: 0.6, air: 2, level: 0.8, at: at(0.003, sr) }, sr, rng);
};
/** Menu select: a glass click, then a rising fifth that lands on a major dyad (D6, then
 * A6 + C#7), bright and ringing: clearly an affirmative 'yes'. */
const select = (out, sr, rng, j) => {
    softClick(out, { material: 'glass', pitch: j.pitch, brightness: br(0.55, j), level: 0.6 * j.transient }, sr, rng);
    chime(out, { notes: hz([1175], j), spacing: 0, bright: br(0.5, j), tail: 0.2 * j.decay, decay: 0.14 * j.decay, level: 0.7, at: at(0.004, sr) }, sr, rng);
    chime(out, { notes: hz([1760, 2217], j), spacing: 0.006, bright: br(0.6, j), tail: 0.22 * j.decay, decay: 0.2 * j.decay, level: 1, at: at(0.004 + 0.055 * j.spacing, sr) }, sr, rng);
};
/** Menu back: the darker inverse of select: a dull plastic click, then a falling fifth
 * (C6 to F5), low brightness, softer second note and a shorter decay. */
const back = (out, sr, rng, j) => {
    softClick(out, { material: 'plastic', pitch: 0.85 * j.pitch, brightness: br(0.2, j), level: 0.6 * j.transient }, sr, rng);
    chime(out, { notes: hz([1047], j), spacing: 0, bright: br(0.15, j), tail: 0.25 * j.decay, decay: 0.14 * j.decay, fm: 0.12, wet: 0.35, level: 0.9, at: at(0.004, sr) }, sr, rng);
    chime(out, { notes: hz([698], j), spacing: 0, bright: br(0.25, j), tail: 0.25 * j.decay, decay: 0.18 * j.decay, fm: 0.25, wet: 0.35, level: 0.75, at: at(0.004 + 0.055 * j.spacing, sr) }, sr, rng);
};
/** Countdown: one sustained A5 bell (t60 0.9 s, cut at 0.3 s) with a strong harmonic FM layer
 * (ratio 3: 2f, 4f, 5f sidebands) whose near-constant envelope keeps the level up after the
 * strike; its energy sits at 0.9..4 kHz, above the engines' low-mid rumble. */
const countdown = (out, sr, rng, j) => {
    softClick(out, { material: 'glass', pitch: 1.2 * j.pitch, brightness: br(0.8, j), level: 0.15 * j.transient }, sr, rng);
    chime(out, { notes: hz([880], j), spacing: 0, bright: br(0.35, j), tail: 0.25 * j.decay, decay: 0.9 * j.decay, fm: 1.2, fmRatio: 3, wet: 0.2, level: 1 }, sr, rng);
};
/** Go: an octave above the countdown, brighter and fuller: a strummed A-major chord (A6, C#7,
 * E7, A7) struck ON the event with a glass click and a bright FM layer, and an A-major stack
 * (A5..A6) blooming underneath over the first 0.24 s. */
const go = (out, sr, rng, j) => {
    softClick(out, { material: 'glass', pitch: 1.3 * j.pitch, brightness: br(0.9, j), level: 0.45 * j.transient }, sr, rng);
    chime(out, { notes: hz([1760, 2217, 2637, 3520], j), spacing: 0.007 * j.spacing, bright: br(0.95, j), tail: 0.3 * j.decay, decay: 0.55 * j.decay, fm: 1.3, fmRatio: 3, wet: 0.25, level: 1 }, sr, rng);
    swell(out, { notes: hz([880, 1109, 1319, 1760], j), seconds: 0.4 * j.spacing, bright: br(0.6, j), level: 0.5 }, sr, rng);
};
/** Lap: an ascending C-major arpeggio that resolves on the octave (C6 E6 G6 C7). */
const lap = (out, sr, rng, j) => {
    chime(out, { notes: hz([1047, 1319, 1568, 2093], j), spacing: 0.06 * j.spacing, bright: br(0.65, j), tail: 0.25 * j.decay, decay: 0.3 * j.decay, fm: 0.25, fmRatio: 3, level: 1 }, sr, rng);
};
/** Place up: a bright glass tick and a quick rising fourth (E6 to A6), ringing. */
const placeUp = (out, sr, rng, j) => {
    softClick(out, { material: 'glass', pitch: 1.1 * j.pitch, brightness: br(0.7, j), level: 0.45 * j.transient }, sr, rng);
    chime(out, { notes: hz([1319, 1760], j), spacing: 0.05 * j.spacing, bright: br(0.75, j), tail: 0.15 * j.decay, decay: 0.18 * j.decay, level: 1 }, sr, rng);
};
/** Place down: the opposite gesture, low and muted: a dull plastic knock and a falling minor
 * third (B5 to G#5), dark and quickly damped. */
const placeDown = (out, sr, rng, j) => {
    softClick(out, { material: 'plastic', pitch: 0.8 * j.pitch, brightness: br(0.15, j), level: 0.4 * j.transient }, sr, rng);
    chime(out, { notes: hz([988], j), spacing: 0, bright: br(0.3, j), tail: 0.15 * j.decay, decay: 0.16 * j.decay, fm: 0.25, level: 0.9, at: at(0.004, sr) }, sr, rng);
    chime(out, { notes: hz([831], j), spacing: 0, bright: br(0.2, j), tail: 0.15 * j.decay, decay: 0.14 * j.decay, fm: 0.25, level: 0.85, at: at(0.004 + 0.06 * j.spacing, sr) }, sr, rng);
};
/** Item get: a glass click plus 3 fast sparkle chime notes. */
const itemGet = (out, sr, rng, j) => {
    softClick(out, { material: 'glass', pitch: 1.15 * j.pitch, brightness: br(0.6, j), level: 0.6 * j.transient }, sr, rng);
    chime(out, { notes: hz([1760, 2217, 2637], j), spacing: 0.035 * j.spacing, bright: br(0.6, j), tail: 0.12 * j.decay, decay: 0.15 * j.decay, fm: 0.25, level: 0.9, at: at(0.008, sr) }, sr, rng);
};
/** Roulette: a ratchet-pawl wood tick with a downward pitch drift. A tick is too short for
 * the shared jitter to decorrelate its variants, so it widens its own seeded pitch (+/- 12%)
 * and drift. */
const roulette = (out, sr, rng, j) => {
    const pitch = j.pitch * (1 + rng.range(-0.12, 0.12));
    const drift = -rng.range(0.04, 0.16) * j.spacing;
    tick(out, { material: 'wood', pitch, drift, level: j.transient }, sr, rng);
    // HAA tuning: the bare tick's crest factor kept the family 3 LU under the ui target at the
    // normalization peak cap (m1 check 5); a 2x oversampled tanh at unit peak thickens its body.
    normalizePeak(out, 0);
    const sat = new Saturator(2);
    sat.set(ROULETTE_DRIVE, 0, 1);
    for (let i = 0; i < out.length; i++)
        out[i] = sat.process(out[i]);
};
/** Every UI and cue family registered by this file (2.49 s x 8 = 19.92 s). */
export const UI_FAMILIES = [
    uiFamily('ui.move', 0.14, move),
    uiFamily('ui.select', 0.25, select),
    uiFamily('ui.back', 0.2, back),
    uiFamily('cue.countdown', 0.3, countdown),
    uiFamily('cue.go', 0.45, go),
    uiFamily('cue.lap', 0.45, lap),
    uiFamily('cue.placeUp', 0.21, placeUp),
    uiFamily('cue.placeDown', 0.21, placeDown),
    uiFamily('cue.itemGet', 0.2, itemGet),
    uiFamily('cue.roulette', 0.08, roulette),
];
