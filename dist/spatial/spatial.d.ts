import type { Listener, Quat, SourcePos, Vec3 } from '../contract.js';
/** Speed of sound in m/s. */
export declare const SPEED_OF_SOUND = 343;
/** Doppler exponent: the pitch ratio is the physical ratio raised to this power. */
export declare const DOPPLER_AMOUNT = 0.5;
/** Distance (m) under which the distance gain is 1. */
export declare const REF_DISTANCE = 4;
/** Default audible range (m); gain is 0 at and beyond it. */
export declare const DEFAULT_RANGE = 80;
/** Lowpass cutoff (Hz) of an unfiltered source. */
export declare const OPEN_LOWPASS_HZ = 20000;
/** Lowpass cutoff (Hz) of a source straight behind the listener. */
export declare const REAR_LOWPASS_HZ = 7000;
/** Distance scale (m) of air absorption: lowpass × clamp(1 - d / (AIR_ABSORPTION_M × scale), 0.3, 1). */
export declare const AIR_ABSORPTION_M = 400;
/** Lowest lowpass cutoff (Hz) reached by occlusion. */
export declare const OCCLUDED_MIN_HZ = 900;
/** Gain (dB) of a fully occluded source. */
export declare const OCCLUDED_GAIN_DB = -9;
/** Pan width used when more than one listener shares the output. */
export declare const SPLIT_PAN_WIDTH = 0.5;
/** Extra reverb send reached at REVERB_SEND_DIST_M and beyond. */
export declare const REVERB_SEND_DIST = 0.35;
/** Distance (m) at which the extra reverb send is full. */
export declare const REVERB_SEND_DIST_M = 60;
/** Lowest Doppler ratio. */
export declare const DOPPLER_MIN = 0.5;
/** Highest Doppler ratio. */
export declare const DOPPLER_MAX = 2;
/** Distance gain: 1 for d ≤ ref, (ref / d) × (1 - smoothstep(0.7 max, max, d)) above, 0 for d ≥ max. */
export declare function distanceGain(d: number, ref?: number, max?: number): number;
/** Rotate v by the quaternion q into out: v' = v + 2w(u×v) + 2u×(u×v), u = (x,y,z). out may be v. */
export declare function rotate(q: Quat, v: Vec3, out: Vec3): Vec3;
/** Listener frame of q: right = q·(1,0,0), up = q·(0,1,0), forward = q·(0,0,-1). */
export declare function listenerAxes(q: Quat, right: Vec3, up: Vec3, fwd: Vec3): void;
/** Spatial result of one source for one listener (gain excludes the 1/N factor). */
export interface OneListener {
    /** Linear gain: distance × occlusion, without 1/N. */
    gain: number;
    /** Pan -1..1 (equal-power). */
    pan: number;
    /** Lowpass cutoff in Hz. */
    lowpass: number;
    /** Reverb send 0..1. */
    send: number;
    /** Doppler pitch ratio. */
    doppler: number;
    /** Source-listener distance in m. */
    dist: number;
}
/**
 * Spatialize src for listener l into out. count (N) only selects the pan width
 * (SPLIT_PAN_WIDTH when N > 1); the 1/N gain factor is applied by the model, not here.
 * occlusion is the occlusion function's value for this listener (0..1).
 */
export declare function spatializeOne(l: Listener, src: SourcePos, count: number, occlusion: number, env: {
    absorptionScale: number;
}, out: OneListener): OneListener;
/** Doppler pitch ratio ((c + vl) / (c - vs))^DOPPLER_AMOUNT, clamped to DOPPLER_MIN..MAX; 1 for d < 1e-6. */
export declare function dopplerRatio(l: Listener, src: SourcePos): number;
