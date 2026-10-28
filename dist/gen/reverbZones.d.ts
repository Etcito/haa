import type { ReverbParams } from '../contract.js';
/** A generic acoustic zone id, each with its own reverb preset. */
export type ReverbZoneId = 'open' | 'canyon' | 'snowfield' | 'cavern' | 'water' | 'city' | 'sky' | 'forest' | 'room' | 'underwater';
/** One reverb preset per zone, the submerged zone included. */
export declare const REVERB_ZONES: Record<ReverbZoneId, ReverbParams>;
/** Per-zone air absorption multiplier (0.6 snowfield / 0.7 forest / 1.4 sky / 1 elsewhere). */
export declare const ABSORPTION_SCALE: Record<Exclude<ReverbZoneId, 'underwater'>, number>;
/** Linear blend of a and b into out (t clamped to 0..1, NaN counts as 0). No allocation. */
export declare function blendReverb(a: ReverbParams, b: ReverbParams, t: number, out: ReverbParams): ReverbParams;
