// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe oversampling and saturation. A polyphase halfband FIR (31 taps per 2x stage,
// 4x = two cascaded stages) and a normalized asymmetric fastTanh saturator evaluated at the
// oversampled rate. Runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.
// No allocation after construction; every class has reset().
import { fastTanh, finiteOr } from "./math.js";
// Halfband of 31 taps h[0..30]: centre h[15] = 0.5, h[15 ± even] = 0.
/** Number of even-indexed taps h[0], h[2], ..., h[30]: the non-trivial polyphase branch. */
const HB_BRANCH = 16;
/** Free coefficients c_i = h[15 ± (2i - 1)], i = 1..8 (the filter is symmetric). */
const HB_FREE = 8;
/**
 * Design bands, in cycles per sample of the oversampled rate. By the halfband symmetry
 * A(f) + A(0.5 - f) = 1, so the ripple on [0, 0.2] (0.4 fs at the base rate) equals the
 * magnitude on [0.3, 0.5]: TRANS_BOUND keeps the passband within 0.05 dB (0.1 dB up+down)
 * up to 0.4 fs, STOP_BOUND gives >= 70 dB of stopband from STOP_EDGE (0.67 fs) upward.
 */
const TRANS_EDGE = 0.3;
const STOP_EDGE = 0.335;
const TRANS_BOUND = 0.0045;
const STOP_BOUND = 2.9e-4;
const GRID = 240;
const LAWSON_ITERS = 120;
/**
 * Weighted minimax halfband design (Lawson's iteratively reweighted least squares), run
 * once at module load in plain deterministic code. A Kaiser-windowed sinc of this length
 * cannot hold both the 0.1 dB passband to 0.4 fs and the 70 dB stopband, so the free taps
 * are solved for directly. Returns the even-indexed taps h[0], h[2], ..., h[30].
 */
function designHalfband() {
    const cosM = new Float64Array(GRID * HB_FREE);
    const inv2 = new Float64Array(GRID);
    const w = new Float64Array(GRID);
    for (let g = 0; g < GRID; g++) {
        const f = TRANS_EDGE + ((0.5 - TRANS_EDGE) * g) / (GRID - 1);
        for (let i = 0; i < HB_FREE; i++)
            cosM[g * HB_FREE + i] = 2 * Math.cos(2 * Math.PI * f * (2 * i + 1));
        const b = f < STOP_EDGE ? TRANS_BOUND : STOP_BOUND;
        inv2[g] = 1 / (b * b);
        w[g] = 1 / GRID;
    }
    const m = new Float64Array(HB_FREE * HB_FREE);
    const rhs = new Float64Array(HB_FREE);
    const c = new Float64Array(HB_FREE);
    for (let it = 0; it < LAWSON_ITERS; it++) {
        // Normal equations of min sum_g w_g / B_g^2 * (0.5 + sum_i c_i cosM[g,i])^2.
        m.fill(0);
        rhs.fill(0);
        for (let g = 0; g < GRID; g++) {
            const wg = w[g] * inv2[g];
            const row = g * HB_FREE;
            for (let i = 0; i < HB_FREE; i++) {
                const ri = wg * cosM[row + i];
                rhs[i] -= ri * 0.5;
                for (let k = 0; k < HB_FREE; k++)
                    m[i * HB_FREE + k] += ri * cosM[row + k];
            }
        }
        solveInPlace(m, rhs, HB_FREE);
        c.set(rhs);
        // Lawson update: weights grow where the weighted error is largest.
        let sum = 0;
        for (let g = 0; g < GRID; g++) {
            let a = 0.5;
            const row = g * HB_FREE;
            for (let i = 0; i < HB_FREE; i++)
                a += c[i] * cosM[row + i];
            w[g] *= Math.abs(a) * Math.sqrt(inv2[g]);
            sum += w[g];
        }
        if (!(sum > 0) || !Number.isFinite(sum))
            break;
        for (let g = 0; g < GRID; g++)
            w[g] /= sum;
    }
    // Even-indexed taps: h[15 - (2i - 1)] = h[15 + (2i - 1)] = c_i.
    const even = new Float64Array(HB_BRANCH);
    for (let i = 0; i < HB_FREE; i++) {
        even[7 - i] = c[i];
        even[8 + i] = c[i];
    }
    return even;
}
/** Gaussian elimination with partial pivoting; the solution replaces b. Load time only. */
function solveInPlace(a, b, n) {
    for (let col = 0; col < n; col++) {
        let p = col;
        for (let r = col + 1; r < n; r++)
            if (Math.abs(a[r * n + col]) > Math.abs(a[p * n + col]))
                p = r;
        if (p !== col) {
            for (let k = 0; k < n; k++) {
                const t = a[col * n + k];
                a[col * n + k] = a[p * n + k];
                a[p * n + k] = t;
            }
            const t = b[col];
            b[col] = b[p];
            b[p] = t;
        }
        const piv = a[col * n + col];
        if (piv === 0)
            continue;
        for (let r = col + 1; r < n; r++) {
            const f = a[r * n + col] / piv;
            if (f === 0)
                continue;
            for (let k = col; k < n; k++)
                a[r * n + k] -= f * a[col * n + k];
            b[r] -= f * b[col];
        }
    }
    for (let r = n - 1; r >= 0; r--) {
        let s = b[r];
        for (let k = r + 1; k < n; k++)
            s -= a[r * n + k] * b[k];
        const piv = a[r * n + r];
        b[r] = piv !== 0 ? s / piv : 0;
    }
}
/** Even-indexed halfband taps h[0], h[2], ..., h[30] (shared by every stage). */
const HB_EVEN = designHalfband();
/** Upsampler branch taps: 2 h[2j] (the zero-stuffing gain of 2 folded in). */
const HB_UP = new Float64Array(HB_BRANCH);
for (let j = 0; j < HB_BRANCH; j++)
    HB_UP[j] = 2 * HB_EVEN[j];
/**
 * One 2x halfband stage with independent interpolator and decimator states. Rings are
 * written twice (i and i + 16) so every convolution reads a contiguous span.
 * Up then down delays by 15 base-rate samples (30 taps at the high rate).
 */
class HalfbandStage {
    upBuf = new Float64Array(HB_BRANCH * 2);
    upIdx = 0;
    evBuf = new Float64Array(HB_BRANCH * 2);
    odBuf = new Float64Array(HB_BRANCH * 2);
    dnIdx = 0;
    /** Writes 2 high-rate samples at out[o], out[o + 1]. */
    up(x, out, o) {
        const idx = (this.upIdx + HB_BRANCH - 1) & (HB_BRANCH - 1);
        this.upIdx = idx;
        const b = this.upBuf;
        b[idx] = x;
        b[idx + HB_BRANCH] = x;
        let y = 0;
        for (let j = 0; j < HB_BRANCH; j++)
            y += HB_UP[j] * b[idx + j];
        out[o] = y;
        // Odd phase: the centre tap alone, 2 * 0.5 * x[m - 7].
        out[o + 1] = b[idx + 7];
    }
    /** Consumes one high-rate pair (even, odd) and returns one base-rate sample. */
    down(e, od) {
        const idx = (this.dnIdx + HB_BRANCH - 1) & (HB_BRANCH - 1);
        this.dnIdx = idx;
        const ev = this.evBuf;
        const ob = this.odBuf;
        ev[idx] = e;
        ev[idx + HB_BRANCH] = e;
        ob[idx] = od;
        ob[idx + HB_BRANCH] = od;
        let y = 0.5 * ob[idx + 8];
        for (let j = 0; j < HB_BRANCH; j++)
            y += HB_EVEN[j] * ev[idx + j];
        return y;
    }
    reset() {
        this.upBuf.fill(0);
        this.evBuf.fill(0);
        this.odBuf.fill(0);
        this.upIdx = 0;
        this.dnIdx = 0;
    }
}
/**
 * Polyphase halfband oversampler: 2x is one stage, 4x two cascaded stages. up() writes
 * `factor` samples into the caller's preallocated array; down() consumes them.
 * An up/down round trip delays by `latency` whole base-rate samples (15 at 2x, 23 at 4x).
 */
export class Oversampler {
    factor;
    latency;
    s1 = new HalfbandStage();
    s2;
    mid = new Float64Array(2);
    /** 4x only: one 2x-rate sample of delay so the round trip lands on a whole base sample. */
    hold = 0;
    constructor(factor) {
        this.factor = factor === 4 ? 4 : 2;
        this.s2 = this.factor === 4 ? new HalfbandStage() : null;
        this.latency = this.factor === 4 ? 23 : 15;
    }
    up(x, out) {
        const s2 = this.s2;
        if (s2 === null) {
            this.s1.up(x, out, 0);
            return;
        }
        const mid = this.mid;
        this.s1.up(x, mid, 0);
        s2.up(mid[0], out, 0);
        s2.up(mid[1], out, 2);
    }
    down(buf) {
        const s2 = this.s2;
        if (s2 === null)
            return this.s1.down(buf[0], buf[1]);
        const b0 = s2.down(buf[0], buf[1]);
        const b1 = s2.down(buf[2], buf[3]);
        const y = this.s1.down(this.hold, b0);
        this.hold = b1;
        return y;
    }
    reset() {
        this.s1.reset();
        if (this.s2 !== null)
            this.s2.reset();
        this.mid.fill(0);
        this.hold = 0;
    }
}
/** Dry-path delay ring (power of two, longer than the 4x latency). */
const DRY_RING = 32;
/**
 * Asymmetric normalized tanh saturator:
 * y = (fastTanh(drive (x + asym)) - fastTanh(drive asym)) / fastTanh(drive),
 * evaluated at the oversampled rate (factor 1 bypasses oversampling), then mixed with the
 * latency-aligned dry signal by `mix`.
 */
export class Saturator {
    factor;
    os;
    buf;
    dry = new Float64Array(DRY_RING);
    dryIdx = 0;
    latency;
    drive = 1;
    asym = 0;
    offset = 0;
    norm = 1;
    mix = 1;
    constructor(factor) {
        this.factor = factor === 4 ? 4 : factor === 2 ? 2 : 1;
        this.os = this.factor === 1 ? null : new Oversampler(this.factor);
        this.buf = new Float32Array(this.factor);
        this.latency = this.os === null ? 0 : this.os.latency;
        this.set(1, 0, 1);
    }
    set(drive, asym, mix) {
        const d = Math.max(1e-3, finiteOr(drive, 1));
        const a = finiteOr(asym, 0);
        this.drive = d;
        this.asym = a;
        this.offset = fastTanh(d * a);
        this.norm = 1 / fastTanh(d);
        const m = finiteOr(mix, 1);
        this.mix = m < 0 ? 0 : m > 1 ? 1 : m;
    }
    shape(x) {
        return (fastTanh(this.drive * (x + this.asym)) - this.offset) * this.norm;
    }
    process(x) {
        let wet;
        const os = this.os;
        if (os === null) {
            wet = this.shape(x);
        }
        else {
            const b = this.buf;
            os.up(x, b);
            for (let i = 0; i < this.factor; i++)
                b[i] = this.shape(b[i]);
            wet = os.down(b);
        }
        const mix = this.mix;
        if (mix >= 1) {
            if (this.latency > 0)
                this.pushDry(x);
            return wet;
        }
        const d = this.latency > 0 ? this.pushDry(x) : x;
        return d + (wet - d) * mix;
    }
    /** Writes x into the dry ring and returns the sample `latency` steps old. */
    pushDry(x) {
        const idx = this.dryIdx;
        this.dry[idx] = x;
        this.dryIdx = (idx + 1) & (DRY_RING - 1);
        return this.dry[(idx - this.latency + DRY_RING) & (DRY_RING - 1)];
    }
    reset() {
        if (this.os !== null)
            this.os.reset();
        this.buf.fill(0);
        this.dry.fill(0);
        this.dryIdx = 0;
    }
}
