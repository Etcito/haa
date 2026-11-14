// The baked acoustic field as the engine's acoustics (docs/physics-sound.md, "Acoustic field"),
// behind `createSfxEngine({ acoustics: { field } })`. Renderer-agnostic: it takes geometry records
// and, optionally, occupancy points (a renderer's depth buffer unprojected, or RAA's SDF samples).
//
// Two levels:
// - coarse (1-2 m cells) over the level, baked once in a worker, nearest to the listener first,
//   re-baked where the static geometry changes (`invalidate`);
// - fine (0.5 m cells) in a box around the listener, re-baked in a second worker from time to time
//   as the player moves, from the static geometry plus the dynamic bodies and the occupancy points.
// The main thread keeps a copy of each (the baked records) and does the lookups and the flood in
// wasm: per source, one trilinear read and one flooded path instead of rays per event.
import type { Reflection, Vec3 } from './acoustics.ts';
import { add, dot, length, normalize, scale, sub, v3, SPEED_OF_SOUND_M_S } from './acoustics.ts';
import type { FieldGrid, FieldPath, FieldSample, SfxGeometry } from './field-wasm.ts';
import { FieldWasm, shEval } from './field-wasm.ts';
import type { FieldWorkerIn, FieldWorkerOut } from './field-worker.ts';
import type { DirectPath, VoiceReverb } from './raytrace.ts';
import { edgeLossDb } from './raytrace.ts';

/** What the engine asks of a field (see engine.ts `acoustics.field`). */
export interface SfxFieldProvider {
  /** Every frame: the listener moved. */
  listener(p: Vec3): void;
  /** True when the field has baked cells around `p`. */
  ready(p: Vec3): boolean;
  path(s: Vec3, l: Vec3, extent: number): DirectPath;
  reverb(s: Vec3, l: Vec3, path: DirectPath): VoiceReverb;
  reflections(s: Vec3, l: Vec3): Reflection[];
}

export interface FineOptions {
  /** Cell size (m). Default 0.5. */
  cell?: number;
  /** Half size of the box around the listener (m). Default [6, 2, 6]. */
  half?: [number, number, number];
  /** Re-bake when the listener has moved this far from the box centre (m). Default 2. */
  moveBy?: number;
  /** ... or after this long (s), to see bodies that moved. Default 3. */
  every?: number;
}

export interface AcousticFieldOptions {
  module: WebAssembly.Module;
  /** A new field worker (`@etcito/haa/field-worker`), e.g. Vite's `new FieldWorker()`. */
  worker: () => Worker;
  /** The static level (fixed colliders). */
  geometry: () => SfxGeometry;
  /** The coarse grid over the level. */
  grid: FieldGrid;
  /** The fine level around the listener; omit to use the coarse level only. */
  fine?: FineOptions | false;
  /** Static plus moving bodies, for the fine level (default: the static level). */
  dynamicGeometry?: () => SfxGeometry;
  /** Occupancy points age out after this (s). Default 8. */
  occupancyLife?: number;
}

export interface FieldStats {
  coarse: { baked: number; left: number; bytes: number; bakeMs: number; casts: number; reached: number };
  fine: { baked: number; left: number; bytes: number; bakeMs: number; rebakes: number } | null;
  floodMs: number;
  lookups: number;
  lookupMs: number;
  occupancy: number;
}

const REF = 2;
/**
 * Sabine diffuse energy of a cell's space relative to the direct sound at 2 m: 16π·(2 m)²/A with
 * A = 0.161·V/RT60, the volume from the mean free path as for a cube (l = 2a/3, V = 3.375·l³; the
 * mean of (4π/3)·r³ over the probe rays read thousands of m³ in a 42 m³ shed, one ray through the
 * door decided it), times the closed share (the doorway hears the room behind it on one side).
 */
function cellEnergy(s: FieldSample): number {
  if (!(s.weight > 0) || !(s.mfp > 0) || !(s.rt60 > 0)) return 0;
  const volume = 3.375 * Math.max(0.3, s.mfp) ** 3;
  const area = (0.161 * volume) / s.rt60;
  return ((16 * Math.PI * REF * REF) / Math.max(1e-3, area)) * Math.max(0, 1 - s.escape);
}

/** The first-order part of an SH set as a vector (x, y, z) in units of the DC term. */
function shVector(c: ArrayLike<number>): Vec3 {
  const dc = Math.max(1e-6, Math.abs(c[0]));
  // Y1-1 ∝ y, Y10 ∝ z, Y11 ∝ x (field-wasm `sh9`); 0.488603 / 0.282095 = √3.
  return scale(v3(c[3], c[1], c[2]), 1 / (dc * Math.sqrt(3)));
}

const AXES: ReadonlyArray<[string, Vec3]> = [
  ['+x', v3(1, 0, 0)], ['-x', v3(-1, 0, 0)], ['+y', v3(0, 1, 0)], ['-y', v3(0, -1, 0)], ['+z', v3(0, 0, 1)], ['-z', v3(0, 0, -1)],
];

interface Level {
  wasm: FieldWasm;
  worker: Worker;
  grid: FieldGrid;
  left: number;
  bakeMs: number;
  casts: number;
  floodCell: string;
  baked: number;
  bytes: number;
}

export class AcousticField implements SfxFieldProvider {
  private readonly o: AcousticFieldOptions;
  private readonly coarse: Level;
  private fine: Level | null = null;
  private pendingFine: Level | null = null;
  private fineCentre: Vec3 | null = null;
  private fineAt = -Infinity;
  private fineRebakes = 0;
  private lastFocus: Vec3 | null = null;
  private listenerPos: Vec3 = v3(0, 0, 0);
  private occupancy: Array<{ p: Float32Array; at: number }> = [];
  private floodMs = 0;
  private lookups = 0;
  private lookupMs = 0;
  private readonly fineOpts: Required<FineOptions> | null;
  /** Called after each batch of baked cells (debug views). */
  onBaked: (() => void) | null = null;

  constructor(o: AcousticFieldOptions) {
    this.o = o;
    this.fineOpts = o.fine === false || !o.fine ? (o.fine === false ? null : { cell: 0.5, half: [6, 2, 6], moveBy: 2, every: 3 }) : { cell: 0.5, half: [6, 2, 6], moveBy: 2, every: 3, ...o.fine };
    this.coarse = this.level(o.grid, o.geometry(), v3(0, 0, 0));
  }

  private level(grid: FieldGrid, geometry: SfxGeometry, focus: Vec3, slice = 200): Level {
    const wasm = new FieldWasm(this.o.module);
    wasm.setScene(geometry);
    wasm.setGrid(grid);
    const worker = this.o.worker();
    const lv: Level = { wasm, worker, grid, left: grid.n[0] * grid.n[1] * grid.n[2], bakeMs: 0, casts: 0, floodCell: '', baked: 0, bytes: 0 };
    worker.onmessage = (e: MessageEvent<FieldWorkerOut>) => {
      const m = e.data;
      if (m.t === 'cells') {
        lv.wasm.ingest(m.records);
        lv.left = m.left;
        lv.bakeMs += m.ms;
        lv.casts += m.casts;
        lv.baked = m.baked;
        lv.bytes = m.bytes;
        lv.floodCell = '';
        this.onBaked?.();
      } else if (m.t === 'dirty') {
        lv.wasm.dirty(m.lo, m.hi);
        lv.floodCell = '';
      }
    };
    const init: FieldWorkerIn = { t: 'init', module: this.o.module, geometry, grid, focus, slice };
    worker.postMessage(init);
    return lv;
  }

  /** Static geometry changed inside the box (a door, a roof): those coarse cells are re-baked. */
  invalidate(lo: Vec3, hi: Vec3): void {
    const geometry = this.o.geometry();
    this.coarse.wasm.setScene(geometry);
    this.coarse.wasm.dirty(lo, hi);
    this.coarse.worker.postMessage({ t: 'geometry', geometry, lo, hi } satisfies FieldWorkerIn);
    this.fineAt = -Infinity;
  }

  /**
   * Occupancy points (x, y, z flat, world): what a renderer saw (its depth buffer unprojected), or
   * samples of an SDF. They become small boxes in the next fine bake and age out after `occupancyLife`.
   */
  occupy(points: Float32Array, now = performance.now() / 1000): void {
    this.occupancy.push({ p: points, at: now });
  }

  private occupancyBoxes(now: number): Float32Array {
    const life = this.o.occupancyLife ?? 8;
    this.occupancy = this.occupancy.filter((x) => now - x.at < life);
    const cell = this.fineOpts?.cell ?? 0.5;
    const h = cell / 2;
    // One box per occupied fine cell (deduplicated).
    const seen = new Set<string>();
    const out: number[] = [];
    for (const { p } of this.occupancy) {
      for (let i = 0; i + 2 < p.length; i += 3) {
        const k = `${Math.floor(p[i] / cell)},${Math.floor(p[i + 1] / cell)},${Math.floor(p[i + 2] / cell)}`;
        if (seen.has(k)) continue;
        seen.add(k);
        const [x, y, z] = k.split(',').map((v) => (Number(v) + 0.5) * cell);
        out.push(x, y, z, h, h, h, 0, 0, 0, 1, 0.1, 9999);
      }
    }
    return new Float32Array(out);
  }

  get occupiedCells(): number {
    return this.occupancyBoxes(performance.now() / 1000).length / 12;
  }

  listener(p: Vec3): void {
    this.listenerPos = p;
    const now = performance.now() / 1000;
    if (!this.lastFocus || length(sub(p, this.lastFocus)) > this.o.grid.cell) {
      this.lastFocus = p;
      this.coarse.worker.postMessage({ t: 'focus', p } satisfies FieldWorkerIn);
    }
    const f = this.fineOpts;
    if (f && (!this.fineCentre || length(sub(p, this.fineCentre)) > f.moveBy || now - this.fineAt > f.every)) this.rebakeFine(p, now);
    this.floodIfMoved(this.coarse, p);
    if (this.fine) this.floodIfMoved(this.fine, p);
  }

  private rebakeFine(p: Vec3, now: number): void {
    const f = this.fineOpts!;
    const n: [number, number, number] = [Math.round((2 * f.half[0]) / f.cell), Math.round((2 * f.half[1]) / f.cell), Math.round((2 * f.half[2]) / f.cell)];
    const origin = v3(Math.floor((p.x - f.half[0]) / f.cell) * f.cell, Math.max(0, Math.floor((p.y - 1.6 - 0.25) / f.cell) * f.cell), Math.floor((p.z - f.half[2]) / f.cell) * f.cell);
    const base = (this.o.dynamicGeometry ?? this.o.geometry)();
    const occ = this.occupancyBoxes(now);
    const boxes = new Float32Array(base.boxes.length + occ.length);
    boxes.set(base.boxes);
    boxes.set(occ, base.boxes.length);
    const geometry = { boxes, triangles: base.triangles };
    const grid: FieldGrid = { origin, cell: f.cell, n };
    // The next fine level bakes in its own worker; the current one serves until it has cells.
    this.pendingFine?.worker.terminate();
    const next = this.level(grid, geometry, p, 400);
    this.pendingFine = next;
    const old = this.fine;
    const swap = next.worker.onmessage;
    next.worker.onmessage = (e) => {
      swap?.call(next.worker, e);
      if (this.fine !== next && next.left === 0) {
        this.fine = next;
        if (this.pendingFine === next) this.pendingFine = null;
        old?.worker.terminate();
        this.fineRebakes++;
      }
    };
    if (!old) this.fine = next;
    this.fineCentre = p;
    this.fineAt = now;
  }

  private floodIfMoved(lv: Level, p: Vec3): void {
    const c = lv.grid.cell;
    const key = `${Math.floor(p.x / c)},${Math.floor(p.y / c)},${Math.floor(p.z / c)}`;
    if (key === lv.floodCell) return;
    const t0 = performance.now();
    lv.wasm.flood(p);
    this.floodMs = performance.now() - t0;
    lv.floodCell = key;
  }

  private inFine(p: Vec3): boolean {
    const g = this.fine?.grid;
    if (!g || this.fine!.left > 0) return false;
    const m = g.cell;
    return p.x > g.origin.x + m && p.x < g.origin.x + g.n[0] * g.cell - m && p.y > g.origin.y && p.y < g.origin.y + g.n[1] * g.cell - m && p.z > g.origin.z + m && p.z < g.origin.z + g.n[2] * g.cell - m;
  }

  private sample(p: Vec3): FieldSample {
    const t0 = performance.now();
    const s = this.inFine(p) ? this.fine!.wasm.sample(p) : this.coarse.wasm.sample(p);
    const r = s.weight > 0.3 ? s : this.coarse.wasm.sample(p);
    this.lookups++;
    this.lookupMs += performance.now() - t0;
    return r;
  }

  ready(p: Vec3): boolean {
    return this.coarse.wasm.sample(p).weight > 0.3 || (this.inFine(p) && this.fine!.wasm.sample(p).weight > 0.3);
  }

  path(s: Vec3, l: Vec3, _extent: number): DirectPath {
    const t0 = performance.now();
    const fine = this.inFine(s) && this.inFine(l);
    let p: FieldPath = (fine ? this.fine!.wasm : this.coarse.wasm).path(s, l);
    if (fine && !p.found) p = this.coarse.wasm.path(s, l);
    this.lookups++;
    this.lookupMs += performance.now() - t0;
    if (!p.found) return { visible: 0, excess: 0, via: s, length: length(sub(s, l)), found: false };
    return { visible: p.visible ? 1 : 0, excess: p.visible ? 0 : p.excess, via: p.visible ? s : p.via, length: p.length, found: true };
  }

  /**
   * The diffuse field of this source at this listener from the two probes: in one space (the path
   * barely bends) the mean of the two cells' Sabine energies and the geometric mean of their decays; coupled (the path
   * goes through an opening) the source's room through the opening's diffraction loss plus the
   * listener's room excited through it, from the opening's side.
   */
  reverb(s: Vec3, l: Vec3, path: DirectPath): VoiceReverb {
    const a = this.sample(s);
    const b = this.sample(l);
    const ea = cellEnergy(a);
    const eb = cellEnergy(b);
    // Two spaces: the path bends through an opening, or the source is seen through one (a hall
    // seen from outside its door: the two probes disagree on the decay by more than 2x).
    const throughOpening = path.found && path.visible === 0 && path.excess > 1;
    const seenThrough = !throughOpening && Math.abs(Math.log(Math.max(0.05, a.rt60) / Math.max(0.05, b.rt60))) > Math.log(2);
    const coupled = throughOpening || seenThrough;
    let energy: number;
    let rt60: number;
    let direction: Vec3;
    if (!coupled) {
      // One space: the mean of the two probes (a source in a doorway feeds the room behind it by half).
      energy = (ea + eb) / 2;
      rt60 = Math.exp((Math.log(Math.max(0.05, a.rt60)) + Math.log(Math.max(0.05, b.rt60))) / 2);
      // The enclosure side of the listener: against its openness.
      direction = scale(shVector(b.open), -1);
    } else {
      // Through an opening: its diffraction loss; seen through one: a tenth of the source room's
      // field (an opening's solid angle; the probes do not know its size).
      const t = throughOpening ? 10 ** (-edgeLossDb(path.excess, 500) * 0.65 / 10) : 0.1;
      const fromSource = ea * t;
      const own = throughOpening ? eb * t : eb;
      energy = fromSource + own;
      const w = fromSource / Math.max(1e-12, energy);
      rt60 = Math.exp(w * Math.log(Math.max(0.05, a.rt60)) + (1 - w) * Math.log(Math.max(0.05, b.rt60)));
      direction = scale(normalize(sub(path.via, l)), 0.7);
    }
    const dl = length(direction);
    if (dl > 1) direction = scale(direction, 1 / dl);
    return {
      energy,
      rt60,
      direction,
      predelay: Math.min(0.08, (0.5 * Math.max(b.mfp, 0.5)) / SPEED_OF_SOUND_M_S),
      absorption: b.absorption,
      hits: Math.round((a.weight + b.weight) * 100),
    };
  }

  /**
   * Early reflections from the source's distance harmonics: the wall in each of the 6 axis
   * directions (its distance, read continuously between cells), mirrored; the listener must be on
   * the source's side of it. An open direction (the sky) has no wall.
   */
  reflections(s: Vec3, l: Vec3): Reflection[] {
    const a = this.sample(s);
    if (!(a.weight > 0)) return [];
    const direct = length(sub(l, s));
    const out: Reflection[] = [];
    for (const [key, axis] of AXES) {
      const d = shEval(a.dist, axis);
      const open = Math.min(1, Math.max(0, shEval(a.open, axis)));
      if (!(d > 0.05) || d > 25 || open > 0.8) continue;
      const wall = add(s, scale(axis, d));
      if (dot(sub(l, wall), axis) >= -0.01) continue;
      const lv = this.inFine(s) ? this.fine!.wasm : this.coarse.wasm;
      const image = add(s, scale(axis, 2 * d));
      const pathLen = length(sub(l, image));
      const delay = (pathLen - direct) / SPEED_OF_SOUND_M_S;
      if (delay < 0.0015) continue;
      const t = dot(sub(wall, image), axis) / Math.max(1e-6, dot(sub(l, image), axis));
      const point = add(image, scale(sub(l, image), t));
      // Two exact rays: the bounce point must be seen from both ends (the axis wall may be a door,
      // or hidden behind a corner: the harmonics cannot tell).
      const back = scale(axis, -0.05);
      if (!lv.clear(add(point, back), l) || !lv.clear(add(point, back), s)) continue;
      out.push({
        image, point, path: pathLen, delay,
        gain: Math.sqrt(Math.max(0, (1 - a.absorption) * 0.65)), absorption: a.absorption, id: 0, key, weight: 1 - open,
      });
    }
    out.sort((x, y) => x.path - y.path);
    return out.slice(0, 4);
  }

  /** Debug view: per baked air cell of both levels x, y, z, escape, rt60, flooded distance, up openness, mean distance; and the level of each. */
  debugCells(): { coarse: Float32Array; fine: Float32Array | null } {
    return { coarse: this.coarse.wasm.debug(), fine: this.fine && this.fine.left === 0 ? this.fine.wasm.debug() : null };
  }

  stats(): FieldStats {
    const st = this.coarse.wasm.stats();
    return {
      coarse: { baked: this.coarse.baked, left: this.coarse.left, bytes: this.coarse.bytes, bakeMs: this.coarse.bakeMs, casts: this.coarse.casts, reached: st.reached },
      fine: this.fine ? { baked: this.fine.baked, left: this.fine.left, bytes: this.fine.bytes, bakeMs: this.fine.bakeMs, rebakes: this.fineRebakes } : null,
      floodMs: this.floodMs,
      lookups: this.lookups,
      lookupMs: this.lookupMs,
      occupancy: this.occupancy.length,
    };
  }

  dispose(): void {
    this.coarse.worker.terminate();
    this.fine?.worker.terminate();
    this.pendingFine?.worker.terminate();
  }
}
