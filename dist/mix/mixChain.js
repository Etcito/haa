// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, renamed buses, string duck kinds, setUnderwater→setMuffle
// DSP-safe mixing chain: 6 buses (level × user gain → EQ → compressor → ducker), the FDN
// reverb return, and the master (trim × user master → muffle lowpass → glue compressor →
// true-peak limiter). Sample-rate agnostic (sr passed in). Every buffer and processor is
// preallocated at construction for BLOCK frames; process() never allocates. Runs unchanged
// in node, on the main thread and in AudioWorkletGlobalScope.
import { BLOCK, BUSES, BUS_INDEX, DEFAULT_USER_VOLUMES, REVERB_INPUT } from "../contract.js";
import { clamp, dbToGain, finiteOr, gainToDb } from "../dsp/math.js";
import { Biquad, Smoother, Svf } from "../dsp/filters.js";
import { Compressor, Ducker, TruePeakLimiter } from "../dsp/dynamics.js";
import { FdnReverb } from "../dsp/reverb.js";
const BUS_COUNT = BUSES.length;
const EQ_BANDS = 3;
/** User and bus-level gain glide. */
const USER_SMOOTH_SEC = 0.05;
/** Muffle lowpass glide. */
const MUFFLE_SMOOTH_SEC = 0.15;
const MUFFLE_OPEN_HZ = 20000;
const MUFFLE_CLOSED_HZ = 700;
const LN_MUFFLE = Math.log(MUFFLE_CLOSED_HZ / MUFFLE_OPEN_HZ);
/** Butterworth Q for the shelves and the muffle lowpass. */
const Q_FLAT = Math.SQRT1_2;
const MUSIC = BUS_INDEX.music;
const AMBIENCE = BUS_INDEX.ambience;
const METER_FLOOR_DB = -200;
/** Muffle amount change below which the lowpass coefficients are kept (the glide is asymptotic). */
const SVF_EPSILON = 1e-6;
export class MixChain {
    meter = { truePeakDb: METER_FLOOR_DB, reductionDb: 0 };
    sr;
    preset;
    // Per bus: two scratch channels, 3 biquads per channel, a compressor, a level glide and a
    // ducker (music and ambience only).
    busL = [];
    busR = [];
    /** Index (bus × 2 + channel) × EQ_BANDS + band. */
    eq = [];
    comps = [];
    levels = [];
    duckers = [];
    // Reverb return.
    reverb;
    returnGain;
    // Master.
    mixL = new Float32Array(BLOCK);
    mixR = new Float32Array(BLOCK);
    gMusic = new Float32Array(BLOCK);
    gSfx = new Float32Array(BLOCK);
    gMaster = new Float32Array(BLOCK);
    userMaster;
    userMusic;
    userSfx;
    trim;
    muffle;
    svfL = new Svf();
    svfR = new Svf();
    svfV = 0;
    glue;
    limiter;
    // Meter interpolation history per channel: x[n-3], x[n-2], x[n-1].
    histL = new Float64Array(3);
    histR = new Float64Array(3);
    peak = 0;
    /** False until the first process() after construction or reset(): parameter changes jump. */
    primed = false;
    constructor(sr, preset) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        const s = this.sr;
        for (let b = 0; b < BUS_COUNT; b++) {
            this.busL.push(new Float32Array(BLOCK));
            this.busR.push(new Float32Array(BLOCK));
            for (let k = 0; k < 2 * EQ_BANDS; k++)
                this.eq.push(new Biquad());
            this.comps.push(new Compressor(s));
            this.levels.push(new Smoother(USER_SMOOTH_SEC, s));
            this.duckers.push(b === MUSIC || b === AMBIENCE ? new Ducker(s) : null);
        }
        this.reverb = new FdnReverb(s);
        this.returnGain = new Smoother(USER_SMOOTH_SEC, s);
        this.userMaster = new Smoother(USER_SMOOTH_SEC, s);
        this.userMusic = new Smoother(USER_SMOOTH_SEC, s);
        this.userSfx = new Smoother(USER_SMOOTH_SEC, s);
        this.trim = new Smoother(USER_SMOOTH_SEC, s);
        this.muffle = new Smoother(MUFFLE_SMOOTH_SEC, s);
        this.glue = new Compressor(s);
        this.limiter = new TruePeakLimiter(s);
        this.userMaster.jump(DEFAULT_USER_VOLUMES.master);
        this.userMusic.jump(DEFAULT_USER_VOLUMES.music);
        this.userSfx.jump(DEFAULT_USER_VOLUMES.sfx);
        this.setSvf(0);
        this.preset = preset;
        this.setPreset(preset);
    }
    /** Re-apply the bus levels, EQ, compressors, glue, limiter, trim and reverb of p. */
    setPreset(p) {
        if (!p)
            return;
        this.preset = p;
        const sr = this.sr;
        for (let b = 0; b < BUS_COUNT; b++) {
            const bp = p.buses?.[BUSES[b]];
            if (!bp)
                continue;
            this.glide(this.levels[b], dbToGain(finiteOr(bp.levelDb, 0)));
            const e = bp.eq;
            for (let ch = 0; ch < 2; ch++) {
                const o = (b * 2 + ch) * EQ_BANDS;
                if (e) {
                    this.eq[o].set('lowshelf', e.lowShelfHz, Q_FLAT, e.lowShelfDb, sr);
                    this.eq[o + 1].set('peaking', e.peakHz, e.peakQ, e.peakDb, sr);
                    this.eq[o + 2].set('highshelf', e.highShelfHz, Q_FLAT, e.highShelfDb, sr);
                }
                else {
                    for (let k = 0; k < EQ_BANDS; k++)
                        this.eq[o + k].set('peaking', 1000, 1, 0, sr);
                }
            }
            if (bp.comp)
                this.comps[b].set(bp.comp);
        }
        const m = p.master;
        if (m) {
            this.glide(this.trim, dbToGain(finiteOr(m.trimDb, 0)));
            if (m.glue)
                this.glue.set(m.glue);
            if (m.limiter)
                this.limiter.set(m.limiter.ceilingDbtp, m.limiter.lookaheadMs, m.limiter.releaseMs);
        }
        if (p.reverb)
            this.setReverb(p.reverb);
    }
    /** User volumes, each clamped to 0..1 and smoothed over 50 ms. */
    setUser(master, music, sfx) {
        this.glide(this.userMaster, clamp(master, 0, 1));
        this.glide(this.userMusic, clamp(music, 0, 1));
        this.glide(this.userSfx, clamp(sfx, 0, 1));
    }
    /**
     * Trigger preset.ducks[kind] on the music and ambience buses, depths × strength (0..1).
     * A kind that is not an own key of preset.ducks is ignored.
     */
    duck(kind, strength) {
        const ducks = this.preset.ducks;
        if (!ducks || !Object.hasOwn(ducks, kind))
            return;
        const d = ducks[kind];
        if (!d)
            return;
        const s = clamp(strength, 0, 1);
        if (s <= 0)
            return;
        this.duckers[MUSIC]?.trigger(finiteOr(d.musicDb, 0) * s, d.attackMs, d.holdMs, d.releaseMs);
        this.duckers[AMBIENCE]?.trigger(finiteOr(d.ambienceDb, 0) * s, d.attackMs, d.holdMs, d.releaseMs);
    }
    /** Reverb parameters and return level; kept until the next setReverb or setPreset. */
    setReverb(p) {
        if (!p)
            return;
        this.reverb.set(p);
        this.glide(this.returnGain, dbToGain(finiteOr(p.returnDb, 0)));
    }
    /** Muffle amount 0..1: the master lowpass moves from 20 kHz (0) to 700 Hz (1) over 150 ms. */
    setMuffle(v) {
        this.glide(this.muffle, clamp(v, 0, 1));
    }
    /**
     * Mix n frames. inputs[i] is the stereo input of bus i (BUS_INDEX order), then REVERB_INPUT;
     * a missing or short input counts as silence (a lone left channel is used for both sides).
     * n above BLOCK is processed in BLOCK chunks.
     */
    process(inputs, outL, outR, n) {
        const total = Math.min(Math.floor(finiteOr(n, 0)), outL.length, outR.length);
        for (let off = 0; off < total; off += BLOCK) {
            this.processBlock(inputs, outL, outR, off, Math.min(BLOCK, total - off));
        }
    }
    /** Clear every filter, dynamics, reverb and meter state; glides snap to their targets. */
    reset() {
        this.resetDsp();
        this.peak = 0;
        this.meter.truePeakDb = METER_FLOOR_DB;
        this.meter.reductionDb = 0;
        this.primed = false;
    }
    processBlock(inputs, outL, outR, off, len) {
        this.primed = true;
        const end = off + len;
        const mixL = this.mixL;
        const mixR = this.mixR;
        const gMusic = this.gMusic;
        const gSfx = this.gSfx;
        const gMaster = this.gMaster;
        for (let i = 0; i < len; i++) {
            gMusic[i] = this.userMusic.next();
            gSfx[i] = this.userSfx.next();
            gMaster[i] = this.userMaster.next();
        }
        mixL.fill(0, 0, len);
        mixR.fill(0, 0, len);
        // 1. Buses: level × user gain → EQ → compressor → ducker (music, ambience) → sum.
        for (let b = 0; b < BUS_COUNT; b++) {
            const bl = this.busL[b];
            const br = this.busR[b];
            const level = this.levels[b];
            const user = b === MUSIC ? gMusic : gSfx;
            const src = inputs.length > b ? inputs[b] : undefined;
            const sl = src ? src[0] : undefined;
            const sr = src ? (src[1] ?? sl) : undefined;
            if (sl && sr && sl.length >= end && sr.length >= end) {
                for (let i = 0; i < len; i++) {
                    const g = level.next() * user[i];
                    bl[i] = sl[off + i] * g;
                    br[i] = sr[off + i] * g;
                }
            }
            else {
                for (let i = 0; i < len; i++)
                    level.next();
                bl.fill(0, 0, len);
                br.fill(0, 0, len);
            }
            const o = b * 2 * EQ_BANDS;
            for (let k = 0; k < EQ_BANDS; k++) {
                this.eq[o + k].processBlock(bl, len);
                this.eq[o + EQ_BANDS + k].processBlock(br, len);
            }
            this.comps[b].processStereo(bl, br, len);
            const duck = this.duckers[b];
            if (duck) {
                for (let i = 0; i < len; i++) {
                    const g = duck.next();
                    mixL[i] += bl[i] * g;
                    mixR[i] += br[i] * g;
                }
            }
            else {
                for (let i = 0; i < len; i++) {
                    mixL[i] += bl[i];
                    mixR[i] += br[i];
                }
            }
        }
        // 2. Reverb return: the send input → FdnReverb → × dB(returnDb) → sum.
        {
            const src = inputs.length > REVERB_INPUT ? inputs[REVERB_INPUT] : undefined;
            const sl = src ? src[0] : undefined;
            const sr = src ? (src[1] ?? sl) : undefined;
            const live = !!(sl && sr && sl.length >= end && sr.length >= end);
            const rv = this.reverb;
            const ret = this.returnGain;
            for (let i = 0; i < len; i++) {
                if (live)
                    rv.process(sl[off + i], sr[off + i]);
                else
                    rv.process(0, 0);
                const g = ret.next();
                mixL[i] += rv.outL * g;
                mixR[i] += rv.outR * g;
            }
        }
        // 3. Master: trim × user master → muffle lowpass → glue → true-peak limiter.
        let bad = false;
        const svfL = this.svfL;
        const svfR = this.svfR;
        for (let i = 0; i < len; i++) {
            const g = this.trim.next() * gMaster[i];
            const v = this.muffle.next();
            if (Math.abs(v - this.svfV) > SVF_EPSILON)
                this.setSvf(v);
            const xl = mixL[i] * g;
            const xr = mixR[i] * g;
            if (xl - xl !== 0 || xr - xr !== 0)
                bad = true;
            svfL.process(xl);
            svfR.process(xr);
            mixL[i] = svfL.lp;
            mixR[i] = svfR.lp;
        }
        this.glue.processStereo(mixL, mixR, len);
        this.limiter.processStereo(mixL, mixR, len);
        // 4. Guard: a non-finite sample anywhere zeroes the block and resets the chain.
        if (!bad) {
            for (let i = 0; i < len; i++) {
                const a = mixL[i];
                const c = mixR[i];
                if (a - a !== 0 || c - c !== 0) {
                    bad = true;
                    break;
                }
            }
        }
        if (bad) {
            outL.fill(0, off, end);
            outR.fill(0, off, end);
            this.resetDsp();
            this.meter.reductionDb = 0;
            return;
        }
        for (let i = 0; i < len; i++) {
            outL[off + i] = mixL[i];
            outR[off + i] = mixR[i];
        }
        this.measure(len);
        this.meter.reductionDb = this.limiter.reductionDb;
    }
    /** Running true-peak estimate of the limiter output: the samples plus a 4x Catmull-Rom interpolation. */
    measure(len) {
        this.peak = Math.max(this.peak, channelPeak(this.mixL, len, this.histL), channelPeak(this.mixR, len, this.histR));
        this.meter.truePeakDb = Math.max(METER_FLOOR_DB, gainToDb(this.peak));
    }
    setSvf(v) {
        this.svfV = v;
        const hz = MUFFLE_OPEN_HZ * Math.exp(LN_MUFFLE * v);
        this.svfL.set(hz, Q_FLAT, this.sr);
        this.svfR.set(hz, Q_FLAT, this.sr);
    }
    /** Set a glide target; before the first block (or after reset) the value jumps. */
    glide(s, target) {
        if (this.primed)
            s.target = target;
        else
            s.jump(target);
    }
    resetDsp() {
        for (let i = 0; i < this.eq.length; i++)
            this.eq[i].reset();
        for (let b = 0; b < BUS_COUNT; b++) {
            this.comps[b].reset();
            this.levels[b].reset();
            this.duckers[b]?.reset();
            this.busL[b].fill(0);
            this.busR[b].fill(0);
        }
        this.reverb.reset();
        this.returnGain.reset();
        this.userMaster.reset();
        this.userMusic.reset();
        this.userSfx.reset();
        this.trim.reset();
        this.muffle.reset();
        this.setSvf(this.muffle.value);
        this.svfL.reset();
        this.svfR.reset();
        this.glue.reset();
        this.limiter.reset();
        this.mixL.fill(0);
        this.mixR.fill(0);
        this.histL.fill(0);
        this.histR.fill(0);
    }
}
/** Max |x| over the samples and the 4x Catmull-Rom points between them; h carries x[n-3..n-1]. */
function channelPeak(x, len, h) {
    let p0 = h[0];
    let p1 = h[1];
    let p2 = h[2];
    let peak = 0;
    for (let i = 0; i < len; i++) {
        const p3 = x[i];
        const a = p3 < 0 ? -p3 : p3;
        if (a > peak)
            peak = a;
        // Points between p1 and p2 at t = 0.25, 0.5, 0.75.
        const c1 = p2 - p0;
        const c2 = 2 * p0 - 5 * p1 + 4 * p2 - p3;
        const c3 = 3 * (p1 - p2) + p3 - p0;
        for (let k = 1; k < 4; k++) {
            const t = k * 0.25;
            const y = p1 + 0.5 * t * (c1 + t * (c2 + t * c3));
            const ay = y < 0 ? -y : y;
            if (ay > peak)
                peak = ay;
        }
        p0 = p1;
        p1 = p2;
        p2 = p3;
    }
    h[0] = p0;
    h[1] = p1;
    h[2] = p2;
    return peak;
}
