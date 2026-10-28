import type { MusicStyle } from '../contract.js';
export type Section = 'intro' | 'mainA' | 'mainB' | 'intense' | 'climax' | 'resolveWin' | 'resolve' | 'post';
export interface MusicState {
    style: number;
    styleB: number;
    styleMix: number;
    intensity: number;
    phase: number;
    climax: number;
    resolved: number;
    won: number;
    sparkle: number;
    tension: number;
}
export interface NoteEvent {
    inst: 'pad' | 'bass' | 'lead' | 'pluck' | 'kick' | 'snare' | 'hat' | 'perc';
    midi: number;
    vel: number;
    step: number;
    len: number;
}
export declare class Arranger {
    private readonly rng;
    private readonly progRng;
    private readonly styles;
    private style;
    private activeStyle;
    private pendingStyle;
    private activeIsB;
    private transpose;
    private step;
    private bar;
    private mainBars;
    private introBars;
    private finishBar;
    private fillBar;
    private crashBar;
    private progIndex;
    private progChord;
    private holdBars;
    private chordDegree;
    private sectionState;
    private bpmBase;
    private rampFrom;
    private bpmValue;
    private rampRemaining;
    private readonly state;
    private readonly pool;
    private readonly gateState;
    private readonly triad;
    private readonly padVoices;
    private readonly prevPad;
    private hasPrevPad;
    private readonly motifSeeds;
    private readonly motifRng;
    private readonly mOn;
    private readonly mDeg;
    private readonly mLen;
    private readonly mCover;
    private mSteps;
    private motifSec;
    private motifStyle;
    private motifBar;
    private varKind;
    private varAmt;
    private arpIndex;
    constructor(seed: number, styles: readonly MusicStyle[]);
    get section(): Section;
    get bpm(): number;
    get styleIndex(): number;
    setState(s: MusicState): void;
    nextStep(out: NoteEvent[]): number;
    /** Bar-quantized state machine, style change, chord and fill decision. */
    private onBar;
    private isFinished;
    private sectionFor;
    private onSectionChange;
    /** Markov choice among the style's progressions: the next one is favoured and an immediate
     *  repeat is forbidden (unless there is only one). */
    private pickProgression;
    /** One chord per bar (one per two bars in intro and post). */
    private advanceChord;
    /** In-scale triad of the degree, folded into one octave, then the inversion nearest the
     *  previous voicing (voice leading). */
    private voiceChord;
    private buildTriad;
    private scalePitch;
    private totalStep;
    /** Pad, bass and drums of the current step; the melodic layers come from emitMelodicLayers. */
    private emitSection;
    private emitPad;
    /** resolveWin: the in-scale major resolve (I, or the relative major III in minor and dorian), a
     *  crash and a held pad. resolve: the softer tonic resolve at velocity <= 0.5, no crash, with a
     *  quiet pluck arpeggio. Both chords and their bass root hold for the 2-bar section, into post. */
    private emitResolve;
    /** Root and fifth; 8ths below intensity 0.6, with 16th syncopation above, an octave under the pad. */
    private emitBass;
    /** Per-section beat with hats scaled by intensity; the fill bar before a section change plays
     *  toms and rising snare 16ths. climax and intense get the full pattern. */
    private emitDrums;
    /** Writes pool[n] and returns n + 1; never allocates. */
    private put;
    /** Intensity 0..1 to the active layers: pad always, then bass 0.2, drums 0.35, pluck 0.5,
     *  lead 0.65 and counter-melody 0.8. Returns the reused gate object. */
    private gate;
    /** Melodic layers: the motif lead, the counter-melody, the pluck arpeggio, the sparkle layer and
     *  the resolveWin flourish. intro, resolve and post emit no lead. */
    private emitMelodicLayers;
    /** A new motif on entering a section or after a style change; then one variation per 2-bar
     *  phrase, cycling original, transposition, displacement and inversion, so the 4th repetition
     *  always varies and a motif never repeats more than twice unchanged. */
    private onMotifBar;
    /** 1 or 2 bars of 4-step rhythm cells; degrees relative to the chord root, stepwise motion
     *  preferred and leaps of at most a 5th. Legato notes overlap the next onset (lead glide). */
    private makeMotif;
    private motifIndex;
    /** Octave that puts the lead's root in 65..76. */
    private leadShift;
    /** The motif as 'lead': an octave up in intense, doubled 12 semitones up at velocity × 0.7 in climax. */
    private emitLead;
    /** Counter-melody: descending chord tones in 8ths, an octave under the lead, in the motif's rests. */
    private emitCounter;
    /** Chord tones up then down, an octave above the pad: 8ths, or 16ths in intense and climax. */
    private emitArp;
    /** sparkle: a bright 16th arpeggio of chord tones 2 octaves above the pad, velocity 0.5-0.7. */
    private emitSparkle;
    /** resolveWin: 4 ascending scale notes ending on the resolve chord's root (degree as emitResolve). */
    private emitFlourish;
    /** Scale quantizer of the melodic layers: snaps down to the nearest pitch of the transposed scale. */
    private mel;
}
