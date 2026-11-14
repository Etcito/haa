//! Geometry of the acoustic tracer: oriented boxes and triangles, each with a surface id and its
//! Sabine absorption, in a bounding volume hierarchy. No physics engine: the game (or its
//! geometry provider) hands the static level over once, and again when it changes.

use core::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub const fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, b: V3) -> V3 {
        v3(self.x + b.x, self.y + b.y, self.z + b.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, b: V3) -> V3 {
        v3(self.x - b.x, self.y - b.y, self.z - b.z)
    }
}
impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

impl V3 {
    pub fn dot(self, b: V3) -> f64 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }
    pub fn cross(self, b: V3) -> V3 {
        v3(self.y * b.z - self.z * b.y, self.z * b.x - self.x * b.z, self.x * b.y - self.y * b.x)
    }
    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn normalize(self) -> V3 {
        let l = self.length();
        if l > 0.0 { self * (1.0 / l) } else { v3(0.0, 0.0, 0.0) }
    }
    fn get(self, a: usize) -> f64 {
        match a {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
    fn min(self, b: V3) -> V3 {
        v3(self.x.min(b.x), self.y.min(b.y), self.z.min(b.z))
    }
    fn max(self, b: V3) -> V3 {
        v3(self.x.max(b.x), self.y.max(b.y), self.z.max(b.z))
    }
}

/// A ray hit, as the TS `RayHit`: distance, point, surface normal, absorption α, surface id.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub toi: f64,
    pub point: V3,
    pub normal: V3,
    pub absorption: f64,
    pub id: u32,
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Centre, the three unit axes of its frame, half extents.
    Box { c: V3, ax: [V3; 3], half: V3 },
    Tri { a: V3, e1: V3, e2: V3, n: V3 },
}

#[derive(Clone, Copy, Debug)]
struct Prim {
    shape: Shape,
    absorption: f64,
    id: u32,
    lo: V3,
    hi: V3,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    lo: V3,
    hi: V3,
    /// Leaf: first primitive index and count (count > 0). Inner: right child index, count 0
    /// (the left child is the next node).
    first: u32,
    count: u32,
    /// Inner: the split axis (the nearer child along the ray is visited first).
    axis: u32,
}

/// Floats per box record: centre (3), half extents (3), rotation quaternion x y z w (4), α, id.
pub const BOX_STRIDE: usize = 12;
/// Floats per triangle record: three vertices (9), α, id.
pub const TRI_STRIDE: usize = 11;

pub struct Scene {
    prims: Vec<Prim>,
    nodes: Vec<Node>,
    /// Ray casts since the last reset (cost accounting).
    pub casts: u64,
}

const LEAF: usize = 4;

impl Scene {
    /// Builds the scene from flat records (`BOX_STRIDE` / `TRI_STRIDE` floats each).
    pub fn new(boxes: &[f32], tris: &[f32]) -> Scene {
        let mut prims = Vec::new();
        for r in boxes.chunks_exact(BOX_STRIDE) {
            let f = |i: usize| f64::from(r[i]);
            let c = v3(f(0), f(1), f(2));
            let half = v3(f(3).abs(), f(4).abs(), f(5).abs());
            let (qx, qy, qz, qw) = (f(6), f(7), f(8), f(9));
            let ql = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt().max(1e-12);
            let (x, y, z, w) = (qx / ql, qy / ql, qz / ql, qw / ql);
            // Columns of the rotation matrix: the box's axes in world space.
            let ax = [
                v3(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)),
                v3(2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)),
                v3(2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)),
            ];
            let ext = v3(
                half.x * ax[0].x.abs() + half.y * ax[1].x.abs() + half.z * ax[2].x.abs(),
                half.x * ax[0].y.abs() + half.y * ax[1].y.abs() + half.z * ax[2].y.abs(),
                half.x * ax[0].z.abs() + half.y * ax[1].z.abs() + half.z * ax[2].z.abs(),
            );
            prims.push(Prim { shape: Shape::Box { c, ax, half }, absorption: f(10), id: r[11] as u32, lo: c - ext, hi: c + ext });
        }
        for r in tris.chunks_exact(TRI_STRIDE) {
            let f = |i: usize| f64::from(r[i]);
            let a = v3(f(0), f(1), f(2));
            let b = v3(f(3), f(4), f(5));
            let c = v3(f(6), f(7), f(8));
            let (e1, e2) = (b - a, c - a);
            let n = e1.cross(e2).normalize();
            let eps = v3(1e-4, 1e-4, 1e-4);
            prims.push(Prim { shape: Shape::Tri { a, e1, e2, n }, absorption: f(9), id: r[10] as u32, lo: a.min(b).min(c) - eps, hi: a.max(b).max(c) + eps });
        }
        let mut s = Scene { prims, nodes: Vec::new(), casts: 0 };
        if !s.prims.is_empty() {
            let n = s.prims.len();
            s.build(0, n);
        }
        s
    }

    pub fn len(&self) -> usize {
        self.prims.len()
    }
    pub fn is_empty(&self) -> bool {
        self.prims.is_empty()
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Top-down build, split at the median of the longest axis of the centroids.
    fn build(&mut self, first: usize, end: usize) -> usize {
        let (mut lo, mut hi) = (v3(f64::MAX, f64::MAX, f64::MAX), v3(f64::MIN, f64::MIN, f64::MIN));
        let (mut clo, mut chi) = (lo, hi);
        for p in &self.prims[first..end] {
            lo = lo.min(p.lo);
            hi = hi.max(p.hi);
            let c = (p.lo + p.hi) * 0.5;
            clo = clo.min(c);
            chi = chi.max(c);
        }
        let index = self.nodes.len();
        self.nodes.push(Node { lo, hi, first: first as u32, count: (end - first) as u32, axis: 0 });
        if end - first <= LEAF {
            return index;
        }
        let span = chi - clo;
        let axis = if span.x >= span.y && span.x >= span.z { 0 } else if span.y >= span.z { 1 } else { 2 };
        let mid = first + (end - first) / 2;
        self.prims[first..end].sort_by(|a, b| {
            let ca = a.lo.get(axis) + a.hi.get(axis);
            let cb = b.lo.get(axis) + b.hi.get(axis);
            ca.total_cmp(&cb)
        });
        self.build(first, mid);
        let right = self.build(mid, end);
        self.nodes[index].first = right as u32;
        self.nodes[index].count = 0;
        self.nodes[index].axis = axis as u32;
        index
    }

    /// Nearest hit along `dir` (unit) within `max_dist`. A ray that starts inside a box hits it at
    /// 0 (Rapier's `solid` cast), its normal facing the ray.
    pub fn cast(&mut self, o: V3, d: V3, max_dist: f64) -> Option<Hit> {
        self.casts += 1;
        if self.nodes.is_empty() {
            return None;
        }
        let inv = v3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut best_t = max_dist;
        let mut best: Option<(usize, V3)> = None;
        let mut stack = [0u32; 64];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let index = stack[sp] as usize;
            let node = self.nodes[index];
            if !slab(node.lo, node.hi, o, inv, best_t) {
                continue;
            }
            if node.count > 0 {
                for i in node.first as usize..(node.first + node.count) as usize {
                    if let Some((t, n)) = hit_prim(&self.prims[i].shape, o, d, best_t) {
                        best_t = t;
                        best = Some((i, n));
                    }
                }
            } else if sp + 2 <= stack.len() {
                // Depth-first layout: the left child follows its parent, `first` is the right one.
                // The nearer child is popped first so `best_t` prunes the other.
                let (l, r) = (index as u32 + 1, node.first);
                // Children are sorted along the split axis: the left one is first for a ray going +axis.
                let near_left = d.get(node.axis as usize) >= 0.0;
                let (a, b) = if near_left { (r, l) } else { (l, r) };
                stack[sp] = a;
                stack[sp + 1] = b;
                sp += 2;
            }
        }
        best.map(|(i, n)| {
            let p = &self.prims[i];
            Hit { toi: best_t, point: o + d * best_t, normal: n, absorption: p.absorption, id: p.id }
        })
    }
}

fn slab(lo: V3, hi: V3, o: V3, inv: V3, max_t: f64) -> bool {
    // Branchless slabs; a NaN (0 * inf, a ray in a slab's plane) is ignored by min/max.
    let (x1, x2) = ((lo.x - o.x) * inv.x, (hi.x - o.x) * inv.x);
    let (y1, y2) = ((lo.y - o.y) * inv.y, (hi.y - o.y) * inv.y);
    let (z1, z2) = ((lo.z - o.z) * inv.z, (hi.z - o.z) * inv.z);
    let t0 = x1.min(x2).max(y1.min(y2)).max(z1.min(z2)).max(0.0);
    let t1 = x1.max(x2).min(y1.max(y2)).min(z1.max(z2)).min(max_t);
    t0 <= t1
}

fn hit_prim(s: &Shape, o: V3, d: V3, max_t: f64) -> Option<(f64, V3)> {
    match *s {
        Shape::Box { c, ax, half } => {
            let rel = o - c;
            let lo = [rel.dot(ax[0]), rel.dot(ax[1]), rel.dot(ax[2])];
            let ld = [d.dot(ax[0]), d.dot(ax[1]), d.dot(ax[2])];
            let h = [half.x, half.y, half.z];
            let (mut t0, mut t1) = (f64::MIN, f64::MAX);
            let mut axis = 0usize;
            for a in 0..3 {
                if ld[a].abs() < 1e-12 {
                    if lo[a] < -h[a] || lo[a] > h[a] {
                        return None;
                    }
                    continue;
                }
                let inv = 1.0 / ld[a];
                let mut ta = (-h[a] - lo[a]) * inv;
                let mut tb = (h[a] - lo[a]) * inv;
                if ta > tb {
                    core::mem::swap(&mut ta, &mut tb);
                }
                if ta > t0 {
                    t0 = ta;
                    axis = a;
                }
                t1 = t1.min(tb);
                if t0 > t1 {
                    return None;
                }
            }
            if t1 < 0.0 {
                return None;
            }
            if t0 < 0.0 {
                // Inside: hit at 0, normal against the ray.
                return Some((0.0, -d));
            }
            if t0 > max_t {
                return None;
            }
            let n = if ld[axis] > 0.0 { -ax[axis] } else { ax[axis] };
            Some((t0, n))
        }
        Shape::Tri { a, e1, e2, n } => {
            let p = d.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-12 {
                return None;
            }
            let inv = 1.0 / det;
            let s = o - a;
            let u = s.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                return None;
            }
            let q = s.cross(e1);
            let v = d.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                return None;
            }
            let t = e2.dot(q) * inv;
            if t < 0.0 || t > max_t {
                return None;
            }
            Some((t, n))
        }
    }
}
