//! haa-trace: the acoustic ray tracer of the sfx runtime (docs/physics-sound.md, "Acoustic model"),
//! off the TypeScript main thread. It owns its geometry (oriented boxes and triangles handed over by
//! a geometry provider: Rapier's fixed colliders today, RAA's level later) in a BVH and runs the
//! four traces of `raytrace.ts` / `acoustics.ts` in one call each, so a whole update crosses the
//! wasm boundary a handful of times instead of once per ray.
//!
//! The wasm ABI (`trace_*`) is plain `extern "C"` over linear memory, no imports, like haa-wasm:
//! - `trace_alloc(floats)` returns a scratch f32 area the host fills with box / triangle records;
//! - `trace_scene(boxes_ptr, n_boxes, tris_ptr, n_tris)` builds the BVH;
//! - each query writes its result as f64 into `trace_out()` (layouts in `packages/haa/src/sfx/trace-wasm.ts`).

pub mod geom;
pub mod trace;

pub use geom::{Scene, V3, v3};
pub use trace::{Caster, ListenerTrace, PathScratch, Reflection, ReverbOptions};

/// Everything a host instance keeps between calls.
pub struct Tracer<C: Caster> {
    pub caster: C,
    pub trace: ListenerTrace,
    pub has_trace: bool,
    arrivals: Vec<(f64, f64)>,
    scratch: PathScratch,
    refl: Vec<Reflection>,
    refl_dirs: Vec<V3>,
}

impl<C: Caster> Tracer<C> {
    pub fn new(caster: C) -> Self {
        Tracer { caster, trace: ListenerTrace::default(), has_trace: false, arrivals: Vec::new(), scratch: PathScratch::default(), refl: Vec::new(), refl_dirs: Vec::new() }
    }

    pub fn listener(&mut self, l: V3, rays: usize, bounces: usize, max_dist: f64, scatter: f64) -> trace::Space {
        trace::trace_listener(&mut self.caster, &mut self.trace, l, rays, bounces, max_dist, scatter);
        self.has_trace = true;
        self.trace.space
    }

    pub fn reverb(&mut self, s: V3, o: &ReverbOptions) -> trace::VoiceReverb {
        trace::voice_reverb(&mut self.caster, &self.trace, s, o, &mut self.arrivals)
    }

    pub fn path(&mut self, s: V3, l: V3, extent: f64, end_skip: f64, source_rays: usize, use_trace: bool) -> trace::DirectPath {
        let t = if use_trace && self.has_trace { Some(&self.trace) } else { None };
        trace::direct_path(&mut self.caster, s, l, t, extent, end_skip, source_rays, &mut self.scratch)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reflections(&mut self, s: V3, l: V3, dirs: usize, max: usize, max_dist: f64, soft: f64, min_delay: f64, scattering: f64) -> &[Reflection] {
        if self.refl_dirs.len() != dirs {
            self.refl_dirs = trace::sphere_directions(dirs);
        }
        trace::early_reflections(&mut self.caster, s, l, &self.refl_dirs, max, max_dist, soft, min_delay, scattering, &mut self.refl);
        &self.refl
    }
}

// --- wasm ABI --------------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod abi {
    use super::*;
    use std::cell::RefCell;

    /// Results of the last query (f64), read by the host at `trace_out()`.
    const OUT_LEN: usize = 4096;

    struct State {
        tracer: Tracer<Scene>,
        input: Vec<f32>,
        out: Vec<f64>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State { tracer: Tracer::new(Scene::new(&[], &[])), input: Vec::new(), out: vec![0.0; OUT_LEN] });
    }

    fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
        STATE.with(|s| f(&mut s.borrow_mut()))
    }

    /// Scratch area of `floats` f32 for the scene records; valid until the next call.
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_alloc(floats: u32) -> u32 {
        with(|s| {
            s.input.clear();
            s.input.resize(floats as usize, 0.0);
            s.input.as_mut_ptr() as u32
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn trace_out() -> u32 {
        with(|s| s.out.as_mut_ptr() as u32)
    }

    /// Builds the scene from the scratch area: `n_boxes` box records, then `n_tris` triangle records.
    /// Returns the BVH node count.
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_scene(n_boxes: u32, n_tris: u32) -> u32 {
        with(|s| {
            let nb = n_boxes as usize * geom::BOX_STRIDE;
            let nt = n_tris as usize * geom::TRI_STRIDE;
            let end = (nb + nt).min(s.input.len());
            let scene = Scene::new(&s.input[..nb.min(end)], &s.input[nb.min(end)..end]);
            let nodes = scene.node_count() as u32;
            s.tracer = Tracer::new(scene);
            nodes
        })
    }

    /// Ray casts since the scene was built (f64: it may pass 2^32).
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_casts() -> f64 {
        with(|s| s.tracer.caster.casts as f64)
    }

    /// One cast; out = [toi, px, py, pz, nx, ny, nz, absorption, id]. Returns 1 on a hit.
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_cast(ox: f64, oy: f64, oz: f64, dx: f64, dy: f64, dz: f64, max: f64) -> u32 {
        with(|s| match s.tracer.caster.cast(v3(ox, oy, oz), v3(dx, dy, dz), max) {
            None => 0,
            Some(h) => {
                s.out[..9].copy_from_slice(&[h.toi, h.point.x, h.point.y, h.point.z, h.normal.x, h.normal.y, h.normal.z, h.absorption, f64::from(h.id)]);
                1
            }
        })
    }

    /// `n` casts from the scratch area (7 f32 each: origin, unit dir, max); writes the distance of
    /// each hit (or -1) back over the scratch area's first `n` floats. A batch benchmark.
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_cast_batch(n: u32) -> u32 {
        with(|s| {
            let mut hits = 0;
            for i in 0..n as usize {
                let r = &s.input[i * 7..i * 7 + 7];
                let f = |k: usize| f64::from(r[k]);
                let t = s.tracer.caster.cast(v3(f(0), f(1), f(2)), v3(f(3), f(4), f(5)), f(6)).map_or(-1.0, |h| h.toi);
                if t >= 0.0 {
                    hits += 1;
                }
                s.input[i] = t as f32;
            }
            hits
        })
    }

    /// The listener trace; out = [escape, leak, meanFreePath, decayPerBounce, rt60, volume,
    /// absorption, candidates, nodes].
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_listener(lx: f64, ly: f64, lz: f64, rays: u32, bounces: u32, max_dist: f64, scatter: f64) {
        with(|s| {
            let sp = s.tracer.listener(v3(lx, ly, lz), rays as usize, bounces.min(32) as usize, max_dist, scatter);
            let t = &s.tracer.trace;
            s.out[..9].copy_from_slice(&[sp.escape, sp.leak, sp.mean_free_path, sp.decay_per_bounce, sp.rt60, sp.volume, sp.absorption, t.candidates.len() as f64, t.nodes.len() as f64]);
        })
    }

    /// First legs of every `every`-th ray of the last trace, for drawing: out = [n, then per ray
    /// x0 y0 z0 x1 y1 z1 hit]; an escaped ray ends 12 m out.
    #[unsafe(no_mangle)]
    pub extern "C" fn trace_listener_rays(every: u32) -> u32 {
        with(|s| {
            let t = &s.tracer.trace;
            let mut n = 0usize;
            let every = every.max(1) as usize;
            for (i, &(first, count, _)) in t.rays.iter().enumerate().step_by(every) {
                let base = 1 + n * 7;
                if base + 7 > s.out.len() {
                    break;
                }
                let (to, hit) = if count > 0 { (t.nodes[first as usize].point, 1.0) } else { (t.origin + t.dirs[i] * 12.0, 0.0) };
                s.out[base..base + 7].copy_from_slice(&[t.origin.x, t.origin.y, t.origin.z, to.x, to.y, to.z, hit]);
                n += 1;
            }
            s.out[0] = n as f64;
            n as u32
        })
    }

    /// One source's diffuse field from the last trace; out = [energy, rt60, dx, dy, dz, predelay,
    /// absorption, hits].
    #[unsafe(no_mangle)]
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn trace_voice_reverb(sx: f64, sy: f64, sz: f64, stride: u32, max_order: u32, reference: f64, max_decay: f64, min_delay: f64, first_share: f64) {
        with(|s| {
            let o = ReverbOptions { stride: stride as usize, max_order, reference, max_decay, min_delay, first_share };
            let r = s.tracer.reverb(v3(sx, sy, sz), &o);
            s.out[..8].copy_from_slice(&[r.energy, r.rt60, r.direction.x, r.direction.y, r.direction.z, r.predelay, r.absorption, f64::from(r.hits)]);
        })
    }

    /// The direct path; out = [visible, excess, viax, viay, viaz, length, found, midx, midy, midz].
    #[unsafe(no_mangle)]
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn trace_direct_path(sx: f64, sy: f64, sz: f64, lx: f64, ly: f64, lz: f64, extent: f64, end_skip: f64, source_rays: u32, use_trace: u32) {
        with(|s| {
            let p = s.tracer.path(v3(sx, sy, sz), v3(lx, ly, lz), extent, end_skip, source_rays as usize, use_trace != 0);
            s.out[..10].copy_from_slice(&[p.visible, p.excess, p.via.x, p.via.y, p.via.z, p.length, if p.found { 1.0 } else { 0.0 }, p.mid.x, p.mid.y, p.mid.z]);
        })
    }

    /// First-order image sources; returns the count, out = per reflection 16 values: image (3),
    /// point (3), path, delay, gain, absorption, id, normal (3), weight, 0.
    #[unsafe(no_mangle)]
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn trace_reflections(sx: f64, sy: f64, sz: f64, lx: f64, ly: f64, lz: f64, dirs: u32, max: u32, max_dist: f64, soft: f64, min_delay: f64, scattering: f64) -> u32 {
        with(|s| {
            let State { tracer, out, .. } = s;
            let r = tracer.reflections(v3(sx, sy, sz), v3(lx, ly, lz), dirs as usize, (max as usize).min(64), max_dist, soft, min_delay, scattering);
            for (i, x) in r.iter().enumerate() {
                out[i * 16..i * 16 + 16].copy_from_slice(&[
                    x.image.x, x.image.y, x.image.z, x.point.x, x.point.y, x.point.z, x.path, x.delay, x.gain, x.absorption, f64::from(x.id), x.normal.x, x.normal.y, x.normal.z, x.weight, 0.0,
                ]);
            }
            r.len() as u32
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geom::BOX_STRIDE;

    /// A closed 10 x 4 x 10 m concrete room (6 slabs) with a 2 m door in its +x wall, as box records.
    fn room() -> Vec<f32> {
        let mut b = Vec::new();
        let mut slab = |c: [f32; 3], h: [f32; 3]| b.extend_from_slice(&[c[0], c[1], c[2], h[0], h[1], h[2], 0.0, 0.0, 0.0, 1.0, 0.02, (b.len() / BOX_STRIDE) as f32]);
        slab([0.0, -0.1, 0.0], [5.2, 0.1, 5.2]); // floor
        slab([0.0, 4.1, 0.0], [5.2, 0.1, 5.2]); // roof
        slab([-5.1, 2.0, 0.0], [0.1, 2.0, 5.2]);
        slab([0.0, 2.0, -5.1], [5.2, 2.0, 0.1]);
        slab([0.0, 2.0, 5.1], [5.2, 2.0, 0.1]);
        slab([5.1, 2.0, -3.0], [0.1, 2.0, 2.0]); // +x wall, door between z -1 and 1
        slab([5.1, 2.0, 3.0], [0.1, 2.0, 2.0]);
        b
    }

    #[test]
    fn room_reverb_and_door_path() {
        let mut t = Tracer::new(Scene::new(&room(), &[]));
        // A ray from the centre to +y hits the roof's underside at 3.4 m with a downward normal.
        let h = t.caster.cast(v3(0.0, 0.6, 0.0), v3(0.0, 1.0, 0.0), 60.0).expect("roof");
        assert!((h.toi - 3.4).abs() < 1e-6 && (h.normal.y + 1.0).abs() < 1e-9 && h.id == 1);
        // Inside the closed room nothing escapes on the first leg except through the door.
        let space = t.listener(v3(0.0, 1.6, 0.0), 256, 6, 60.0, 0.35);
        assert!(space.escape < 0.05, "escape {}", space.escape);
        assert!(space.rt60 > 1.0, "a bare concrete room rings: rt60 {}", space.rt60);
        // A source outside, in front of the door but off-axis: hidden, reached around the door jamb.
        let p = t.path(v3(8.0, 1.0, 4.0), v3(0.0, 1.6, 0.0), 0.0, 0.2, 48, true);
        assert!(p.visible < 1.0 && p.found && p.excess > 0.0, "{p:?}");
    }
}
