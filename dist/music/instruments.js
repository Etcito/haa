// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, explicit field instead of a parameter property
// Music instruments (module music, owner M9). DSP-safe: it imports only dsp/* and contract,
// never random-number globals, Date, performance, DOM or Web Audio. No allocation after construction;
// every buffer is preallocated for BLOCK frames (2xBLOCK where a stage oversamples).
//
// TIMING RULE (binding for every instrument here): `when` in noteOn/noteOff is a frame
// offset 0..n-1 inside the NEXT process(l, r, n) call. Each instrument keeps a preallocated
// event queue (>= 16 events); process applies the queued events sample-accurately. process
// ADDS stereo into l and r. A non-finite voice output zeroes that voice and resets it.
// Part 1 exports Instrument, PadInstrument and BassInstrument; part 2 adds Lead and Pluck;
// part 3 adds Kick, Snare, Hat and Perc. The GM drum map below is exported so the arranger
// uses exactly the same numbers.
import { BLOCK } from "../contract.js";
import { TAU, clamp, dbToGain, midiToHz } from "../dsp/math.js";
import { AudioRng } from "../dsp/rng.js";
import { Adsr, ExpDecay } from "../dsp/env.js";
import { Svf, DcBlocker, Biquad, OnePole } from "../dsp/filters.js";
import { DelayLine } from "../dsp/delay.js";
import { Saturator } from "../dsp/shaper.js";
import { ModalBank } from "../dsp/modal.js";
import { TransientShaper } from "../dsp/dynamics.js";
import { WhiteNoise, VelvetNoise } from "../dsp/noise.js";
const QCAP = 32;
/** A small preallocated note-event queue. */
class NoteQueue {
    when = new Int32Array(QCAP);
    midi = new Int32Array(QCAP);
    vel = new Float32Array(QCAP);
    on = new Uint8Array(QCAP);
    count = 0;
    push(w, m, v, o) {
        if (this.count >= QCAP)
            return;
        const i = this.count++;
        this.when[i] = clamp(w, 0, BLOCK - 1) | 0;
        this.midi[i] = m | 0;
        this.vel[i] = clamp(v, 0, 1);
        this.on[i] = o ? 1 : 0;
    }
}
/** Quadrature rotating-phasor additive stack: one complex multiply per partial per sample. */
class PartialStack {
    ratio;
    amp;
    re;
    im;
    rc;
    rs;
    count;
    constructor(count) {
        this.count = count;
        this.ratio = new Float64Array(count);
        this.amp = new Float64Array(count);
        this.re = new Float64Array(count);
        this.im = new Float64Array(count);
        this.rc = new Float64Array(count);
        this.rs = new Float64Array(count);
    }
    setPartial(i, ratio, amp, phase) {
        this.ratio[i] = ratio;
        this.amp[i] = amp;
        this.re[i] = Math.cos(phase);
        this.im[i] = Math.sin(phase);
    }
    /** Set one partial's rotation and renormalize its phasor (once per block per partial). */
    setPartialHz(i, hz, sr) {
        const th = (TAU * hz) / sr;
        this.rc[i] = Math.cos(th);
        this.rs[i] = Math.sin(th);
        const m = this.re[i] * this.re[i] + this.im[i] * this.im[i];
        if (m > 1e-12 && m !== 1) {
            const g = 1 / Math.sqrt(m);
            this.re[i] *= g;
            this.im[i] *= g;
        }
        else if (!(m >= 0)) {
            this.re[i] = 1;
            this.im[i] = 0;
        }
    }
    setHz(f0, sr) { for (let i = 0; i < this.count; i++)
        this.setPartialHz(i, f0 * this.ratio[i], sr); }
    /** One complex multiply per partial; returns sum(amp_i * sin(phase_i)). A non-finite phasor
     *  is caught once per block by setPartialHz (the per-sample check cost more than the pad). */
    next() {
        let y = 0;
        const re = this.re, im = this.im, rc = this.rc, rs = this.rs, amp = this.amp;
        for (let i = 0; i < this.count; i++) {
            const r = re[i], m = im[i], c = rc[i], sn = rs[i];
            const ni = r * sn + m * c;
            re[i] = r * c - m * sn;
            im[i] = ni;
            y += amp[i] * ni;
        }
        return y;
    }
    reset() { this.re.fill(0); this.im.fill(0); }
}
const PAD_NOTES = 4, PAD_VOICES = 7, PAD_PARTIALS = 4, PAD_N = PAD_VOICES * PAD_PARTIALS;
/** One pad note: 7 detuned 4-partial voices through its own Svf, amp and filter envelopes. The
 *  filter envelope also tilts the partial amplitudes: the note blooms bright over its 0.6 s
 *  attack, mellows through the 2.5 s decay (upper partials fall faster than the fundamental)
 *  and darkens further in the release, while the LFO keeps the sustained timbre moving. */
class PadNote {
    stack = new PartialStack(PAD_N);
    ba = new Float64Array(PAD_N);
    dp = new Float32Array(PAD_N);
    di = new Float32Array(PAD_N);
    dd = new Float32Array(PAD_N);
    tmp = new Float32Array(PAD_PARTIALS);
    filter = new Svf();
    amp = new Adsr();
    fenv = new Adsr();
    base = 0;
    li = 0;
    lp = 0;
    note = -1;
    age = 0;
    active = false;
    /** Seed the 7 detuned voices, trigger the envelopes and set the first block's filter. */
    start(midi, vel, rng, sr, n) {
        this.note = midi;
        this.base = midiToHz(midi);
        this.age = 0;
        this.active = true;
        const tmp = this.tmp;
        for (let v = 0; v < PAD_VOICES; v++) {
            const det = Math.pow(2, (-12 + (24 * v) / (PAD_VOICES - 1) + rng.range(-1, 1)) / 1200);
            let sum = 0;
            for (let p = 0; p < PAD_PARTIALS; p++) {
                tmp[p] = Math.pow(p + 1, -1.6) * (0.7 + 0.6 * rng.next());
                sum += tmp[p];
            }
            const g = sum > 0 ? 1 / sum : 0;
            for (let p = 0; p < PAD_PARTIALS; p++) {
                const i = v * PAD_PARTIALS + p;
                this.dp[i] = rng.next();
                this.di[i] = rng.range(0.05, 0.3) / sr;
                this.dd[i] = rng.range(-3, 3);
                this.stack.setPartial(i, det * (p + 1 + rng.range(-0.015, 0.015)), tmp[p] * g, rng.next());
                this.ba[i] = tmp[p] * g;
            }
        }
        this.amp.set(0.9, 1.4, 0.75 * (0.5 + 0.5 * clamp(vel, 0, 1)), 1.8, sr);
        this.amp.gate(true);
        this.fenv.set(0.6, 2.5, 0.25, 1.5, sr);
        this.fenv.gate(true);
        this.li = rng.range(0.05, 0.3) / sr;
        this.lp = rng.next();
        this.update(sr, n);
    }
    release() { this.amp.gate(false); this.fenv.gate(false); }
    /** Re-strikes a sounding note of the same pitch: both envelopes rise again from their current
     *  level (the timbre re-blooms) with no gap and no stolen voice. */
    restrike(vel, sr) {
        this.age = 0;
        this.amp.set(0.9, 1.4, 0.75 * (0.5 + 0.5 * clamp(vel, 0, 1)), 1.8, sr);
        this.amp.gate(true);
        this.fenv.set(0.6, 2.5, 0.25, 1.5, sr);
        this.fenv.gate(true);
    }
    /** Block-rate modulation: the filter envelope, the cutoff LFO and the per-partial drift. */
    update(sr, n) {
        if (!this.active)
            return;
        let fe = 0;
        for (let i = 0; i < n; i++)
            fe = this.fenv.next();
        const lfo = Math.sin(TAU * this.lp);
        this.lp += this.li * n;
        if (this.lp >= 1)
            this.lp -= Math.floor(this.lp);
        this.filter.set(clamp(this.base * (1.5 + 6 * fe) * (1 + 0.35 * lfo), 60, 0.45 * sr), 0.9, sr);
        // Partial p is scaled by bright^p: the fundamental holds, the upper partials follow fe.
        const bright = clamp(0.3 + 0.7 * fe + 0.08 * lfo, 0.05, 1), amp = this.stack.amp;
        for (let i = 0; i < PAD_N; i++) {
            const p = i % PAD_PARTIALS;
            amp[i] = this.ba[i] * (p === 0 ? 1 : p === 1 ? bright : p === 2 ? bright * bright : bright * bright * bright);
        }
        for (let i = 0; i < PAD_N; i++) {
            const cents = this.dd[i] * Math.sin(TAU * this.dp[i]);
            this.dp[i] += this.di[i] * n;
            if (this.dp[i] >= 1)
                this.dp[i] -= Math.floor(this.dp[i]);
            this.stack.setPartialHz(i, this.base * this.stack.ratio[i] * Math.pow(2, cents / 1200), sr);
        }
        if (!this.amp.active)
            this.active = false;
    }
    sample() { this.filter.process(this.stack.next()); const a = this.amp.next(); return Number.isFinite(a) ? this.filter.lp * a : 0; }
    reset() { this.stack.reset(); this.filter.reset(); this.amp.reset(); this.fenv.reset(); this.active = false; this.note = -1; this.age = 0; }
}
/** A 4-note polyphonic pad: 7 detuned voices per note and a stereo chorus on the sum. */
export class PadInstrument {
    sr;
    rng;
    notes = [];
    q = new NoteQueue();
    mono = new Float32Array(BLOCK);
    ca;
    cb;
    ia;
    ib;
    base;
    depth;
    pa = 0;
    pb = 0;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        for (let i = 0; i < PAD_NOTES; i++)
            this.notes.push(new PadNote());
        const md = Math.ceil(0.02 * this.sr) + 8;
        this.ca = new DelayLine(md);
        this.cb = new DelayLine(md);
        this.ia = this.rng.range(0.3, 0.8) / this.sr;
        this.ib = this.rng.range(0.3, 0.8) / this.sr;
        this.base = 0.010 * this.sr;
        this.depth = 0.005 * this.sr;
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(midi, when) { this.q.push(when, midi, 0, false); }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        for (let i = 0; i < this.notes.length; i++) {
            const nt = this.notes[i];
            if (nt.active) {
                nt.age++;
                nt.update(this.sr, f);
            }
        }
        for (let s = 0; s < f; s++) {
            this.applyEvents(s, f);
            let sum = 0;
            for (let i = 0; i < this.notes.length; i++) {
                const nt = this.notes[i];
                if (nt.active) {
                    const y = nt.sample();
                    if (Number.isFinite(y))
                        sum += y;
                }
            }
            this.mono[s] = sum;
        }
        this.q.count = 0;
        this.chorus(l, r, f);
    }
    /** Applies every queued event at exactly frame s (note stealing happens here). */
    applyEvents(s, f) {
        const q = this.q;
        for (let i = 0; i < q.count; i++) {
            if (q.when[i] !== s)
                continue;
            if (q.on[i] === 1)
                this.startNote(q.midi[i], q.vel[i], f);
            else
                this.stopNote(q.midi[i]);
        }
    }
    startNote(midi, vel, f) {
        for (let i = 0; i < this.notes.length; i++) {
            const nt = this.notes[i];
            if (nt.active && nt.note === midi) {
                nt.restrike(vel, this.sr);
                return;
            }
        }
        let slot = 0, oldest = -1;
        for (let i = 0; i < this.notes.length; i++) {
            if (!this.notes[i].active) {
                slot = i;
                break;
            }
            if (this.notes[i].age > oldest) {
                oldest = this.notes[i].age;
                slot = i;
            }
        }
        if (this.notes[slot].active)
            this.notes[slot].reset();
        this.notes[slot].start(midi, vel, this.rng, this.sr, f);
    }
    stopNote(midi) {
        for (let i = 0; i < this.notes.length; i++) {
            const nt = this.notes[i];
            if (nt.active && nt.note === midi)
                nt.release();
        }
    }
    /** Two slow modulated delay lines on the summed pad: dry + wet into l and r. */
    chorus(l, r, n) {
        const da = this.base + this.depth * Math.sin(TAU * this.pa), db = this.base + this.depth * Math.sin(TAU * this.pb);
        this.pa += this.ia * n;
        if (this.pa >= 1)
            this.pa -= Math.floor(this.pa);
        this.pb += this.ib * n;
        if (this.pb >= 1)
            this.pb -= Math.floor(this.pb);
        for (let s = 0; s < n; s++) {
            const x = this.mono[s];
            this.ca.write(x);
            this.cb.write(x);
            l[s] += 0.06 * (0.7 * x + 0.5 * this.ca.read(da));
            r[s] += 0.06 * (0.7 * x + 0.5 * this.cb.read(db));
        }
    }
    reset() {
        this.q.count = 0;
        for (let i = 0; i < this.notes.length; i++)
            this.notes[i].reset();
        this.mono.fill(0);
        this.ca.reset();
        this.cb.reset();
        this.pa = 0;
        this.pb = 0;
    }
}
const BASS_PARTIALS = 9;
/** A monophonic bass: a sub plus 8 harmonics, 2x saturated, lowpassed, glide on overlap. */
export class BassInstrument {
    sr;
    q = new NoteQueue();
    stack = new PartialStack(BASS_PARTIALS);
    sat = new Saturator(2);
    filter = new Svf();
    amp = new Adsr();
    fenv = new Adsr();
    dc = new DcBlocker();
    glide;
    f0 = 0;
    target = 0;
    vel = 0;
    active = false;
    gate = false;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        const rng = new AudioRng(seed >>> 0);
        this.glide = Math.exp(-1 / (rng.range(0.04, 0.08) * this.sr));
        let sum = 0.6;
        for (let h = 1; h <= 8; h++)
            sum += Math.pow(h, -1.35);
        const g = 1 / sum;
        this.stack.setPartial(0, 1, 0.6 * g, 0);
        for (let h = 1; h <= 8; h++)
            this.stack.setPartial(h, h, Math.pow(h, -1.35) * g, 0);
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(midi, when) { this.q.push(when, midi, 0, false); }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        if (this.active)
            this.update(f);
        for (let s = 0; s < f; s++) {
            this.applyEvents(s, f);
            if (!this.active)
                continue;
            const y = this.sample();
            if (!Number.isFinite(y)) {
                this.resetVoice();
                continue;
            }
            l[s] += y;
            r[s] += y;
        }
        this.q.count = 0;
    }
    applyEvents(s, f) {
        const q = this.q;
        for (let i = 0; i < q.count; i++) {
            if (q.when[i] !== s)
                continue;
            if (q.on[i] === 1)
                this.startNote(q.midi[i], q.vel[i], f);
            else {
                this.gate = false;
                this.amp.gate(false);
                this.fenv.gate(false);
            }
        }
    }
    /** Starts fresh or, while a note is already held, retargets and glides. */
    startNote(midi, vel, f) {
        this.target = midiToHz(midi);
        this.vel = clamp(vel, 0, 1);
        if (!this.gate) {
            this.f0 = this.target;
            this.amp.set(0.004, 0.25, 0.8, 0.18, this.sr);
            this.amp.gate(true);
            this.fenv.set(0.003, 0.18, 0.2, 0.15, this.sr);
            this.fenv.gate(true);
            this.active = true;
        }
        this.gate = true;
        this.update(f);
    }
    /** Block-rate glide and velocity-scaled filter envelope. */
    update(n) {
        if (!this.active)
            return;
        this.f0 = this.target + (this.f0 - this.target) * Math.pow(this.glide, n);
        this.stack.setHz(this.f0, this.sr);
        let fe = 0;
        for (let i = 0; i < n; i++)
            fe = this.fenv.next();
        this.filter.set(clamp(this.f0 * (3 + 6 * fe) * (0.5 + 0.5 * this.vel), 80, 0.45 * this.sr), 0.9, this.sr);
        this.sat.set(2.5, 0.08, 0.7);
        if (!this.amp.active)
            this.active = false;
    }
    /** One sample: stack -> 2x saturator -> filter -> amp -> DC blocker, centered near -6 dBFS. */
    sample() {
        const raw = this.stack.next();
        if (!Number.isFinite(raw)) {
            this.stack.reset();
            return 0;
        }
        this.filter.process(this.sat.process(raw));
        const a = this.amp.next();
        const y = this.dc.process(this.filter.lp * a);
        return Number.isFinite(y) ? y * 0.45 : 0;
    }
    resetVoice() {
        this.stack.reset();
        this.sat.reset();
        this.filter.reset();
        this.amp.reset();
        this.fenv.reset();
        this.dc.reset();
        this.active = false;
        this.gate = false;
    }
    reset() { this.q.count = 0; this.f0 = 0; this.target = 0; this.vel = 0; this.resetVoice(); }
}
// --- Part 2: lead and pluck -------------------------------------------------
const LEAD_FORMANTS = 3, LEAD_FORMANT_N = LEAD_FORMANTS * 3;
/** One monophonic lead voice: a 2-operator FM pair built from rotating phasors (no
 *  per-sample trig) plus three seeded additive formant bands. Glide, delayed vibrato and
 *  the Haas offset are applied by the instrument around it. */
class LeadVoice {
    mod = new PartialStack(1);
    form = new PartialStack(LEAD_FORMANT_N);
    fa = new Float64Array(LEAD_FORMANT_N);
    amp = new Adsr();
    ienv = new Adsr();
    fenv = new Adsr();
    filter = new Svf();
    fc = new Float32Array(LEAD_FORMANTS);
    ratio;
    glide = 0.999;
    cr = 1;
    ci = 0;
    wc = 0;
    wm = 0;
    idx = 0;
    f0 = 0;
    target = 0;
    vel = 0;
    age = 0;
    vib = 0;
    vibDepth = 0;
    active = false;
    gate = false;
    note = -1;
    constructor(rng) {
        this.ratio = rng.next() < 0.5 ? 1 : 2;
        const base = [700, 1200, 2600];
        for (let b = 0; b < LEAD_FORMANTS; b++)
            this.fc[b] = base[b] * (1 + rng.range(-0.04, 0.04));
        this.mod.setPartial(0, this.ratio, 1, 0);
    }
    start(midi, vel, rng, sr) {
        const fresh = !this.gate;
        this.note = midi;
        this.target = midiToHz(midi);
        this.vel = clamp(vel, 0, 1);
        this.active = true;
        if (fresh) {
            this.f0 = this.target;
            this.age = 0;
            this.vib = 0;
            this.cr = 1;
            this.ci = 0;
            this.amp.set(0.006, 0.25, 0.85, 0.12, sr);
            this.amp.gate(true);
            this.ienv.set(0.004, 0.3, 0.22, 0.25, sr);
            this.ienv.gate(true);
            this.fenv.set(0.005, 0.45, 0.3, 0.3, sr);
            this.fenv.gate(true);
        }
        this.gate = true;
        const w = [0.45, 1, 0.45], g = [0.24, 0.166, 0.12];
        for (let b = 0; b < LEAD_FORMANTS; b++)
            for (let k = 0; k < 3; k++) {
                this.fa[b * 3 + k] = w[k] * g[b];
                this.form.setPartial(b * 3 + k, (this.fc[b] * (1 + (k - 1) * 0.05)) / this.f0, w[k] * g[b], rng.next());
            }
        this.update(sr, 1);
    }
    release() { this.gate = false; this.amp.gate(false); this.ienv.gate(false); this.fenv.gate(false); }
    /** Block-rate glide, delayed vibrato, FM index envelope, the filter envelope (bright attack
     *  closing to a mellow sustain), the formant bands (the 2.6 kHz band follows the index
     *  envelope, the 1.2 kHz band opens over 0.35 s like a vowel) and phasor renormalization. */
    update(sr, n) {
        if (!this.active)
            return;
        this.f0 = this.target + (this.f0 - this.target) * Math.pow(this.glide, n);
        this.age += n / sr;
        this.vibDepth = clamp((this.age - 0.25) / 0.25, 0, 1);
        this.vib += (5.5 * n) / sr;
        if (this.vib >= 1)
            this.vib -= Math.floor(this.vib);
        const f = this.f0 * Math.pow(2, (15 * this.vibDepth * Math.sin(TAU * this.vib)) / 1200);
        let ie = 0;
        for (let i = 0; i < n; i++)
            ie = this.ienv.next();
        this.idx = ie * (1.2 + 2.8 * this.vel);
        let fe = 0;
        for (let i = 0; i < n; i++)
            fe = this.fenv.next();
        this.filter.set(clamp(f * (2.5 + 10 * fe * (0.4 + 0.6 * this.vel)) + 400, 200, 0.45 * sr), 0.8, sr);
        const open = clamp(this.age / 0.35, 0, 1), fa = this.fa, amp = this.form.amp;
        for (let k = 0; k < 3; k++) {
            amp[k] = fa[k];
            amp[3 + k] = fa[3 + k] * (0.4 + 0.6 * open);
            amp[6 + k] = fa[6 + k] * (0.3 + 0.9 * ie);
        }
        this.mod.setHz(f, sr);
        this.wc = (TAU * f) / sr;
        this.wm = (TAU * f * this.ratio) / sr;
        const m = this.cr * this.cr + this.ci * this.ci;
        if (m > 1e-12) {
            const g = 1 / Math.sqrt(m);
            this.cr *= g;
            this.ci *= g;
        }
        else {
            this.cr = 1;
            this.ci = 0;
        }
        this.form.setHz(f, sr);
        if (!this.amp.active)
            this.active = false;
    }
    /** Carrier rotated by fc + index * fm * modulator, small-angle (no per-sample trig). */
    sample() {
        const phi = this.wc + this.idx * this.wm * this.mod.next();
        const c2 = 1 - 0.5 * phi * phi;
        const nr = this.cr * c2 - this.ci * phi, ni = this.cr * phi + this.ci * c2;
        if (Number.isFinite(nr) && Number.isFinite(ni)) {
            this.cr = nr;
            this.ci = ni;
        }
        else {
            this.cr = 1;
            this.ci = 0;
        }
        const g = 1.5 - 0.5 * (this.cr * this.cr + this.ci * this.ci);
        this.cr *= g;
        this.ci *= g;
        this.filter.process(0.5 * this.ci + 0.3 * this.form.next());
        const y = this.filter.lp * this.amp.next();
        return Number.isFinite(y) ? y : 0;
    }
    reset() {
        this.mod.reset();
        this.form.reset();
        this.amp.reset();
        this.ienv.reset();
        this.fenv.reset();
        this.filter.reset();
        this.active = false;
        this.gate = false;
        this.note = -1;
        this.age = 0;
        this.vib = 0;
        this.cr = 1;
        this.ci = 0;
    }
}
/** A monophonic FM lead with additive formants, glide, delayed vibrato and a Haas spread. */
export class LeadInstrument {
    sr;
    q = new NoteQueue();
    voice;
    rng;
    mono = new Float32Array(BLOCK);
    haas;
    hd;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        this.voice = new LeadVoice(this.rng);
        this.voice.glide = Math.exp(-1 / (this.rng.range(0.03, 0.06) * this.sr));
        this.haas = new DelayLine(Math.ceil(0.015 * this.sr) + 8);
        this.hd = this.rng.range(0.005, 0.011) * this.sr;
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(midi, when) { this.q.push(when, midi, 0, false); }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        if (this.voice.active)
            this.voice.update(this.sr, f);
        for (let s = 0; s < f; s++) {
            const q = this.q;
            for (let i = 0; i < q.count; i++) {
                if (q.when[i] !== s)
                    continue;
                if (q.on[i] === 1)
                    this.voice.start(q.midi[i], q.vel[i], this.rng, this.sr);
                else
                    this.voice.release();
            }
            const y = this.voice.active ? this.voice.sample() : 0;
            if (Number.isFinite(y))
                this.mono[s] = y;
            else {
                this.voice.reset();
                this.mono[s] = 0;
            }
        }
        this.q.count = 0;
        for (let s = 0; s < f; s++) {
            const x = this.mono[s];
            this.haas.write(x);
            const wet = this.haas.read(this.hd);
            l[s] += 0.5 * x;
            r[s] += 0.5 * (0.7 * x + 0.5 * wet);
        }
    }
    reset() { this.q.count = 0; this.voice.reset(); this.mono.fill(0); this.haas.reset(); }
}
const PLUCK_NOTES = 6;
/** One Karplus-Strong voice: a seeded, velocity-brightened noise burst in a delay loop
 *  with a one-pole loss filter and a first-order dispersion allpass (seeded stiffness). */
class PluckVoice {
    line;
    loss = new OnePole();
    bright = new OnePole();
    amp = new Adsr();
    delay = 1;
    refl = 0.996;
    ap = 0.2;
    apX = 0;
    apY = 0;
    gL = 0.7;
    gR = 0.7;
    active = false;
    note = -1;
    stamp = 0;
    constructor(maxSamples) { this.line = new DelayLine(maxSamples); }
    start(midi, vel, rng, sr, stamp) {
        const f0 = midiToHz(midi);
        this.note = midi;
        this.stamp = stamp;
        this.active = true;
        this.delay = clamp(sr / f0, 2, this.line.maxSamples - 1);
        this.refl = 0.994 + 0.005 * rng.next();
        this.loss.setLowpass(clamp(2200 + 4200 * vel + rng.range(-400, 400), 400, 0.45 * sr), sr);
        this.ap = rng.range(0.06, 0.28);
        this.apX = 0;
        this.apY = 0;
        this.bright.setLowpass(clamp(700 + 7500 * vel + rng.range(-300, 300), 300, 0.45 * sr), sr);
        const d = Math.max(2, Math.round(this.delay));
        for (let i = 0; i < d; i++)
            this.line.write(this.bright.process(rng.next() * 2 - 1) * 0.7);
        this.amp.set(0.001, 0.04, 1, 0.07, sr);
        this.amp.gate(true);
        const a = (clamp(rng.range(-0.3, 0.3), -0.3, 0.3) + 1) * Math.PI / 4;
        this.gL = Math.cos(a);
        this.gR = Math.sin(a);
    }
    release() { this.amp.gate(false); }
    sample() {
        let x = this.line.read(this.delay);
        x = this.loss.process(x);
        const y = this.ap * x + this.apX - this.ap * this.apY;
        this.apX = x;
        this.apY = y;
        this.line.write(y * this.refl);
        return y * this.amp.next();
    }
    reset() {
        this.line.reset();
        this.loss.reset();
        this.bright.reset();
        this.amp.reset();
        this.apX = 0;
        this.apY = 0;
        this.active = false;
        this.note = -1;
    }
}
/** A 6-note plucked string with body resonance on the sum and a per-voice pan spread. */
export class PluckInstrument {
    sr;
    q = new NoteQueue();
    rng;
    voices = [];
    b1L = new Biquad();
    b2L = new Biquad();
    b1R = new Biquad();
    b2R = new Biquad();
    seq = 0;
    gain = 0.5;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        const maxS = Math.ceil(this.sr / 20) + 8;
        for (let i = 0; i < PLUCK_NOTES; i++)
            this.voices.push(new PluckVoice(maxS));
        const f1 = 200 * (1 + this.rng.range(-0.06, 0.06)), f2 = 900 * (1 + this.rng.range(-0.06, 0.06));
        this.b1L.set('peaking', f1, 1.4, 4.5, this.sr);
        this.b2L.set('peaking', f2, 1.7, 3.5, this.sr);
        this.b1R.set('peaking', f1, 1.4, 4.5, this.sr);
        this.b2R.set('peaking', f2, 1.7, 3.5, this.sr);
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(midi, when) { this.q.push(when, midi, 0, false); }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        for (let s = 0; s < f; s++) {
            const q = this.q;
            for (let i = 0; i < q.count; i++) {
                if (q.when[i] !== s)
                    continue;
                if (q.on[i] === 1)
                    this.startNote(q.midi[i], q.vel[i]);
                else
                    this.stopNote(q.midi[i]);
            }
            let ml = 0, mr = 0;
            for (let i = 0; i < this.voices.length; i++) {
                const v = this.voices[i];
                if (!v.active)
                    continue;
                const y = v.sample();
                if (!Number.isFinite(y)) {
                    v.reset();
                    continue;
                }
                ml += y * v.gL;
                mr += y * v.gR;
            }
            l[s] += this.b2L.process(this.b1L.process(ml)) * this.gain;
            r[s] += this.b2R.process(this.b1R.process(mr)) * this.gain;
        }
        this.q.count = 0;
    }
    startNote(midi, vel) {
        let slot = 0, oldest = Infinity;
        for (let i = 0; i < this.voices.length; i++) {
            if (!this.voices[i].active) {
                slot = i;
                break;
            }
            if (this.voices[i].stamp < oldest) {
                oldest = this.voices[i].stamp;
                slot = i;
            }
        }
        if (this.voices[slot].active)
            this.voices[slot].reset();
        this.voices[slot].start(midi, vel, this.rng, this.sr, ++this.seq);
    }
    stopNote(midi) {
        for (let i = 0; i < this.voices.length; i++) {
            const v = this.voices[i];
            if (v.active && v.note === midi)
                v.release();
        }
    }
    reset() {
        this.q.count = 0;
        this.seq = 0;
        for (let i = 0; i < this.voices.length; i++)
            this.voices[i].reset();
        this.b1L.reset();
        this.b2L.reset();
        this.b1R.reset();
        this.b2R.reset();
    }
}
// --- Part 3: kick, snare, hat and percussion --------------------------------
/** General MIDI drum map, shared verbatim with the arranger. */
export const DRUM_NOTES = {
    kick: 36, snare: 38, rim: 37,
    hatClosed: 42, hatOpen: 46, crash: 49,
    tomLow: 45, tomMid: 47, tomHigh: 50,
    shaker: 70, shakerAlt: 82,
};
/** A one-shot kick: a saturated pitch-swept body (150 -> ~48 Hz, exponential curve) plus a
 *  2-4 ms band-passed click. Velocity scales level and sweep depth. noteOff is a no-op. */
export class KickInstrument {
    sr;
    q = new NoteQueue();
    rng;
    sat;
    click;
    white;
    amp = new ExpDecay();
    clickEnv = new ExpDecay();
    phase = 0;
    fEnd = 48;
    fStart = 48;
    sweep = 0;
    sweepCoef = 0;
    t60 = 0.35;
    clickT60 = 0.003;
    clickAmp = 0.35;
    level = 1;
    active = false;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        this.white = new WhiteNoise(this.rng);
        this.sat = new Saturator(2);
        this.click = new Svf();
        this.fEnd = 44 + this.rng.range(0, 8);
        this.t60 = this.rng.range(0.25, 0.45);
        this.clickT60 = this.rng.range(0.002, 0.004);
        this.click.set(this.rng.range(1800, 3200), 0.9, this.sr);
        this.sat.set(2, 0.1, 0.8);
    }
    noteOn(_midi, vel, when) { this.q.push(when, 0, vel, true); }
    noteOff(_midi, _when) { }
    trigger(vel) {
        const v = clamp(vel, 0, 1);
        this.level = 0.35 + 0.65 * v;
        this.fStart = 150 + 40 * v;
        this.sweep = 1;
        this.sweepCoef = Math.pow(1e-4, 1 / (0.03 * this.sr));
        this.phase = 0;
        this.amp.trigger(1, this.t60, this.sr);
        this.clickEnv.trigger(1, this.clickT60, this.sr);
        this.active = true;
    }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        for (let s = 0; s < f; s++) {
            const q = this.q;
            for (let i = 0; i < q.count; i++)
                if (q.when[i] === s && q.on[i] === 1)
                    this.trigger(q.vel[i]);
            if (!this.active)
                continue;
            const env = this.amp.next();
            const hz = this.fEnd + (this.fStart - this.fEnd) * this.sweep;
            this.sweep *= this.sweepCoef;
            this.phase += (TAU * hz) / this.sr;
            if (this.phase >= TAU)
                this.phase -= TAU;
            const click = this.click.process(this.white.next());
            const cl = this.click.bp * this.clickEnv.next();
            const y = this.sat.process(Math.sin(this.phase) * env * 0.9 + cl * this.clickAmp) * this.level;
            if (!Number.isFinite(y)) {
                this.resetVoice();
                continue;
            }
            l[s] += y;
            r[s] += y;
            if (env < 1e-4)
                this.active = false;
        }
        this.q.count = 0;
    }
    resetVoice() { this.sat.reset(); this.click.reset(); this.amp.reset(); this.clickEnv.reset(); this.active = false; }
    reset() { this.q.count = 0; this.resetVoice(); }
}
const SNARE_MODES = 2;
/** A one-shot snare: a 2-mode modal shell (about 180/330 Hz, seeded ±5%) excited by an
 *  impulse plus band-passed noise, with a transient shaper on the attack. Note 37 is a
 *  quieter rim/ghost. noteOff is a no-op. */
export class SnareInstrument {
    sr;
    q = new NoteQueue();
    rng;
    shell;
    noiseF;
    white;
    ts;
    noiseEnv = new ExpDecay();
    modes = [];
    excite = 0;
    level = 1;
    noiseAmp = 0.55;
    noiseT60 = 0.12;
    tail = 0;
    active = false;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        this.shell = new ModalBank(SNARE_MODES, this.sr);
        this.noiseF = new Svf();
        this.white = new WhiteNoise(this.rng);
        this.ts = new TransientShaper(this.sr);
        for (let i = 0; i < SNARE_MODES; i++)
            this.modes.push({ hz: 100, t60: 1, gain: 1 });
        this.ts.set(6, -1.5);
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(_midi, _when) { }
    trigger(midi, vel) {
        const v = clamp(vel, 0, 1);
        const j = 1 + this.rng.range(-0.05, 0.05);
        this.shell.reset();
        if (midi === DRUM_NOTES.rim) {
            this.modes[0].hz = 400 * j;
            this.modes[0].t60 = 0.06;
            this.modes[0].gain = 1;
            this.modes[1].hz = 690 * j;
            this.modes[1].t60 = 0.05;
            this.modes[1].gain = 0.7;
            this.noiseF.set(this.rng.range(3000, 4200), 1.1, this.sr);
            this.level = 0.3 * (0.4 + 0.6 * v);
            this.noiseAmp = 0.5;
            this.noiseT60 = 0.05;
        }
        else {
            this.modes[0].hz = 180 * j;
            this.modes[0].t60 = 0.16;
            this.modes[0].gain = 1;
            this.modes[1].hz = 330 * j;
            this.modes[1].t60 = 0.12;
            this.modes[1].gain = 0.8;
            this.noiseF.set(this.rng.range(2000, 5000), 0.9, this.sr);
            this.level = 0.4 + 0.6 * v;
            this.noiseAmp = 0.55;
            this.noiseT60 = 0.12;
        }
        this.shell.setModes(this.modes);
        this.noiseEnv.trigger(1, this.noiseT60, this.sr);
        this.ts.reset();
        this.excite = 1;
        this.active = true;
    }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        for (let s = 0; s < f; s++) {
            const q = this.q;
            for (let i = 0; i < q.count; i++)
                if (q.when[i] === s && q.on[i] === 1)
                    this.trigger(q.midi[i], q.vel[i]);
            if (!this.active)
                continue;
            const sh = this.shell.process(this.excite);
            this.excite = 0;
            this.noiseF.process(this.white.next());
            const ne = this.noiseEnv.next();
            const y = this.ts.process(sh + this.noiseF.bp * ne * this.noiseAmp) * this.level;
            if (!Number.isFinite(y)) {
                this.resetVoice();
                continue;
            }
            l[s] += y;
            r[s] += y;
            this.tail = ne;
        }
        // A decayed hit stops costing DSP until the next trigger.
        if (this.active && this.tail < 1e-4 && this.shell.energy() < 1e-8)
            this.active = false;
        this.q.count = 0;
    }
    resetVoice() { this.shell.reset(); this.noiseF.reset(); this.ts.reset(); this.noiseEnv.reset(); this.active = false; this.excite = 0; }
    reset() { this.q.count = 0; this.resetVoice(); }
}
const HAT_PARTIALS = 6;
/** One-shot hats: 6 inharmonic metallic modal partials (3-12 kHz) plus high-passed white
 *  noise at 7 kHz. Closed (42) is choked by its own retrigger, open (46) rings 0.35 s and
 *  crash (49) 1.5 s with more energy. Every hit draws a seeded decay (±10%) and level
 *  (±1 dB) variation, so repeated hats never repeat identically. noteOff is a no-op. */
export class HatInstrument {
    sr;
    q = new NoteQueue();
    rng;
    bank;
    hp;
    white;
    noiseEnv = new ExpDecay();
    modes = [];
    ratios = [1, 1.447, 1.617, 1.926, 2.502, 2.663];
    excite = 0;
    level = 1;
    noiseAmp = 0.35;
    tail = 0;
    active = false;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        this.bank = new ModalBank(HAT_PARTIALS, this.sr);
        this.hp = new Biquad();
        this.white = new WhiteNoise(this.rng);
        const base = this.rng.range(3200, 4200);
        for (let i = 0; i < HAT_PARTIALS; i++)
            this.modes.push({ hz: base * this.ratios[i], t60: 1, gain: 1 - i * 0.1 });
        this.hp.set('highpass', 7000, 0.7, 0, this.sr);
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(_midi, _when) { }
    trigger(midi, vel) {
        const v = clamp(vel, 0, 1);
        const crash = midi === DRUM_NOTES.crash;
        const open = midi === DRUM_NOTES.hatOpen;
        let t60 = crash ? 1.5 : open ? 0.35 : this.rng.range(0.04, 0.08);
        t60 *= this.rng.range(0.9, 1.1);
        this.level = (crash ? 0.55 : open ? 0.45 : 0.4) * (0.4 + 0.6 * v) * dbToGain(this.rng.range(-1, 1));
        this.noiseAmp = crash ? 0.5 : open ? 0.4 : 0.35;
        this.bank.reset();
        this.bank.setModes(this.modes, this.rng.range(0.995, 1.005), t60);
        this.noiseEnv.trigger(1, t60 * 0.7, this.sr);
        this.excite = 1;
        this.active = true;
    }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        for (let s = 0; s < f; s++) {
            const q = this.q;
            for (let i = 0; i < q.count; i++)
                if (q.when[i] === s && q.on[i] === 1)
                    this.trigger(q.midi[i], q.vel[i]);
            if (!this.active)
                continue;
            const m = this.bank.process(this.excite);
            this.excite = 0;
            const nz = this.hp.process(this.white.next());
            const ne = this.noiseEnv.next();
            const y = (m * 0.6 + nz * this.noiseAmp * ne) * this.level;
            if (!Number.isFinite(y)) {
                this.resetVoice();
                continue;
            }
            l[s] += y;
            r[s] += y;
            this.tail = ne;
        }
        if (this.active && this.tail < 1e-4 && this.bank.energy() < 1e-8)
            this.active = false;
        this.q.count = 0;
    }
    resetVoice() { this.bank.reset(); this.hp.reset(); this.noiseEnv.reset(); this.active = false; this.excite = 0; }
    reset() { this.q.count = 0; this.resetVoice(); }
}
const PERC_VOICES = 4;
/** One perc voice: a 3-mode modal tom or a velvet-noise shaker with a 2-grain envelope. */
class PercVoice {
    sr;
    bank;
    f;
    velvet;
    env1 = new ExpDecay();
    env2 = new ExpDecay();
    modes = [];
    excite = 0;
    g2 = -1;
    kind = 0;
    level = 1;
    gL = 0.7;
    gR = 0.7;
    tail = 0;
    active = false;
    stamp = 0;
    note = -1;
    constructor(sr, rng) {
        this.sr = sr;
        this.bank = new ModalBank(3, sr);
        this.f = new Svf();
        this.velvet = new VelvetNoise(rng);
        for (let i = 0; i < 3; i++)
            this.modes.push({ hz: 100, t60: 1, gain: 1 });
    }
    trigger(midi, vel, rng, stamp) {
        this.note = midi;
        this.stamp = stamp;
        this.active = true;
        const v = clamp(vel, 0, 1);
        let pan = 0;
        if (midi === DRUM_NOTES.shaker || midi === DRUM_NOTES.shakerAlt) {
            this.kind = 1;
            this.f.set((midi === DRUM_NOTES.shaker ? 6500 : 8500) * (1 + rng.range(-0.05, 0.05)), 1.2, this.sr);
            this.env1.trigger(1, 0.04, this.sr);
            this.env2.trigger(0, 0.05, this.sr);
            this.g2 = Math.round(0.055 * this.sr);
            this.level = 0.35 * (0.4 + 0.6 * v);
            pan = (midi === DRUM_NOTES.shaker ? -0.25 : 0.25) + rng.range(-0.08, 0.08);
        }
        else {
            this.kind = 0;
            let base = midi === DRUM_NOTES.tomLow ? 100 : midi === DRUM_NOTES.tomHigh ? 190 : 140;
            base *= 1 + rng.range(-0.04, 0.04);
            this.modes[0].hz = base;
            this.modes[0].t60 = 0.35;
            this.modes[0].gain = 1;
            this.modes[1].hz = base * 1.55;
            this.modes[1].t60 = 0.25;
            this.modes[1].gain = 0.6;
            this.modes[2].hz = base * 2.3;
            this.modes[2].t60 = 0.18;
            this.modes[2].gain = 0.35;
            this.bank.reset();
            this.bank.setModes(this.modes);
            this.excite = 1;
            this.level = 0.4 + 0.6 * v;
            pan = (midi === DRUM_NOTES.tomLow ? -0.5 : midi === DRUM_NOTES.tomHigh ? 0.5 : 0) + rng.range(-0.1, 0.1);
        }
        const a = (clamp(pan, -1, 1) + 1) * Math.PI / 4;
        this.gL = Math.cos(a);
        this.gR = Math.sin(a);
    }
    sample() {
        if (this.kind === 0) {
            const y = this.bank.process(this.excite) * this.level;
            this.excite = 0;
            return y;
        }
        if (this.g2 > 0) {
            this.g2--;
            if (this.g2 === 0)
                this.env2.trigger(1, 0.05, this.sr);
        }
        this.f.process(this.velvet.next(4000, this.sr));
        this.tail = this.env1.next() + this.env2.next();
        return this.f.bp * this.tail * this.level;
    }
    /** Block-rate: a decayed tom or shaker frees its slot and its DSP. */
    settle() {
        if (this.kind === 0 ? this.bank.energy() < 1e-8 : this.g2 <= 0 && this.tail < 1e-4)
            this.active = false;
    }
    reset() {
        this.bank.reset();
        this.f.reset();
        this.env1.reset();
        this.env2.reset();
        this.excite = 0;
        this.g2 = -1;
        this.active = false;
        this.note = -1;
    }
}
/** Modal toms (45/47/50) and velvet shakers (70/82), panned per note. */
export class PercInstrument {
    sr;
    q = new NoteQueue();
    rng;
    voices = [];
    seq = 0;
    constructor(sr, seed) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.rng = new AudioRng(seed >>> 0);
        for (let i = 0; i < PERC_VOICES; i++)
            this.voices.push(new PercVoice(this.sr, this.rng.fork(i + 1)));
    }
    noteOn(midi, vel, when) { this.q.push(when, midi, vel, true); }
    noteOff(_midi, _when) { }
    process(l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        for (let s = 0; s < f; s++) {
            const q = this.q;
            for (let i = 0; i < q.count; i++)
                if (q.when[i] === s && q.on[i] === 1)
                    this.startNote(q.midi[i], q.vel[i]);
            let ml = 0, mr = 0;
            for (let i = 0; i < this.voices.length; i++) {
                const v = this.voices[i];
                if (!v.active)
                    continue;
                const y = v.sample();
                if (!Number.isFinite(y)) {
                    v.reset();
                    continue;
                }
                ml += y * v.gL;
                mr += y * v.gR;
            }
            l[s] += ml;
            r[s] += mr;
        }
        for (let i = 0; i < this.voices.length; i++)
            if (this.voices[i].active)
                this.voices[i].settle();
        this.q.count = 0;
    }
    startNote(midi, vel) {
        let slot = 0, oldest = Infinity;
        for (let i = 0; i < this.voices.length; i++) {
            if (!this.voices[i].active) {
                slot = i;
                break;
            }
            if (this.voices[i].stamp < oldest) {
                oldest = this.voices[i].stamp;
                slot = i;
            }
        }
        this.voices[slot].trigger(midi, vel, this.rng, ++this.seq);
    }
    reset() { this.q.count = 0; this.seq = 0; for (let i = 0; i < this.voices.length; i++)
        this.voices[i].reset(); }
}
