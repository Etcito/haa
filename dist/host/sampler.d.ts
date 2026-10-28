/** A play request (the fields of SamplerMsg's 'play' variant, without the tag). */
export interface SamplerPlay {
    voice: number;
    id: string;
    variant: number;
    bus: number;
    gain: number;
    rate: number;
    sp: Float32Array;
}
export declare class Sampler {
    private readonly sr;
    private readonly voices;
    private readonly families;
    private activeCount;
    /** Scratch target coefficients for the voice being rendered. */
    private readonly tl;
    private readonly tr;
    constructor(sr: number);
    /** Number of active voices (including ones fading out). */
    get playing(): number;
    /** Store a family's variants and their native sample rate, keeping the arrays by reference. */
    load(id: string, variants: Float32Array[], rate: number): void;
    /** True when a family id has been loaded. */
    has(id: string): boolean;
    /** Number of loaded variants of a family (0 when unknown). */
    variantCount(id: string): number;
    /** Start a voice in slot c.voice; an occupied slot is faded out over 5 ms first.
     *  Unknown ids, variants and slot numbers are ignored. */
    play(c: SamplerPlay): void;
    /** Set new rate and spatial targets for a voice; they are smoothed over 10 ms. */
    update(voice: number, rate: number, sp: Float32Array): void;
    /** Fade a voice out linearly over fade seconds (dropping a pending replacement); a
     *  non-positive fade stops it at once. */
    stop(voice: number, fade: number): void;
    /** Fade every active voice out over fade seconds. */
    stopAll(fade: number): void;
    /** True when the slot is free (finished, stopped or never used). */
    voiceDone(voice: number): boolean;
    /** OVERWRITES the MIX_INPUTS stereo pairs, then ADDS every voice into its bus and send pairs. */
    process(out: readonly [Float32Array, Float32Array][], n: number): void;
    /** Per-listener target pan × gain coefficients and lowpass coefficients from v.sp; returns N. */
    private coefs;
    /** Start the slot's pending play with its coefficients already at their targets. */
    private start;
    /** End the slot's voice; a pending replacement starts at once (it sounds from the next block). */
    private end;
}
