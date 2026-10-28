import type { BedDef, FamilyDef, MixPreset, StereoRender } from '../contract.js';
/**
 * Render `seconds` of stereo audio by calling `fn(l, r, n, frame)` once per block of at most
 * BLOCK frames. `l` and `r` are zeroed before every call (fn ADDS into them); `frame` is the
 * index of the block's first frame. The output has round(seconds × sampleRate) frames.
 */
export declare function renderOffline(fn: (l: Float32Array, r: Float32Array, n: number, frame: number) => void, seconds: number, sampleRate?: number, name?: string): StereoRender;
/**
 * The normalized variants of one family (renderFamilyVariants with the preset's category
 * target) in order, separated by gapSec (default 0.25 s) of silence, placed centered:
 * × √½ on both channels, so L === R.
 */
export declare function renderFamilyStrip(def: FamilyDef, preset: MixPreset, o?: {
    gapSec?: number;
    sampleRate?: number;
}): StereoRender;
/**
 * Run one BedVoice from def.create(sr, seed) for `seconds`. The params p start as a copy of
 * def.defaults; before every block, script(t, p) runs with t = the block start in seconds,
 * then the voice processes p at offset 0. Returns the raw bed output (no mix, no trim).
 */
export declare function renderBedDemo(def: BedDef, o: {
    seconds: number;
    seed: number;
    script: (t: number, p: Float32Array) => void;
    sampleRate?: number;
    name?: string;
}): StereoRender;
