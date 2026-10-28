import type { BedDef, FamilyDef } from '../contract.js';
/** The single baked wind pass-by whoosh family. */
export declare const WIND_FAMILIES: readonly FamilyDef[];
/**
 * HAA tuning: the wind bed gain staging (linear), a pure function of speed. Above 30 m/s the
 * WindModel band gains, whistle and buffeting push the raw bed over the peak ceiling (+8 dBFS at
 * 50 m/s in the m1 sweep), so the reduction grows smoothly from 30 m/s to 50 m/s and beyond;
 * below 30 m/s only the fixed trim applies and the character is unchanged.
 */
export declare function windBedGain(speed: number): number;
/**
 * The wind bed: one listener-level voice on the ambience bus, with the eight params in the
 * order WindModel reads them: speed, speed01, air, boost, slip, yaw, underwater and level.
 */
export declare const WIND_BED: BedDef;
