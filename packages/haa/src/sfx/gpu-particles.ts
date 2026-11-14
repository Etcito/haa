// GPU debris particles for sound (prototype, docs/physics-sound.md "Compute"): thousands of
// non-gameplay particles (pebbles, shards, drops, sparks) integrated and collided against the static
// level on the GPU. The sound side never reads a particle: each step accumulates, per spatial cell
// and per surface material, what a swarm's cluster voice needs (impact count, summed energy and
// mass, centroid, sliding), compacts the non-empty cells into a short list, and that list is read
// back asynchronously. `createParticleSound` turns it into cluster loops (density, energy, size,
// hardness), the same inputs SwarmTracker gives a cluster preset.
//
// Written from scratch (WGSL), after Dimforge Nexus's idea of projecting a particle on the
// boundary of each collider (src_mpm_shaders/collision/collide.rs, Apache-2.0 / MIT): no Nexus code
// is copied, its shaders are rust-gpu Rust and its particles are MPM, not free debris.
import type { Vec3 } from './acoustics.ts';
import type { SfxEngine, SfxLoop } from './engine.ts';
import type { SfxGeometry } from './trace-wasm.ts';
import { BOX_STRIDE } from './trace-wasm.ts';
import type { GpuBuffer, GpuDevice } from './gpu-min.ts';
import { BUF, MAP_READ, STAGE_COMPUTE } from './gpu-min.ts';
import { DENSITY_FULL, energyFromJoules, sizeFromMass } from './contract.ts';

const MAX_CELLS_OUT = 512;
/** u32 per accumulator slot: count, energy (µJ), mass (g), x y z (mm from the cell corner), slides, slide speed (mm/s). */
const SLOT_U32 = 8;

const WGSL = /* wgsl */ `
struct Box { c: vec4f, h: vec4f, ax: vec4f, ay: vec4f, az: vec4f };   // c.w unused, h.w material index
struct Particle { p: vec4f, v: vec4f, s: vec4u };                      // p.w radius, v.w mass (kg); s: material, sleep frames, alive, age (frames)
struct Params { origin: vec4f, dims: vec4u, gravity: vec4f, count: u32, boxes: u32, dt: f32, materials: u32, restitution: f32, friction: f32, impactSpeed: f32, lifeFrames: u32 };
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> boxes: array<Box>;
@group(0) @binding(2) var<storage, read_write> parts: array<Particle>;
@group(0) @binding(3) var<storage, read_write> acc: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> list: array<u32>;          // [0] count, then (slot, 8 values) per entry
@group(0) @binding(5) var<storage, read_write> listCount: atomic<u32>;

fn slotOf(p: vec3f, mat: u32) -> i32 {
  let c = vec3i(floor((p - P.origin.xyz) / P.origin.w));
  if (any(c < vec3i(0)) || any(c >= vec3i(P.dims.xyz))) { return -1; }
  return ((c.z * i32(P.dims.y) + c.y) * i32(P.dims.x) + c.x) * i32(P.materials) + i32(mat);
}

@compute @workgroup_size(64)
fn step(@builtin(global_invocation_id) gid: vec3u) {
  let i = gid.x;
  if (i >= P.count) { return; }
  var q = parts[i];
  if (q.s.z == 0u) { return; }
  q.s.w += 1u;
  if (q.s.w > P.lifeFrames) { q.s.z = 0u; parts[i] = q; return; }
  if (q.s.y >= 30u) { parts[i] = q; return; }   // asleep
  var v = q.v.xyz + P.gravity.xyz * P.dt;
  var p = q.p.xyz + v * P.dt;
  let r = q.p.w;
  var contact = false;
  for (var b = 0u; b < P.boxes; b++) {
    let bx = boxes[b];
    let rel = p - bx.c.xyz;
    let l = vec3f(dot(rel, bx.ax.xyz), dot(rel, bx.ay.xyz), dot(rel, bx.az.xyz));
    let h = bx.h.xyz;
    let cl = clamp(l, -h, h);
    var dl = l - cl;
    var dist = length(dl);
    var nl: vec3f;
    var depth: f32;
    if (dist > 1e-6) {
      if (dist >= r) { continue; }
      nl = dl / dist;
      depth = r - dist;
    } else {
      // Centre inside the box: out through the nearest face.
      let pen = h - abs(l);
      if (pen.x <= pen.y && pen.x <= pen.z) { nl = vec3f(sign(l.x), 0.0, 0.0); depth = pen.x + r; }
      else if (pen.y <= pen.z) { nl = vec3f(0.0, sign(l.y), 0.0); depth = pen.y + r; }
      else { nl = vec3f(0.0, 0.0, sign(l.z)); depth = pen.z + r; }
    }
    let n = normalize(bx.ax.xyz * nl.x + bx.ay.xyz * nl.y + bx.az.xyz * nl.z);
    p += n * depth;
    contact = true;
    let vn = dot(v, n);
    if (vn < 0.0) {
      let vt = v - n * vn;
      let speed = -vn;
      v = vt * (1.0 - P.friction) - n * (vn * P.restitution);
      let slot = slotOf(p, u32(bx.h.w));
      if (slot >= 0) {
        let base = u32(slot) * ${SLOT_U32}u;
        let rp = (p - (P.origin.xyz + vec3f(vec3i(floor((p - P.origin.xyz) / P.origin.w))) * P.origin.w)) * 1000.0;
        if (speed > P.impactSpeed) {
          let joules = 0.5 * q.v.w * speed * speed;
          atomicAdd(&acc[base], 1u);
          atomicAdd(&acc[base + 1u], u32(min(joules * 1e6, 4e6)));
          atomicAdd(&acc[base + 2u], u32(q.v.w * 1000.0));
          atomicAdd(&acc[base + 3u], u32(rp.x));
          atomicAdd(&acc[base + 4u], u32(rp.y));
          atomicAdd(&acc[base + 5u], u32(rp.z));
        } else if (length(vt) > 0.1) {
          atomicAdd(&acc[base + 6u], 1u);
          atomicAdd(&acc[base + 7u], u32(length(vt) * 1000.0));
        }
      }
    }
  }
  if (contact && length(v) < 0.05) { q.s.y += 1u; } else { q.s.y = 0u; }
  q.p = vec4f(p, r);
  q.v = vec4f(v, q.v.w);
  parts[i] = q;
}

// One thread per accumulator slot: a non-empty slot goes to the list, and every slot is cleared.
@compute @workgroup_size(64)
fn compact(@builtin(global_invocation_id) gid: vec3u) {
  let s = gid.x;
  let total = P.dims.x * P.dims.y * P.dims.z * P.materials;
  if (s >= total) { return; }
  let base = s * ${SLOT_U32}u;
  let n = atomicLoad(&acc[base]) + atomicLoad(&acc[base + 6u]);
  if (n == 0u) { return; }
  let k = atomicAdd(&listCount, 1u);
  if (k < ${MAX_CELLS_OUT}u) {
    let o = 1u + k * ${SLOT_U32 + 1}u;
    list[o] = s;
    for (var j = 0u; j < ${SLOT_U32}u; j++) { list[o + 1u + j] = atomicExchange(&acc[base + j], 0u); }
  } else {
    for (var j = 0u; j < ${SLOT_U32}u; j++) { atomicStore(&acc[base + j], 0u); }
  }
}

@compute @workgroup_size(1)
fn finish() {
  list[0] = min(atomicExchange(&listCount, 0u), ${MAX_CELLS_OUT}u);
}
`;

export interface ParticleBurst {
  position: Vec3;
  count: number;
  /** Initial speed (m/s) and its spread (a share of it, random directions in the upper half). */
  speed: number;
  spread?: number;
  /** Mean velocity added to every particle (an explosion's push, a pour). */
  velocity?: Vec3;
  radius?: number;
  /** Mass of one particle (kg). */
  mass?: number;
  /** Striker material index (the sound bridge's table). */
  material?: number;
}

/** One cell and surface material's impacts in one step. */
export interface ParticleCell {
  cell: number;
  material: number;
  impacts: number;
  joules: number;
  massKg: number;
  centroid: Vec3;
  slides: number;
  slideSpeed: number;
}

export interface ParticleReadback {
  cells: ParticleCell[];
  /** Simulated seconds this readback covers. */
  dt: number;
  /** Step submit to mapped list (ms). */
  latencyMs: number;
  /** GPU time of the step + compact passes (ms), when timestamp queries exist. */
  gpuMs: number;
}

export interface GpuParticles {
  burst(b: ParticleBurst): void;
  /** One simulation step of `dt` s; its cell list arrives later through `onReadback`. */
  step(dt: number): void;
  onReadback(cb: (r: ParticleReadback) => void): void;
  /** Alive particles (CPU count of spawned minus expired, approximate). */
  readonly capacity: number;
  dispose(): void;
}

export interface GpuParticlesOptions {
  capacity?: number;
  /** Accumulation cell size (m). Default 4 (a swarm's radius). */
  cell?: number;
  /** Material index of each geometry surface id (box record id): the surface's sound material. */
  materialOf?: (surfaceId: number) => number;
  materials?: number;
  restitution?: number;
  friction?: number;
  lifetime?: number;
}

export async function createGpuParticles(device: GpuDevice, geometry: SfxGeometry, o: GpuParticlesOptions = {}): Promise<GpuParticles> {
  const capacity = o.capacity ?? 16384;
  const cellSize = o.cell ?? 4;
  const materials = o.materials ?? 8;
  // Grid over the level's bounds (+ a margin above for flying debris).
  const nb = geometry.boxes.length / BOX_STRIDE;
  const lo = [Infinity, Infinity, Infinity];
  const hi = [-Infinity, -Infinity, -Infinity];
  const boxData = new Float32Array(Math.max(1, nb) * 20);
  for (let i = 0; i < nb; i++) {
    const r = geometry.boxes.subarray(i * BOX_STRIDE, i * BOX_STRIDE + BOX_STRIDE);
    const ext = Math.hypot(r[3], r[4], r[5]);
    for (let a = 0; a < 3; a++) {
      lo[a] = Math.min(lo[a], r[a] - ext);
      hi[a] = Math.max(hi[a], r[a] + ext);
    }
    const [x, y, z, w] = [r[6], r[7], r[8], r[9]];
    boxData.set([
      r[0], r[1], r[2], 0, r[3], r[4], r[5], o.materialOf ? o.materialOf(r[11]) : 0,
      1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w), 0,
      2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w), 0,
      2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y), 0,
    ], i * 20);
  }
  hi[1] += 8;
  const dims = [0, 1, 2].map((a) => Math.max(1, Math.ceil((hi[a] - lo[a]) / cellSize)));
  const slots = dims[0] * dims[1] * dims[2] * materials;
  const module = device.createShaderModule({ code: WGSL });
  const info = await module.getCompilationInfo();
  const errors = info.messages.filter((m: { type: string }) => m.type === 'error');
  if (errors.length) throw new Error('[haa] gpu-particles WGSL: ' + errors.map((m: { lineNum: number; message: string }) => `${m.lineNum} ${m.message}`).join('; '));
  const layout = device.createBindGroupLayout({
    entries: [
      { binding: 0, visibility: STAGE_COMPUTE, buffer: { type: 'uniform' } },
      { binding: 1, visibility: STAGE_COMPUTE, buffer: { type: 'read-only-storage' } },
      ...[2, 3, 4, 5].map((b) => ({ binding: b, visibility: STAGE_COMPUTE, buffer: { type: 'storage' } })),
    ],
  });
  const pl = device.createPipelineLayout({ bindGroupLayouts: [layout] });
  const pipe = (entryPoint: string): unknown => device.createComputePipeline({ layout: pl, compute: { module, entryPoint } });
  const [pStep, pCompact, pFinish] = [pipe('step'), pipe('compact'), pipe('finish')];
  const params = device.createBuffer({ size: 80, usage: BUF.UNIFORM | BUF.COPY_DST });
  const boxBuf = device.createBuffer({ size: boxData.byteLength, usage: BUF.STORAGE | BUF.COPY_DST });
  device.queue.writeBuffer(boxBuf, 0, boxData);
  const parts = device.createBuffer({ size: capacity * 48, usage: BUF.STORAGE | BUF.COPY_DST });
  const acc = device.createBuffer({ size: slots * SLOT_U32 * 4, usage: BUF.STORAGE | BUF.COPY_DST });
  const listBytes = (1 + MAX_CELLS_OUT * (SLOT_U32 + 1)) * 4;
  const list = device.createBuffer({ size: listBytes, usage: BUF.STORAGE | BUF.COPY_SRC });
  const listCount = device.createBuffer({ size: 4, usage: BUF.STORAGE });
  const bind = device.createBindGroup({ layout, entries: [params, boxBuf, parts, acc, list, listCount].map((buffer, binding) => ({ binding, resource: { buffer } })) });
  const timestamps = device.features.has('timestamp-query');
  const ring: Array<{ buf: GpuBuffer; busy: boolean; q: GpuBuffer | null }> = Array.from({ length: 4 }, () => ({
    buf: device.createBuffer({ size: listBytes, usage: BUF.MAP_READ | BUF.COPY_DST }),
    busy: false,
    q: timestamps ? device.createBuffer({ size: 16, usage: BUF.MAP_READ | BUF.COPY_DST }) : null,
  }));
  const qset = timestamps ? device.createQuerySet({ type: 'timestamp', count: 2 }) : null;
  const qres = timestamps ? device.createBuffer({ size: 16, usage: BUF.QUERY_RESOLVE | BUF.COPY_SRC }) : null;
  const listeners: Array<(r: ParticleReadback) => void> = [];
  let next = 0;
  let pendingDt = 0;
  const lifeFrames = Math.round((o.lifetime ?? 8) * 60);
  const writeParams = (dt: number): void => {
    const b = new ArrayBuffer(80);
    new Float32Array(b, 0, 4).set([lo[0], lo[1], lo[2], cellSize]);
    new Uint32Array(b, 16, 4).set([dims[0], dims[1], dims[2], 0]);
    new Float32Array(b, 32, 4).set([0, -9.81, 0, 0]);
    new Uint32Array(b, 48, 2).set([capacity, nb]);
    new Float32Array(b, 56, 1).set([dt]);
    new Uint32Array(b, 60, 1).set([materials]);
    new Float32Array(b, 64, 3).set([o.restitution ?? 0.35, o.friction ?? 0.2, 0.35]);
    new Uint32Array(b, 76, 1).set([lifeFrames]);
    device.queue.writeBuffer(params, 0, b);
  };
  const decode = (u: Uint32Array, dt: number, latencyMs: number, gpuMs: number): ParticleReadback => {
    const n = u[0];
    const cells: ParticleCell[] = [];
    for (let k = 0; k < n; k++) {
      const b = 1 + k * (SLOT_U32 + 1);
      const slot = u[b];
      const material = slot % materials;
      const cell = Math.floor(slot / materials);
      const cx = cell % dims[0];
      const cy = Math.floor(cell / dims[0]) % dims[1];
      const cz = Math.floor(cell / (dims[0] * dims[1]));
      const impacts = u[b + 1];
      const inv = impacts > 0 ? 1 / (impacts * 1000) : 0;
      cells.push({
        cell, material, impacts, joules: u[b + 2] / 1e6, massKg: u[b + 3] / 1000,
        centroid: impacts > 0
          ? { x: lo[0] + cx * cellSize + u[b + 4] * inv, y: lo[1] + cy * cellSize + u[b + 5] * inv, z: lo[2] + cz * cellSize + u[b + 6] * inv }
          : { x: lo[0] + (cx + 0.5) * cellSize, y: lo[1] + (cy + 0.5) * cellSize, z: lo[2] + (cz + 0.5) * cellSize },
        slides: u[b + 7], slideSpeed: u[b + 8] / 1000,
      });
    }
    return { cells, dt, latencyMs, gpuMs };
  };
  return {
    capacity,
    burst(bu) {
      const n = Math.min(bu.count, capacity);
      const data = new ArrayBuffer(n * 48);
      const f = new Float32Array(data);
      const u = new Uint32Array(data);
      const spread = bu.speed * (bu.spread ?? 0.6);
      const vel = bu.velocity ?? { x: 0, y: 0, z: 0 };
      for (let i = 0; i < n; i++) {
        const z = Math.random();
        const a = Math.random() * 2 * Math.PI;
        const s = Math.sqrt(1 - z * z);
        const sp = bu.speed + (Math.random() * 2 - 1) * spread;
        const j = (Math.random() - 0.5) * 0.2;
        f.set([bu.position.x + j, bu.position.y + j, bu.position.z + j, bu.radius ?? 0.01, vel.x + s * Math.cos(a) * sp, vel.y + z * sp, vel.z + s * Math.sin(a) * sp, bu.mass ?? 0.01], i * 12);
        u.set([bu.material ?? 0, 0, 1, 0], i * 12 + 8);
      }
      // A ring over the capacity: the oldest particles give way to a new burst.
      const first = next;
      const tail = Math.min(n, capacity - first);
      device.queue.writeBuffer(parts, first * 48, new Uint8Array(data, 0, tail * 48));
      if (tail < n) device.queue.writeBuffer(parts, 0, new Uint8Array(data, tail * 48));
      next = (first + n) % capacity;
    },
    step(dt) {
      writeParams(dt);
      pendingDt += dt;
      const enc = device.createCommandEncoder();
      const pass = enc.beginComputePass(qset ? { timestampWrites: { querySet: qset, beginningOfPassWriteIndex: 0, endOfPassWriteIndex: 1 } } : {});
      pass.setBindGroup(0, bind);
      pass.setPipeline(pStep);
      pass.dispatchWorkgroups(Math.ceil(capacity / 64));
      // Compact and read back only when a staging buffer is free; otherwise the counts keep
      // accumulating on the GPU and the next readback covers several steps (its `dt` says how long).
      const slot = ring.find((r) => !r.busy);
      if (slot) {
        pass.setPipeline(pCompact);
        pass.dispatchWorkgroups(Math.ceil(slots / 64));
        pass.setPipeline(pFinish);
        pass.dispatchWorkgroups(1);
      }
      pass.end();
      if (!slot) {
        device.queue.submit([enc.finish()]);
        return;
      }
      enc.copyBufferToBuffer(list, 0, slot.buf, 0, listBytes);
      if (qset && qres && slot.q) {
        enc.resolveQuerySet(qset, 0, 2, qres, 0);
        enc.copyBufferToBuffer(qres, 0, slot.q, 0, 16);
      }
      device.queue.submit([enc.finish()]);
      slot.busy = true;
      const covered = pendingDt;
      pendingDt = 0;
      const t0 = performance.now();
      void Promise.all([slot.buf.mapAsync(MAP_READ), slot.q?.mapAsync(MAP_READ)]).then(() => {
        const latency = performance.now() - t0;
        const u = new Uint32Array(slot.buf.getMappedRange().slice(0));
        slot.buf.unmap();
        let gpuMs = NaN;
        if (slot.q) {
          const q = new BigUint64Array(slot.q.getMappedRange().slice(0));
          slot.q.unmap();
          gpuMs = Number(q[1] - q[0]) / 1e6;
        }
        slot.busy = false;
        const r = decode(u, covered, latency, gpuMs);
        for (const cb of listeners) cb(r);
      });
    },
    onReadback(cb) {
      listeners.push(cb);
    },
    dispose() {
      for (const b of [params, boxBuf, parts, acc, list, listCount, ...ring.map((r) => r.buf)]) b.destroy();
    },
  };
}

export interface ParticleSoundOptions {
  /** Cluster preset of each striker material index (e.g. 'particle-cluster', 'glass-cluster'). */
  preset: (material: number) => string | null;
  /** Hardness (0..1) of a surface material index. */
  hardness?: (material: number) => number;
  /** Rate and means time constant (s). Default 0.08, as SwarmTracker. */
  tau?: number;
  /** A cell's voice fades out this long (s) after its last impact. Default 0.15 + 0.2 fade. */
  hold?: number;
}

/**
 * Cluster voices from the particle readbacks: one loop per (cell, surface material), its `density`
 * the impact rate (leaky, ÷ 200), `energy` / `size` from the mean joules and mass per impact, at the
 * impacts' centroid. The first loud impacts of a burst are the game's to play as discrete voices
 * (`sfx.play`): a readback is a frame or two late, which a swarm hides and a single crack does not.
 */
export function createParticleSound(sfx: SfxEngine, o: ParticleSoundOptions): { feed(r: ParticleReadback): void; voices(): number } {
  const tau = o.tau ?? 0.08;
  const hold = o.hold ?? 0.15;
  const live = new Map<string, { loop: SfxLoop; rate: number; joules: number; mass: number; quiet: number; pos: Vec3 }>();
  return {
    voices: () => live.size,
    feed(r) {
      const dt = Math.max(1e-3, r.dt);
      const k = 1 - Math.exp(-dt / tau);
      const seen = new Set<string>();
      for (const c of r.cells) {
        if (c.impacts === 0) continue;
        const preset = o.preset(c.material);
        if (!preset) continue;
        const key = `${c.cell}:${c.material}`;
        seen.add(key);
        let v = live.get(key);
        const meanJ = c.joules / c.impacts;
        const meanM = c.massKg / c.impacts;
        if (!v) {
          const loop = sfx.loop(preset, c.centroid, { density: 0, energy: energyFromJoules(meanJ), size: sizeFromMass(meanM), hardness: o.hardness?.(c.material) ?? 0.5 }, { label: 'particles' });
          v = { loop, rate: 0, joules: meanJ, mass: meanM, quiet: 0, pos: c.centroid };
          live.set(key, v);
        }
        v.rate += (c.impacts / dt - v.rate) * k;
        v.joules += (meanJ - v.joules) * k;
        v.mass += (meanM - v.mass) * k;
        v.quiet = 0;
        v.pos = c.centroid;
        v.loop.move(c.centroid);
        v.loop.set({ density: Math.min(1, v.rate / DENSITY_FULL), energy: energyFromJoules(v.joules), size: sizeFromMass(v.mass) });
      }
      for (const [key, v] of live) {
        if (seen.has(key)) continue;
        v.rate *= 1 - k;
        v.quiet += dt;
        v.loop.set({ density: Math.min(1, v.rate / DENSITY_FULL) });
        if (v.quiet > hold) {
          v.loop.stop(0.2);
          live.delete(key);
        }
      }
    },
  };
}
