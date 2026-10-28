import type { MixPreset, ReverbParams } from '../contract.js';
/** Running meter values (debugging). */
export interface MixMeter {
    /** Maximum true peak of the limiter output since the last reset(), dBTP (4x interpolated estimate). */
    truePeakDb: number;
    /** Limiter gain reduction over the last block, dB (>= 0). */
    reductionDb: number;
}
export declare class MixChain {
    readonly meter: MixMeter;
    private readonly sr;
    private preset;
    private readonly busL;
    private readonly busR;
    /** Index (bus × 2 + channel) × EQ_BANDS + band. */
    private readonly eq;
    private readonly comps;
    private readonly levels;
    private readonly duckers;
    private readonly reverb;
    private readonly returnGain;
    private readonly mixL;
    private readonly mixR;
    private readonly gMusic;
    private readonly gSfx;
    private readonly gMaster;
    private readonly userMaster;
    private readonly userMusic;
    private readonly userSfx;
    private readonly trim;
    private readonly muffle;
    private readonly svfL;
    private readonly svfR;
    private svfV;
    private readonly glue;
    private readonly limiter;
    private readonly histL;
    private readonly histR;
    private peak;
    /** False until the first process() after construction or reset(): parameter changes jump. */
    private primed;
    constructor(sr: number, preset: MixPreset);
    /** Re-apply the bus levels, EQ, compressors, glue, limiter, trim and reverb of p. */
    setPreset(p: MixPreset): void;
    /** User volumes, each clamped to 0..1 and smoothed over 50 ms. */
    setUser(master: number, music: number, sfx: number): void;
    /**
     * Trigger preset.ducks[kind] on the music and ambience buses, depths × strength (0..1).
     * A kind that is not an own key of preset.ducks is ignored.
     */
    duck(kind: string, strength: number): void;
    /** Reverb parameters and return level; kept until the next setReverb or setPreset. */
    setReverb(p: ReverbParams): void;
    /** Muffle amount 0..1: the master lowpass moves from 20 kHz (0) to 700 Hz (1) over 150 ms. */
    setMuffle(v: number): void;
    /**
     * Mix n frames. inputs[i] is the stereo input of bus i (BUS_INDEX order), then REVERB_INPUT;
     * a missing or short input counts as silence (a lone left channel is used for both sides).
     * n above BLOCK is processed in BLOCK chunks.
     */
    process(inputs: readonly (readonly [Float32Array, Float32Array])[], outL: Float32Array, outR: Float32Array, n: number): void;
    /** Clear every filter, dynamics, reverb and meter state; glides snap to their targets. */
    reset(): void;
    private processBlock;
    /** Running true-peak estimate of the limiter output: the samples plus a 4x Catmull-Rom interpolation. */
    private measure;
    private setSvf;
    /** Set a glide target; before the first block (or after reset) the value jumps. */
    private glide;
    private resetDsp;
}
