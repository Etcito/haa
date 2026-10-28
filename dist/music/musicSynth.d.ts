import type { BedVoice, MusicStyle } from '../contract.js';
/** The music bed params, in the order of the param block. */
export declare const MUSIC_PARAMS: readonly ["style", "styleB", "styleMix", "intensity", "phase", "climax", "resolved", "won", "sparkle", "tension", "fadeBars", "level"];
export declare class MusicSynth implements BedVoice {
    private readonly sr;
    private readonly styles;
    private readonly arranger;
    private readonly insts;
    private readonly scratchL;
    private readonly scratchR;
    private readonly reverb;
    private readonly comp;
    private readonly smoothers;
    private readonly state;
    private readonly raw;
    private readonly events;
    private readonly outL;
    private readonly outR;
    private readonly sendBuf;
    private readonly dcL;
    private readonly dcR;
    private readonly instGain;
    private readonly offMidi;
    private readonly offInst;
    private readonly offAt;
    private readonly palFrom;
    private readonly palTo;
    private readonly palNow;
    private offCount;
    private pending;
    private stepIndex;
    private palSteps;
    /** Length in steps of the running palette crossfade. */
    private palLen;
    private curStyle;
    private idle;
    private idleTime;
    private dyn;
    private readonly dynCoef;
    constructor(sr: number, seed: number, styles?: readonly MusicStyle[]);
    /** Writes the six palette levels of one style into out. */
    private readPalette;
    /** Full reset: voices, reverb, glue, smoothers and the step clock. */
    reset(): void;
    /** Resets the DSP state without touching the clock (idle entry and the non-finite guard). */
    private resetVoices;
    process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void;
    /** Fires the step at frame `when`: due note-offs, the palette crossfade, then the arranger. */
    private fireStep;
    /** Starts a palette crossfade of fadeBars bars (clamped 1..16) when the dominant style changes. */
    private updatePalette;
}
