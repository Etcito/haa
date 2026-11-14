// WebGPU acoustic tracer (prototype, docs/physics-sound.md "Compute"): the listener trace and every
// voice's diffuse field (next-event estimation from the bounce points) in two compute passes, the
// sums read back asynchronously. The compute is WGSL; this file only uploads, dispatches and maps.
// It answers the question "how far do the ray counts scale on the GPU": it ports `traceListener`
// and the energy/direction of `voiceReverb`, not the arrival-time statistics (RT60 of the paths,
// pre-delay), the direct path or the image sources, which stay in the wasm tracer.
import type { Vec3 } from './acoustics.ts';
import type { SfxGeometry } from './trace-wasm.ts';
import { BOX_STRIDE } from './trace-wasm.ts';
import type { GpuBuffer, GpuDevice } from './gpu-min.ts';
import { BUF, MAP_READ, STAGE_COMPUTE } from './gpu-min.ts';

const WGSL = /* wgsl */ `
struct Box { c: vec4f, h: vec4f, ax: vec4f, ay: vec4f, az: vec4f };   // c.w absorption, h.w id
struct Node { p: vec4f, n: vec4f };                                     // p.w throughput, n.w path; n = 0: none
struct Params { listener: vec4f, rays: u32, bounces: u32, voices: u32, boxes: u32, scatter: f32, maxDist: f32, w: f32, refDist: f32 };
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> boxes: array<Box>;
@group(0) @binding(2) var<storage, read_write> nodes: array<Node>;
@group(0) @binding(3) var<storage, read_write> rayStats: array<vec4f>;  // per ray: escaped0, leaked, firstToi, segSum ; segCount in node 0's alpha
@group(0) @binding(4) var<storage, read> srcs: array<vec4f>;
@group(0) @binding(5) var<storage, read_write> partial: array<vec4f>;    // per (voice, workgroup): energy, dir
@group(0) @binding(6) var<storage, read_write> sums: array<vec4f>;       // [0..3] space sums, then per voice
@group(0) @binding(7) var<storage, read_write> rayAbs: array<vec4f>;     // per ray: absFirst, hit0, energy at bounce 1, at last bounce

struct Hit { t: f32, n: vec3f, a: f32, ok: bool };

fn castRay(o: vec3f, d: vec3f, maxT: f32) -> Hit {
  var best = Hit(maxT, vec3f(0.0), 0.0, false);
  for (var i = 0u; i < P.boxes; i++) {
    let b = boxes[i];
    let rel = o - b.c.xyz;
    let lo = vec3f(dot(rel, b.ax.xyz), dot(rel, b.ay.xyz), dot(rel, b.az.xyz));
    let ld = vec3f(dot(d, b.ax.xyz), dot(d, b.ay.xyz), dot(d, b.az.xyz));
    let inv = 1.0 / ld;
    let t1 = (-b.h.xyz - lo) * inv;
    let t2 = (b.h.xyz - lo) * inv;
    let tmin = min(t1, t2);
    let tmax = max(t1, t2);
    let t0 = max(max(tmin.x, tmin.y), tmin.z);
    let tf = min(min(tmax.x, tmax.y), tmax.z);
    if (t0 > tf || tf < 0.0) { continue; }
    if (t0 < 0.0) { return Hit(0.0, -d, b.c.w, true); }
    if (t0 < best.t) {
      var n = b.ax.xyz; var s = ld.x;
      if (t0 == tmin.y) { n = b.ay.xyz; s = ld.y; }
      if (t0 == tmin.z) { n = b.az.xyz; s = ld.z; }
      if (s > 0.0) { n = -n; }
      best = Hit(t0, n, b.c.w, true);
    }
  }
  return best;
}

fn hash(a: f32, b: f32) -> f32 { let s = sin(a * 12.9898 + b * 78.233) * 43758.5453; return s - floor(s); }

fn cosineAround(n: vec3f, u1: f32, u2: f32) -> vec3f {
  var t = vec3f(1.0, 0.0, 0.0);
  if (abs(n.y) < 0.9) { t = vec3f(0.0, 1.0, 0.0); }
  let tx = normalize(cross(t, n));
  let ty = cross(n, tx);
  let r = sqrt(u1);
  let phi = 6.2831853 * u2;
  return normalize(tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - u1)));
}

@compute @workgroup_size(64)
fn trace(@builtin(global_invocation_id) gid: vec3u) {
  let i = gid.x;
  if (i >= P.rays) { return; }
  let fi = f32(i);
  let y = 1.0 - (2.0 * (fi + 0.5)) / f32(P.rays);
  let r = sqrt(max(0.0, 1.0 - y * y));
  let a = fi * 2.39996323;
  var dir = vec3f(r * cos(a), y, r * sin(a));
  var o = P.listener.xyz;
  var path = 0.0; var thr = 1.0;
  var st = vec4f(0.0); var ab = vec4f(0.0);
  for (var k = 1u; k <= P.bounces; k++) {
    let idx = i * P.bounces + (k - 1u);
    let h = castRay(o, dir, P.maxDist);
    if (!h.ok) {
      if (k == 1u) { st.x = 1.0; }
      st.y = 1.0;
      for (var m = k; m <= P.bounces; m++) { nodes[i * P.bounces + (m - 1u)].n = vec4f(0.0); }
      break;
    }
    var n = normalize(h.n);
    if (dot(n, dir) > 0.0) { n = -n; }
    path += h.t;
    if (k == 1u) { st.z = h.t; ab.x = h.a; ab.y = 1.0; ab.z = thr; } else { st.w += h.t; }
    if (k == P.bounces) { ab.w = thr; }
    let hp = o + dir * h.t;
    // p.w: the energy this surface sends on, throughput × (1 − α) (what the energy pass weighs).
    thr *= max(0.0, 1.0 - h.a);
    nodes[idx] = Node(vec4f(hp, thr), vec4f(n, path));
    let spec = dir - n * (2.0 * dot(dir, n));
    let diffuse = cosineAround(n, hash(fi, f32(k)), hash(f32(k) + 17.0, fi + 3.0));
    dir = normalize(spec * (1.0 - P.scatter) + diffuse * P.scatter);
    if (dot(dir, n) <= 0.01) { dir = diffuse; }
    o = hp + n * 1e-3;
  }
  rayStats[i] = st;
  rayAbs[i] = ab;
}

var<workgroup> red: array<vec4f, 64>;

// One thread per (node, voice): does the bounce point see the source? Lambert energy toward the listener.
@compute @workgroup_size(64)
fn nee(@builtin(global_invocation_id) gid: vec3u, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3u) {
  let j = gid.x;
  let v = gid.y;
  var c = vec4f(0.0);
  let total = P.rays * P.bounces;
  if (j < total) {
    let node = nodes[j];
    let ray = j / P.bounces;
    let order = j % P.bounces + 1u;
    if (dot(node.n.xyz, node.n.xyz) > 0.5) {
      let s = srcs[v].xyz;
      let toS = s - node.p.xyz;
      let rr = length(toS);
      let d = toS / max(rr, 1e-6);
      let cosv = dot(node.n.xyz, d);
      let direct = length(s - P.listener.xyz);
      let tt = (node.n.w + rr - direct) / 343.0;
      if (rr >= 0.05 && cosv > 0.0 && tt >= 0.005) {
        var seen = true;
        if (rr > 0.25) { seen = !castRay(node.p.xyz + node.n.xyz * 0.02, d, rr - 0.2).ok; }
        if (seen) {
          var share = 1.0;
          if (order == 1u) { share = 0.35; }
          let e = share * P.w * node.p.w * cosv * P.refDist * P.refDist / (3.14159265 * max(rr, 0.5) * max(rr, 0.5));
          let fi = f32(ray);
          let y = 1.0 - (2.0 * (fi + 0.5)) / f32(P.rays);
          let r = sqrt(max(0.0, 1.0 - y * y));
          let a = fi * 2.39996323;
          c = vec4f(e, e * r * cos(a), e * y, e * r * sin(a));
        }
      }
    }
  }
  red[li] = c;
  workgroupBarrier();
  for (var s = 32u; s > 0u; s >>= 1u) {
    if (li < s) { red[li] += red[li + s]; }
    workgroupBarrier();
  }
  if (li == 0u) { partial[v * ((total + 63u) / 64u) + wg.x] = red[0]; }
}

// One workgroup per voice (y), plus workgroup y = voices for the listener's space sums.
@compute @workgroup_size(64)
fn reduce(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3u) {
  let v = wg.y;
  var acc = vec4f(0.0);
  var acc2 = vec4f(0.0);
  if (v < P.voices) {
    let groups = (P.rays * P.bounces + 63u) / 64u;
    for (var g = li; g < groups; g += 64u) { acc += partial[v * groups + g]; }
  } else {
    for (var i = li; i < P.rays; i += 64u) { acc += rayStats[i]; acc2 += rayAbs[i]; }
  }
  red[li] = acc;
  workgroupBarrier();
  for (var s = 32u; s > 0u; s >>= 1u) { if (li < s) { red[li] += red[li + s]; } workgroupBarrier(); }
  if (li == 0u) {
    if (v < P.voices) { sums[2u + v] = red[0]; } else { sums[0] = red[0]; }
  }
  workgroupBarrier();
  if (v >= P.voices) {
    red[li] = acc2;
    workgroupBarrier();
    for (var s = 32u; s > 0u; s >>= 1u) { if (li < s) { red[li] += red[li + s]; } workgroupBarrier(); }
    if (li == 0u) { sums[1] = red[0]; }
  }
}
`;

export interface GpuTraceResult {
  /** Share of rays that hit nothing, share that left within the bounces, mean free path proxy. */
  escape: number;
  leak: number;
  meanFirstHit: number;
  /** Per voice: diffuse energy re the direct sound at 2 m (first bounce at the scattered share) and its direction. */
  voices: Array<{ energy: number; direction: Vec3 }>;
  /** Submit to mapped result (ms): the read-back latency the engine would see. */
  latencyMs: number;
  /** GPU time of the passes (ms) when the device has timestamp queries, else NaN. */
  gpuMs: number;
}

export interface GpuTracer {
  trace(listener: Vec3, sources: Vec3[], o?: { rays?: number; bounces?: number }): Promise<GpuTraceResult>;
  setGeometry(g: SfxGeometry): void;
  dispose(): void;
}

/** Boxes as the shader reads them: centre + α, half extents + id, the three axes. */
function packBoxes(g: SfxGeometry): Float32Array {
  const n = g.boxes.length / BOX_STRIDE;
  const out = new Float32Array(Math.max(1, n) * 20);
  for (let i = 0; i < n; i++) {
    const r = g.boxes.subarray(i * BOX_STRIDE, i * BOX_STRIDE + BOX_STRIDE);
    const [x, y, z, w] = [r[6], r[7], r[8], r[9]];
    const ax = [1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)];
    const ay = [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)];
    const az = [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)];
    out.set([r[0], r[1], r[2], r[10], r[3], r[4], r[5], r[11], ...ax, 0, ...ay, 0, ...az, 0], i * 20);
  }
  return out;
}

export async function createGpuTracer(device: GpuDevice, geometry: SfxGeometry): Promise<GpuTracer> {
  const module = device.createShaderModule({ code: WGSL });
  const info = await module.getCompilationInfo();
  const errors = info.messages.filter((m: { type: string }) => m.type === 'error');
  if (errors.length) throw new Error('[haa] gpu-trace WGSL: ' + errors.map((m: { lineNum: number; linePos: number; message: string }) => `${m.lineNum}:${m.linePos} ${m.message}`).join('; '));
  const layout = device.createBindGroupLayout({
    entries: [
      { binding: 0, visibility: STAGE_COMPUTE, buffer: { type: 'uniform' } },
      { binding: 1, visibility: STAGE_COMPUTE, buffer: { type: 'read-only-storage' } },
      ...[2, 3].map((b) => ({ binding: b, visibility: STAGE_COMPUTE, buffer: { type: 'storage' as const } })),
      { binding: 4, visibility: STAGE_COMPUTE, buffer: { type: 'read-only-storage' } },
      ...[5, 6, 7].map((b) => ({ binding: b, visibility: STAGE_COMPUTE, buffer: { type: 'storage' as const } })),
    ],
  });
  const pl = device.createPipelineLayout({ bindGroupLayouts: [layout] });
  const pipe = (entryPoint: string) => device.createComputePipeline({ layout: pl, compute: { module, entryPoint } });
  const [pTrace, pNee, pReduce] = [pipe('trace'), pipe('nee'), pipe('reduce')];
  const timestamps = device.features.has('timestamp-query');
  const qset = timestamps ? device.createQuerySet({ type: 'timestamp', count: 2 }) : null;
  const qbuf = timestamps ? device.createBuffer({ size: 16, usage: BUF.QUERY_RESOLVE | BUF.COPY_SRC }) : null;
  const qread = timestamps ? device.createBuffer({ size: 16, usage: BUF.MAP_READ | BUF.COPY_DST }) : null;
  const params = device.createBuffer({ size: 48, usage: BUF.UNIFORM | BUF.COPY_DST });
  let boxBuf: GpuBuffer;
  let nBoxes = 0;
  const setGeometry = (g: SfxGeometry): void => {
    const data = packBoxes(g);
    boxBuf?.destroy();
    boxBuf = device.createBuffer({ size: data.byteLength, usage: BUF.STORAGE | BUF.COPY_DST });
    device.queue.writeBuffer(boxBuf, 0, data);
    nBoxes = g.boxes.length / BOX_STRIDE;
  };
  setGeometry(geometry);
  let cap = { rays: 0, bounces: 0, voices: 0 };
  let bufs: Record<'nodes' | 'rayStats' | 'srcs' | 'partial' | 'sums' | 'rayAbs' | 'read', GpuBuffer> | null = null;
  const ensure = (rays: number, bounces: number, voices: number): void => {
    if (bufs && rays <= cap.rays && bounces <= cap.bounces && voices <= cap.voices) return;
    if (bufs) for (const b of Object.values(bufs)) b.destroy();
    cap = { rays: Math.max(rays, cap.rays), bounces: Math.max(bounces, cap.bounces), voices: Math.max(voices, cap.voices, 1) };
    const S = BUF.STORAGE;
    const groups = Math.ceil((cap.rays * cap.bounces) / 64);
    bufs = {
      nodes: device.createBuffer({ size: cap.rays * cap.bounces * 32, usage: S }),
      rayStats: device.createBuffer({ size: cap.rays * 16, usage: S }),
      rayAbs: device.createBuffer({ size: cap.rays * 16, usage: S }),
      srcs: device.createBuffer({ size: cap.voices * 16, usage: S | BUF.COPY_DST }),
      partial: device.createBuffer({ size: cap.voices * groups * 16, usage: S }),
      sums: device.createBuffer({ size: (2 + cap.voices) * 16, usage: S | BUF.COPY_SRC }),
      read: device.createBuffer({ size: (2 + cap.voices) * 16, usage: BUF.MAP_READ | BUF.COPY_DST }),
    };
  };
  return {
    setGeometry,
    async trace(listener, sources, o = {}) {
      const rays = o.rays ?? 256;
      const bounces = o.bounces ?? 6;
      const voices = sources.length;
      ensure(rays, bounces, voices);
      const b = bufs!;
      const p = new ArrayBuffer(48);
      new Float32Array(p, 0, 4).set([listener.x, listener.y, listener.z, 0]);
      new Uint32Array(p, 16, 4).set([rays, bounces, voices, nBoxes]);
      new Float32Array(p, 32, 4).set([0.35, 60, (4 * Math.PI) / rays, 2]);
      device.queue.writeBuffer(params, 0, p);
      device.queue.writeBuffer(b.srcs, 0, new Float32Array(sources.flatMap((s) => [s.x, s.y, s.z, 0])));
      const bind = device.createBindGroup({
        layout,
        entries: [params, boxBuf, b.nodes, b.rayStats, b.srcs, b.partial, b.sums, b.rayAbs].map((buffer, binding) => ({ binding, resource: { buffer } })),
      });
      const t0 = performance.now();
      const enc = device.createCommandEncoder();
      const pass = enc.beginComputePass(qset ? { timestampWrites: { querySet: qset, beginningOfPassWriteIndex: 0, endOfPassWriteIndex: 1 } } : {});
      pass.setBindGroup(0, bind);
      pass.setPipeline(pTrace);
      pass.dispatchWorkgroups(Math.ceil(rays / 64));
      if (voices > 0) {
        pass.setPipeline(pNee);
        pass.dispatchWorkgroups(Math.ceil((rays * bounces) / 64), voices);
      }
      pass.setPipeline(pReduce);
      pass.dispatchWorkgroups(1, voices + 1);
      pass.end();
      enc.copyBufferToBuffer(b.sums, 0, b.read, 0, (2 + voices) * 16);
      if (qset && qbuf && qread) {
        enc.resolveQuerySet(qset, 0, 2, qbuf, 0);
        enc.copyBufferToBuffer(qbuf, 0, qread, 0, 16);
      }
      device.queue.submit([enc.finish()]);
      await b.read.mapAsync(MAP_READ);
      const latencyMs = performance.now() - t0;
      const s = new Float32Array(b.read.getMappedRange().slice(0));
      b.read.unmap();
      let gpuMs = NaN;
      if (qread) {
        await qread.mapAsync(MAP_READ);
        const q = new BigUint64Array(qread.getMappedRange().slice(0));
        qread.unmap();
        gpuMs = Number(q[1] - q[0]) / 1e6;
      }
      const hits0 = s[5];
      return {
        escape: s[0] / rays,
        leak: s[1] / rays,
        meanFirstHit: hits0 > 0 ? s[2] / hits0 : 0,
        voices: sources.map((_, v) => {
          const e = s[8 + v * 4];
          return { energy: e, direction: e > 0 ? { x: s[9 + v * 4] / e, y: s[10 + v * 4] / e, z: s[11 + v * 4] / e } : { x: 0, y: 0, z: 0 } };
        }),
        latencyMs,
        gpuMs,
      };
    },
    dispose() {
      if (bufs) for (const b of Object.values(bufs)) b.destroy();
      boxBuf?.destroy();
      params.destroy();
    },
  };
}
