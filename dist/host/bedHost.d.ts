import type { BedDef } from '../contract.js';
/** One procedural bed instance: params, spatial block, fade and guard around a BedVoice. */
export declare class BedHost {
    private readonly sr;
    private readonly def;
    private readonly voice;
    private readonly gain;
    private readonly target;
    private readonly current;
    private readonly smooth;
    private readonly coef;
    private readonly dopplerIndex;
    private readonly sp;
    private readonly lowpass;
    private readonly tmpL;
    private readonly tmpR;
    private readonly tmpMono;
    private rate;
    private fadeLeft;
    private fadeStep;
    private fadeGain;
    private ended;
    /** Create the voice from def.create(sr, seed); trimDb applies to the voice output. */
    constructor(sr: number, def: BedDef, seed: number, trimDb?: number);
    /** True once end() was called and its fade has completed; the output stays silent. */
    get done(): boolean;
    /** Set target values in def.params order; non-finite values are ignored. */
    setParams(v: ArrayLike<number>): void;
    /** Copy the spatial block and the Doppler rate (written into a 'doppler' param when present). */
    setSpatial(rate: number, sp: Float32Array): void;
    /** Ramp linearly to silence over fadeSec seconds, then report done. */
    end(fadeSec: number): void;
    /** Reset the voice, the filters, the fade and the params to their defaults. */
    reset(): void;
    /** OVERWRITE bus and send with n frames of the bed (silence once done). */
    process(bus: [Float32Array, Float32Array], send: [Float32Array, Float32Array], n: number): void;
    private block;
    /** Mono (L+R)/2 through each listener's lowpass, equal-power pan and gain, summed in place. */
    private spatialize;
    /** FLAT spatial values for one listener: count 1, gain 1, pan 0, open lowpass, base send. */
    private flatSpatial;
}
