// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe meters: ITU-R BS.1770-4 / EBU R128 loudness, BS.1770-4 Annex 2 true peak,
// sample peak, RMS, windowed RMS, normalized cross-correlation and a non-finite counter.
// Runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.
// LoudnessMeter grows its block store (proof-time use); TruePeakDetector never allocates
// after construction. Every class has reset().
import { gainToDb } from "./math.js";
// ---------------------------------------------------------------------------------------
// K-weighting (BS.1770-4): a high-shelf pre-filter followed by the RLB highpass. The
// coefficients are recomputed for any sample rate from the analog prototypes (bilinear
// transform with prewarping); at 48 kHz they equal the published tables:
//   pre: b = [1.53512485958697, -2.69169618940638, 1.19839281085285]
//        a = [1, -1.69065929318241, 0.73248077421585]
//   RLB: b = [1, -2, 1], a = [1, -1.99004745483398, 0.99007225036621]
// ---------------------------------------------------------------------------------------
const PRE_HZ = 1681.974450955533;
const PRE_GAIN_DB = 3.999843853973347;
const PRE_Q = 0.7071752369554196;
const PRE_VB_EXP = 0.4996667741545416;
const RLB_HZ = 38.13547087602444;
const RLB_Q = 0.5003270373238773;
/** Loudness offset of BS.1770: L = -0.691 + 10 log10(sum G_i z_i). */
const LUFS_OFFSET = -0.691;
const ABS_GATE_LUFS = -70;
const REL_GATE_LU = -10;
/** Sub-blocks per 400 ms momentary block and per short-term window (3 s). */
const MOMENTARY_SUBS = 4;
const SHORT_TERM_SUBS = 30;
/** Normalized direct-form-I biquad in double precision (the meter needs no modulation). */
class KBiquad {
    b0 = 1;
    b1 = 0;
    b2 = 0;
    a1 = 0;
    a2 = 0;
    x1 = 0;
    x2 = 0;
    y1 = 0;
    y2 = 0;
    process(x) {
        const y = this.b0 * x + this.b1 * this.x1 + this.b2 * this.x2 - this.a1 * this.y1 - this.a2 * this.y2;
        this.x2 = this.x1;
        this.x1 = x;
        this.y2 = this.y1;
        this.y1 = y;
        return y;
    }
    reset() {
        this.x1 = 0;
        this.x2 = 0;
        this.y1 = 0;
        this.y2 = 0;
    }
}
function setPreFilter(f, sr) {
    const K = Math.tan((Math.PI * PRE_HZ) / sr);
    const Vh = Math.pow(10, PRE_GAIN_DB / 20);
    const Vb = Math.pow(Vh, PRE_VB_EXP);
    const a0 = 1 + K / PRE_Q + K * K;
    f.b0 = (Vh + (Vb * K) / PRE_Q + K * K) / a0;
    f.b1 = (2 * (K * K - Vh)) / a0;
    f.b2 = (Vh - (Vb * K) / PRE_Q + K * K) / a0;
    f.a1 = (2 * (K * K - 1)) / a0;
    f.a2 = (1 - K / PRE_Q + K * K) / a0;
}
function setRlbFilter(f, sr) {
    const K = Math.tan((Math.PI * RLB_HZ) / sr);
    const a0 = 1 + K / RLB_Q + K * K;
    f.b0 = 1;
    f.b1 = -2;
    f.b2 = 1;
    f.a1 = (2 * (K * K - 1)) / a0;
    f.a2 = (1 - K / RLB_Q + K * K) / a0;
}
function energyToLufs(e) {
    return e > 0 ? LUFS_OFFSET + 10 * Math.log10(e) : -Infinity;
}
function lufsToEnergy(l) {
    return Math.pow(10, (l - LUFS_OFFSET) / 10);
}
/**
 * ITU-R BS.1770-4 / EBU R128 loudness meter for a stereo program (channel weights 1, 1).
 * Energy accumulates per 100 ms sub-block; each new sub-block closes a 400 ms block made of
 * the last 4 sub-blocks (75% overlap) and, once 30 exist, a short-term window of 3 s.
 */
export class LoudnessMeter {
    sampleRate;
    subLen;
    preL = new KBiquad();
    rlbL = new KBiquad();
    preR = new KBiquad();
    rlbR = new KBiquad();
    /** Mean-square energy of the last SHORT_TERM_SUBS sub-blocks (ring). */
    subs = new Float64Array(SHORT_TERM_SUBS);
    subCount = 0;
    subAcc = 0;
    subFill = 0;
    /** Loudness (LUFS) of every completed 400 ms block; grown by doubling. */
    blocks = new Float64Array(1024);
    blockCount = 0;
    momMax = -Infinity;
    stMax = -Infinity;
    constructor(sr) {
        this.sampleRate = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.subLen = Math.max(1, Math.round(this.sampleRate * 0.1));
        setPreFilter(this.preL, this.sampleRate);
        setPreFilter(this.preR, this.sampleRate);
        setRlbFilter(this.rlbL, this.sampleRate);
        setRlbFilter(this.rlbR, this.sampleRate);
    }
    /** Feeds n stereo frames. Non-finite samples are metered as 0. */
    push(l, r, n) {
        const m = Math.min(n, l.length, r.length);
        for (let i = 0; i < m; i++) {
            const xl = l[i];
            const xr = r[i];
            const zl = this.rlbL.process(this.preL.process(Number.isFinite(xl) ? xl : 0));
            const zr = this.rlbR.process(this.preR.process(Number.isFinite(xr) ? xr : 0));
            this.subAcc += zl * zl + zr * zr;
            if (++this.subFill >= this.subLen)
                this.closeSubBlock();
        }
    }
    closeSubBlock() {
        this.subs[this.subCount % SHORT_TERM_SUBS] = this.subAcc / this.subLen;
        this.subCount++;
        this.subAcc = 0;
        this.subFill = 0;
        if (this.subCount >= MOMENTARY_SUBS) {
            const lufs = energyToLufs(this.meanLastSubs(MOMENTARY_SUBS));
            if (this.blockCount >= this.blocks.length) {
                const grown = new Float64Array(this.blocks.length * 2);
                grown.set(this.blocks);
                this.blocks = grown;
            }
            this.blocks[this.blockCount++] = lufs;
            if (lufs > this.momMax)
                this.momMax = lufs;
        }
        if (this.subCount >= SHORT_TERM_SUBS) {
            const lufs = energyToLufs(this.meanLastSubs(SHORT_TERM_SUBS));
            if (lufs > this.stMax)
                this.stMax = lufs;
        }
    }
    meanLastSubs(count) {
        let sum = 0;
        for (let k = 1; k <= count; k++)
            sum += this.subs[(this.subCount - k) % SHORT_TERM_SUBS];
        return sum / count;
    }
    /** Gated integrated loudness in LUFS; -Infinity when no block passes the gates. */
    integrated() {
        let sum = 0;
        let count = 0;
        for (let i = 0; i < this.blockCount; i++) {
            const b = this.blocks[i];
            if (b > ABS_GATE_LUFS) {
                sum += lufsToEnergy(b);
                count++;
            }
        }
        if (count === 0)
            return -Infinity;
        const relGate = energyToLufs(sum / count) + REL_GATE_LU;
        sum = 0;
        count = 0;
        for (let i = 0; i < this.blockCount; i++) {
            const b = this.blocks[i];
            if (b > ABS_GATE_LUFS && b > relGate) {
                sum += lufsToEnergy(b);
                count++;
            }
        }
        return count === 0 ? -Infinity : energyToLufs(sum / count);
    }
    /** Highest momentary (400 ms) loudness in LUFS; -Infinity before the first block. */
    momentaryMax() {
        return this.momMax;
    }
    /** Highest short-term (3 s) loudness in LUFS; -Infinity before the first 3 s. */
    shortTermMax() {
        return this.stMax;
    }
    /** Number of completed 400 ms blocks. */
    get blockTotal() {
        return this.blockCount;
    }
    reset() {
        this.preL.reset();
        this.rlbL.reset();
        this.preR.reset();
        this.rlbR.reset();
        this.subs.fill(0);
        this.subCount = 0;
        this.subAcc = 0;
        this.subFill = 0;
        this.blockCount = 0;
        this.momMax = -Infinity;
        this.stMax = -Infinity;
    }
}
// ---------------------------------------------------------------------------------------
// True peak (BS.1770-4 Annex 2): 4x oversampling with the published 48-tap interpolation
// FIR, split into 4 polyphase branches of 12 taps. Phase 3 is phase 0 reversed and phase 2
// is phase 1 reversed. The group delay is (48 - 1) / 2 / 4 = 5.875 input samples.
// ---------------------------------------------------------------------------------------
/** Oversampling factor of the true-peak interpolator. */
export const TP_OVERSAMPLE = 4;
/** Taps per polyphase branch (48 / 4). */
export const TP_TAPS = 12;
/** The 4 polyphase branches of the BS.1770-4 Annex 2 filter; y_k[n] = sum_j h_k[j] x[n - j]. */
export const TP_PHASE_TAPS = [
    new Float64Array([
        0.001708984375, 0.010986328125, -0.0196533203125, 0.033203125, -0.0594482421875, 0.1373291015625,
        0.97216796875, -0.102294921875, 0.0476074218750, -0.0266113281250, 0.0148925781250, -0.00830078125,
    ]),
    new Float64Array([
        -0.0291748046875, 0.029296875, -0.0517578125, 0.089111328125, -0.16650390625, 0.465087890625,
        0.77978515625, -0.2003173828125, 0.1015625, -0.0582275390625, 0.0330810546875, -0.0189208984375,
    ]),
    new Float64Array([
        -0.0189208984375, 0.0330810546875, -0.0582275390625, 0.1015625, -0.2003173828125, 0.77978515625,
        0.465087890625, -0.16650390625, 0.089111328125, -0.0517578125, 0.029296875, -0.0291748046875,
    ]),
    new Float64Array([
        -0.00830078125, 0.0148925781250, -0.0266113281250, 0.0476074218750, -0.102294921875, 0.97216796875,
        0.1373291015625, -0.0594482421875, 0.033203125, -0.0196533203125, 0.010986328125, 0.001708984375,
    ]),
];
/**
 * Streaming 4x true-peak detector: push one input sample, get the max |y| over the 4
 * interpolated phases at that position (delayed by the filter's ~5.9-sample group delay).
 * No allocation after construction.
 */
export class TruePeakDetector {
    /** Doubled history ring: hist[idx + j] = x[n - j] for j in 0..TP_TAPS-1. */
    hist = new Float64Array(2 * TP_TAPS);
    idx = 0;
    push(x) {
        const v = Number.isFinite(x) ? x : 0;
        this.idx = this.idx === 0 ? TP_TAPS - 1 : this.idx - 1;
        const h = this.hist;
        const o = this.idx;
        h[o] = v;
        h[o + TP_TAPS] = v;
        let peak = 0;
        for (let k = 0; k < TP_OVERSAMPLE; k++) {
            const c = TP_PHASE_TAPS[k];
            let y = 0;
            for (let j = 0; j < TP_TAPS; j++)
                y += c[j] * h[o + j];
            const a = y < 0 ? -y : y;
            if (a > peak)
                peak = a;
        }
        return peak;
    }
    reset() {
        this.hist.fill(0);
        this.idx = 0;
    }
}
function channelTruePeak(x, det) {
    det.reset();
    let peak = 0;
    for (let i = 0; i < x.length; i++) {
        const p = det.push(x[i]);
        if (p > peak)
            peak = p;
    }
    // Flush the filter so the interpolation around the last samples is measured too.
    for (let i = 0; i < TP_TAPS; i++) {
        const p = det.push(0);
        if (p > peak)
            peak = p;
    }
    return peak;
}
/**
 * True peak in dBTP: the max |x| of the 4x oversampled signal (BS.1770-4 Annex 2) over
 * both channels. r may be null for mono. sr is accepted for the API; the interpolator is
 * the fixed 4x Annex 2 filter.
 */
export function truePeakDb(l, r, sr) {
    void sr;
    const det = new TruePeakDetector();
    let peak = channelTruePeak(l, det);
    if (r)
        peak = Math.max(peak, channelTruePeak(r, det));
    return gainToDb(peak);
}
// ---------------------------------------------------------------------------------------
// Simple measurements.
// ---------------------------------------------------------------------------------------
/** Sample peak in dBFS over both channels (r may be null); -200 for silence. */
export function samplePeakDb(l, r) {
    let peak = 0;
    for (let i = 0; i < l.length; i++) {
        const a = Math.abs(l[i]);
        if (a > peak)
            peak = a;
    }
    if (r) {
        for (let i = 0; i < r.length; i++) {
            const a = Math.abs(r[i]);
            if (a > peak)
                peak = a;
        }
    }
    return gainToDb(peak);
}
/** RMS in dBFS over x[from, to) (clamped to the array); -200 for silence or an empty range. */
export function rmsDb(x, from, to) {
    const a = Math.max(0, Math.floor(from));
    const b = Math.min(x.length, Math.floor(to));
    if (!(b > a))
        return -200;
    let sum = 0;
    for (let i = a; i < b; i++)
        sum += x[i] * x[i];
    return gainToDb(Math.sqrt(sum / (b - a)));
}
/**
 * RMS in dBFS of consecutive non-overlapping windows of windowSec. A trailing partial window
 * is dropped, except that a signal shorter than one window yields one window over all of it.
 */
export function windowRmsDb(x, sr, windowSec) {
    const win = Math.max(1, Math.round(sr * windowSec));
    if (x.length === 0)
        return new Float32Array(0);
    const count = Math.floor(x.length / win);
    if (count === 0)
        return Float32Array.of(rmsDb(x, 0, x.length));
    const out = new Float32Array(count);
    for (let w = 0; w < count; w++)
        out[w] = rmsDb(x, w * win, (w + 1) * win);
    return out;
}
/**
 * Max over lags -maxLag..maxLag of |sum_i a[i] b[i + lag]| / sqrt(sum a^2 * sum b^2),
 * summing only where both indices exist. 0 when either signal is silent.
 */
export function maxNormXcorr(a, b, maxLag) {
    let ea = 0;
    let eb = 0;
    for (let i = 0; i < a.length; i++)
        ea += a[i] * a[i];
    for (let i = 0; i < b.length; i++)
        eb += b[i] * b[i];
    const den = Math.sqrt(ea * eb);
    if (!(den > 0))
        return 0;
    const L = Math.max(0, Math.floor(maxLag));
    let best = 0;
    for (let lag = -L; lag <= L; lag++) {
        const i0 = Math.max(0, -lag);
        const i1 = Math.min(a.length, b.length - lag);
        let s = 0;
        for (let i = i0; i < i1; i++)
            s += a[i] * b[i + lag];
        const c = Math.abs(s) / den;
        if (c > best)
            best = c;
    }
    return best;
}
/** Number of NaN or infinite samples. */
export function countNonFinite(x) {
    let n = 0;
    for (let i = 0; i < x.length; i++)
        if (!Number.isFinite(x[i]))
            n++;
    return n;
}
