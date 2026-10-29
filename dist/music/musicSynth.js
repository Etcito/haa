// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: registered styles, generic params, fadeBars crossfade
// The adaptive music bed voice (module music, owner M3). DSP-safe: it imports only ../contract.ts,
// ../dsp/*.ts, ../registry/registry.ts, ./arranger.ts and ./instruments.ts; never the platform RNG, clocks,
// performance, DOM or Web Audio. It owns one Arranger and one instance of every instrument, schedules
// the arranger's 16th-note NoteEvents sample-accurately from the bpm (with the style swing), applies
// the palette levels and a light FdnReverb send, glues the bus with a gentle Compressor and ADDS the
// result into l/r. Nothing allocates in process(): the MusicState, the event array, the
// per-instrument BLOCK scratch, the note-off ring and the mix buffers are all preallocated.
//
// Params (MUSIC_PARAMS order): style, styleB, styleMix, intensity, phase, climax, resolved, won,
// sparkle, tension, fadeBars, level. A style change crossfades the palette linearly over fadeBars
// bars (clamped 1..16); sparkle raises the pluck level by +3 dB. The discrete params (style, styleB,
// styleMix, phase, climax, resolved, won, fadeBars) are read unsmoothed; the continuous ones go
// through the internal smoothers. When the level target is 0 and the smoothed level has stayed
// below 1e-4 for 0.5 s all DSP is skipped (the instruments reset once) until the level rises; a
// non-finite block zeroes the output and resets every instrument and the reverb.
import { BLOCK } from "../contract.js";
import { Arranger } from "./arranger.js";
import { PadInstrument, BassInstrument, LeadInstrument, PluckInstrument, KickInstrument, SnareInstrument, HatInstrument, PercInstrument, } from "./instruments.js";
import { FdnReverb } from "../dsp/reverb.js";
import { Compressor } from "../dsp/dynamics.js";
import { DcBlocker, Smoother } from "../dsp/filters.js";
import { clamp, dbToGain, finiteOr } from "../dsp/math.js";
import { hashSeed } from "../dsp/rng.js";
import { listStyles } from "../registry/registry.js";
/** The music bed params, in the order of the param block. */
export const MUSIC_PARAMS = [
    'style', 'styleB', 'styleMix', 'intensity', 'phase', 'climax', 'resolved', 'won',
    'sparkle', 'tension', 'fadeBars', 'level',
];
const PARAM_COUNT = MUSIC_PARAMS.length;
/** 1 for the continuous params that go through the smoothers, 0 for the discrete raw ones. */
const SMOOTHED = new Uint8Array([0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 1]);
const INST_COUNT = 8;
/** Steps per bar: the palette crossfade lasts 16 × fadeBars 16th-note steps. */
const BAR_STEPS = 16;
const FADE_BARS_MAX = 16;
/** Pending note-off ring; only the four sustained instruments use it. */
const OFF_RING = 256;
/** Nominal bus scale at level 1: the mix preset owns the rest of the balance. */
const MASTER = 0.65;
const REVERB_RETURN = 0.8;
const SPARKLE_DB = 3;
/** Arrangement dynamics: the bus gain of each section (dB), glided over DYN_SEC. The arranger
 *  thickens the texture with intensity; this lifts the climax clearly above the main sections and
 *  lets the resolve and post settle. */
// HAA tuning: wider steps (intro -3 → mainA -2 → mainB -1 → intense +0.5 → climax +3.5) so the arc
// audibly builds; resolveWin +1 lets the held win chord land as a payoff before post settles at -3.5.
const SECTION_DB = {
    intro: -3, mainA: -2, mainB: -1, intense: 0.5, climax: 3.5, resolveWin: 1, resolve: -2, post: -3.5,
};
const DYN_SEC = 0.5;
const IDLE_SEC = 0.5;
const IDLE_EPS = 1e-4;
const REVERB = { size: 0.55, decay: 1.8, damping: 6000, predelay: 0.02, returnDb: 0 };
const COMP = { thresholdDb: -14, ratio: 2, kneeDb: 6, attackMs: 10, releaseMs: 150, makeupDb: 0 };
/** Per-instrument reverb send: pad, bass, lead, pluck, kick, snare, hat, perc. */
const SEND = [0.35, 0, 0.25, 0.3, 0, 0.12, 0, 0.15];
/** The first four instruments sustain and therefore need a scheduled note-off. */
const MELODIC = 4;
const INST_INDEX = {
    pad: 0, bass: 1, lead: 2, pluck: 3, kick: 4, snare: 5, hat: 6, perc: 7,
};
export class MusicSynth {
    sr;
    styles;
    arranger;
    insts;
    scratchL = [];
    scratchR = [];
    reverb;
    comp;
    smoothers = [];
    state = {
        style: 0, styleB: 0, styleMix: 0, intensity: 0, phase: 0,
        climax: 0, resolved: 0, won: 0, sparkle: 0, tension: 0,
    };
    raw = new Float32Array(PARAM_COUNT);
    events = [];
    outL = new Float32Array(BLOCK);
    outR = new Float32Array(BLOCK);
    sendBuf = new Float32Array(BLOCK);
    dcL = new DcBlocker();
    dcR = new DcBlocker();
    instGain = new Float32Array(INST_COUNT);
    offMidi = new Int16Array(OFF_RING);
    offInst = new Uint8Array(OFF_RING);
    offAt = new Int32Array(OFF_RING);
    palFrom = new Float32Array(6);
    palTo = new Float32Array(6);
    palNow = new Float32Array(6);
    offCount = 0;
    pending = 0;
    stepIndex = 0;
    palSteps = 0;
    /** Length in steps of the running palette crossfade. */
    palLen = 1;
    curStyle = 0;
    idle = false;
    idleTime = 0;
    dyn = 1;
    dynCoef;
    constructor(sr, seed, styles = listStyles()) {
        this.sr = sr > 1 && Number.isFinite(sr) ? sr : 48000;
        this.styles = Object.freeze(styles.slice());
        this.arranger = new Arranger(hashSeed(seed, 'arr'), this.styles);
        this.insts = [
            new PadInstrument(this.sr, hashSeed(seed, 'pad')),
            new BassInstrument(this.sr, hashSeed(seed, 'bass')),
            new LeadInstrument(this.sr, hashSeed(seed, 'lead')),
            new PluckInstrument(this.sr, hashSeed(seed, 'pluck')),
            new KickInstrument(this.sr, hashSeed(seed, 'kick')),
            new SnareInstrument(this.sr, hashSeed(seed, 'snare')),
            new HatInstrument(this.sr, hashSeed(seed, 'hat')),
            new PercInstrument(this.sr, hashSeed(seed, 'perc')),
        ];
        for (let i = 0; i < INST_COUNT; i++) {
            this.scratchL.push(new Float32Array(BLOCK));
            this.scratchR.push(new Float32Array(BLOCK));
        }
        this.dynCoef = 1 - Math.exp(-BLOCK / (DYN_SEC * this.sr));
        this.reverb = new FdnReverb(this.sr);
        this.reverb.set(REVERB);
        this.comp = new Compressor(this.sr);
        this.comp.set(COMP);
        for (let i = 0; i < PARAM_COUNT; i++)
            this.smoothers.push(new Smoother(0.015 / BLOCK, this.sr));
        for (let i = 0; i < 32; i++)
            this.events.push({ inst: 'pad', midi: 0, vel: 0, step: 0, len: 0 });
        this.readPalette(0, this.palNow);
        this.palFrom.set(this.palNow);
        this.palTo.set(this.palNow);
    }
    /** Writes the six palette levels of one style into out. */
    readPalette(idx, out) {
        const p = this.styles[idx].palette;
        out[0] = p.pad;
        out[1] = p.bass;
        out[2] = p.lead;
        out[3] = p.pluck;
        out[4] = p.drums;
        out[5] = p.perc;
    }
    /** Full reset: voices, reverb, glue, smoothers and the step clock. */
    reset() {
        this.resetVoices();
        for (let i = 0; i < this.smoothers.length; i++) {
            this.smoothers[i].target = 0;
            this.smoothers[i].reset();
        }
        this.pending = 0;
        this.stepIndex = 0;
        this.palSteps = 0;
        this.idle = false;
        this.idleTime = 0;
        this.dyn = 1;
        this.outL.fill(0);
        this.outR.fill(0);
    }
    /** Resets the DSP state without touching the clock (idle entry and the non-finite guard). */
    resetVoices() {
        for (let i = 0; i < INST_COUNT; i++) {
            this.insts[i].reset();
            this.scratchL[i].fill(0);
            this.scratchR[i].fill(0);
        }
        this.reverb.reset();
        this.comp.reset();
        this.dcL.reset();
        this.dcR.reset();
        this.offCount = 0;
    }
    process(p, offset, l, r, n) {
        const f = Math.min(n, BLOCK, l.length, r.length);
        if (f <= 0)
            return;
        const raw = this.raw;
        for (let i = 0; i < PARAM_COUNT; i++)
            raw[i] = finiteOr(p[offset + i], 0);
        const s = this.state;
        s.style = raw[0];
        s.styleB = raw[1];
        s.styleMix = raw[2];
        s.intensity = raw[3];
        s.phase = raw[4];
        s.climax = raw[5];
        s.resolved = raw[6];
        s.won = raw[7];
        s.sparkle = raw[8];
        s.tension = raw[9];
        this.arranger.setState(s);
        // Only the continuous params are smoothed: an index jump never passes through other styles.
        for (let i = 0; i < PARAM_COUNT; i++) {
            if (SMOOTHED[i] === 0)
                continue;
            const sm = this.smoothers[i];
            sm.target = raw[i];
            raw[i] = sm.next();
        }
        const level = clamp(raw[11], 0, 8);
        // Level-0 fast path: skip all DSP once the smoothed level has been near zero for 0.5 s.
        if (level < IDLE_EPS) {
            this.idleTime += f / this.sr;
            if (!this.idle && this.idleTime >= IDLE_SEC) {
                this.idle = true;
                this.resetVoices();
            }
        }
        else {
            this.idle = false;
            this.idleTime = 0;
        }
        if (this.idle)
            return;
        this.updatePalette(raw[2] > 0.5 ? raw[1] : raw[0], raw[10]);
        for (let i = 0; i < INST_COUNT; i++) {
            this.scratchL[i].fill(0, 0, f);
            this.scratchR[i].fill(0, 0, f);
        }
        // Sample-accurate 16th-note clock: pending is the fractional distance to the next step.
        while (this.pending < f) {
            const when = Math.max(0, Math.floor(this.pending)) | 0;
            this.fireStep(when);
            const bpm = this.arranger.bpm;
            const base = (60 / (bpm > 1 ? bpm : 120) / 4) * this.sr;
            const sw = this.styles[this.curStyle].swing;
            this.pending += base * (1 + ((this.stepIndex & 1) === 0 ? sw : -sw));
            this.stepIndex++;
        }
        this.pending -= f;
        for (let i = 0; i < INST_COUNT; i++)
            this.insts[i].process(this.scratchL[i], this.scratchR[i], f);
        // Palette levels per instrument; 'drums' covers kick, snare and hat; sparkle brightens pluck.
        const sparkle = raw[8] > 0.5;
        const g = this.instGain;
        g[0] = this.palNow[0];
        g[1] = this.palNow[1];
        g[2] = this.palNow[2];
        g[3] = this.palNow[3] * (sparkle ? dbToGain(SPARKLE_DB) : 1);
        g[4] = this.palNow[4];
        g[5] = this.palNow[4];
        g[6] = this.palNow[4];
        g[7] = this.palNow[5];
        const outL = this.outL, outR = this.outR, send = this.sendBuf;
        outL.fill(0, 0, f);
        outR.fill(0, 0, f);
        send.fill(0, 0, f);
        for (let i = 0; i < INST_COUNT; i++) {
            const gi = g[i];
            if (gi === 0)
                continue;
            const sl = this.scratchL[i], sr = this.scratchR[i];
            const si = SEND[i] * 0.5 * gi;
            if (si === 0) {
                for (let j = 0; j < f; j++) {
                    outL[j] += sl[j] * gi;
                    outR[j] += sr[j] * gi;
                }
            }
            else {
                for (let j = 0; j < f; j++) {
                    const a = sl[j] * gi, b = sr[j] * gi;
                    outL[j] += a;
                    outR[j] += b;
                    send[j] += (a + b) * si;
                }
            }
        }
        for (let j = 0; j < f; j++) {
            this.reverb.process(send[j], send[j]);
            outL[j] += this.reverb.outL * REVERB_RETURN;
            outR[j] += this.reverb.outR * REVERB_RETURN;
        }
        this.comp.processStereo(outL, outR, f);
        for (let j = 0; j < f; j++) {
            if (!Number.isFinite(outL[j]) || !Number.isFinite(outR[j])) {
                this.resetVoices();
                return;
            }
        }
        this.dyn += (dbToGain(SECTION_DB[this.arranger.section]) - this.dyn) * this.dynCoef;
        const lvl = level * MASTER * this.dyn;
        for (let j = 0; j < f; j++) {
            l[j] += this.dcL.process(outL[j]) * lvl;
            r[j] += this.dcR.process(outR[j]) * lvl;
        }
    }
    /** Fires the step at frame `when`: due note-offs, the palette crossfade, then the arranger. */
    fireStep(when) {
        let w = 0;
        for (let i = 0; i < this.offCount; i++) {
            if (this.offAt[i] <= this.stepIndex)
                this.insts[this.offInst[i]].noteOff(this.offMidi[i], when);
            else {
                this.offMidi[w] = this.offMidi[i];
                this.offInst[w] = this.offInst[i];
                this.offAt[w] = this.offAt[i];
                w++;
            }
        }
        this.offCount = w;
        if (this.palSteps > 0) {
            this.palSteps--;
            const t = 1 - this.palSteps / this.palLen;
            for (let i = 0; i < 6; i++)
                this.palNow[i] = this.palFrom[i] + (this.palTo[i] - this.palFrom[i]) * t;
            if (this.palSteps === 0)
                this.palNow.set(this.palTo);
        }
        const count = this.arranger.nextStep(this.events);
        for (let i = 0; i < count; i++) {
            const e = this.events[i];
            const ii = INST_INDEX[e.inst];
            this.insts[ii].noteOn(e.midi, e.vel, when);
            if (ii < MELODIC && e.len > 0 && this.offCount < OFF_RING) {
                this.offMidi[this.offCount] = e.midi;
                this.offInst[this.offCount] = ii;
                this.offAt[this.offCount] = e.step + e.len;
                this.offCount++;
            }
        }
    }
    /** Starts a palette crossfade of fadeBars bars (clamped 1..16) when the dominant style changes. */
    updatePalette(eff, fadeBars) {
        const idx = clamp(Math.floor(eff), 0, this.styles.length - 1) | 0;
        if (idx === this.curStyle)
            return;
        this.palFrom.set(this.palNow);
        this.readPalette(idx, this.palTo);
        this.palLen = BAR_STEPS * clamp(Math.round(fadeBars), 1, FADE_BARS_MAX);
        this.palSteps = this.palLen;
        this.curStyle = idx;
    }
}
