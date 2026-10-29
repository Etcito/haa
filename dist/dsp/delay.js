// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe delay primitives: a circular delay line with 4-point Hermite fractional reads
// and a damped tube waveguide. Sample-rate agnostic (sr passed in). No allocation after
// construction; every class has reset(). Runs unchanged in node, on the main thread and in
// AudioWorkletGlobalScope.
import { TAU, clamp, finiteOr } from "./math.js";
import { OnePole } from "./filters.js";
/**
 * Circular buffer. read(d) returns the signal d samples before the most recent write
 * (d in [1, maxSamples], fractional, 4-point 3rd-order Hermite interpolation).
 */
export class DelayLine {
    maxSamples;
    buf;
    mask;
    pos = 0;
    constructor(maxSamples) {
        this.maxSamples = Math.max(1, Math.ceil(finiteOr(maxSamples, 1)));
        let size = 4;
        while (size < this.maxSamples + 4)
            size *= 2;
        this.buf = new Float32Array(size);
        this.mask = size - 1;
    }
    write(x) {
        this.buf[this.pos] = x;
        this.pos = (this.pos + 1) & this.mask;
    }
    read(delay) {
        const d = clamp(delay, 1, this.maxSamples);
        const i = Math.floor(d);
        const t = d - i;
        const buf = this.buf;
        const mask = this.mask;
        // Sample k steps back from the last write sits at pos - 1 - k.
        const base = this.pos - 1 - i;
        const xm1 = buf[(base + 1) & mask];
        const x0 = buf[base & mask];
        const x1 = buf[(base - 1) & mask];
        const x2 = buf[(base - 2) & mask];
        const c = (x1 - xm1) * 0.5;
        const v = x0 - x1;
        const w = c + v;
        const a = w + v + (x2 - x0) * 0.5;
        const b = w + a;
        return ((a * t - b) * t + c) * t + x0;
    }
    reset() {
        this.buf.fill(0);
        this.pos = 0;
    }
}
/**
 * Tube resonator: a delay of sr / hz with a one-pole damping lowpass and a reflection
 * coefficient in the loop. y[n] = x[n] + reflection * lowpass(y[n - sr / hz]).
 * The lowpass phase delay at hz is subtracted so the loop stays tuned.
 */
export class Waveguide {
    line;
    damp = new OnePole();
    sr;
    delay = 1;
    reflection = 0;
    constructor(maxSamples, sr) {
        this.line = new DelayLine(maxSamples);
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.damp.setLowpass(0.49 * this.sr, this.sr);
    }
    set(hz, reflection, dampingHz) {
        const sr = this.sr;
        const f = clamp(finiteOr(hz, 440), 1, 0.45 * sr);
        const dh = clamp(finiteOr(dampingHz, 0.49 * sr), 10, 0.49 * sr);
        this.damp.setLowpass(dh, sr);
        // One-pole lowpass H = (1 - a) / (1 - a z^-1): phase delay at w.
        const a = Math.exp((-TAU * dh) / sr);
        const w = (TAU * f) / sr;
        const tau = Math.atan2(a * Math.sin(w), 1 - a * Math.cos(w)) / w;
        // read(d) looks back from the previous write, so the loop period is d + 1 + tau.
        this.delay = clamp(sr / f - 1 - tau, 1, this.line.maxSamples);
        this.reflection = clamp(finiteOr(reflection, 0), -0.999, 0.999);
    }
    process(x) {
        const y = x + this.reflection * this.damp.process(this.line.read(this.delay));
        this.line.write(y);
        return y;
    }
    reset() {
        this.line.reset();
        this.damp.reset();
    }
}
