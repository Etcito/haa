import type { SpatialModel } from '../contract.js';
/**
 * Create the spatial model. absorptionScale (default 1) scales the air absorption distance.
 * With 0 listeners, one listener at the origin with the identity quat is used; N =
 * min(listeners.length, MAX_LISTENERS). Listener l gets gain × 1/N; sp[SP.SEND] and rate come
 * from the listener with the highest gain (ties: the lowest index). A null source gets gain
 * 1/N, pan 0 and lowpass 20000 per listener, send REVERB_SEND_BASE and rate 1.
 */
export declare function createSpatial(o?: {
    absorptionScale?: number;
}): SpatialModel;
