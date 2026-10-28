// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, render writes into the given buffer, BedDef v0 fields; tuned for m1 safety/loudness (HAA tuning comments)
// The baked 'wind.passby' whoosh family and the wind bed (module content-core). DSP-safe: the same
// render() runs in the node proof, the build-time bake and the in-browser fallback.
//
// One family, bus 'effects', category 'fx', 12 seeded variants of 0.7 s each (8.4 s total,
// well under BANK_BUDGET_SECONDS). A variant draws its sweep, Q, envelope and noise character
// from the family variant stream, then renders a polished whoosh:
//   - white plus pink noise (both from the variant stream) through two Svf band-passes;
//   - the main band sweeps from ~3 kHz down to ~650 Hz, falling faster after the envelope
//     peak (a Doppler-like pitch fall); the second sits at ~1.6x that centre, 6..10 dB lower;
//   - a slight, seeded low-rumble bed;
//   - an asymmetric envelope peaking at 0.28..0.38 s that stays within 20 dB of the peak
//     between 0.15 and 0.55 s, then a 20 ms fade to 0 at exactly maxSeconds.
// A re-render from the same seed is bit-identical. Output is normalized to -6 dBFS sample peak.
import { BLOCK } from "../contract.js";
import { hashSeed } from "../dsp/rng.js";
import { DcBlocker, Svf } from "../dsp/filters.js";
import { BrownNoise, PinkNoise, WhiteNoise } from "../dsp/noise.js";
import { dbToGain, finiteOr, smoothstep } from "../dsp/math.js";
import { WindModel } from "../gen/windModel.js";
/** Variants per family (within FAMILY_VARIANTS_MIN..MAX). */
const VARIANTS = 12;
/** Family length cap in seconds. */
const MAX_SECONDS = 0.7;
/** Start-of-sweep base range (Hz) and the +/- jitter applied on top. */
const START_HZ_LO = 2500;
const START_HZ_HI = 3500;
/** End-of-sweep base range (Hz). */
const END_HZ_LO = 500;
const END_HZ_HI = 800;
/** Sweep-frequency jitter (+/- fraction of the picked value). */
const SWEEP_JITTER = 0.15;
/** Band-pass Q range. */
const Q_LO = 1.5;
const Q_HI = 4;
/** Envelope peak time range (s) and the rise/fall shape exponents. */
const PEAK_LO = 0.28;
const PEAK_HI = 0.38;
const RISE_EXP_LO = 1.2;
const RISE_EXP_HI = 2.2;
/** Capped so the envelope stays within 20 dB of the peak at 0.55 s for every peak time. */
const FALL_EXP_LO = 1.2;
const FALL_EXP_HI = 2;
/** Fraction of the sweep completed by the envelope peak; the rest falls faster after it. */
const U_PEAK_LO = 0.2;
const U_PEAK_HI = 0.4;
/** The second band-pass sits at this multiple of the main centre, with a little jitter. */
const SECOND_RATIO = 1.6;
const SECOND_RATIO_JITTER = 0.05;
const SECOND_RATIO_Q = 1.3;
/** The second band's level relative to the main one (dB). */
const SECOND_DB_LO = -10;
const SECOND_DB_HI = -6;
/** Slight low-rumble amount and its low-pass corner. */
const RUMBLE_LO = 0;
const RUMBLE_HI = 0.15;
const RUMBLE_HZ = 90;
/** The satisfying end fade. */
const FADE_SEC = 0.02;
/** White/pink blend weights of the noise source. */
const WHITE_MIX = 0.7;
const PINK_MIX = 0.5;
/** Output sample peak target. */
const TARGET_DB = -6;
/**
 * render() for one seeded 'wind.passby' variant. Writes the first round(MAX_SECONDS x sr)
 * frames of the given zeroed `out` (capped at out.length) and returns that count.
 */
function renderPassby(out, sampleRate, rng) {
    const sr = sampleRate > 1 && Number.isFinite(sampleRate) ? sampleRate : 48000;
    const n = Math.min(out.length, Math.round(MAX_SECONDS * sr));
    // Seeded, jittered sweep and character.
    const startHz = rng.range(START_HZ_LO, START_HZ_HI) * (1 + rng.range(-SWEEP_JITTER, SWEEP_JITTER));
    const endHz = rng.range(END_HZ_LO, END_HZ_HI) * (1 + rng.range(-SWEEP_JITTER, SWEEP_JITTER));
    const q = rng.range(Q_LO, Q_HI);
    const tPeak = rng.range(PEAK_LO, PEAK_HI);
    const riseExp = rng.range(RISE_EXP_LO, RISE_EXP_HI);
    const fallExp = rng.range(FALL_EXP_LO, FALL_EXP_HI);
    const uPeak = rng.range(U_PEAK_LO, U_PEAK_HI);
    const secondRatio = SECOND_RATIO * (1 + rng.range(-SECOND_RATIO_JITTER, SECOND_RATIO_JITTER));
    const secondGain = dbToGain(rng.range(SECOND_DB_LO, SECOND_DB_HI));
    const rumbleAmt = rng.range(RUMBLE_LO, RUMBLE_HI);
    // Independent seeded streams so every draw above stays reproducible.
    const white = new WhiteNoise(rng.fork(0x9e3779b9));
    const pink = new PinkNoise(rng.fork(0x85ebca6b));
    const rumble = new BrownNoise(rng.fork(0xc2b2ae35));
    const mainSvf = new Svf();
    const secondSvf = new Svf();
    const rumbleLp = new Svf();
    const span = Math.max(1e-4, MAX_SECONDS - tPeak);
    const fadeN = Math.max(1, Math.round(FADE_SEC * sr));
    for (let i = 0; i < n; i++) {
        const t = i / sr;
        // Non-linear sweep progress: slow to the peak, faster after it.
        const u = t <= tPeak
            ? uPeak * (t / tPeak)
            : Math.min(1, uPeak + (1 - uPeak) * ((t - tPeak) / span));
        const mainHz = startHz * Math.pow(endHz / startHz, u);
        mainSvf.set(mainHz, q, sr);
        secondSvf.set(mainHz * secondRatio, q * SECOND_RATIO_Q, sr);
        const src = WHITE_MIX * white.next() + PINK_MIX * pink.next();
        mainSvf.process(src);
        secondSvf.process(src);
        let s = mainSvf.bp + secondGain * secondSvf.bp;
        if (rumbleAmt > 0) {
            rumbleLp.set(RUMBLE_HZ, Math.SQRT1_2, sr);
            rumbleLp.process(rumble.next());
            s += rumbleAmt * rumbleLp.lp;
        }
        // Asymmetric envelope, peaking at tPeak.
        const env = t <= tPeak
            ? Math.pow(t / tPeak, riseExp)
            : Math.pow(Math.max(0, 1 - (t - tPeak) / span), fallExp);
        // Explicit 20 ms fade to 0 at the end of the buffer.
        const fade = i >= n - fadeN && fadeN > 1
            ? 0.5 * (1 + Math.cos((Math.PI * (i - (n - fadeN))) / (fadeN - 1)))
            : 1;
        out[i] = s * env * fade;
    }
    // HAA tuning: the brown-noise rumble left |DC| up to 6.8e-4 once normalized (m1 check 2);
    // a DcBlocker over the whole written length removes it before the peak normalization.
    const dc = new DcBlocker();
    for (let i = 0; i < n; i++)
        out[i] = dc.process(out[i]);
    // Normalize to a -6 dBFS sample peak (silence stays silence).
    let peak = 0;
    for (let i = 0; i < n; i++) {
        const a = Math.abs(out[i]);
        if (a > peak)
            peak = a;
    }
    if (peak > 0) {
        const gain = dbToGain(TARGET_DB) / peak;
        for (let i = 0; i < n; i++)
            out[i] *= gain;
    }
    return n;
}
/** The single baked wind pass-by whoosh family. */
export const WIND_FAMILIES = [
    {
        id: 'wind.passby',
        bus: 'effects',
        category: 'fx',
        variants: VARIANTS,
        maxSeconds: MAX_SECONDS,
        seed: hashSeed('wind', 'passby'),
        render: (out, sr, rng, _v) => renderPassby(out, sr, rng),
    },
];
/** Wind bed param count and the indices the gain staging reads or writes. */
const BED_PARAMS = 8;
const BED_SPEED = 0;
const BED_LEVEL = 7;
/** HAA tuning: fixed wind bed trim (dB); the raw bed peaked at -2.1 dBFS already at ~21 m/s. */
const BED_TRIM_DB = -3;
/** HAA tuning: the speed (m/s) above which the bed gain is reduced. */
const BED_REDUCE_FROM = 30;
/** HAA tuning: the reduction reached by BED_REDUCE_KNEE m/s, where the whistle and the
 * buffeting come in, then the extra reduction per doubling of speed above BED_REDUCE_FROM. */
const BED_REDUCE_KNEE = 40;
const BED_REDUCE_KNEE_DB = 7;
const BED_REDUCE_OCT_DB = 6;
/**
 * HAA tuning: the wind bed gain staging (linear), a pure function of speed. Above 30 m/s the
 * WindModel band gains, whistle and buffeting push the raw bed over the peak ceiling (+8 dBFS at
 * 50 m/s in the m1 sweep), so the reduction grows smoothly from 30 m/s to 50 m/s and beyond;
 * below 30 m/s only the fixed trim applies and the character is unchanged.
 */
export function windBedGain(speed) {
    const s = Math.max(0, finiteOr(speed, 0));
    const over = Math.max(1, s / BED_REDUCE_FROM);
    const db = BED_TRIM_DB
        - BED_REDUCE_KNEE_DB * smoothstep(BED_REDUCE_FROM, BED_REDUCE_KNEE, s)
        - BED_REDUCE_OCT_DB * Math.log2(over);
    return dbToGain(db);
}
/**
 * HAA tuning: the wind bed voice, a wrapper around WindModel that copies the params into a
 * scratch block preallocated here (the caller's block is never mutated, process allocates
 * nothing) and scales `level` by windBedGain(speed). WindModel ramps level per sample. The
 * model renders into preallocated stereo scratch, which a DcBlocker per channel cleans before
 * it is added to l/r: the brown-noise low end left |DC| 1.2e-3 in the 1 s windows above
 * 45 m/s (m1 check 2).
 */
class WindBedVoice {
    model;
    scratch = new Float32Array(BED_PARAMS);
    bufL = new Float32Array(BLOCK);
    bufR = new Float32Array(BLOCK);
    dcL = new DcBlocker();
    dcR = new DcBlocker();
    constructor(sr, seed) {
        this.model = new WindModel(sr, seed);
    }
    process(p, offset, l, r, n) {
        if (offset < 0)
            return;
        const q = this.scratch;
        for (let i = 0; i < BED_PARAMS; i++)
            q[i] = offset + i < p.length ? p[offset + i] : 0;
        q[BED_LEVEL] *= windBedGain(q[BED_SPEED]);
        const len = Math.max(0, Math.min(n | 0, l.length, r.length, BLOCK));
        if (len === 0)
            return;
        const bl = this.bufL, br = this.bufR;
        bl.fill(0, 0, len);
        br.fill(0, 0, len);
        this.model.process(q, 0, bl, br, len);
        const dl = this.dcL, dr = this.dcR;
        for (let i = 0; i < len; i++) {
            l[i] += dl.process(bl[i]);
            r[i] += dr.process(br[i]);
        }
    }
    reset() {
        this.model.reset();
        this.dcL.reset();
        this.dcR.reset();
        this.bufL.fill(0);
        this.bufR.fill(0);
    }
}
/**
 * The wind bed: one listener-level voice on the ambience bus, with the eight params in the
 * order WindModel reads them: speed, speed01, air, boost, slip, yaw, underwater and level.
 */
export const WIND_BED = {
    id: 'wind',
    bus: 'ambience',
    params: ['speed', 'speed01', 'air', 'boost', 'slip', 'yaw', 'underwater', 'level'],
    defaults: [0, 0, 0, 0, 0, 0, 0, 1],
    positional: false,
    create: (sr, seed) => new WindBedVoice(sr, seed),
};
