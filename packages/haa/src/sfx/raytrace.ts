// Geometric acoustics by ray tracing (docs/physics-sound.md, "Acoustic model"). Pure: the caller
// supplies the ray cast (Rapier, three.js, a game engine). Three products, all continuous in the
// listener and source positions:
//
// - `traceListener`: N rays from the listener, each followed over K bounces (specular plus a fixed
//   share of scattering, deterministic so the same place gives the same answer). It measures the
//   listener's space: open share, mean free path, energy decay per bounce (absorption AND rays
//   leaving through openings), hence the reverb time; and keeps the bounce points.
// - `voiceReverb`: next-event estimation from those bounce points to one source. Each bounce point
//   that sees the source adds the energy a Lambert surface sends back to the listener along that
//   ray. The sum is the diffuse field of THIS source at THIS listener, with its arrival direction,
//   its decay (the decay of the paths that carry it) and its gap after the direct sound. A source
//   in a hall heard from outside gets its tail through the door, from the door's direction.
// - `directPath`: the direct sound. A disk of rays around the source gives the visible share;
//   when it is hidden, the shortest path around the obstacles (one or two bends, through the points
//   the listener and the source can see) gives the excess path length and the direction it arrives
//   from, and `diffractionLevels` turns the excess into a frequency-dependent loss.
import type { RayCast, RaySegment, Vec3 } from './acoustics.ts';
import { add, cross, dot, length, normalize, scale, sub, v3, SPEED_OF_SOUND_M_S } from './acoustics.ts';

/** `n` unit vectors spread evenly over the sphere (Fibonacci lattice). */
export function sphereDirections(n: number): Vec3[] {
  const out: Vec3[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < n; i++) {
    const y = 1 - (2 * (i + 0.5)) / n;
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    const a = i * golden;
    out.push(v3(r * Math.cos(a), y, r * Math.sin(a)));
  }
  return out;
}

/** Deterministic hash in [0, 1). */
function hash(a: number, b: number): number {
  const s = Math.sin(a * 12.9898 + b * 78.233) * 43758.5453;
  return s - Math.floor(s);
}

/** A unit vector around `n`, cosine-weighted, from two numbers in [0, 1). */
function cosineAround(n: Vec3, u1: number, u2: number): Vec3 {
  const t = Math.abs(n.y) < 0.9 ? v3(0, 1, 0) : v3(1, 0, 0);
  const tx = normalize(cross(t, n));
  const ty = cross(n, tx);
  const r = Math.sqrt(u1);
  const phi = 2 * Math.PI * u2;
  const z = Math.sqrt(Math.max(0, 1 - u1));
  return normalize(add(add(scale(tx, r * Math.cos(phi)), scale(ty, r * Math.sin(phi))), scale(n, z)));
}

// --- listener trace --------------------------------------------------------------------------

export interface TraceNode {
  point: Vec3;
  /** Surface normal, turned toward the side the ray came from. */
  normal: Vec3;
  absorption: number;
  /** Path length from the listener to this point (m). */
  path: number;
  /** Energy that reaches this point (product of 1 − α of the earlier bounces). */
  throughput: number;
  /** 1 for the first bounce. */
  order: number;
}

export interface TraceRay {
  dir: Vec3;
  nodes: TraceNode[];
  /** Order at which the ray left the scene (0: before its first hit), or -1 if it never did. */
  escapedAt: number;
}

export interface TraceOptions {
  /** Rays from the listener. Default 256. */
  rays?: number;
  /** Bounces followed per ray. Default 6. */
  bounces?: number;
  /** A ray that hits nothing within this (m) has left. Default 60. */
  maxDist?: number;
  /** Share of each reflection that scatters (0 mirror, 1 Lambert). Default 0.35. */
  scattering?: number;
}

export interface SpaceEstimate {
  /** Share of the rays from the listener that hit nothing: 1 is open ground with no walls. */
  escape: number;
  /** Share of the rays that left the scene within the followed bounces (openings, open sky). */
  leak: number;
  /** Mean distance between bounces (m). */
  meanFreePath: number;
  /** Energy kept per bounce, absorption and openings together. */
  decayPerBounce: number;
  /** Reverb time (s) of the space around the listener, from the decay per bounce and the mean free path. */
  rt60: number;
  /** Volume estimate (m³): mean of (4π/3)·r³ over the first hits, the open share excluded. */
  volume: number;
  /** Mean α of the first hits. */
  absorption: number;
}

export interface ListenerTrace {
  origin: Vec3;
  rays: TraceRay[];
  /** Points the listener sees directly (along the rays' free first segments), for the direct path search. */
  candidates: Vec3[];
  space: SpaceEstimate;
}

/** Reverb times are clamped to this range (s). */
export const TRACE_RT60_RANGE: readonly [number, number] = [0.1, 4.5];
const CANDIDATE_STEPS = [0.6, 1.2, 2, 3, 4.5, 6.5, 9, 13, 18];

export function traceListener(listener: Vec3, cast: RayCast, o: TraceOptions = {}, segments?: RaySegment[]): ListenerTrace {
  const n = o.rays ?? 256;
  const bounces = o.bounces ?? 6;
  const maxDist = o.maxDist ?? 60;
  const scatter = o.scattering ?? 0.35;
  const dirs = sphereDirections(n);
  const rays: TraceRay[] = [];
  const candidates: Vec3[] = [];
  const energyAt = new Float64Array(bounces + 1);
  let escaped0 = 0;
  let leaked = 0;
  let segSum = 0;
  let segCount = 0;
  let volSum = 0;
  let absSum = 0;
  let hits0 = 0;
  for (let i = 0; i < n; i++) {
    const ray: TraceRay = { dir: dirs[i], nodes: [], escapedAt: -1 };
    let origin = listener;
    let dir = dirs[i];
    let path = 0;
    let throughput = 1;
    for (let k = 1; k <= bounces; k++) {
      const hit = cast(origin, dir, maxDist);
      if (k === 1) {
        const free = hit ? hit.toi - 0.3 : 20;
        for (const s of CANDIDATE_STEPS) if (s < free) candidates.push(add(listener, scale(dir, s)));
        if (hit && hit.toi > 0.9) candidates.push(add(listener, scale(dir, hit.toi - 0.3)));
      }
      if (!hit) {
        ray.escapedAt = k - 1;
        if (k === 1) escaped0++;
        leaked++;
        if (segments && k === 1 && i % 4 === 0) segments.push({ from: origin, to: add(origin, scale(dir, 12)), hit: false, kind: 'room' });
        break;
      }
      // Drawn: the first leg of every 4th ray (all of them hide the scene).
      if (segments && k === 1 && i % 4 === 0) segments.push({ from: origin, to: hit.point, hit: true, kind: 'room' });
      let nrm = normalize(hit.normal);
      if (dot(nrm, dir) > 0) nrm = scale(nrm, -1);
      path += hit.toi;
      if (k === 1) {
        hits0++;
        volSum += (4 / 3) * Math.PI * hit.toi ** 3;
        absSum += hit.absorption;
      } else {
        segSum += hit.toi;
        segCount++;
      }
      energyAt[k] += throughput;
      ray.nodes.push({ point: hit.point, normal: nrm, absorption: hit.absorption, path, throughput, order: k });
      throughput *= Math.max(0, 1 - hit.absorption);
      const spec = sub(dir, scale(nrm, 2 * dot(dir, nrm)));
      const diffuse = cosineAround(nrm, hash(i, k), hash(k + 17, i + 3));
      dir = normalize(add(scale(spec, 1 - scatter), scale(diffuse, scatter)));
      if (dot(dir, nrm) <= 0.01) dir = diffuse;
      origin = add(hit.point, scale(nrm, 1e-3));
    }
    rays.push(ray);
  }
  const escape = escaped0 / n;
  const meanFreePath = segCount ? segSum / segCount : hits0 ? Math.cbrt(volSum / hits0 / ((4 / 3) * Math.PI)) : maxDist;
  const first = energyAt[1];
  const last = energyAt[bounces];
  const decayPerBounce = first > 0 && bounces > 1 ? Math.min(0.995, (Math.max(last, 1e-9) / first) ** (1 / (bounces - 1))) : 0;
  return {
    origin: listener,
    rays,
    candidates,
    space: {
      escape,
      leak: leaked / n,
      meanFreePath,
      decayPerBounce,
      rt60: rt60From(decayPerBounce, meanFreePath),
      volume: hits0 ? (volSum / hits0) * (1 - escape) : 0,
      absorption: hits0 ? absSum / hits0 : 1,
    },
  };
}

/** Reverb time from the energy kept per bounce and the mean free path: 60 dB of decay at c·t/l bounces. */
export function rt60From(decayPerBounce: number, meanFreePath: number): number {
  const [lo, hi] = TRACE_RT60_RANGE;
  if (!(decayPerBounce > 0)) return lo;
  const dbPerBounce = -10 * Math.log10(decayPerBounce);
  if (dbPerBounce <= 1e-6) return hi;
  return Math.min(hi, Math.max(lo, (60 * meanFreePath) / (SPEED_OF_SOUND_M_S * dbPerBounce)));
}

// --- one source's diffuse field --------------------------------------------------------------

export interface VoiceReverb {
  /** Energy of the diffuse field relative to the direct sound at `ref` metres (≥ 0). */
  energy: number;
  /** Reverb time (s) of the paths that carry it. */
  rt60: number;
  /** Energy-weighted arrival direction from the listener; its length is how directional it is (0 diffuse, 1 one direction). */
  direction: Vec3;
  /** Gap between the direct sound and the first diffuse energy (s). */
  predelay: number;
  /** Energy-weighted absorption of the surfaces it comes from. */
  absorption: number;
  /** Bounce points that saw the source. */
  hits: number;
}

export interface VoiceReverbOptions {
  /** Use every `stride`-th ray of the trace (cost). Default 2. */
  stride?: number;
  /** Bounces used (≤ the trace's). Default all. */
  maxOrder?: number;
  /** Reference distance of the direct sound (m). Default 2. */
  ref?: number;
  /** Energy kept per bounce is clamped below this for the tail beyond the traced bounces. Default 0.985. */
  maxDecay?: number;
  /**
   * Energy arriving less than this (s) after the direct sound is part of the direct sound (the
   * ground right under a footstep), not of the diffuse field. Default 0.005.
   */
  minDelay?: number;
  /**
   * Share of a first bounce that scatters: the specular rest is the image-source taps' job, so the
   * first order adds only this share to the diffuse field. Later orders are fully diffuse. Default 0.35.
   */
  firstOrderShare?: number;
}

export function voiceReverb(trace: ListenerTrace, source: Vec3, cast: RayCast, o: VoiceReverbOptions = {}): VoiceReverb {
  const stride = Math.max(1, o.stride ?? 2);
  const ref = o.ref ?? 2;
  const maxOrder = o.maxOrder ?? 99;
  const minDelay = o.minDelay ?? 0.005;
  const firstShare = o.firstOrderShare ?? 0.35;
  const arrivals: Array<[number, number]> = [];
  const used = Math.ceil(trace.rays.length / stride);
  const w = (4 * Math.PI) / used;
  const direct = length(sub(source, trace.origin));
  const byOrder: number[] = [];
  let total = 0;
  let dx = 0;
  let dy = 0;
  let dz = 0;
  let absW = 0;
  let hits = 0;
  // Decay of the contributing rays: their throughput at the first and last traced bounce, weighted.
  let wLogKept = 0;
  let wEsc = 0;
  let wSeg = 0;
  let wSegN = 0;
  let wBounces = 0;
  let maxK = 0;
  for (let i = 0; i < trace.rays.length; i += stride) {
    const ray = trace.rays[i];
    let rayE = 0;
    for (const node of ray.nodes) {
      if (node.order > maxOrder) break;
      const toS = sub(source, node.point);
      const r = length(toS);
      if (r < 0.05) continue;
      const d = scale(toS, 1 / r);
      const cos = dot(node.normal, d);
      if (cos <= 0) continue;
      const t = (node.path + r - direct) / SPEED_OF_SOUND_M_S;
      if (t < minDelay) continue;
      const from = add(node.point, scale(node.normal, 0.02));
      if (r > 0.25 && cast(from, d, r - 0.2)) continue;
      const share = node.order === 1 ? firstShare : 1;
      const c = (share * w * node.throughput * (1 - node.absorption) * cos * ref * ref) / (Math.PI * Math.max(r, 0.5) ** 2);
      if (!(c > 0)) continue;
      hits++;
      rayE += c;
      total += c;
      byOrder[node.order] = (byOrder[node.order] ?? 0) + c;
      maxK = Math.max(maxK, node.order);
      dx += c * ray.dir.x;
      dy += c * ray.dir.y;
      dz += c * ray.dir.z;
      absW += c * node.absorption;
      arrivals.push([t, c]);
    }
    if (rayE > 0 && ray.nodes.length >= 2) {
      const a = ray.nodes[0];
      const z = ray.nodes[ray.nodes.length - 1];
      // Per-bounce rates of the paths that carry this source: absorption, and rays leaving.
      wLogKept += rayE * Math.log(Math.max(1e-9, z.throughput * (1 - z.absorption)));
      wEsc += ray.escapedAt >= 0 ? rayE : 0;
      wSeg += rayE * ((z.path - a.path) / (ray.nodes.length - 1));
      wSegN += rayE;
      wBounces += rayE * ray.nodes.length;
    }
  }
  if (!(total > 0)) {
    return { energy: 0, rt60: trace.space.rt60, direction: v3(0, 0, 0), predelay: 0, absorption: trace.space.absorption, hits: 0 };
  }
  let decay = trace.space.decayPerBounce;
  if (wBounces > 0) {
    const kept = Math.exp(wLogKept / wBounces);
    const leave = Math.min(1, wEsc / wBounces);
    decay = kept * (1 - leave);
  }
  if (!(decay > 0)) decay = trace.space.decayPerBounce;
  decay = Math.min(o.maxDecay ?? 0.985, decay);
  const mfp = wSegN > 0 ? wSeg / wSegN : trace.space.meanFreePath;
  // Energy beyond the traced bounces: the last orders carried on at the measured decay.
  const last = byOrder[maxK] ?? 0;
  const prev = (byOrder[maxK - 1] ?? last / Math.max(decay, 1e-3)) * decay;
  const tailRef = (last + prev) / 2;
  const tail = (tailRef * decay) / Math.max(1e-3, 1 - decay);
  // The decay rate gives the reverb time of a closed space; where the energy is only a few early
  // bounces (a facade outdoors) it overstates the tail. The centre time of the arrivals (the tail
  // counted as an exponential after the last traced arrival) bounds it: an exponential that starts
  // at t0 has its centre at t0 + RT60 / 13.8.
  const rateRt = rt60From(decay, mfp);
  let sumT = 0;
  let t0 = Infinity;
  let tLast = 0;
  for (const [t, c] of arrivals) {
    sumT += t * c;
    if (t < t0) t0 = t;
    if (t > tLast) tLast = t;
  }
  const centre = (sumT + tail * (tLast + rateRt / 13.8)) / (total + tail);
  const centreRt = 13.8 * Math.max(0, centre - t0);
  return {
    energy: total + tail,
    rt60: Math.max(TRACE_RT60_RANGE[0], Math.min(rateRt, centreRt)),
    direction: v3(dx / total, dy / total, dz / total),
    predelay: Math.max(0, Math.min(0.08, onsetTime(arrivals, 0.15 * total))),
    absorption: absW / total,
    hits,
  };
}

/** Time at which the arrivals (time, energy) have delivered the given energy. */
function onsetTime(arrivals: Array<[number, number]>, energy: number): number {
  arrivals.sort((a, b) => a[0] - b[0]);
  let acc = 0;
  for (const [t, c] of arrivals) {
    acc += c;
    if (acc >= energy) return t;
  }
  return arrivals.length ? arrivals[arrivals.length - 1][0] : 0;
}

// --- direct path -----------------------------------------------------------------------------

export interface DirectPath {
  /** Share of the probe rays (a disk around the source) that reach it unobstructed. */
  visible: number;
  /** Extra length of the shortest path around the obstacles (m); 0 when visible. */
  excess: number;
  /** First bend of that path (the edge or opening the sound arrives from), or the source itself. */
  via: Vec3;
  /** Length of that path (m). */
  length: number;
  /** False when no path around the obstacles was found (the sound goes through the walls). */
  found: boolean;
}

export interface DirectPathOptions {
  /** Radius of the probed disk (m): the source's size, at least 0.3. */
  extent?: number;
  /** The last metres of each probe are not tested (the surface the sound comes from). Default 0.2. */
  endSkip?: number;
  /** Rays from the source for paths with two bends. Default 48. */
  sourceRays?: number;
}

/** Disk offsets (unit radius) of the visibility probes: the centre, 6 at half radius, 8 at full radius. */
const DISK: ReadonlyArray<[number, number]> = (() => {
  const out: Array<[number, number]> = [[0, 0]];
  for (let i = 0; i < 6; i++) out.push([0.5 * Math.cos((i * Math.PI) / 3), 0.5 * Math.sin((i * Math.PI) / 3)]);
  for (let i = 0; i < 8; i++) out.push([Math.cos(((i + 0.5) * Math.PI) / 4), Math.sin(((i + 0.5) * Math.PI) / 4)]);
  return out;
})();

function clear(from: Vec3, to: Vec3, cast: RayCast, endSkip: number): boolean {
  const seg = sub(to, from);
  const len = length(seg);
  if (len <= endSkip + 1e-3) return true;
  return !cast(from, scale(seg, 1 / len), len - endSkip);
}

const BEND_PLANES = [0.1, 0.3, 0.5, 0.7, 0.9];
const BEND_DIRECTIONS = 12;
const BEND_STEPS = [0.15, 0.3, 0.6, 1.2, 2.4, 4.8, 9.6];

/** Shortest one-bend path around the obstacles between `listener` and `source` (see `directPath`). */
function bendAround(source: Vec3, listener: Vec3, d: number, fwd: Vec3, side: Vec3, up: Vec3, cast: RayCast, endSkip: number): { via: Vec3; len: number } | null {
  let best: { via: Vec3; len: number } | null = null;
  for (const f of BEND_PLANES) {
    const centre = add(listener, scale(fwd, d * f));
    for (let j = 0; j < BEND_DIRECTIONS; j++) {
      const a = (j * 2 * Math.PI) / BEND_DIRECTIONS;
      const u = add(scale(side, Math.cos(a)), scale(up, Math.sin(a)));
      const at = (rho: number): Vec3 => add(centre, scale(u, rho));
      const ok = (rho: number): boolean => {
        const pt = at(rho);
        return clear(listener, pt, cast, 0) && clear(pt, source, cast, endSkip);
      };
      let lo = 0;
      let hi = -1;
      for (const rho of BEND_STEPS) {
        // Every offset from `lo` on gives a longer path than the best one found: it cannot win.
        if (best && Math.hypot(d * f, lo) + Math.hypot(d * (1 - f), lo) >= best.len) break;
        if (ok(rho)) {
          hi = rho;
          break;
        }
        lo = rho;
      }
      if (hi < 0) continue;
      for (let k = 0; k < 5; k++) {
        const m = (lo + hi) / 2;
        if (ok(m)) hi = m;
        else lo = m;
      }
      const via = at(hi);
      const len = length(sub(via, listener)) + length(sub(source, via));
      if (!best || len < best.len) best = { via, len };
    }
  }
  return best;
}

export function directPath(
  source: Vec3,
  listener: Vec3,
  trace: ListenerTrace | null,
  cast: RayCast,
  o: DirectPathOptions = {},
  segments?: RaySegment[],
): DirectPath {
  const endSkip = o.endSkip ?? 0.2;
  const toS = sub(source, listener);
  const d = length(toS);
  const none: DirectPath = { visible: 1, excess: 0, via: source, length: d, found: true };
  if (d <= endSkip + 1e-3) return none;
  const fwd = scale(toS, 1 / d);
  let side = cross(fwd, v3(0, 1, 0));
  if (length(side) < 1e-3) side = v3(1, 0, 0);
  side = normalize(side);
  const up = normalize(cross(side, fwd));
  const radius = Math.max(0.3, o.extent ?? 0);
  let seen = 0;
  let probes = 0;
  for (const [a, b] of DISK) {
    const target = add(source, add(scale(side, a * radius), scale(up, b * radius)));
    // A probe point inside the floor or a wall the source lies against is not part of the source.
    if ((a !== 0 || b !== 0) && !clear(source, target, cast, 0)) continue;
    probes++;
    const ok = clear(listener, target, cast, endSkip);
    if (ok) seen++;
    segments?.push({ from: listener, to: target, hit: !ok, kind: 'occlusion' });
  }
  const visible = seen / Math.max(1, probes);
  if (visible >= 1) return none;
  // One bend: in planes across the straight line (at 10..90 % of it), for each of 12 directions,
  // the smallest offset whose point both ends see, found by bisection. The offset of a given
  // direction changes continuously as the source moves, so the excess path (and the loss) glides
  // instead of hopping between sample points.
  let best: { via: Vec3; len: number; mid?: Vec3 } | null = bendAround(source, listener, d, fwd, side, up, cast, endSkip);
  if (!trace) {
    return best
      ? { visible, excess: Math.max(0, best.len - d), via: best.via, length: best.len, found: true }
      : { visible, excess: 0, via: source, length: d, found: visible > 0 };
  }
  const one = trace.candidates
    .map((q) => ({ q, len: length(sub(q, listener)) + length(sub(source, q)) }))
    .filter((c) => c.len < d + 40)
    .sort((a, b) => a.len - b.len);
  // No bend across the line found (a source deep in a room, seen through its door): the points the
  // listener sees, shortest first; every 8th, then the 7 before the first that passes.
  if (!best) {
    let hitAt = -1;
    for (let i = 7; i < one.length + 7; i += 8) {
      const k = Math.min(i, one.length - 1);
      if (clear(one[k].q, source, cast, endSkip)) {
        hitAt = k;
        break;
      }
    }
    for (let i = Math.max(0, hitAt - 7); hitAt >= 0 && i <= hitAt; i++) {
      if (i === hitAt || clear(one[i].q, source, cast, endSkip)) {
        best = { via: one[i].q, len: one[i].len };
        break;
      }
    }
  }
  // Two bends: through a point the listener sees and a point the source sees.
  const sourceSide: Array<{ r: Vec3; len: number }> = [];
  for (const dir of sphereDirections(o.sourceRays ?? 48)) {
    const hit = cast(source, dir, 30);
    const free = hit ? hit.toi - 0.3 : 20;
    for (const s of CANDIDATE_STEPS) {
      if (s >= free) break;
      const r = add(source, scale(dir, s));
      sourceSide.push({ r, len: s + length(sub(listener, r)) });
    }
  }
  sourceSide.sort((a, b) => a.len - b.len);
  const qs = one.slice(0, 48);
  const rs = sourceSide.slice(0, 48);
  const pairs: Array<{ q: Vec3; r: Vec3; len: number }> = [];
  for (const q of qs) {
    const lq = length(sub(q.q, listener));
    for (const r of rs) {
      const len = lq + length(sub(r.r, q.q)) + length(sub(source, r.r));
      if (!best || len < best.len) pairs.push({ q: q.q, r: r.r, len });
    }
  }
  pairs.sort((a, b) => a.len - b.len);
  for (let i = 0; i < Math.min(200, pairs.length); i++) {
    if (best && pairs[i].len >= best.len) break;
    if (clear(pairs[i].q, pairs[i].r, cast, 0)) {
      best = { via: pairs[i].q, mid: pairs[i].r, len: pairs[i].len };
      break;
    }
  }
  if (!best) return { visible, excess: 0, via: source, length: d, found: visible > 0 };
  if (segments) {
    segments.push({ from: listener, to: best.via, hit: false, kind: 'path' });
    if (best.mid) {
      segments.push({ from: best.via, to: best.mid, hit: false, kind: 'path' });
      segments.push({ from: best.mid, to: source, hit: false, kind: 'path' });
    } else segments.push({ from: best.via, to: source, hit: false, kind: 'path' });
  }
  return { visible, excess: Math.max(0, best.len - d), via: best.via, length: best.len, found: true };
}

// --- diffraction loss ------------------------------------------------------------------------

/** Frequencies (Hz) at which the loss is evaluated: broadband below the first shelf, then two shelves. */
export const DIFFRACTION_BANDS_HZ: readonly [number, number, number] = [250, 1000, 4000];
/** Scale of the edge-diffraction loss (1 = Kurze-Anderson; a game mix keeps hidden sounds present). */
export const DIFFRACTION_SCALE = 0.65;
/** Loss (dB, before scaling) of a sound with no path around the walls: heard through them. */
export const TRANSMISSION_LOSS_DB: readonly [number, number, number] = [28, 38, 50];
const MAX_LOSS_DB = 45;

/** Edge diffraction loss (dB, ≥ 0) at frequency f for an excess path δ (m): Kurze-Anderson from the Fresnel number. */
export function edgeLossDb(excess: number, hz: number): number {
  const fresnel = (2 * Math.max(0, excess) * hz) / SPEED_OF_SOUND_M_S;
  const x = Math.sqrt(2 * Math.PI * fresnel);
  if (x < 1e-4) return 5;
  return 5 + 20 * Math.log10(x / Math.tanh(x));
}

export interface DirectLevels {
  /** Blocked share (1 − visible). */
  blocked: number;
  /** Loss (dB, ≤ 0) at the three `DIFFRACTION_BANDS_HZ`. */
  bandsDb: [number, number, number];
}

/**
 * Loss of the direct path. A visible share v plays at full level; the hidden rest arrives around
 * the edge with the loss of its excess path (5 dB at the shadow boundary, more and more in the
 * highs as the path bends further). Energy-blended, so the level glides as the source slides
 * behind a wall. `strength` (0..2) scales every dB figure; no path at all is the wall's transmission.
 */
export function diffractionLevels(path: Pick<DirectPath, 'visible' | 'excess' | 'found'>, strength = 1): DirectLevels {
  const s = Math.min(2, Math.max(0, Number.isFinite(strength) ? strength : 1)) * DIFFRACTION_SCALE;
  const v = Math.min(1, Math.max(0, path.visible));
  const bandsDb = DIFFRACTION_BANDS_HZ.map((hz, i) => {
    const shadow = path.found || v > 0 ? edgeLossDb(path.excess, hz) : TRANSMISSION_LOSS_DB[i];
    const hidden = 10 ** (-Math.min(MAX_LOSS_DB, shadow * s) / 10);
    return 10 * Math.log10(v + (1 - v) * hidden);
  }) as [number, number, number];
  return { blocked: 1 - v, bandsDb };
}
