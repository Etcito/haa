//! The acoustic traces of `packages/haa/src/sfx/raytrace.ts` and `acoustics.ts`, ported one to one
//! (same constants, same order of operations, f64), so the TS engine can use either: the listener
//! trace, one source's diffuse field, the direct path (occlusion, bend around the obstacles) and the
//! first-order image sources. Generic over the ray cast: this crate's BVH, or Rapier's query pipeline
//! (feature `rapier`).

use crate::geom::{Hit, V3, v3};

pub const SPEED_OF_SOUND_M_S: f64 = 343.0;
pub const TRACE_RT60_RANGE: (f64, f64) = (0.1, 4.5);
const CANDIDATE_STEPS: [f64; 9] = [0.6, 1.2, 2.0, 3.0, 4.5, 6.5, 9.0, 13.0, 18.0];

/// Anything that answers a ray cast: nearest hit along the unit `d` within `max`.
pub trait Caster {
    fn cast(&mut self, o: V3, d: V3, max: f64) -> Option<Hit>;
}

impl Caster for crate::geom::Scene {
    fn cast(&mut self, o: V3, d: V3, max: f64) -> Option<Hit> {
        crate::geom::Scene::cast(self, o, d, max)
    }
}

/// `n` unit vectors spread evenly over the sphere (Fibonacci lattice), as `sphereDirections`.
pub fn sphere_directions(n: usize) -> Vec<V3> {
    let golden = core::f64::consts::PI * (3.0 - 5f64.sqrt());
    (0..n)
        .map(|i| {
            let y = 1.0 - (2.0 * (i as f64 + 0.5)) / n as f64;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = i as f64 * golden;
            v3(r * a.cos(), y, r * a.sin())
        })
        .collect()
}

fn hash(a: f64, b: f64) -> f64 {
    let s = (a * 12.9898 + b * 78.233).sin() * 43758.5453;
    s - s.floor()
}

fn cosine_around(n: V3, u1: f64, u2: f64) -> V3 {
    let t = if n.y.abs() < 0.9 { v3(0.0, 1.0, 0.0) } else { v3(1.0, 0.0, 0.0) };
    let tx = t.cross(n).normalize();
    let ty = n.cross(tx);
    let r = u1.sqrt();
    let phi = 2.0 * core::f64::consts::PI * u2;
    let z = (1.0 - u1).max(0.0).sqrt();
    (tx * (r * phi.cos()) + ty * (r * phi.sin()) + n * z).normalize()
}

#[derive(Clone, Copy, Debug)]
pub struct TraceNode {
    pub point: V3,
    pub normal: V3,
    pub absorption: f64,
    pub path: f64,
    pub throughput: f64,
    pub order: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Space {
    pub escape: f64,
    pub leak: f64,
    pub mean_free_path: f64,
    pub decay_per_bounce: f64,
    pub rt60: f64,
    pub volume: f64,
    pub absorption: f64,
}

/// The listener trace: rays (their nodes in one flat list), candidates, the space estimate.
#[derive(Default)]
pub struct ListenerTrace {
    pub origin: V3,
    pub dirs: Vec<V3>,
    pub nodes: Vec<TraceNode>,
    /// Per ray: first node index, node count, escape order (-1 never).
    pub rays: Vec<(u32, u32, i32)>,
    pub candidates: Vec<V3>,
    pub space: Space,
}

pub fn rt60_from(decay: f64, mfp: f64) -> f64 {
    let (lo, hi) = TRACE_RT60_RANGE;
    if decay <= 0.0 || decay.is_nan() {
        return lo;
    }
    let db = -10.0 * decay.log10();
    if db <= 1e-6 {
        return hi;
    }
    ((60.0 * mfp) / (SPEED_OF_SOUND_M_S * db)).clamp(lo, hi)
}

pub fn trace_listener<C: Caster>(c: &mut C, out: &mut ListenerTrace, listener: V3, n: usize, bounces: usize, max_dist: f64, scatter: f64) {
    if out.dirs.len() != n {
        out.dirs = sphere_directions(n);
    }
    out.origin = listener;
    out.nodes.clear();
    out.rays.clear();
    out.candidates.clear();
    let mut energy_at = vec![0.0f64; bounces + 1];
    let (mut escaped0, mut leaked, mut seg_sum, mut seg_count, mut vol_sum, mut abs_sum, mut hits0) = (0usize, 0usize, 0.0, 0usize, 0.0, 0.0, 0usize);
    for i in 0..n {
        let first = out.nodes.len() as u32;
        let mut escaped_at = -1i32;
        let mut origin = listener;
        let mut dir = out.dirs[i];
        let (mut path, mut throughput) = (0.0f64, 1.0f64);
        for k in 1..=bounces {
            let hit = c.cast(origin, dir, max_dist);
            if k == 1 {
                let free = hit.map_or(20.0, |h| h.toi - 0.3);
                for s in CANDIDATE_STEPS {
                    if s < free {
                        out.candidates.push(listener + dir * s);
                    }
                }
                if let Some(h) = hit
                    && h.toi > 0.9
                {
                    out.candidates.push(listener + dir * (h.toi - 0.3));
                }
            }
            let Some(h) = hit else {
                escaped_at = k as i32 - 1;
                if k == 1 {
                    escaped0 += 1;
                }
                leaked += 1;
                break;
            };
            let mut nrm = h.normal.normalize();
            if nrm.dot(dir) > 0.0 {
                nrm = -nrm;
            }
            path += h.toi;
            if k == 1 {
                hits0 += 1;
                vol_sum += (4.0 / 3.0) * core::f64::consts::PI * h.toi.powi(3);
                abs_sum += h.absorption;
            } else {
                seg_sum += h.toi;
                seg_count += 1;
            }
            energy_at[k] += throughput;
            out.nodes.push(TraceNode { point: h.point, normal: nrm, absorption: h.absorption, path, throughput, order: k as u32 });
            throughput *= (1.0 - h.absorption).max(0.0);
            let spec = dir - nrm * (2.0 * dir.dot(nrm));
            let diffuse = cosine_around(nrm, hash(i as f64, k as f64), hash(k as f64 + 17.0, i as f64 + 3.0));
            dir = (spec * (1.0 - scatter) + diffuse * scatter).normalize();
            if dir.dot(nrm) <= 0.01 {
                dir = diffuse;
            }
            origin = h.point + nrm * 1e-3;
        }
        out.rays.push((first, out.nodes.len() as u32 - first, escaped_at));
    }
    let escape = escaped0 as f64 / n as f64;
    let mfp = if seg_count > 0 {
        seg_sum / seg_count as f64
    } else if hits0 > 0 {
        (vol_sum / hits0 as f64 / ((4.0 / 3.0) * core::f64::consts::PI)).cbrt()
    } else {
        max_dist
    };
    let (first, last) = (energy_at[1], energy_at[bounces]);
    let decay = if first > 0.0 && bounces > 1 { (last.max(1e-9) / first).powf(1.0 / (bounces as f64 - 1.0)).min(0.995) } else { 0.0 };
    out.space = Space {
        escape,
        leak: leaked as f64 / n as f64,
        mean_free_path: mfp,
        decay_per_bounce: decay,
        rt60: rt60_from(decay, mfp),
        volume: if hits0 > 0 { (vol_sum / hits0 as f64) * (1.0 - escape) } else { 0.0 },
        absorption: if hits0 > 0 { abs_sum / hits0 as f64 } else { 1.0 },
    };
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VoiceReverb {
    pub energy: f64,
    pub rt60: f64,
    pub direction: V3,
    pub predelay: f64,
    pub absorption: f64,
    pub hits: u32,
}

pub struct ReverbOptions {
    pub stride: usize,
    pub max_order: u32,
    pub reference: f64,
    pub max_decay: f64,
    pub min_delay: f64,
    pub first_share: f64,
}

pub fn voice_reverb<C: Caster>(c: &mut C, t: &ListenerTrace, source: V3, o: &ReverbOptions, arrivals: &mut Vec<(f64, f64)>) -> VoiceReverb {
    let stride = o.stride.max(1);
    arrivals.clear();
    let used = t.rays.len().div_ceil(stride);
    let w = (4.0 * core::f64::consts::PI) / used as f64;
    let direct = (source - t.origin).length();
    let mut by_order = [0.0f64; 33];
    let (mut total, mut dsum, mut abs_w, mut hits) = (0.0f64, v3(0.0, 0.0, 0.0), 0.0f64, 0u32);
    let (mut w_log_kept, mut w_esc, mut w_seg, mut w_seg_n, mut w_bounces, mut max_k) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
    let mut i = 0;
    while i < t.rays.len() {
        let (first, count, escaped_at) = t.rays[i];
        let nodes = &t.nodes[first as usize..(first + count) as usize];
        let mut ray_e = 0.0;
        for node in nodes {
            if node.order > o.max_order {
                break;
            }
            let to_s = source - node.point;
            let r = to_s.length();
            if r < 0.05 {
                continue;
            }
            let d = to_s * (1.0 / r);
            let cos = node.normal.dot(d);
            if cos <= 0.0 {
                continue;
            }
            let tt = (node.path + r - direct) / SPEED_OF_SOUND_M_S;
            if tt < o.min_delay {
                continue;
            }
            let from = node.point + node.normal * 0.02;
            if r > 0.25 && c.cast(from, d, r - 0.2).is_some() {
                continue;
            }
            let share = if node.order == 1 { o.first_share } else { 1.0 };
            let cc = (share * w * node.throughput * (1.0 - node.absorption) * cos * o.reference * o.reference) / (core::f64::consts::PI * r.max(0.5).powi(2));
            if cc <= 0.0 || cc.is_nan() {
                continue;
            }
            hits += 1;
            ray_e += cc;
            total += cc;
            let k = (node.order as usize).min(32);
            by_order[k] += cc;
            max_k = max_k.max(k);
            dsum = dsum + t.dirs[i] * cc;
            abs_w += cc * node.absorption;
            arrivals.push((tt, cc));
        }
        if ray_e > 0.0 && nodes.len() >= 2 {
            let (a, z) = (nodes[0], nodes[nodes.len() - 1]);
            w_log_kept += ray_e * (z.throughput * (1.0 - z.absorption)).max(1e-9).ln();
            w_esc += if escaped_at >= 0 { ray_e } else { 0.0 };
            w_seg += ray_e * ((z.path - a.path) / (nodes.len() as f64 - 1.0));
            w_seg_n += ray_e;
            w_bounces += ray_e * nodes.len() as f64;
        }
        i += stride;
    }
    if total <= 0.0 || total.is_nan() {
        return VoiceReverb { energy: 0.0, rt60: t.space.rt60, direction: v3(0.0, 0.0, 0.0), predelay: 0.0, absorption: t.space.absorption, hits: 0 };
    }
    let mut decay = t.space.decay_per_bounce;
    if w_bounces > 0.0 {
        let kept = (w_log_kept / w_bounces).exp();
        let leave = (w_esc / w_bounces).min(1.0);
        decay = kept * (1.0 - leave);
    }
    if decay <= 0.0 || decay.is_nan() {
        decay = t.space.decay_per_bounce;
    }
    decay = decay.min(o.max_decay);
    let mfp = if w_seg_n > 0.0 { w_seg / w_seg_n } else { t.space.mean_free_path };
    let last = by_order[max_k];
    let prev = (if max_k >= 1 && by_order[max_k - 1] != 0.0 { by_order[max_k - 1] } else { last / decay.max(1e-3) }) * decay;
    let tail_ref = (last + prev) / 2.0;
    let tail = (tail_ref * decay) / (1.0 - decay).max(1e-3);
    let rate_rt = rt60_from(decay, mfp);
    let (mut sum_t, mut t0, mut t_last) = (0.0f64, f64::INFINITY, 0.0f64);
    for &(tt, cc) in arrivals.iter() {
        sum_t += tt * cc;
        t0 = t0.min(tt);
        t_last = t_last.max(tt);
    }
    let centre = (sum_t + tail * (t_last + rate_rt / 13.8)) / (total + tail);
    let centre_rt = 13.8 * (centre - t0).max(0.0);
    // Onset: time by which 15 % of the traced energy has arrived.
    arrivals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = 0.15 * total;
    let mut acc = 0.0;
    let mut onset = arrivals.last().map_or(0.0, |a| a.0);
    for &(tt, cc) in arrivals.iter() {
        acc += cc;
        if acc >= target {
            onset = tt;
            break;
        }
    }
    VoiceReverb {
        energy: total + tail,
        rt60: rate_rt.min(centre_rt).max(TRACE_RT60_RANGE.0),
        direction: dsum * (1.0 / total),
        predelay: onset.clamp(0.0, 0.08),
        absorption: abs_w / total,
        hits,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DirectPath {
    pub visible: f64,
    pub excess: f64,
    pub via: V3,
    pub length: f64,
    pub found: bool,
    /// Second bend of a two-bend path (for drawing), or `via`.
    pub mid: V3,
}

fn clear<C: Caster>(c: &mut C, from: V3, to: V3, end_skip: f64) -> bool {
    let seg = to - from;
    let len = seg.length();
    if len <= end_skip + 1e-3 {
        return true;
    }
    c.cast(from, seg * (1.0 / len), len - end_skip).is_none()
}

const BEND_PLANES: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];
const BEND_DIRECTIONS: usize = 12;
const BEND_STEPS: [f64; 7] = [0.15, 0.3, 0.6, 1.2, 2.4, 4.8, 9.6];

#[allow(clippy::too_many_arguments)]
fn bend_around<C: Caster>(c: &mut C, source: V3, listener: V3, d: f64, fwd: V3, side: V3, up: V3, end_skip: f64) -> Option<(V3, f64)> {
    let mut best: Option<(V3, f64)> = None;
    for f in BEND_PLANES {
        let centre = listener + fwd * (d * f);
        for j in 0..BEND_DIRECTIONS {
            let a = (j as f64 * 2.0 * core::f64::consts::PI) / BEND_DIRECTIONS as f64;
            let u = side * a.cos() + up * a.sin();
            let at = |rho: f64| centre + u * rho;
            let ok = |c: &mut C, rho: f64| {
                let pt = at(rho);
                clear(c, listener, pt, 0.0) && clear(c, pt, source, end_skip)
            };
            let (mut lo, mut hi) = (0.0f64, -1.0f64);
            for rho in BEND_STEPS {
                if let Some((_, bl)) = best
                    && (d * f).hypot(lo) + (d * (1.0 - f)).hypot(lo) >= bl
                {
                    break;
                }
                if ok(c, rho) {
                    hi = rho;
                    break;
                }
                lo = rho;
            }
            if hi < 0.0 {
                continue;
            }
            for _ in 0..5 {
                let m = (lo + hi) / 2.0;
                if ok(c, m) {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            let via = at(hi);
            let len = (via - listener).length() + (source - via).length();
            if best.is_none_or(|(_, bl)| len < bl) {
                best = Some((via, len));
            }
        }
    }
    best
}

/// Scratch buffers of `direct_path`, reused across calls.
#[derive(Default)]
pub struct PathScratch {
    one: Vec<(V3, f64)>,
    source_side: Vec<(V3, f64)>,
    pairs: Vec<(V3, V3, f64)>,
    source_dirs: Vec<V3>,
}

#[allow(clippy::too_many_arguments)]
pub fn direct_path<C: Caster>(c: &mut C, source: V3, listener: V3, trace: Option<&ListenerTrace>, extent: f64, end_skip: f64, source_rays: usize, s: &mut PathScratch) -> DirectPath {
    let to_s = source - listener;
    let d = to_s.length();
    let none = DirectPath { visible: 1.0, excess: 0.0, via: source, length: d, found: true, mid: source };
    if d <= end_skip + 1e-3 {
        return none;
    }
    let fwd = to_s * (1.0 / d);
    let mut side = fwd.cross(v3(0.0, 1.0, 0.0));
    if side.length() < 1e-3 {
        side = v3(1.0, 0.0, 0.0);
    }
    let side = side.normalize();
    let up = side.cross(fwd).normalize();
    let radius = extent.max(0.3);
    let (mut seen, mut probes) = (0usize, 0usize);
    for k in 0..15 {
        let (a, b) = if k == 0 {
            (0.0, 0.0)
        } else if k <= 6 {
            let t = (k - 1) as f64 * core::f64::consts::PI / 3.0;
            (0.5 * t.cos(), 0.5 * t.sin())
        } else {
            let t = ((k - 7) as f64 + 0.5) * core::f64::consts::PI / 4.0;
            (t.cos(), t.sin())
        };
        let target = source + side * (a * radius) + up * (b * radius);
        if k != 0 && !clear(c, source, target, 0.0) {
            continue;
        }
        probes += 1;
        if clear(c, listener, target, end_skip) {
            seen += 1;
        }
    }
    let visible = seen as f64 / probes.max(1) as f64;
    if visible >= 1.0 {
        return none;
    }
    let mut best: Option<(V3, f64, Option<V3>)> = bend_around(c, source, listener, d, fwd, side, up, end_skip).map(|(v, l)| (v, l, None));
    let Some(trace) = trace else {
        return match best {
            Some((via, len, _)) => DirectPath { visible, excess: (len - d).max(0.0), via, length: len, found: true, mid: via },
            None => DirectPath { visible, excess: 0.0, via: source, length: d, found: visible > 0.0, mid: source },
        };
    };
    s.one.clear();
    for &q in &trace.candidates {
        let len = (q - listener).length() + (source - q).length();
        if len < d + 40.0 {
            s.one.push((q, len));
        }
    }
    s.one.sort_by(|a, b| a.1.total_cmp(&b.1));
    if best.is_none() {
        let mut hit_at: isize = -1;
        let mut i = 7usize;
        while i < s.one.len() + 7 {
            let k = i.min(s.one.len() - 1);
            if clear(c, s.one[k].0, source, end_skip) {
                hit_at = k as isize;
                break;
            }
            i += 8;
        }
        if hit_at >= 0 {
            let h = hit_at as usize;
            for i in h.saturating_sub(7)..=h {
                if i == h || clear(c, s.one[i].0, source, end_skip) {
                    best = Some((s.one[i].0, s.one[i].1, None));
                    break;
                }
            }
        }
    }
    if s.source_dirs.len() != source_rays {
        s.source_dirs = sphere_directions(source_rays);
    }
    s.source_side.clear();
    for k in 0..s.source_dirs.len() {
        let dir = s.source_dirs[k];
        let free = c.cast(source, dir, 30.0).map_or(20.0, |h| h.toi - 0.3);
        for st in CANDIDATE_STEPS {
            if st >= free {
                break;
            }
            let r = source + dir * st;
            s.source_side.push((r, st + (listener - r).length()));
        }
    }
    s.source_side.sort_by(|a, b| a.1.total_cmp(&b.1));
    s.pairs.clear();
    for &(q, _) in s.one.iter().take(48) {
        let lq = (q - listener).length();
        for &(r, _) in s.source_side.iter().take(48) {
            let len = lq + (r - q).length() + (source - r).length();
            if best.is_none_or(|b| len < b.1) {
                s.pairs.push((q, r, len));
            }
        }
    }
    s.pairs.sort_by(|a, b| a.2.total_cmp(&b.2));
    for i in 0..s.pairs.len().min(200) {
        let (q, r, len) = s.pairs[i];
        if best.is_some_and(|b| len >= b.1) {
            break;
        }
        if clear(c, q, r, 0.0) {
            best = Some((q, len, Some(r)));
            break;
        }
    }
    match best {
        None => DirectPath { visible, excess: 0.0, via: source, length: d, found: visible > 0.0, mid: source },
        Some((via, len, mid)) => DirectPath { visible, excess: (len - d).max(0.0), via, length: len, found: true, mid: mid.unwrap_or(via) },
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Reflection {
    pub image: V3,
    pub point: V3,
    pub path: f64,
    pub delay: f64,
    pub gain: f64,
    pub absorption: f64,
    pub id: u32,
    /// The plane's normal (toward the source), rounded by the TS side into the tap key.
    pub normal: V3,
    pub weight: f64,
}

fn reaches<C: Caster>(c: &mut C, from: V3, point: V3, id: u32) -> bool {
    let d = (point - from).length();
    if d < 1e-3 {
        return true;
    }
    match c.cast(from, (point - from).normalize(), d + 0.1) {
        None => false,
        Some(h) => h.id == id && (h.toi - d).abs() < (d * 0.05).max(0.15),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn early_reflections<C: Caster>(c: &mut C, source: V3, listener: V3, dirs: &[V3], max: usize, max_dist: f64, soft: f64, min_delay: f64, scattering: f64, out: &mut Vec<Reflection>) {
    out.clear();
    let direct = (listener - source).length();
    let mut seen: Vec<(u32, i32, i32, i32)> = Vec::new();
    for &dir in dirs {
        let Some(hit) = c.cast(source, dir, max_dist) else { continue };
        let mut n = hit.normal.normalize();
        if (source - hit.point).dot(n) < 0.0 {
            n = -n;
        }
        // The TS key rounds the normal to one decimal (toFixed(1)).
        let key = (hit.id, (n.x * 10.0).round() as i32, (n.y * 10.0).round() as i32, (n.z * 10.0).round() as i32);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let ds = (source - hit.point).dot(n);
        let dl = (listener - hit.point).dot(n);
        if dl <= 0.01 || ds < 0.0 {
            continue;
        }
        let image = source - n * (2.0 * ds);
        let di = -ds;
        let t = di / (di - dl);
        let point = image + (listener - image) * t;
        let path = (listener - image).length();
        let delay = ((path - direct) / SPEED_OF_SOUND_M_S).max(0.0);
        if delay < min_delay {
            continue;
        }
        let valid = |c: &mut C, p: V3| reaches(c, listener, p, hit.id) && reaches(c, source, p, hit.id);
        let weight = if soft > 0.0 {
            let t1 = n.cross(if n.y.abs() < 0.9 { v3(0.0, 1.0, 0.0) } else { v3(1.0, 0.0, 0.0) }).normalize();
            let t2 = n.cross(t1);
            let mut ok = if valid(c, point) { 2 } else { 0 };
            for (a, b) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                if valid(c, point + t1 * (a * soft) + t2 * (b * soft)) {
                    ok += 1;
                }
            }
            let w = ok as f64 / 6.0;
            if w <= 0.0 {
                continue;
            }
            w
        } else {
            if !valid(c, point) {
                continue;
            }
            1.0
        };
        out.push(Reflection {
            image,
            point,
            path,
            delay,
            gain: ((1.0 - hit.absorption) * (1.0 - scattering)).max(0.0).sqrt(),
            absorption: hit.absorption,
            id: hit.id,
            normal: n,
            weight,
        });
    }
    out.sort_by(|a, b| a.path.total_cmp(&b.path));
    out.truncate(max);
}
