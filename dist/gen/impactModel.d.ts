import type { MaterialId } from '../contract.js';
import { type Mode } from '../dsp/modal.js';
import type { AudioRng } from '../dsp/rng.js';
export type ImpactSize = 'small' | 'medium' | 'large';
export interface MaterialSpec {
    modes: readonly Mode[];
    baseHz: number;
    jitter: number;
    brightness: number;
    transient: {
        hpHz: number;
        noise: number;
    };
    debris: {
        density: number;
        t60: number;
        band: readonly [number, number];
        level: number;
    };
}
/** Mode tables are [ratio to baseHz, t60 s, gain]. */
export declare const MATERIAL_SPECS: Record<MaterialId, MaterialSpec>;
export declare const SIZE_SCALE: Record<ImpactSize, {
    freq: number;
    decay: number;
    gain: number;
}>;
export interface ImpactOpts {
    material: MaterialId;
    size: ImpactSize;
    /** 0..1 */
    velocity: number;
    /** Contact duration in seconds, 0.0005..0.008. */
    contact: number;
}
/**
 * Renders one impact and ADDS it into out[0..used); returns used. Allocates only here, at
 * call entry. Non-finite options fall back to a medium metal hit at velocity 1, contact 1.5 ms.
 */
export declare function renderImpact(o: ImpactOpts, sr: number, rng: AudioRng, out: Float32Array): number;
