// Spatial math of HAA: the DSP-safe per-listener formulas of the shared contract ('createSpatial
// formulas'), on plain {x,y,z} vectors and {x,y,z,w} quaternions. Written for HAA (not ported// from the origin engine). Nothing here allocates per call: results go into caller-owned objects.
import type { Listener, Quat, SourcePos, Vec3 } from '../contract.ts'
import { REVERB_SEND_BASE } from '../contract.ts'
import { clamp, smoothstep } from '../dsp/math.ts'

/** Speed of sound in m/s. */
export const SPEED_OF_SOUND = 343
/** Doppler exponent: the pitch ratio is the physical ratio raised to this power. */
export const DOPPLER_AMOUNT = 0.5
/** Distance (m) under which the distance gain is 1. */
export const REF_DISTANCE = 4
/** Default audible range (m); gain is 0 at and beyond it. */
export const DEFAULT_RANGE = 80
/** Lowpass cutoff (Hz) of an unfiltered source. */
export const OPEN_LOWPASS_HZ = 20000
/** Lowpass cutoff (Hz) of a source straight behind the listener. */
export const REAR_LOWPASS_HZ = 7000
/** Distance scale (m) of air absorption: lowpass × clamp(1 - d / (AIR_ABSORPTION_M × scale), 0.3, 1). */
export const AIR_ABSORPTION_M = 400
/** Lowest lowpass cutoff (Hz) reached by occlusion. */
export const OCCLUDED_MIN_HZ = 900
/** Gain (dB) of a fully occluded source. */
export const OCCLUDED_GAIN_DB = -9
/** Pan width used when more than one listener shares the output. */
export const SPLIT_PAN_WIDTH = 0.5
/** Extra reverb send reached at REVERB_SEND_DIST_M and beyond. */
export const REVERB_SEND_DIST = 0.35
/** Distance (m) at which the extra reverb send is full. */
export const REVERB_SEND_DIST_M = 60
/** Lowest Doppler ratio. */
export const DOPPLER_MIN = 0.5
/** Highest Doppler ratio. */
export const DOPPLER_MAX = 2

/** Distance gain: 1 for d ≤ ref, (ref / d) × (1 - smoothstep(0.7 max, max, d)) above, 0 for d ≥ max. */
export function distanceGain(d: number, ref = REF_DISTANCE, max = DEFAULT_RANGE): number {
  if (d <= ref) return 1
  if (d >= max) return 0
  return (ref / d) * (1 - smoothstep(0.7 * max, max, d))
}

/** Rotate v by the quaternion q into out: v' = v + 2w(u×v) + 2u×(u×v), u = (x,y,z). out may be v. */
export function rotate(q: Quat, v: Vec3, out: Vec3): Vec3 {
  const ux = q.x, uy = q.y, uz = q.z, w = q.w
  const vx = v.x, vy = v.y, vz = v.z
  const cx = uy * vz - uz * vy
  const cy = uz * vx - ux * vz
  const cz = ux * vy - uy * vx
  out.x = vx + 2 * w * cx + 2 * (uy * cz - uz * cy)
  out.y = vy + 2 * w * cy + 2 * (uz * cx - ux * cz)
  out.z = vz + 2 * w * cz + 2 * (ux * cy - uy * cx)
  return out
}

const UNIT_RIGHT: Vec3 = { x: 1, y: 0, z: 0 }
const UNIT_UP: Vec3 = { x: 0, y: 1, z: 0 }
const UNIT_FWD: Vec3 = { x: 0, y: 0, z: -1 }

/** Listener frame of q: right = q·(1,0,0), up = q·(0,1,0), forward = q·(0,0,-1). */
export function listenerAxes(q: Quat, right: Vec3, up: Vec3, fwd: Vec3): void {
  rotate(q, UNIT_RIGHT, right)
  rotate(q, UNIT_UP, up)
  rotate(q, UNIT_FWD, fwd)
}

/** Spatial result of one source for one listener (gain excludes the 1/N factor). */
export interface OneListener {
  /** Linear gain: distance × occlusion, without 1/N. */
  gain: number
  /** Pan -1..1 (equal-power). */
  pan: number
  /** Lowpass cutoff in Hz. */
  lowpass: number
  /** Reverb send 0..1. */
  send: number
  /** Doppler pitch ratio. */
  doppler: number
  /** Source-listener distance in m. */
  dist: number
}

const sRight: Vec3 = { x: 0, y: 0, z: 0 }
const sUp: Vec3 = { x: 0, y: 0, z: 0 }
const sFwd: Vec3 = { x: 0, y: 0, z: 0 }

/**
 * Spatialize src for listener l into out. count (N) only selects the pan width
 * (SPLIT_PAN_WIDTH when N > 1); the 1/N gain factor is applied by the model, not here.
 * occlusion is the occlusion function's value for this listener (0..1).
 */
export function spatializeOne(l: Listener, src: SourcePos, count: number, occlusion: number,
  env: { absorptionScale: number }, out: OneListener): OneListener {
  const dx = src.pos.x - l.pos.x, dy = src.pos.y - l.pos.y, dz = src.pos.z - l.pos.z
  const d = Math.sqrt(dx * dx + dy * dy + dz * dz)
  out.dist = d
  if (d < 1e-6) {
    out.gain = 1
    out.pan = 0
    out.lowpass = OPEN_LOWPASS_HZ
    out.send = REVERB_SEND_BASE
    out.doppler = 1
    return out
  }
  listenerAxes(l.quat, sRight, sUp, sFwd)
  const x = (dx * sRight.x + dy * sRight.y + dz * sRight.z) / d
  const z = (dx * sFwd.x + dy * sFwd.y + dz * sFwd.z) / d
  const az = Math.atan2(x, z)
  out.pan = Math.sin(az) * (count > 1 ? SPLIT_PAN_WIDTH : 1)
  const rear = Math.max(0, -Math.cos(az))
  let lowpass = OPEN_LOWPASS_HZ - rear * (OPEN_LOWPASS_HZ - REAR_LOWPASS_HZ)
  lowpass *= clamp(1 - d / (AIR_ABSORPTION_M * env.absorptionScale), 0.3, 1)
  let gain = distanceGain(d, REF_DISTANCE, src.range ?? DEFAULT_RANGE)
  const o = clamp(Math.max(src.occlusion ?? 0, occlusion), 0, 1)
  if (o > 0) {
    lowpass = Math.max(OCCLUDED_MIN_HZ, lowpass * (1 - 0.95 * o))
    gain *= Math.pow(10, (OCCLUDED_GAIN_DB * o) / 20)
  }
  out.gain = gain
  out.lowpass = lowpass
  out.send = REVERB_SEND_BASE + REVERB_SEND_DIST * clamp(d / REVERB_SEND_DIST_M, 0, 1)
  out.doppler = dopplerRatio(l, src)
  return out
}

/** Doppler pitch ratio ((c + vl) / (c - vs))^DOPPLER_AMOUNT, clamped to DOPPLER_MIN..MAX; 1 for d < 1e-6. */
export function dopplerRatio(l: Listener, src: SourcePos): number {
  const ux = l.pos.x - src.pos.x, uy = l.pos.y - src.pos.y, uz = l.pos.z - src.pos.z
  const d = Math.sqrt(ux * ux + uy * uy + uz * uz)
  if (d < 1e-6) return 1
  const sv = src.vel, lv = l.vel
  const vs = sv ? (sv.x * ux + sv.y * uy + sv.z * uz) / d : 0
  const vl = lv ? -(lv.x * ux + lv.y * uy + lv.z * uz) / d : 0
  const ratio = Math.pow((SPEED_OF_SOUND + vl) / (SPEED_OF_SOUND - vs), DOPPLER_AMOUNT)
  return clamp(ratio, DOPPLER_MIN, DOPPLER_MAX)
}
