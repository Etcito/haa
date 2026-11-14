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
  /** Share of the sideways and upward rays closed by a near surface (weight (4 m / distance)², at most 1). */
  enclosure: number;
  /** Reverb time (s), Sabine from the mean free path, escaped rays counted as α = 1. */
  rt60: number;
  /**
   * Amplitude of the diffuse field relative to the direct sound at the reference distance (2 m):
   * the reverb bus gain for an impulse response of unit energy. 0 in the open, up to `WET_MAX`.
   */
  wet: number;
}

export interface RoomOptions {
  maxDist?: number;
  directions?: readonly Vec3[];
}

export const RT60_RANGE: readonly [number, number] = [0.1, 1.8];
/** Largest diffuse-to-direct amplitude at 2 m (+6 dB): a bare concrete room is louder still, a game mix is not. */
export const WET_MAX = 2;
/** A surface nearer than this (m) closes its ray fully; a farther one sends discrete echoes more than a diffuse field. */
export const ENCLOSURE_NEAR_M = 4;

/**
 * Sabine: RT60 = 0.161·V / (S·ᾱ). The mean free path of a room is l = 4V/S, so
 * RT60 = 0.161·l / (4·ᾱ) ≈ 0.04·l/ᾱ. A ray that escapes is a perfectly absorbing patch of wall
 * (open sky), so ᾱ_eff = (1 − e)·ᾱ_hits + e.
 *
 * Level: the diffuse field of a room of constant R = S·ᾱ/(1 − ᾱ) has an energy 16π·r²/R times the
 * direct sound at r, so 64π/R times the direct sound at the 2 m reference (where the direct path
 * is at full level). The surface is taken from the mean free path as for a cube, S ≈ 13.5·l².
 * Sabine assumes a closed, diffuse room; with open sky the energy leaves after one or two bounces,
 * so the level is scaled by the enclosure³ (the sideways and upward rays closed by a near surface):
 * open ground and a lone far wall give no diffuse field (their echoes are the early reflections),
 * a corner of two walls a short faint one, a closed room the full Sabine level.
 */
export function estimateRoom(listener: Vec3, cast: RayCast, o: RoomOptions = {}, segments?: RaySegment[]): RoomEstimate {
  const maxDist = o.maxDist ?? 50;
  const dirs = o.directions ?? RAY_DIRECTIONS;
  let hits = 0;
  let sumToi = 0;
  let sumAbs = 0;
  let open = 0;
  let closed = 0;
  for (const dir of dirs) {
    const hit = cast(listener, dir, maxDist);
    if (hit) {
      hits++;
      sumToi += hit.toi;
      sumAbs += hit.absorption;
    }
    if (dir.y > -0.1) {
      open++;
      if (hit) closed += Math.min(1, (ENCLOSURE_NEAR_M / Math.max(hit.toi, 1e-3)) ** 2);
    }
    segments?.push({ from: listener, to: hit ? hit.point : add(listener, scale(dir, Math.min(maxDist, 12))), hit: !!hit, kind: 'room' });
  }
  const escape = 1 - hits / dirs.length;
  const meanFreePath = hits ? sumToi / hits : maxDist;
  const absorption = hits ? sumAbs / hits : 1;
  const enclosure = open ? closed / open : 0;
  const alpha = Math.min(0.999, Math.max(0.01, (1 - escape) * absorption + escape));
  const rt60 = Math.min(RT60_RANGE[1], Math.max(RT60_RANGE[0], (0.161 * meanFreePath) / (4 * alpha)));
  const surface = 13.5 * meanFreePath * meanFreePath;
  const roomConstant = (surface * alpha) / (1 - alpha);
  const energy = ((64 * Math.PI) / roomConstant) * enclosure ** 3;
  const wet = hits ? Math.min(WET_MAX, Math.sqrt(energy)) : 0;
  return { escape, meanFreePath, absorption, enclosure, rt60, wet };
}

// --- occlusion -------------------------------------------------------------------------------

export interface Occlusion {
  /** Fraction of the probe rays blocked, 0..1. */
  blocked: number;
  /** Broadband linear gain: −7 dB when fully blocked (at strength 1). */
  gain: number;
  /** Gains (dB, ≤ 0) of the two high shelves at `OCCLUSION_SHELF_HZ`: −4 and −5 dB fully blocked. */
  shelves: readonly [number, number];
}

/**
 * A blocked path at strength 1 (a wall muffles, it does not mute). Diffraction around an edge loses
 * more the shorter the wavelength, about 3 dB per octave over the speech band (Maekawa), so the
 * loss is a broadband −7 dB plus two gentle high shelves: about −7 dB below 300 Hz, −11 dB at
 * 1.2 kHz and −16 dB above 4 kHz. A bright sound behind a wall stays recognisable.
 */
export const OCCLUDED_GAIN_DB = -7;
export const OCCLUSION_SHELF_HZ: readonly [number, number] = [600, 2500];
export const OCCLUDED_SHELF_DB: readonly [number, number] = [-4, -5];
/** Broadband gain of a fully blocked path at strength 1 (−7 dB). */
export const OCCLUDED_GAIN = 10 ** (OCCLUDED_GAIN_DB / 20);
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
  if (d <= endSkip + 1e-3) return occlusionLevels(0);
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
 * Gain and shelves for a blocked fraction. Every loss is linear in dB with `strength · blocked`:
 * fully blocked at strength 1 is −7 dB broadband and −4 / −5 dB shelves (−16 dB in the highs),
 * strength 2 doubles every figure.
 */
export function occlusionLevels(blocked: number, strength = 1): Occlusion {
  const b = Math.min(1, Math.max(0, blocked));
  const [lo, hi] = OCCLUSION_STRENGTH_RANGE;
  const x = b * Math.min(hi, Math.max(lo, Number.isFinite(strength) ? strength : 1));
  return {
    blocked: b,
    gain: 10 ** ((OCCLUDED_GAIN_DB * x) / 20),
    shelves: x > 0 ? [OCCLUDED_SHELF_DB[0] * x, OCCLUDED_SHELF_DB[1] * x] : [0, 0],
  };
}

/** Air absorption as a low-pass: 20 kHz near, about 10 kHz at 50 m. */
export function airCutoff(distance: number): number {
  return OPEN_CUTOFF_HZ / (1 + Math.max(0, distance) / 50);
}
