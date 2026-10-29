import type { ReverbParams } from '../contract.js';
export declare class FdnReverb {
    private readonly sr;
    private readonly lines;
    private readonly damps;
    private readonly predelayLine;
    private readonly maxLen;
    private readonly glide;
    private readonly modDepth;
    private readonly len;
    private readonly lenTarget;
    private readonly gain;
    private readonly gainTarget;
    private readonly modC;
    private readonly modS;
    private readonly rotC;
    private readonly rotS;
    private readonly v;
    private predelay;
    private predelayTarget;
    private fresh;
    private _outL;
    private _outR;
    constructor(sr: number);
    get outL(): number;
    get outR(): number;
    /** Sets new targets; lengths, gains and predelay glide toward them (safe every block). */
    set(p: ReverbParams): void;
    process(inL: number, inR: number): void;
    reset(): void;
    private snap;
    private resetModulation;
}
