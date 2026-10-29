import { AudioRng } from '../dsp/rng.js';
import { Adsr, ExpDecay } from '../dsp/env.js';
import { Svf, DcBlocker, Biquad, OnePole } from '../dsp/filters.js';
import { DelayLine } from '../dsp/delay.js';
import { Saturator } from '../dsp/shaper.js';
import { ModalBank, type Mode } from '../dsp/modal.js';
import { TransientShaper } from '../dsp/dynamics.js';
import { WhiteNoise, VelvetNoise } from '../dsp/noise.js';
export interface Instrument {
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(midi: number, when: number): void;
    process(l: Float32Array, r: Float32Array, n: number): void;
    reset(): void;
}
/** A small preallocated note-event queue. */
declare class NoteQueue {
    when: Int32Array<ArrayBuffer>;
    midi: Int32Array<ArrayBuffer>;
    vel: Float32Array<ArrayBuffer>;
    on: Uint8Array<ArrayBuffer>;
    count: number;
    push(w: number, m: number, v: number, o: boolean): void;
}
/** Quadrature rotating-phasor additive stack: one complex multiply per partial per sample. */
declare class PartialStack {
    ratio: Float64Array;
    amp: Float64Array;
    re: Float64Array;
    im: Float64Array;
    rc: Float64Array;
    rs: Float64Array;
    readonly count: number;
    constructor(count: number);
    setPartial(i: number, ratio: number, amp: number, phase: number): void;
    /** Set one partial's rotation and renormalize its phasor (once per block per partial). */
    setPartialHz(i: number, hz: number, sr: number): void;
    setHz(f0: number, sr: number): void;
    /** One complex multiply per partial; returns sum(amp_i * sin(phase_i)). A non-finite phasor
     *  is caught once per block by setPartialHz (the per-sample check cost more than the pad). */
    next(): number;
    reset(): void;
}
/** One pad note: 7 detuned 4-partial voices through its own Svf, amp and filter envelopes. The
 *  filter envelope also tilts the partial amplitudes: the note blooms bright over its 0.6 s
 *  attack, mellows through the 2.5 s decay (upper partials fall faster than the fundamental)
 *  and darkens further in the release, while the LFO keeps the sustained timbre moving. */
declare class PadNote {
    stack: PartialStack;
    ba: Float64Array<ArrayBuffer>;
    dp: Float32Array<ArrayBuffer>;
    di: Float32Array<ArrayBuffer>;
    dd: Float32Array<ArrayBuffer>;
    tmp: Float32Array<ArrayBuffer>;
    filter: Svf;
    amp: Adsr;
    fenv: Adsr;
    base: number;
    li: number;
    lp: number;
    note: number;
    age: number;
    active: boolean;
    /** Seed the 7 detuned voices, trigger the envelopes and set the first block's filter. */
    start(midi: number, vel: number, rng: AudioRng, sr: number, n: number): void;
    release(): void;
    /** Re-strikes a sounding note of the same pitch: both envelopes rise again from their current
     *  level (the timbre re-blooms) with no gap and no stolen voice. */
    restrike(vel: number, sr: number): void;
    /** Block-rate modulation: the filter envelope, the cutoff LFO and the per-partial drift. */
    update(sr: number, n: number): void;
    sample(): number;
    reset(): void;
}
/** A 4-note polyphonic pad: 7 detuned voices per note and a stereo chorus on the sum. */
export declare class PadInstrument implements Instrument {
    sr: number;
    rng: AudioRng;
    notes: PadNote[];
    q: NoteQueue;
    mono: Float32Array<ArrayBuffer>;
    ca: DelayLine;
    cb: DelayLine;
    ia: number;
    ib: number;
    base: number;
    depth: number;
    pa: number;
    pb: number;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(midi: number, when: number): void;
    process(l: Float32Array, r: Float32Array, n: number): void;
    /** Applies every queued event at exactly frame s (note stealing happens here). */
    private applyEvents;
    private startNote;
    private stopNote;
    /** Two slow modulated delay lines on the summed pad: dry + wet into l and r. */
    private chorus;
    reset(): void;
}
/** A monophonic bass: a sub plus 8 harmonics, 2x saturated, lowpassed, glide on overlap. */
export declare class BassInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    stack: PartialStack;
    sat: Saturator;
    filter: Svf;
    amp: Adsr;
    fenv: Adsr;
    dc: DcBlocker;
    glide: number;
    f0: number;
    target: number;
    vel: number;
    active: boolean;
    gate: boolean;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(midi: number, when: number): void;
    process(l: Float32Array, r: Float32Array, n: number): void;
    private applyEvents;
    /** Starts fresh or, while a note is already held, retargets and glides. */
    private startNote;
    /** Block-rate glide and velocity-scaled filter envelope. */
    private update;
    /** One sample: stack -> 2x saturator -> filter -> amp -> DC blocker, centered near -6 dBFS. */
    private sample;
    private resetVoice;
    reset(): void;
}
/** One monophonic lead voice: a 2-operator FM pair built from rotating phasors (no
 *  per-sample trig) plus three seeded additive formant bands. Glide, delayed vibrato and
 *  the Haas offset are applied by the instrument around it. */
declare class LeadVoice {
    mod: PartialStack;
    form: PartialStack;
    fa: Float64Array<ArrayBuffer>;
    amp: Adsr;
    ienv: Adsr;
    fenv: Adsr;
    filter: Svf;
    fc: Float32Array<ArrayBuffer>;
    ratio: number;
    glide: number;
    cr: number;
    ci: number;
    wc: number;
    wm: number;
    idx: number;
    f0: number;
    target: number;
    vel: number;
    age: number;
    vib: number;
    vibDepth: number;
    active: boolean;
    gate: boolean;
    note: number;
    constructor(rng: AudioRng);
    start(midi: number, vel: number, rng: AudioRng, sr: number): void;
    release(): void;
    /** Block-rate glide, delayed vibrato, FM index envelope, the filter envelope (bright attack
     *  closing to a mellow sustain), the formant bands (the 2.6 kHz band follows the index
     *  envelope, the 1.2 kHz band opens over 0.35 s like a vowel) and phasor renormalization. */
    update(sr: number, n: number): void;
    /** Carrier rotated by fc + index * fm * modulator, small-angle (no per-sample trig). */
    sample(): number;
    reset(): void;
}
/** A monophonic FM lead with additive formants, glide, delayed vibrato and a Haas spread. */
export declare class LeadInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    voice: LeadVoice;
    rng: AudioRng;
    mono: Float32Array<ArrayBuffer>;
    haas: DelayLine;
    hd: number;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(midi: number, when: number): void;
    process(l: Float32Array, r: Float32Array, n: number): void;
    reset(): void;
}
/** One Karplus-Strong voice: a seeded, velocity-brightened noise burst in a delay loop
 *  with a one-pole loss filter and a first-order dispersion allpass (seeded stiffness). */
declare class PluckVoice {
    line: DelayLine;
    loss: OnePole;
    bright: OnePole;
    amp: Adsr;
    delay: number;
    refl: number;
    ap: number;
    apX: number;
    apY: number;
    gL: number;
    gR: number;
    active: boolean;
    note: number;
    stamp: number;
    constructor(maxSamples: number);
    start(midi: number, vel: number, rng: AudioRng, sr: number, stamp: number): void;
    release(): void;
    sample(): number;
    reset(): void;
}
/** A 6-note plucked string with body resonance on the sum and a per-voice pan spread. */
export declare class PluckInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    rng: AudioRng;
    voices: PluckVoice[];
    b1L: Biquad;
    b2L: Biquad;
    b1R: Biquad;
    b2R: Biquad;
    seq: number;
    gain: number;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(midi: number, when: number): void;
    process(l: Float32Array, r: Float32Array, n: number): void;
    private startNote;
    private stopNote;
    reset(): void;
}
/** General MIDI drum map, shared verbatim with the arranger. */
export declare const DRUM_NOTES: {
    readonly kick: 36;
    readonly snare: 38;
    readonly rim: 37;
    readonly hatClosed: 42;
    readonly hatOpen: 46;
    readonly crash: 49;
    readonly tomLow: 45;
    readonly tomMid: 47;
    readonly tomHigh: 50;
    readonly shaker: 70;
    readonly shakerAlt: 82;
};
/** A one-shot kick: a saturated pitch-swept body (150 -> ~48 Hz, exponential curve) plus a
 *  2-4 ms band-passed click. Velocity scales level and sweep depth. noteOff is a no-op. */
export declare class KickInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    rng: AudioRng;
    sat: Saturator;
    click: Svf;
    white: WhiteNoise;
    amp: ExpDecay;
    clickEnv: ExpDecay;
    phase: number;
    fEnd: number;
    fStart: number;
    sweep: number;
    sweepCoef: number;
    t60: number;
    clickT60: number;
    clickAmp: number;
    level: number;
    active: boolean;
    constructor(sr: number, seed: number);
    noteOn(_midi: number, vel: number, when: number): void;
    noteOff(_midi: number, _when: number): void;
    private trigger;
    process(l: Float32Array, r: Float32Array, n: number): void;
    private resetVoice;
    reset(): void;
}
/** A one-shot snare: a 2-mode modal shell (about 180/330 Hz, seeded ±5%) excited by an
 *  impulse plus band-passed noise, with a transient shaper on the attack. Note 37 is a
 *  quieter rim/ghost. noteOff is a no-op. */
export declare class SnareInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    rng: AudioRng;
    shell: ModalBank;
    noiseF: Svf;
    white: WhiteNoise;
    ts: TransientShaper;
    noiseEnv: ExpDecay;
    modes: Mode[];
    excite: number;
    level: number;
    noiseAmp: number;
    noiseT60: number;
    tail: number;
    active: boolean;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(_midi: number, _when: number): void;
    private trigger;
    process(l: Float32Array, r: Float32Array, n: number): void;
    private resetVoice;
    reset(): void;
}
/** One-shot hats: 6 inharmonic metallic modal partials (3-12 kHz) plus high-passed white
 *  noise at 7 kHz. Closed (42) is choked by its own retrigger, open (46) rings 0.35 s and
 *  crash (49) 1.5 s with more energy. Every hit draws a seeded decay (±10%) and level
 *  (±1 dB) variation, so repeated hats never repeat identically. noteOff is a no-op. */
export declare class HatInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    rng: AudioRng;
    bank: ModalBank;
    hp: Biquad;
    white: WhiteNoise;
    noiseEnv: ExpDecay;
    modes: Mode[];
    ratios: number[];
    excite: number;
    level: number;
    noiseAmp: number;
    tail: number;
    active: boolean;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(_midi: number, _when: number): void;
    private trigger;
    process(l: Float32Array, r: Float32Array, n: number): void;
    private resetVoice;
    reset(): void;
}
/** One perc voice: a 3-mode modal tom or a velvet-noise shaker with a 2-grain envelope. */
declare class PercVoice {
    sr: number;
    bank: ModalBank;
    f: Svf;
    velvet: VelvetNoise;
    env1: ExpDecay;
    env2: ExpDecay;
    modes: Mode[];
    excite: number;
    g2: number;
    kind: number;
    level: number;
    gL: number;
    gR: number;
    tail: number;
    active: boolean;
    stamp: number;
    note: number;
    constructor(sr: number, rng: AudioRng);
    trigger(midi: number, vel: number, rng: AudioRng, stamp: number): void;
    sample(): number;
    /** Block-rate: a decayed tom or shaker frees its slot and its DSP. */
    settle(): void;
    reset(): void;
}
/** Modal toms (45/47/50) and velvet shakers (70/82), panned per note. */
export declare class PercInstrument implements Instrument {
    sr: number;
    q: NoteQueue;
    rng: AudioRng;
    voices: PercVoice[];
    seq: number;
    constructor(sr: number, seed: number);
    noteOn(midi: number, vel: number, when: number): void;
    noteOff(_midi: number, _when: number): void;
    process(l: Float32Array, r: Float32Array, n: number): void;
    private startNote;
    reset(): void;
}
export {};
