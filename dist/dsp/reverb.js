// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe feedback delay network reverb: 8 mutually prime delay lines, a normalized 8x8
// Hadamard feedback (fast Walsh-Hadamard butterflies), per-line one-pole damping, RT60 gains,
// a predelay line and slow per-line delay modulation. Sample-rate agnostic (sr passed in).
// No allocation after construction; every class has reset(). Runs unchanged in node, on the
// main thread and in AudioWorkletGlobalScope.
import { TAU, clamp, finiteOr } from "./math.js";
import { OnePole } from "./filters.js";
import { DelayLine } from "./delay.js";
const LINES = 8;
/** Mutually prime (all prime) base lengths in samples at 48 kHz. */
const BASE_LENGTHS = [1031, 1327, 1523, 1811, 2239, 2677, 3169, 3931];
const SIZE_MIN_SCALE = 0.4;
const SIZE_RANGE_SCALE = 1.2;
const MAX_PREDELAY_SEC = 0.2;
const MOD_DEPTH_SEC = 0.0003;
const MOD_RATE_MIN = 0.1;
const MOD_RATE_MAX = 0.3;
/** Time constant of the parameter glides that keep set() click-free. */
const GLIDE_SEC = 0.08;
const HADAMARD_NORM = 1 / Math.sqrt(LINES);
const OUT_GAIN = 1 / Math.sqrt(LINES);
/** Two orthogonal ±1 tap patterns: decorrelated left and right outputs. */
const TAP_L = [1, -1, 1, 1, -1, 1, -1, -1];
const TAP_R = [1, 1, -1, 1, 1, -1, -1, -1];
export class FdnReverb {
    sr;
    lines = [];
    damps = [];
    predelayLine;
    maxLen;
    glide;
    modDepth;
    // Current (gliding) and target per-line lengths and feedback gains.
    len = new Float64Array(LINES);
    lenTarget = new Float64Array(LINES);
    gain = new Float64Array(LINES);
    gainTarget = new Float64Array(LINES);
    // Modulation rotators (cos, sin) and their per-sample rotation.
    modC = new Float64Array(LINES);
    modS = new Float64Array(LINES);
    rotC = new Float64Array(LINES);
    rotS = new Float64Array(LINES);
    v = new Float64Array(LINES);
    predelay = 1;
    predelayTarget = 1;
    fresh = true;
    _outL = 0;
    _outR = 0;
    constructor(sr) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        const srScale = this.sr / 48000;
        this.modDepth = MOD_DEPTH_SEC * this.sr;
        this.maxLen = BASE_LENGTHS[LINES - 1] * (SIZE_MIN_SCALE + SIZE_RANGE_SCALE) * srScale;
        const cap = Math.ceil(this.maxLen + this.modDepth + 4);
        for (let i = 0; i < LINES; i++) {
            this.lines.push(new DelayLine(cap));
            this.damps.push(new OnePole());
            const rate = MOD_RATE_MIN + ((MOD_RATE_MAX - MOD_RATE_MIN) * ((i * 5) % LINES)) / (LINES - 1);
            const w = (TAU * rate) / this.sr;
            this.rotC[i] = Math.cos(w);
            this.rotS[i] = Math.sin(w);
        }
        this.predelayLine = new DelayLine(Math.ceil(MAX_PREDELAY_SEC * this.sr) + 4);
        this.glide = 1 - Math.exp(-1 / (GLIDE_SEC * this.sr));
        this.set({ size: 0.5, decay: 1.5, damping: 6000, predelay: 0.02, returnDb: 0 });
        this.fresh = true;
        this.resetModulation();
    }
    get outL() {
        return this._outL;
    }
    get outR() {
        return this._outR;
    }
    /** Sets new targets; lengths, gains and predelay glide toward them (safe every block). */
    set(p) {
        const sr = this.sr;
        const size = clamp(finiteOr(p.size, 0.5), 0, 1);
        const decay = clamp(finiteOr(p.decay, 1.5), 0.05, 60);
        const damping = clamp(finiteOr(p.damping, 6000), 20, 0.49 * sr);
        const scale = (SIZE_MIN_SCALE + SIZE_RANGE_SCALE * size) * (sr / 48000);
        for (let i = 0; i < LINES; i++) {
            const len = clamp(BASE_LENGTHS[i] * scale, 1 + this.modDepth, this.maxLen);
            this.lenTarget[i] = len;
            this.gainTarget[i] = Math.pow(10, (-3 * len) / (decay * sr));
            this.damps[i].setLowpass(damping, sr);
        }
        this.predelayTarget = clamp(finiteOr(p.predelay, 0) * sr, 1, MAX_PREDELAY_SEC * sr);
        if (this.fresh) {
            this.snap();
            this.fresh = false;
        }
    }
    process(inL, inR) {
        const k = this.glide;
        this.predelay += (this.predelayTarget - this.predelay) * k;
        this.predelayLine.write(0.5 * (finiteOr(inL, 0) + finiteOr(inR, 0)));
        const x = this.predelayLine.read(this.predelay);
        const v = this.v;
        const depth = this.modDepth;
        let l = 0;
        let r = 0;
        for (let i = 0; i < LINES; i++) {
            this.len[i] += (this.lenTarget[i] - this.len[i]) * k;
            this.gain[i] += (this.gainTarget[i] - this.gain[i]) * k;
            // Rotate the modulation phasor; the first-order correction keeps it on the unit circle.
            const c = this.modC[i];
            const s = this.modS[i];
            const nc = c * this.rotC[i] - s * this.rotS[i];
            const ns = c * this.rotS[i] + s * this.rotC[i];
            const norm = 1.5 - 0.5 * (nc * nc + ns * ns);
            this.modC[i] = nc * norm;
            this.modS[i] = ns * norm;
            const y = this.damps[i].process(this.lines[i].read(this.len[i] + depth * this.modS[i])) * this.gain[i];
            v[i] = y;
            l += TAP_L[i] * y;
            r += TAP_R[i] * y;
        }
        // Normalized 8x8 Hadamard feedback: three butterfly stages of the fast Walsh-Hadamard transform.
        for (let h = 1; h < LINES; h <<= 1) {
            for (let i = 0; i < LINES; i += h << 1) {
                for (let j = i; j < i + h; j++) {
                    const a = v[j];
                    const b = v[j + h];
                    v[j] = a + b;
                    v[j + h] = a - b;
                }
            }
        }
        for (let i = 0; i < LINES; i++)
            this.lines[i].write(v[i] * HADAMARD_NORM + x);
        l *= OUT_GAIN;
        r *= OUT_GAIN;
        if (!Number.isFinite(l) || !Number.isFinite(r)) {
            this.reset();
            return;
        }
        this._outL = l;
        this._outR = r;
    }
    reset() {
        for (let i = 0; i < LINES; i++) {
            this.lines[i].reset();
            this.damps[i].reset();
            this.v[i] = 0;
        }
        this.predelayLine.reset();
        this.snap();
        this.resetModulation();
        this._outL = 0;
        this._outR = 0;
    }
    snap() {
        this.len.set(this.lenTarget);
        this.gain.set(this.gainTarget);
        this.predelay = this.predelayTarget;
    }
    resetModulation() {
        for (let i = 0; i < LINES; i++) {
            // Fixed per-line phases, spread by the golden ratio.
            const ph = TAU * ((i * 0.6180339887) % 1);
            this.modC[i] = Math.cos(ph);
            this.modS[i] = Math.sin(ph);
        }
    }
}
