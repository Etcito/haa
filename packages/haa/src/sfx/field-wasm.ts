// The baked acoustic field's wasm (crates/haa-field, `pkg/haa-field.wasm`) and its geometry input.
// No three.js: the level arrives as flat records (`SfxGeometry`), today read from Rapier's fixed
// colliders (`geometryFromRapier`), later from RAA's level. Used by the bake worker and, as a copy
// fed with the baked cells, on the main thread for the lookups and the flood.
import type { Vec3 } from './acoustics.ts';
import { v3 } from './acoustics.ts';

/** Floats per box: centre (3), half extents (3), rotation quaternion x y z w (4), absorption, surface id. */
export const BOX_STRIDE = 12;
/** Floats per triangle: three vertices (9), absorption, surface id. */
export const TRI_STRIDE = 11;
/** Spherical-harmonic coefficients per quantity (order 1: Y00, Y1-1 ∝ y, Y10 ∝ z, Y11 ∝ x). */
export const FIELD_SH = 4;
/** Reflecting planes stored per cell. */
export const FIELD_PLANES = 6;
/** Floats per baked cell record (crates/haa-field `RECORD`). */
export const FIELD_RECORD = 2 + FIELD_SH + FIELD_SH + 5 + 2 + 1 + FIELD_PLANES * 5;

/** The static level as the field reads it: oriented boxes and triangles with their Sabine α. */
export interface SfxGeometry {
  boxes: Float32Array;
  triangles: Float32Array;
}

/** The grid of the field: origin (lower corner), cell size (m), cell counts. */
export interface FieldGrid {
  origin: Vec3;
  cell: number;
  n: [number, number, number];
}

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
 * Reads the level from a Rapier world (the colliders `include` accepts): cuboids as boxes, triangle
 * meshes as triangles, balls, cylinders and capsules as the box around them. (Same records as the
 * `compute` branch's tracer, haa-trace.)
 */
export function geometryFromRapier(world: RWorldLike, absorptionOf: (c: RColliderLike) => number, include: (c: RColliderLike) => boolean): SfxGeometry {
  const boxes: number[] = [];
  const tris: number[] = [];
  let id = 0;
  world.forEachCollider((c) => {
    if (!include(c)) return;
    const a = absorptionOf(c);
    const t = c.translation();
    const q = c.rotation();
    const s = c.shape;
    if (s.vertices && s.indices) {
      const v = s.vertices;
      const rot = (i: number): number[] => rotate(q, v[i * 3], v[i * 3 + 1], v[i * 3 + 2]).map((x, k) => x + [t.x, t.y, t.z][k]);
      for (let i = 0; i + 2 < s.indices.length; i += 3) tris.push(...rot(s.indices[i]), ...rot(s.indices[i + 1]), ...rot(s.indices[i + 2]), a, id);
      id++;
      return;
    }
    let h: Vec3 | null = s.halfExtents ?? null;
    if (!h && s.radius !== undefined) {
      const r = s.radius;
      h = s.halfHeight !== undefined ? v3(r, s.halfHeight + (s.type === 2 ? r : 0), r) : v3(r, r, r);
    }
    if (!h) return;
    boxes.push(t.x, t.y, t.z, h.x, h.y, h.z, q.x, q.y, q.z, q.w, a, id++);
  });
  return { boxes: new Float32Array(boxes), triangles: new Float32Array(tris) };
}

function rotate(q: { x: number; y: number; z: number; w: number }, x: number, y: number, z: number): number[] {
  const cx = q.y * z - q.z * y;
  const cy = q.z * x - q.x * z;
  const cz = q.x * y - q.y * x;
  return [x + 2 * (q.w * cx + q.y * cz - q.z * cy), y + 2 * (q.w * cy + q.z * cx - q.x * cz), z + 2 * (q.w * cz + q.x * cy - q.y * cx)];
}

interface FieldExports {
  memory: WebAssembly.Memory;
  field_alloc(floats: number): number;
  field_out(): number;
  field_records(): number;
  field_scene(boxes: number, tris: number, rebuild: number): void;
  field_grid(ox: number, oy: number, oz: number, cell: number, nx: number, ny: number, nz: number): void;
  field_bake(budget: number, fx: number, fy: number, fz: number): number;
  field_take(): number;
  field_ingest(n: number): void;
  field_dirty(lx: number, ly: number, lz: number, hx: number, hy: number, hz: number): number;
  field_sample(x: number, y: number, z: number): void;
  field_flood(x: number, y: number, z: number, max: number): number;
  field_clear(ax: number, ay: number, az: number, bx: number, by: number, bz: number): number;
  field_path(sx: number, sy: number, sz: number, lx: number, ly: number, lz: number, wantAperture: number): void;
  field_planes(x: number, y: number, z: number): void;
  field_flood_export(): number;
  field_flood_import(floats: number): void;
  field_debug(): number;
  field_stats(): void;
}

export interface FieldSample {
  weight: number;
  rt60: number;
  mfp: number;
  volume: number;
  absorption: number;
  escape: number;
  dist: Float64Array;
  open: Float64Array;
}

export interface FieldPath {
  found: boolean;
  visible: boolean;
  lattice: number;
  length: number;
  excess: number;
  via: Vec3;
  corners: number;
  /** Narrowest free cross-section along the path (m², Infinity when not measured), where, and at the two ends. */
  aperture: number;
  aperturePoint: Vec3;
  atListener: number;
  atSource: number;
}

/** A reflecting plane: unit normal toward the air, offset (n · point on the plane), absorption. */
export interface FieldPlane {
  n: Vec3;
  offset: number;
  absorption: number;
}

/** A haa-field instance: scene, grid, bake, records in and out, lookups. */
export class FieldWasm {
  private readonly x: FieldExports;

  constructor(module: WebAssembly.Module) {
    this.x = new WebAssembly.Instance(module, {}).exports as unknown as FieldExports;
  }

  /** The level's records; `rebuild` false only stores them for the next `setGrid` (one BVH build). */
  setScene(g: SfxGeometry, rebuild = true): void {
    const nb = g.boxes.length / BOX_STRIDE;
    const nt = g.triangles.length / TRI_STRIDE;
    const ptr = this.x.field_alloc(g.boxes.length + g.triangles.length);
    const view = new Float32Array(this.x.memory.buffer, ptr, g.boxes.length + g.triangles.length);
    view.set(g.boxes, 0);
    view.set(g.triangles, g.boxes.length);
    this.x.field_scene(nb, nt, rebuild ? 1 : 0);
  }

  setGrid(g: FieldGrid): void {
    this.x.field_grid(g.origin.x, g.origin.y, g.origin.z, g.cell, g.n[0], g.n[1], g.n[2]);
  }

  /** Bake up to `budget` cells nearest `focus`; returns the cells left. */
  bake(budget: number, focus: Vec3): number {
    return this.x.field_bake(budget, focus.x, focus.y, focus.z);
  }

  /** The records of the cells baked since the last call (a copy). */
  take(): Float32Array {
    const n = this.x.field_take();
    return new Float32Array(this.x.memory.buffer, this.x.field_records(), n * FIELD_RECORD).slice();
  }

  ingest(records: Float32Array): void {
    const ptr = this.x.field_alloc(records.length);
    new Float32Array(this.x.memory.buffer, ptr, records.length).set(records);
    this.x.field_ingest(records.length / FIELD_RECORD);
  }

  dirty(lo: Vec3, hi: Vec3): number {
    return this.x.field_dirty(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);
  }

  private out(n: number): Float64Array {
    return new Float64Array(this.x.memory.buffer, this.x.field_out(), n);
  }

  sample(p: Vec3): FieldSample {
    this.x.field_sample(p.x, p.y, p.z);
    const o = this.out(6 + 2 * FIELD_SH);
    return { weight: o[0], rt60: o[1], mfp: o[2], volume: o[3], absorption: o[4], escape: o[5], dist: o.slice(6, 6 + FIELD_SH), open: o.slice(6 + FIELD_SH, 6 + 2 * FIELD_SH) };
  }

  /** One ray: true when the segment crosses no geometry. */
  clear(a: Vec3, b: Vec3): boolean {
    return this.x.field_clear(a.x, a.y, a.z, b.x, b.y, b.z) === 1;
  }

  /** Flood from the listener up to a path length of `max` (m). */
  flood(l: Vec3, max = 40): number {
    return this.x.field_flood(l.x, l.y, l.z, max);
  }

  path(s: Vec3, l: Vec3, wantAperture = false): FieldPath {
    this.x.field_path(s.x, s.y, s.z, l.x, l.y, l.z, wantAperture ? 1 : 0);
    const o = this.out(15);
    return {
      found: o[0] > 0, visible: o[1] > 0, lattice: o[2], length: o[3], excess: o[4], via: v3(o[5], o[6], o[7]), corners: o[8],
      aperture: wantAperture ? o[9] : Infinity, aperturePoint: v3(o[10], o[11], o[12]), atListener: o[13], atSource: o[14],
    };
  }

  /** The reflecting planes of the air cell nearest `p`. */
  planes(p: Vec3): FieldPlane[] {
    this.x.field_planes(p.x, p.y, p.z);
    const n = this.out(1)[0];
    const o = this.out(1 + n * 5);
    const out: FieldPlane[] = [];
    for (let j = 0; j < n; j++) {
      const k = 1 + j * 5;
      out.push({ n: v3(o[k], o[k + 1], o[k + 2]), offset: o[k + 3], absorption: o[k + 4] });
    }
    return out;
  }

  /** The current flood (distance, parent per cell), to hand to another instance of the same grid. */
  floodExport(): Float32Array {
    const n = this.x.field_flood_export();
    return new Float32Array(this.x.memory.buffer, this.x.field_records(), n).slice();
  }

  floodImport(data: Float32Array): void {
    const ptr = this.x.field_alloc(data.length);
    new Float32Array(this.x.memory.buffer, ptr, data.length).set(data);
    this.x.field_flood_import(data.length);
  }

  /** Per baked air cell: x, y, z, escape, rt60, flooded distance, up openness, plane count. */
  debug(): Float32Array {
    const n = this.x.field_debug();
    return new Float32Array(this.x.memory.buffer, this.x.field_records(), n * 8).slice();
  }

  stats(): { baked: number; bricks: number; bytes: number; reached: number; casts: number } {
    this.x.field_stats();
    const o = this.out(5);
    return { baked: o[0], bricks: o[1], bytes: o[2], reached: o[3], casts: o[4] };
  }
}

/** Real spherical harmonics of order 0 and 1 (same basis as the Rust side). */
export function sh4(d: Vec3): number[] {
  return [0.282095, 0.488603 * d.y, 0.488603 * d.z, 0.488603 * d.x];
}

/** Evaluate stored SH coefficients in a direction. */
export function shEval(c: ArrayLike<number>, d: Vec3): number {
  const y = sh4(d);
  let s = 0;
  for (let k = 0; k < FIELD_SH; k++) s += c[k] * y[k];
  return s;
}
