// The Rust/wasm acoustic tracer (crates/haa-trace, `pkg/haa-trace.wasm`) and its geometry input.
// No three.js: the level comes from a geometry provider as flat records (`SfxGeometry`), today
// read from Rapier's fixed colliders (`geometryFromRapier`), later from RAA's level.
import type { RayCast, RayHit, Reflection, Vec3 } from './acoustics.ts';
import { v3 } from './acoustics.ts';
import type { DirectPath, ListenerTrace, VoiceReverb } from './raytrace.ts';
import type { AcousticTracer, TracerListenerOptions, TracerVoiceOptions } from './tracer.ts';
import { REFLECTION_OPTIONS, SOURCE_DIRS } from './tracer.ts';

/** Floats per box: centre (3), half extents (3), rotation quaternion x y z w (4), absorption, surface id. */
export const BOX_STRIDE = 12;
/** Floats per triangle: three vertices (9), absorption, surface id. */
export const TRI_STRIDE = 11;

/**
 * The static level as the acoustic tracer reads it: oriented boxes and triangles, each with the
 * Sabine α of its material and a stable surface id (an image-source tap follows its surface).
 */
export interface SfxGeometry {
  boxes: Float32Array;
  triangles: Float32Array;
  /**
   * The host id of each surface id (a record's id is an index into it): Rapier's collider handles
   * are bit-packed floats that an f32 record cannot hold. Without it, hits report the record id.
   */
  ids?: number[];
}

/** Structural view of the Rapier 0.21 API used here (rapier3d-compat or the deterministic build). */
interface RShape {
  type: number;
  halfExtents?: Vec3;
  radius?: number;
  halfHeight?: number;
  vertices?: Float32Array;
  indices?: Uint32Array;
}
interface RColliderLike {
  handle: number;
  shape: RShape;
  translation(): Vec3;
  rotation(): { x: number; y: number; z: number; w: number };
}
interface RWorldLike {
  forEachCollider(f: (c: RColliderLike) => void): void;
}

/**
 * Reads the level from a Rapier world: every collider `include` accepts (the fixed ones, as the
 * engine's ray cast does), cuboids as boxes, triangle meshes as triangles, and balls, cylinders,
 * capsules and cones as the box around them (small objects; the level is boxes and meshes).
 */
export function geometryFromRapier(
  world: RWorldLike,
  absorptionOf: (c: RColliderLike) => number,
  include: (c: RColliderLike) => boolean,
): SfxGeometry {
  const boxes: number[] = [];
  const tris: number[] = [];
  const ids: number[] = [];
  world.forEachCollider((c) => {
    if (!include(c)) return;
    const a = absorptionOf(c);
    const id = ids.length;
    ids.push(c.handle);
    const t = c.translation();
    const q = c.rotation();
    const s = c.shape;
    if (s.vertices && s.indices) {
      const v = s.vertices;
      const rot = (i: number): number[] => rotate(q, v[i * 3], v[i * 3 + 1], v[i * 3 + 2]).map((x, k) => x + [t.x, t.y, t.z][k]);
      for (let i = 0; i + 2 < s.indices.length; i += 3) tris.push(...rot(s.indices[i]), ...rot(s.indices[i + 1]), ...rot(s.indices[i + 2]), a, id);
      return;
    }
    let h: Vec3 | null = s.halfExtents ?? null;
    if (!h && s.radius !== undefined) {
      const r = s.radius;
      const hh = (s.halfHeight ?? 0) + (s.halfHeight !== undefined && s.type === 2 ? r : 0);
      h = s.halfHeight !== undefined ? v3(r, s.type === 2 ? hh : s.halfHeight, r) : v3(r, r, r);
    }
    if (!h) return;
    boxes.push(t.x, t.y, t.z, h.x, h.y, h.z, q.x, q.y, q.z, q.w, a, id);
  });
  return { boxes: new Float32Array(boxes), triangles: new Float32Array(tris), ids };
}

function rotate(q: { x: number; y: number; z: number; w: number }, x: number, y: number, z: number): number[] {
  // v' = v + 2w(u × v) + 2u × (u × v)
  const cx = q.y * z - q.z * y;
  const cy = q.z * x - q.x * z;
  const cz = q.x * y - q.y * x;
  return [
    x + 2 * (q.w * cx + q.y * cz - q.z * cy),
    y + 2 * (q.w * cy + q.z * cx - q.x * cz),
    z + 2 * (q.w * cz + q.x * cy - q.y * cx),
  ];
}

interface TraceExports {
  memory: WebAssembly.Memory;
  trace_alloc(floats: number): number;
  trace_out(): number;
  trace_scene(boxes: number, tris: number): number;
  trace_casts(): number;
  trace_cast(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number): number;
  trace_cast_batch(n: number): number;
  trace_listener(lx: number, ly: number, lz: number, rays: number, bounces: number, maxDist: number, scatter: number): void;
  trace_listener_rays(every: number): number;
  trace_voice_reverb(sx: number, sy: number, sz: number, stride: number, maxOrder: number, ref: number, maxDecay: number, minDelay: number, firstShare: number): void;
  trace_direct_path(sx: number, sy: number, sz: number, lx: number, ly: number, lz: number, extent: number, endSkip: number, sourceRays: number, useTrace: number): void;
  trace_reflections(sx: number, sy: number, sz: number, lx: number, ly: number, lz: number, dirs: number, max: number, maxDist: number, soft: number, minDelay: number, scattering: number): number;
}

export interface WasmTracer extends AcousticTracer {
  /** A synchronous ray cast against the same BVH (rain exposure, anything else that casts). */
  readonly cast: RayCast;
  setGeometry(g: SfxGeometry): void;
  /** Ray casts since the geometry was set. */
  casts(): number;
  /** BVH nodes of the current geometry. */
  readonly nodes: number;
  /** Raw exports (benchmarks). */
  readonly exports: TraceExports;
}

/** Instantiates `haa-trace.wasm` (no imports) over `geometry`. Works on the main thread and in workers. */
export function createWasmTracer(module: WebAssembly.Module, geometry: SfxGeometry): WasmTracer {
  const x = new WebAssembly.Instance(module, {}).exports as unknown as TraceExports;
  let nodes = 0;
  let ids: number[] | undefined;
  const hostId = (i: number): number => ids?.[i] ?? i;
  const out = (): Float64Array => new Float64Array(x.memory.buffer, x.trace_out(), 4096);
  const setGeometry = (g: SfxGeometry): void => {
    const nb = g.boxes.length / BOX_STRIDE;
    const nt = g.triangles.length / TRI_STRIDE;
    const ptr = x.trace_alloc(g.boxes.length + g.triangles.length);
    const f = new Float32Array(x.memory.buffer, ptr, g.boxes.length + g.triangles.length);
    f.set(g.boxes, 0);
    f.set(g.triangles, g.boxes.length);
    nodes = x.trace_scene(nb, nt);
    ids = g.ids;
  };
  setGeometry(geometry);
  let traced = false;
  const reverb = (s: Vec3, stride: number, ref: number): VoiceReverb => {
    x.trace_voice_reverb(s.x, s.y, s.z, stride, 99, ref, 0.985, 0.005, 0.35);
    const o = out();
    return { energy: o[0], rt60: o[1], direction: v3(o[2], o[3], o[4]), predelay: o[5], absorption: o[6], hits: o[7] };
  };
  const tracer: WasmTracer = {
    exports: x,
    get nodes() {
      return nodes;
    },
    setGeometry(g) {
      setGeometry(g);
      traced = false;
    },
    casts: () => x.trace_casts(),
    cast(origin, dir, maxDist): RayHit | null {
      if (!x.trace_cast(origin.x, origin.y, origin.z, dir.x, dir.y, dir.z, maxDist)) return null;
      const o = out();
      return { toi: o[0], point: v3(o[1], o[2], o[3]), normal: v3(o[4], o[5], o[6]), absorption: o[7], id: hostId(o[8]) };
    },
    listener(position: Vec3, o: TracerListenerOptions, segments, request = true) {
      if (!request) return null;
      x.trace_listener(position.x, position.y, position.z, o.rays, o.bounces, 60, 0.35);
      const r = out();
      const trace: ListenerTrace = {
        origin: position,
        rays: [],
        candidates: [],
        space: { escape: r[0], leak: r[1], meanFreePath: r[2], decayPerBounce: r[3], rt60: r[4], volume: r[5], absorption: r[6] },
      };
      traced = true;
      if (segments) {
        const n = x.trace_listener_rays(4);
        const s = out();
        for (let i = 0; i < n; i++) {
          const b = 1 + i * 7;
          segments.push({ from: v3(s[b], s[b + 1], s[b + 2]), to: v3(s[b + 3], s[b + 4], s[b + 5]), hit: s[b + 6] > 0, kind: 'room' });
        }
      }
      const feet = reverb(v3(position.x, position.y - 1.5, position.z), o.reverbStride * 2, o.ref);
      return { trace, feet };
    },
    voice(_key, s: Vec3, l: Vec3, o: TracerVoiceOptions, segments, request = true) {
      if (!request) return null;
      x.trace_direct_path(s.x, s.y, s.z, l.x, l.y, l.z, o.extent, 0.2, 48, traced ? 1 : 0);
      let r = out();
      const path: DirectPath = { visible: r[0], excess: r[1], via: v3(r[2], r[3], r[4]), length: r[5], found: r[6] > 0 };
      if (segments && path.excess > 0) {
        const mid = v3(r[7], r[8], r[9]);
        segments.push({ from: l, to: path.via, hit: false, kind: 'path' }, { from: path.via, to: mid, hit: false, kind: 'path' }, { from: mid, to: s, hit: false, kind: 'path' });
      }
      const rev = traced ? reverb(s, o.reverbStride, o.ref) : null;
      const n = x.trace_reflections(s.x, s.y, s.z, l.x, l.y, l.z, SOURCE_DIRS.length, o.taps, 40, REFLECTION_OPTIONS.soft, REFLECTION_OPTIONS.minDelay, REFLECTION_OPTIONS.scattering);
      r = out();
      const reflections: Reflection[] = [];
      for (let i = 0; i < n; i++) {
        const b = i * 16;
        const nx = r[b + 11];
        const ny = r[b + 12];
        const nz = r[b + 13];
        const id = hostId(r[b + 10]);
        const point = v3(r[b + 3], r[b + 4], r[b + 5]);
        reflections.push({
          image: v3(r[b], r[b + 1], r[b + 2]), point, path: r[b + 6], delay: r[b + 7], gain: r[b + 8], absorption: r[b + 9], id,
          key: `${id}:${nx.toFixed(1)},${ny.toFixed(1)},${nz.toFixed(1)}`, weight: r[b + 14],
        });
        if (segments) segments.push({ from: s, to: point, hit: true, kind: 'reflection' }, { from: point, to: l, hit: true, kind: 'reflection' });
      }
      return { path, reverb: rev, reflections };
    },
  };
  return tracer;
}
