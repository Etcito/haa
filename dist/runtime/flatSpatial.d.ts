import type { SpatialModel } from '../contract.js';
/**
 * Every listener l < N gets gain 1/N, pan 0 and lowpass 20000, with send REVERB_SEND_BASE
 * and rate 1, where N = max(1, min(listeners.length, MAX_LISTENERS)). Positions and
 * occlusion are ignored; slots of listeners >= N are zeroed.
 */
export declare const FLAT_SPATIAL: SpatialModel;
