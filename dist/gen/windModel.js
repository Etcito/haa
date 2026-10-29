// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// WindModel: the listener-level wind bed voice (module gen, owner M5). DSP-safe: it runs
// unchanged in node, on the main thread and in AudioWorkletGlobalScope.
//
// It is never plain white noise scaled in volume. Two decorrelated sources (L and R) each mix
// a PinkNoise with a smaller BrownNoise blend. Five SVF bandpasses per channel shape the
// spectrum at WIND_BANDS. Every band's linear gain is dB(levelDb) x min((speed/30)^exp, +12 dB),
// so the higher bands (larger exp) grow faster with speed and the spectrum rises non-linearly.
// The gains are recomputed at a 32-sample control rate and ramp linearly per sample (no zipper).
//
// Turbulence is a per-band seeded slow random modulator (0.3-4 Hz, smoothed white at the control
// rate, normalized to unit std) with a 2 dB std limited to +/-4 dB, growing with speed01. Seeded
// gusts fire every 1.5-5 s with a 0.5-1.5 s raised-sine envelope, adding +2..5 dB weighted to
// bands 0-2. Buffeting is a 20-60 Hz amplitude modulation (rate ∝ speed) applied to bands 0-1
// above speed01 0.7 with a smoothstep depth; its oscillator is only ever a gain, never an output.
//
// A two-polarity flutter whistle runs two Q-12 SVF bandpasses on the mono noise, drifting in
// 1.5-3 kHz ∝ speed (up to 50 m/s) with a +/-5% seeded drift, brought in by
// smoothstep(0.7, 1.2, speed01) and kept well under the bands so it never turns into a whine.
//
//   air        +3 dB overall and bands 0-1 cut by 9 dB (thin and exposed, same loudness)
//   boost      a bright highpassed rush plus an 8 dB lift of bands 3-4 (about +4 LU, brightest)
//   slip       -4 dB and a 2.5 kHz one-pole lowpass per channel, crossfaded by slip
//   yaw        an equal-power L/R balance, pan = clamp(yaw x 0.2, -0.3, 0.3)
//   underwater fades the whole voice to 0 over 150 ms (three 50 ms time constants)
//   level      the final multiplier
//
// process() ADDS into l/r, never allocates and never throws; a non-finite value resets the
// whole state and drops the block. When the level target is below 1e-4 and the internal output
// gain below 1e-5 it returns without touching a buffer, so an idle listener slot costs nothing.
// Nominal peaks sit around -6 dBFS at 30-40 m/s before the mix.
import { BLOCK } from "../contract.js";
import { OnePole, Smoother, Svf } from "../dsp/filters.js";
import { BrownNoise, PinkNoise } from "../dsp/noise.js";
import { AudioRng } from "../dsp/rng.js";
import { TAU, clamp, dbToGain, finiteOr, smoothstep } from "../dsp/math.js";
/** The five wind bands. The base level falls with frequency while the exponent rises, so fast
 *  air gains high end faster than low end: the speed sensation is a timbre change, not volume. */
export const WIND_BANDS = [
    { hz: 120, q: 0.9, levelDb: 11, exp: 1.6 },
    { hz: 400, q: 0.9, levelDb: 8, exp: 2.0 },
    { hz: 1200, q: 1.0, levelDb: 4, exp: 2.6 },
    { hz: 3500, q: 1.0, levelDb: 1, exp: 3.2 },
    { hz: 9000, q: 1.0, levelDb: -2, exp: 3.8 },
];
/** Param order in the flat block: speed, speed01, air, boost, slip, yaw, underwater, level. */
const P_SPEED = 0;
const P_SPEED01 = 1;
const P_AIR = 2;
const P_BOOST = 3;
const P_SLIP = 4;
const P_YAW = 5;
const P_UNDERWATER = 6;
const P_LEVEL = 7;
/** Samples between control-rate updates. */
const CONTROL = 32;
const BANDS = WIND_BANDS.length;
/** Speed the exponents are referenced to (m/s). */
const SPEED_REF = 30;
/** Speed that maps to a full whistle/buffeting build (m/s). */
const SPEED_FULL = 50;
/** dB cap of a band's base gain (+12 dB). */
const GAIN_CAP_DB = 12;
/** Brown blend of each channel source. */
const BROWN_MIX = 0.3;
/** Turbulence depth at full speed01 (dB per std) and seeded rate band (Hz). */
const TURB_MAX_DB = 2;
const TURB_HZ_LO = 0.3;
const TURB_HZ_HI = 4;
/** The smoothed modulators are normalized to unit std, then limited to this many std. */
const MOD_LIMIT = 2;
/** Gust schedule: wait band (s), envelope length band (s), amplitude band (dB). */
const GUST_WAIT_LO = 1.5;
const GUST_WAIT_HI = 5;
const GUST_DUR_LO = 0.5;
const GUST_DUR_HI = 1.5;
const GUST_DB_LO = 2;
const GUST_DB_HI = 5;
/** Gust weight per band: the low bands carry the gust. */
const GUST_WEIGHT = [1, 0.85, 0.55, 0.2, 0.05];
/** Buffeting: rate band (Hz), depth (linear AM), and the speed01 smoothstep range. */
const BUFF_HZ_LO = 20;
const BUFF_HZ_HI = 60;
const BUFF_DEPTH = 0.55;
const BUFF_SS_LO = 0.7;
const BUFF_SS_HI = 1.2;
/** Whistle: Q, center band (Hz), speed reference (m/s), drift depth, level and transition range. */
const WHISTLE_Q = 12;
const WHISTLE_HZ_LO = 1500;
const WHISTLE_HZ_HI = 3000;
const WHISTLE_SECOND = 1.3;
const WHISTLE_DRIFT = 0.025;
const WHISTLE_LEVEL = 0.3;
const WHISTLE_SS_LO = 0.7;
const WHISTLE_SS_HI = 1.2;
/** Air tilt: overall dB and the bands 0-1 cut (dB). */
const AIR_DB = 3;
const AIR_CUT_DB = -9;
/** Boost: highpass Hz, highpassed rush level and the bands 3-4 lift (dB). */
const BOOST_HP_HZ = 5000;
const BOOST_HP_GAIN = 3.5;
const BOOST_BAND_DB = 8;
/** Slip: overall dB and one-pole lowpass Hz. */
const SLIP_DB = -4;
const SLIP_LP_HZ = 2500;
/** Yaw: pan scale and pan limit. */
const YAW_PAN = 0.2;
const PAN_MAX = 0.3;
/** Underwater fade time constant (s): ~95% of the fade in 150 ms. */
const UNDER_SEC = 0.05;
/** Early-out thresholds. */
const LEVEL_EPS = 1e-4;
const OUT_EPS = 1e-5;
/** The listener-level wind voice. See the file header for the full model. */
export class WindModel {
    sr;
    controlDt;
    /** Decorrelated L/R sources: pink plus a smaller brown blend, from seeded forks. */
    rngPinkL;
    rngBrownL;
    rngPinkR;
    rngBrownR;
    pinkL;
    brownL;
    pinkR;
    brownR;
    pinkLSeed;
    brownLSeed;
    pinkRSeed;
    brownRSeed;
    /** One bandpass per band and channel. */
    bandL = [];
    bandR = [];
    /** Per-sample linear ramps of the band gains. */
    bandCur = new Float64Array(BANDS);
    bandStep = new Float64Array(BANDS);
    /** Turbulence: one seeded slow modulator per band. */
    turbRng;
    turbSeed;
    turbVal = new Float64Array(BANDS);
    turbCoef = new Float64Array(BANDS);
    /** Input scale giving each smoothed modulator unit std: sqrt(3 (2 - c) / c) for uniform white. */
    turbNorm = new Float64Array(BANDS);
    /** Gusts. */
    gustRng;
    gustSeed;
    gustTimer = 0;
    gustDur = 1;
    gustAmp = 0;
    gustPhase = 0;
    /** Buffeting quadrature oscillator (a gain only, never routed to the output). */
    bufCos = 1;
    bufSin = 0;
    /** Flutter whistle. */
    whistle0 = new Svf();
    whistle1 = new Svf();
    driftRng;
    driftSeed;
    driftCoef;
    driftNorm;
    driftVal0 = 0;
    driftVal1 = 0;
    /** Highpassed bright rush for boost. */
    boostHp = new OnePole();
    /** Drafting-pocket lowpass, one pole per channel. */
    slipLpL = new OnePole();
    slipLpR = new OnePole();
    /** Underwater fade (150 ms). */
    under;
    /** Scalar ramps. */
    whCur = 0;
    whStep = 0;
    boostCur = 0;
    boostStep = 0;
    slipCur = 0;
    slipStep = 0;
    slipGainCur = 1;
    slipGainStep = 0;
    panCurL = 1;
    panCurR = 1;
    panStepL = 0;
    panStepR = 0;
    levelCur = 0;
    levelStep = 0;
    /** The final scalar gain applied to the last sample, for the early-out test. */
    outGain = 0;
    /** Preallocated scratch so a non-finite sample never reaches the caller's buffers. */
    bufL = new Float32Array(BLOCK);
    bufR = new Float32Array(BLOCK);
    /** Per-call smoothed control values. */
    speed = 0;
    speed01 = 0;
    air = 0;
    boost = 0;
    slip = 0;
    yaw = 0;
    underwater = 0;
    level = 0;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.controlDt = CONTROL / this.sr;
        const base = new AudioRng(seed >>> 0);
        const rP = base.fork(0x9e3779b9);
        const rB = base.fork(0x85ebca6b);
        const rP2 = base.fork(0xc2b2ae35);
        const rB2 = base.fork(0x27d4eb2f);
        this.rngPinkL = rP;
        this.rngBrownL = rB;
        this.rngPinkR = rP2;
        this.rngBrownR = rB2;
        this.pinkL = new PinkNoise(rP);
        this.brownL = new BrownNoise(rB);
        this.pinkR = new PinkNoise(rP2);
        this.brownR = new BrownNoise(rB2);
        this.pinkLSeed = rP.state;
        this.brownLSeed = rB.state;
        this.pinkRSeed = rP2.state;
        this.brownRSeed = rB2.state;
        this.turbRng = base.fork(0x165667b1);
        this.turbSeed = this.turbRng.state;
        this.gustRng = base.fork(0xd3a2646c);
        this.gustSeed = this.gustRng.state;
        this.driftRng = base.fork(0xfd7046c5);
        this.driftSeed = this.driftRng.state;
        for (let b = 0; b < BANDS; b++) {
            const band = WIND_BANDS[b];
            const l = new Svf();
            const r = new Svf();
            l.set(band.hz, band.q, this.sr);
            r.set(band.hz, band.q, this.sr);
            this.bandL.push(l);
            this.bandR.push(r);
            this.turbCoef[b] = 1 - Math.exp(-TAU * (TURB_HZ_LO + (TURB_HZ_HI - TURB_HZ_LO) * this.turbRng.next()) * this.controlDt);
            this.turbNorm[b] = Math.sqrt((3 * (2 - this.turbCoef[b])) / this.turbCoef[b]);
        }
        this.driftCoef = 1 - Math.exp(-TAU * 0.7 * this.controlDt);
        this.driftNorm = Math.sqrt((3 * (2 - this.driftCoef)) / this.driftCoef);
        this.whistle0.set(WHISTLE_HZ_LO, WHISTLE_Q, this.sr);
        this.whistle1.set(WHISTLE_HZ_LO * WHISTLE_SECOND, WHISTLE_Q, this.sr);
        this.boostHp.setHighpass(BOOST_HP_HZ, this.sr);
        this.slipLpL.setLowpass(SLIP_LP_HZ, this.sr);
        this.slipLpR.setLowpass(SLIP_LP_HZ, this.sr);
        this.under = new Smoother(UNDER_SEC, this.sr);
        this.reset();
    }
    reset() {
        this.rngPinkL.state = this.pinkLSeed;
        this.rngBrownL.state = this.brownLSeed;
        this.rngPinkR.state = this.pinkRSeed;
        this.rngBrownR.state = this.brownRSeed;
        this.pinkL.reset();
        this.brownL.reset();
        this.pinkR.reset();
        this.brownR.reset();
        this.turbRng.state = this.turbSeed;
        this.gustRng.state = this.gustSeed;
        this.driftRng.state = this.driftSeed;
        for (let b = 0; b < BANDS; b++) {
            this.bandL[b].reset();
            this.bandR[b].reset();
            this.turbVal[b] = 0;
            this.bandCur[b] = 0;
            this.bandStep[b] = 0;
        }
        this.gustTimer = GUST_WAIT_LO + this.gustRng.range(0, GUST_WAIT_HI - GUST_WAIT_LO);
        this.gustDur = 1;
        this.gustAmp = 0;
        this.gustPhase = 0;
        this.bufCos = 1;
        this.bufSin = 0;
        this.whistle0.reset();
        this.whistle1.reset();
        this.driftVal0 = 0;
        this.driftVal1 = 0;
        this.boostHp.reset();
        this.slipLpL.reset();
        this.slipLpR.reset();
        this.under.jump(0);
        this.whCur = 0;
        this.whStep = 0;
        this.boostCur = 0;
        this.boostStep = 0;
        this.slipCur = 0;
        this.slipStep = 0;
        this.slipGainCur = 1;
        this.slipGainStep = 0;
        this.panCurL = 1;
        this.panCurR = 1;
        this.panStepL = 0;
        this.panStepR = 0;
        this.levelCur = 0;
        this.levelStep = 0;
        this.outGain = 0;
        this.speed = 0;
        this.speed01 = 0;
        this.air = 0;
        this.boost = 0;
        this.slip = 0;
        this.yaw = 0;
        this.underwater = 0;
        this.level = 0;
    }
    /** ADDS the wind voice into l/r[0..n), in chunks of at most BLOCK frames. */
    process(p, offset, l, r, n) {
        if (offset < 0)
            return;
        const len = Math.max(0, Math.min(n | 0, Math.min(l.length, r.length, BLOCK)));
        if (len === 0)
            return;
        this.speed = Math.max(0, finiteOr(p[offset + P_SPEED], 0));
        this.speed01 = clamp(finiteOr(p[offset + P_SPEED01], this.speed / SPEED_REF), 0, 1.5);
        this.air = clamp(finiteOr(p[offset + P_AIR], 0), 0, 1);
        this.boost = clamp(finiteOr(p[offset + P_BOOST], 0), 0, 1);
        this.slip = clamp(finiteOr(p[offset + P_SLIP], 0), 0, 1);
        this.yaw = finiteOr(p[offset + P_YAW], 0);
        this.underwater = clamp(finiteOr(p[offset + P_UNDERWATER], 0), 0, 1);
        this.level = Math.max(0, finiteOr(p[offset + P_LEVEL], 0));
        // Early out: level has fully faded and nothing rings any more.
        if (this.level < LEVEL_EPS && this.outGain < OUT_EPS)
            return;
        this.under.target = this.underwater;
        let i = 0;
        while (i < len) {
            const m = Math.min(CONTROL, len - i);
            this.control(m);
            this.renderChunk(m);
            let bad = false;
            for (let j = 0; j < m; j++) {
                if (!Number.isFinite(this.bufL[j]) || !Number.isFinite(this.bufR[j])) {
                    bad = true;
                    break;
                }
            }
            if (bad) {
                this.reset();
                return;
            }
            for (let j = 0; j < m; j++) {
                l[i + j] += this.bufL[j];
                r[i + j] += this.bufR[j];
            }
            i += m;
        }
        this.outGain = Math.abs(this.levelCur) * Math.abs(this.slipGainCur) * (1 - this.under.value);
    }
    /** Recompute every control-rate target and turn it into a per-sample linear ramp. */
    control(m) {
        const inv = 1 / m;
        // Gusts: a seeded schedule with a raised-sine envelope weighted to the low bands.
        this.gustTimer -= this.controlDt;
        if (this.gustPhase > 0) {
            this.gustPhase += this.controlDt / this.gustDur;
            if (this.gustPhase >= 1)
                this.gustPhase = 0;
        }
        else if (this.gustTimer <= 0) {
            this.gustAmp = GUST_DB_LO + this.gustRng.range(0, GUST_DB_HI - GUST_DB_LO);
            this.gustDur = GUST_DUR_LO + this.gustRng.range(0, GUST_DUR_HI - GUST_DUR_LO);
            this.gustPhase = 1e-6;
            this.gustTimer = GUST_WAIT_LO + this.gustRng.range(0, GUST_WAIT_HI - GUST_WAIT_LO);
        }
        const gustEnv = this.gustPhase > 0 ? Math.sin(Math.PI * this.gustPhase) : 0;
        // Buffeting quadrature rotation (a gain only).
        const buffRate = BUFF_HZ_LO + (BUFF_HZ_HI - BUFF_HZ_LO) * clamp(this.speed / SPEED_FULL, 0, 1);
        const w = TAU * buffRate * this.controlDt;
        const c = Math.cos(w);
        const s = Math.sin(w);
        const ns = this.bufSin * c + this.bufCos * s;
        this.bufCos = this.bufCos * c - this.bufSin * s;
        this.bufSin = ns;
        const buffMod = 1 + BUFF_DEPTH * smoothstep(BUFF_SS_LO, BUFF_SS_HI, this.speed01) * this.bufSin;
        const turbDepth = TURB_MAX_DB * Math.min(this.speed01, 1);
        const spe = this.speed > 1e-3 ? this.speed : 1e-3;
        const logRatio = Math.log10(spe / SPEED_REF);
        const airDb = AIR_DB * this.air;
        for (let b = 0; b < BANDS; b++) {
            const band = WIND_BANDS[b];
            let term = 20 * band.exp * logRatio;
            if (term > GAIN_CAP_DB)
                term = GAIN_CAP_DB;
            let gDb = band.levelDb + term;
            const white = (this.turbRng.next() * 2 - 1) * this.turbNorm[b];
            this.turbVal[b] += (white - this.turbVal[b]) * this.turbCoef[b];
            gDb += turbDepth * clamp(this.turbVal[b], -MOD_LIMIT, MOD_LIMIT);
            gDb += this.gustAmp * gustEnv * GUST_WEIGHT[b];
            gDb += airDb;
            if (b < 2)
                gDb += AIR_CUT_DB * this.air;
            else if (b >= 3)
                gDb += BOOST_BAND_DB * this.boost;
            let g = dbToGain(gDb);
            if (b < 2)
                g *= buffMod;
            this.bandStep[b] = (g - this.bandCur[b]) * inv;
        }
        // Flutter whistle: two drifting Q-12 bandpasses.
        const d0 = (this.driftRng.next() * 2 - 1) * this.driftNorm;
        this.driftVal0 += (d0 - this.driftVal0) * this.driftCoef;
        const d1 = (this.driftRng.next() * 2 - 1) * this.driftNorm;
        this.driftVal1 += (d1 - this.driftVal1) * this.driftCoef;
        const whBase = WHISTLE_HZ_LO + (WHISTLE_HZ_HI - WHISTLE_HZ_LO) * clamp(this.speed / SPEED_FULL, 0, 1);
        this.whistle0.set(whBase * (1 + WHISTLE_DRIFT * clamp(this.driftVal0, -MOD_LIMIT, MOD_LIMIT)), WHISTLE_Q, this.sr);
        this.whistle1.set(whBase * WHISTLE_SECOND * (1 + WHISTLE_DRIFT * clamp(this.driftVal1, -MOD_LIMIT, MOD_LIMIT)), WHISTLE_Q, this.sr);
        const whT = WHISTLE_LEVEL * smoothstep(WHISTLE_SS_LO, WHISTLE_SS_HI, this.speed01);
        this.whStep = (whT - this.whCur) * inv;
        const boostT = BOOST_HP_GAIN * this.boost;
        this.boostStep = (boostT - this.boostCur) * inv;
        this.slipStep = (this.slip - this.slipCur) * inv;
        const slipGainT = dbToGain(SLIP_DB * this.slip);
        this.slipGainStep = (slipGainT - this.slipGainCur) * inv;
        const pan = clamp(this.yaw * YAW_PAN, -PAN_MAX, PAN_MAX);
        const a = ((pan + 1) * Math.PI) / 4;
        this.panStepL = (Math.cos(a) - this.panCurL) * inv;
        this.panStepR = (Math.sin(a) - this.panCurR) * inv;
        this.levelStep = (this.level - this.levelCur) * inv;
    }
    /** Renders m <= CONTROL frames into the scratch buffers. */
    renderChunk(m) {
        const pinkW = 1 - BROWN_MIX;
        for (let j = 0; j < m; j++) {
            const sl = this.pinkL.next() * pinkW + this.brownL.next() * BROWN_MIX;
            const srN = this.pinkR.next() * pinkW + this.brownR.next() * BROWN_MIX;
            let accL = 0;
            let accR = 0;
            for (let b = 0; b < BANDS; b++) {
                const bl = this.bandL[b];
                const br = this.bandR[b];
                bl.process(sl);
                br.process(srN);
                const g = this.bandCur[b];
                accL += bl.bp * g;
                accR += br.bp * g;
            }
            const mono = (sl + srN) * 0.5;
            const hp = this.boostHp.process(mono) * this.boostCur;
            accL += hp;
            accR += hp;
            this.whistle0.process(mono);
            this.whistle1.process(mono);
            const wh = (this.whistle0.bp + this.whistle1.bp) * this.whCur;
            accL += wh;
            accR += wh;
            const ll = this.slipLpL.process(accL);
            const rr = this.slipLpR.process(accR);
            accL += (ll - accL) * this.slipCur;
            accR += (rr - accR) * this.slipCur;
            const mg = this.levelCur * this.slipGainCur * (1 - this.under.next());
            accL *= mg;
            accR *= mg;
            this.bufL[j] = accL * this.panCurL;
            this.bufR[j] = accR * this.panCurR;
            for (let b = 0; b < BANDS; b++)
                this.bandCur[b] += this.bandStep[b];
            this.whCur += this.whStep;
            this.boostCur += this.boostStep;
            this.slipCur += this.slipStep;
            this.slipGainCur += this.slipGainStep;
            this.panCurL += this.panStepL;
            this.panCurR += this.panStepR;
            this.levelCur += this.levelStep;
        }
    }
}
