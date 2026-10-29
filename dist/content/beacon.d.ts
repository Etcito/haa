import type { BedDef, BedVoice } from '../contract.js';
/**
 * The beacon voice. Params (level, rate, doppler) are copied from p[offset..] every block;
 * every buffer and mode record is preallocated, so process() allocates nothing.
 */
export declare class BeaconModel implements BedVoice {
    private readonly sr;
    private readonly bank;
    private readonly modes;
    private readonly whiteRng;
    private readonly pinkRng;
    private readonly whiteSeed;
    private readonly pinkSeed;
    private readonly white;
    private readonly pink;
    private readonly hum;
    private readonly dc;
    private readonly buf;
    private readonly burstLen;
    private doppler;
    /** Samples since the last ping, and the samples left in the current burst. */
    private sinceBurst;
    private burstPos;
    constructor(sr: number, seed: number);
    private setDoppler;
    process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void;
    reset(): void;
}
/** The beacon bed: a positional effects-bus source whose 'doppler' param the bed host writes. */
export declare const BEACON_BED: BedDef;
