// Geometric acoustics from a handful of rays (docs/physics-sound.md): first-order image sources
// for the early reflections, a Sabine reverb time from the mean free path for the late tail,
// and a partial occlusion from three listener-to-source rays. Pure: the caller supplies the ray
// cast (Rapier, three.js, a game engine) and applies the numbers to audio nodes.

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

/** A ray hit: distance, point, surface normal, the surface's absorption α and a surface id. */
export interface RayHit {
  toi: number;
  point: Vec3;
  normal: Vec3;
  absorption: number;
  /** Stable id of the hit surface (a collider handle): a reflection is valid only off that surface. */
  id: number;
}

/** Casts a ray from `origin` along the unit vector `dir` up to `maxDist`; null when nothing is hit. */
export type RayCast = (origin: Vec3, dir: Vec3, maxDist: number) => RayHit | null;

/** A cast segment, for drawing: `hit` false means the ray escaped. */
export interface RaySegment {
  from: Vec3;
  to: Vec3;
  hit: boolean;
  kind: 'room' | 'reflection' | 'occlusion';
}

export const SPEED_OF_SOUND_M_S = 343;

export const v3 = (x: number, y: number, z: number): Vec3 => ({ x, y, z });
export const add = (a: Vec3, b: Vec3): Vec3 => v3(a.x + b.x, a.y + b.y, a.z + b.z);
export const sub = (a: Vec3, b: Vec3): Vec3 => v3(a.x - b.x, a.y - b.y, a.z - b.z);
export const scale = (a: Vec3, s: number): Vec3 => v3(a.x * s, a.y * s, a.z * s);
export const dot = (a: Vec3, b: Vec3): number => a.x * b.x + a.y * b.y + a.z * b.z;
export const length = (a: Vec3): number => Math.hypot(a.x, a.y, a.z);
export function normalize(a: Vec3): Vec3 {
  const l = length(a);
  return l > 0 ? scale(a, 1 / l) : v3(0, 0, 0);
}
export const cross = (a: Vec3, b: Vec3): Vec3 =>
  v3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);

/** The 6 axis directions and the 8 cube diagonals: 14 unit rays covering the sphere evenly enough. */
export const RAY_DIRECTIONS: readonly Vec3[] = (() => {
  const dirs: Vec3[] = [v3(1, 0, 0), v3(-1, 0, 0), v3(0, 1, 0), v3(0, -1, 0), v3(0, 0, 1), v3(0, 0, -1)];
  for (const x of [-1, 1]) for (const y of [-1, 1]) for (const z of [-1, 1]) dirs.push(normalize(v3(x, y, z)));
  return dirs;
})();

// --- early reflections -----------------------------------------------------------------------

export interface Reflection {
  /** Mirror image of the source across the reflecting surface: where the echo seems to come from. */
  image: Vec3;
  /** Where the sound bounces. */
  point: Vec3;
  /** Image-to-listener path length (m). */
  path: number;
  /** Arrival after the direct sound (s). */
  delay: number;
  /** Amplitude reflection factor √(1 − α); the 1/r loss comes from the panner at `image`. */
  gain: number;
  absorption: number;
  id: number;
}

export interface ReflectionOptions {
  max?: number;
  maxDist?: number;
  directions?: readonly Vec3[];
}

/**
 * First-order image sources: rays from the source find nearby surfaces; each surface (plane at the
 * hit point, its normal) mirrors the source; the bounce point is where the image-to-listener line
 * crosses the plane, and it counts only if both legs reach that same surface unobstructed.
 * Keeps the `max` shortest paths.
 */
export function earlyReflections(
  source: Vec3,
  listener: Vec3,
  cast: RayCast,
  o: ReflectionOptions = {},
  segments?: RaySegment[],
): Reflection[] {
  const max = o.max ?? 4;
  const maxDist = o.maxDist ?? 40;
  const direct = length(sub(listener, source));
  const seen = new Set<string>();
  const out: Reflection[] = [];
  for (const dir of o.directions ?? RAY_DIRECTIONS) {
    const hit = cast(source, dir, maxDist);
    if (!hit) continue;
    let n = normalize(hit.normal);
    if (dot(sub(source, hit.point), n) < 0) n = scale(n, -1);
    const planeKey = `${hit.id}:${n.x.toFixed(1)},${n.y.toFixed(1)},${n.z.toFixed(1)}`;
    if (seen.has(planeKey)) continue;
    seen.add(planeKey);
    const ds = dot(sub(source, hit.point), n);
    const dl = dot(sub(listener, hit.point), n);
    if (dl <= 0.01 || ds < 0) continue; // listener behind the surface
    const image = sub(source, scale(n, 2 * ds));
    const di = -ds;
    const t = di / (di - dl);
    const point = add(image, scale(sub(listener, image), t));
    if (!reaches(listener, point, hit.id, cast) || !reaches(source, point, hit.id, cast)) continue;
    const path = length(sub(listener, image));
    out.push({
      image,
      point,
      path,
      delay: Math.max(0, (path - direct) / SPEED_OF_SOUND_M_S),
      gain: Math.sqrt(Math.max(0, 1 - hit.absorption)),
      absorption: hit.absorption,
      id: hit.id,
    });
  }
  out.sort((a, b) => a.path - b.path);
  const kept = out.slice(0, max);
  if (segments) {
    for (const r of kept) {
      segments.push({ from: source, to: r.point, hit: true, kind: 'reflection' });
      segments.push({ from: r.point, to: listener, hit: true, kind: 'reflection' });
    }
  }
  return kept;
}

/** True when a ray from `from` to `point` first meets the surface `id` at (about) that point. */
function reaches(from: Vec3, point: Vec3, id: number, cast: RayCast): boolean {
  const d = length(sub(point, from));
  if (d < 1e-3) return true;
  const hit = cast(from, normalize(sub(point, from)), d + 0.1);
  if (!hit) return false;
  return hit.id === id && Math.abs(hit.toi - d) < Math.max(0.15, d * 0.05);
}

// --- late reverb -----------------------------------------------------------------------------

export interface RoomEstimate {
  /** Fraction of rays that escaped (no hit within `maxDist`): 1 is open air. */
  escape: number;
  /** Mean hit distance (m), the mean free path estimate. */
  meanFreePath: number;
  /** Mean α of the surfaces hit. */
  absorption: number;
  /** Reverb time (s), Sabine from the mean free path, escaped rays counted as α = 1. */
  rt60: number;
  /** Reverb send level 0..1: enclosure × reflectivity. */
  wet: number;
}

export interface RoomOptions {
  maxDist?: number;
  directions?: readonly Vec3[];
}

export const RT60_RANGE: readonly [number, number] = [0.1, 1.8];

/**
 * Sabine: RT60 = 0.161·V / (S·ᾱ). The mean free path of a room is l = 4V/S, so
 * RT60 = 0.161·l / (4·ᾱ) ≈ 0.04·l/ᾱ. A ray that escapes is a perfectly absorbing patch of wall
 * (open sky), so ᾱ_eff = (1 − e)·ᾱ_hits + e.
 */
export function estimateRoom(listener: Vec3, cast: RayCast, o: RoomOptions = {}, segments?: RaySegment[]): RoomEstimate {
  const maxDist = o.maxDist ?? 50;
  const dirs = o.directions ?? RAY_DIRECTIONS;
  let hits = 0;
  let sumToi = 0;
  let sumAbs = 0;
  for (const dir of dirs) {
    const hit = cast(listener, dir, maxDist);
    if (hit) {
      hits++;
      sumToi += hit.toi;
      sumAbs += hit.absorption;
    }
    segments?.push({ from: listener, to: hit ? hit.point : add(listener, scale(dir, Math.min(maxDist, 12))), hit: !!hit, kind: 'room' });
  }
  const escape = 1 - hits / dirs.length;
  const meanFreePath = hits ? sumToi / hits : maxDist;
  const absorption = hits ? sumAbs / hits : 1;
  const alpha = Math.max(0.01, (1 - escape) * absorption + escape);
  const rt60 = Math.min(RT60_RANGE[1], Math.max(RT60_RANGE[0], (0.161 * meanFreePath) / (4 * alpha)));
  const wet = Math.min(1, (1 - escape) ** 2 * Math.sqrt(1 - Math.min(1, absorption)));
  return { escape, meanFreePath, absorption, rt60, wet };
}

// --- occlusion -------------------------------------------------------------------------------

export interface Occlusion {
  /** Fraction of the probe rays blocked, 0..1. */
  blocked: number;
  /** Linear gain to apply: −10 dB when fully blocked (at strength 1). */
  gain: number;
  /** Low-pass cutoff (Hz): 20 kHz open, 1.2 kHz fully blocked (at strength 1), log-interpolated. */
  cutoff: number;
}

/** Gain of a fully blocked path at strength 1: −10 dB (a wall muffles, it does not mute). */
export const OCCLUDED_GAIN = 0.316;
/** Low-pass cutoff of a fully blocked path at strength 1. */
export const OCCLUDED_CUTOFF_HZ = 1200;
export const OPEN_CUTOFF_HZ = 20000;
/** Range of the occlusion strength: 0 turns occlusion off, 1 is the default, 2 doubles the loss in dB. */
export const OCCLUSION_STRENGTH_RANGE: readonly [number, number] = [0, 2];

/**
 * Three rays from the listener: to the source and to two points 0.35 m beside it (a soft edge
 * instead of an on/off switch when the source slides behind a wall corner). The last `endSkip`
 * metres are not tested: the surface the sound comes from never occludes itself.
 */
export function occlusion(source: Vec3, listener: Vec3, cast: RayCast, endSkip = 0.2, segments?: RaySegment[]): Occlusion {
  const toSource = sub(source, listener);
  const d = length(toSource);
  if (d <= endSkip + 1e-3) return { blocked: 0, gain: 1, cutoff: OPEN_CUTOFF_HZ };
  const fwd = scale(toSource, 1 / d);
  let side = cross(fwd, v3(0, 1, 0));
  if (length(side) < 1e-3) side = v3(1, 0, 0);
  side = normalize(side);
  let up = normalize(cross(side, fwd));
  if (up.y < 0) up = scale(up, -1);
  const targets = [source, add(source, scale(side, 0.35)), add(source, scale(up, 0.35))];
  let blocked = 0;
  for (const target of targets) {
    const seg = sub(target, listener);
    const len = length(seg);
    const hit = cast(listener, scale(seg, 1 / len), Math.max(0, len - endSkip));
    if (hit) blocked++;
    segments?.push({ from: listener, to: hit ? hit.point : target, hit: !!hit, kind: 'occlusion' });
  }
  return occlusionLevels(blocked / targets.length);
}

/**
 * Gain and cutoff for a blocked fraction. The loss is linear in dB: `strength · blocked` times
 * −10 dB, and the cutoff falls on a log axis by the same amount (1.2 kHz at 1, 72 Hz at 2).
 */
export function occlusionLevels(blocked: number, strength = 1): Occlusion {
  const b = Math.min(1, Math.max(0, blocked));
  const [lo, hi] = OCCLUSION_STRENGTH_RANGE;
  const x = b * Math.min(hi, Math.max(lo, Number.isFinite(strength) ? strength : 1));
  return {
    blocked: b,
    gain: OCCLUDED_GAIN ** x,
    cutoff: OPEN_CUTOFF_HZ * (OCCLUDED_CUTOFF_HZ / OPEN_CUTOFF_HZ) ** x,
  };
}

/** Air absorption as a low-pass: 20 kHz near, about 10 kHz at 50 m. */
export function airCutoff(distance: number): number {
  return OPEN_CUTOFF_HZ / (1 + Math.max(0, distance) / 50);
}
