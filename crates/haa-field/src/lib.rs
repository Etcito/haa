//! haa-field: a baked, sparse acoustic field of the level (docs/physics-sound.md, "Acoustic field").
//!
//! Instead of casting rays for every sound event, the level is probed once on a coarse voxel grid
//! (1-2 m cells), nearest to the listener first, in the background:
//! - per air cell, order-2 real spherical harmonics (9 coefficients) of the distance to the nearest
//!   obstacle per direction (early reflections: delay and direction of the walls around) and of the
//!   openness per direction (1 where a ray escapes to the sky: where the room is open, where rain
//!   comes from), plus scalars from a short bounce trace (reverb time, mean free path, volume,
//!   absorption, escape share) and the 26 neighbour links that a straight segment can cross;
//! - a propagation field flooded from the listener's cell through the links (Dijkstra): every cell
//!   knows how far sound travels to it around the obstacles. A source then reads its occlusion and
//!   the direction it arrives from with one lookup, the path string-pulled into a few corners.
//!
//! Geometry and ray casts come from haa-trace (oriented boxes, triangles, a BVH). RAA's world is an
//! SDF: a bake could sample the SDF at the cell centres (solid test), sphere-trace it for the
//! distances, and step it for the links, without any ray against triangles.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use haa_trace::trace::{self, ListenerTrace};
use haa_trace::{Scene, V3, v3};

/// Spherical-harmonic coefficients per quantity: order 1. Order 2 cut the per-direction distance error
/// only from 5.9 to 4.9 m RMS (openness 0.24 to 0.21) in a room with a door and cubes, and the
/// runtime reads only the first-order vector (the enclosed side) and the mean: the reflections come
/// from stored planes, not from the harmonics.
pub const SH: usize = 4;
/// Reflecting planes stored per cell (the nearest distinct surfaces seen by the 64 probes).
pub const PLANES: usize = 6;
/// Probe directions per cell.
pub const DIRS: usize = 64;
/// Distances are clamped here (m): farther walls are "far".
pub const FAR: f64 = 30.0;
/// A ray that hits nothing within this (m) has escaped.
pub const ESCAPE: f64 = 60.0;
const BRICK: usize = 4;
/// Floats per exported cell record: index, state, dist SH, open SH, 5 scalars, 2 edge halves, plane count, planes.
pub const RECORD: usize = 2 + SH + SH + 5 + 2 + 1 + PLANES * 5;

#[derive(Clone, Copy, Default)]
pub struct Cell {
    /// 0 not baked, 1 air, 2 solid.
    pub state: u8,
    pub dist: [f32; SH],
    pub open: [f32; SH],
    pub rt60: f32,
    pub mfp: f32,
    pub volume: f32,
    pub absorption: f32,
    pub escape: f32,
    /// Bit k: the segment to neighbour k (`OFFSETS`) is clear.
    pub edges: u32,
    /// Reflecting planes: unit normal (toward the cell), offset dot(n, point on plane), absorption.
    pub planes: [[f32; 5]; PLANES],
    pub nplanes: u8,
}

/// The 26 neighbours of a cell.
pub const OFFSETS: [(i32, i32, i32); 26] = {
    let mut out = [(0, 0, 0); 26];
    let mut n = 0;
    let mut x = -1;
    while x <= 1 {
        let mut y = -1;
        while y <= 1 {
            let mut z = -1;
            while z <= 1 {
                if x != 0 || y != 0 || z != 0 {
                    out[n] = (x, y, z);
                    n += 1;
                }
                z += 1;
            }
            y += 1;
        }
        x += 1;
    }
    out
};

/// Reverb time from the energy kept per bounce and the mean free path, 0.1-8.5 s.
pub fn rt60(decay: f64, mfp: f64) -> f64 {
    if !(decay > 0.0) {
        return 0.1;
    }
    let db = -10.0 * decay.log10();
    if db <= 1e-6 {
        return 8.5;
    }
    (60.0 * mfp / (343.0 * db)).clamp(0.1, 8.5)
}

/// Real spherical harmonics of order 0 and 1 at a unit direction (Y00, Y1-1 ∝ y, Y10 ∝ z, Y11 ∝ x).
pub fn sh9(d: V3) -> [f64; SH] {
    [0.282_095, 0.488_603 * d.y, 0.488_603 * d.z, 0.488_603 * d.x]
}

/// Evaluate the stored SH coefficients in a direction (order 1).
pub fn sh_eval(c: &[f32; SH], d: V3, _order: usize) -> f64 {
    let y = sh9(d);
    (0..SH).map(|k| f64::from(c[k]) * y[k]).sum()
}

#[derive(Clone, Copy)]
pub struct Grid {
    pub origin: V3,
    pub cell: f64,
    pub n: [usize; 3],
}

impl Grid {
    pub fn len(&self) -> usize {
        self.n[0] * self.n[1] * self.n[2]
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn index(&self, i: [i64; 3]) -> Option<usize> {
        if (0..3).any(|a| i[a] < 0 || i[a] >= self.n[a] as i64) {
            return None;
        }
        Some((i[2] as usize * self.n[1] + i[1] as usize) * self.n[0] + i[0] as usize)
    }
    pub fn coords(&self, idx: usize) -> [i64; 3] {
        let x = idx % self.n[0];
        let y = (idx / self.n[0]) % self.n[1];
        let z = idx / (self.n[0] * self.n[1]);
        [x as i64, y as i64, z as i64]
    }
    pub fn centre(&self, idx: usize) -> V3 {
        let c = self.coords(idx);
        self.origin + v3((c[0] as f64 + 0.5) * self.cell, (c[1] as f64 + 0.5) * self.cell, (c[2] as f64 + 0.5) * self.cell)
    }
    /// The cell whose lower corner of the 8 surrounding centres is below `p` (for trilinear lookups).
    pub fn base(&self, p: V3) -> ([i64; 3], [f64; 3]) {
        let f = |v: f64, o: f64| (v - o) / self.cell - 0.5;
        let r = [f(p.x, self.origin.x), f(p.y, self.origin.y), f(p.z, self.origin.z)];
        let b = [r[0].floor() as i64, r[1].floor() as i64, r[2].floor() as i64];
        (b, [r[0] - b[0] as f64, r[1] - b[1] as f64, r[2] - b[2] as f64])
    }
}

/// Room scalars and SH read at a point (trilinear over the baked air cells around it).
#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub weight: f64,
    pub dist: [f32; SH],
    pub open: [f32; SH],
    pub rt60: f64,
    pub mfp: f64,
    pub volume: f64,
    pub absorption: f64,
    pub escape: f64,
}

#[derive(Clone, Copy, Default)]
pub struct Aperture {
    /// Narrowest free cross-section along the path (m²) and where it is.
    pub area: f64,
    pub point: V3,
    /// Cross-section at the listener's and at the source's end.
    pub at_listener: f64,
    pub at_source: f64,
}

#[derive(Clone, Copy, Default)]
pub struct PathResult {
    pub found: bool,
    pub visible: bool,
    /// Length along the grid links, and after string-pulling into straight legs (m).
    pub lattice: f64,
    pub length: f64,
    pub excess: f64,
    /// First corner after the listener (or the source when visible).
    pub via: V3,
    pub corners: usize,
    pub aperture: Aperture,
}

#[derive(Copy, Clone, PartialEq)]
struct Item(f32, u32);
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

pub struct Field {
    pub scene: Scene,
    pub grid: Grid,
    bricks: HashMap<usize, Box<[Cell; BRICK * BRICK * BRICK]>>,
    dirs: Vec<V3>,
    basis: Vec<[f64; SH]>,
    queue: Vec<u32>,
    queued_focus: Option<V3>,
    pub baked_new: Vec<u32>,
    pub baked: usize,
    trace: ListenerTrace,
    flood_dist: Vec<f32>,
    flood_parent: Vec<i32>,
    pub flood_reached: usize,
    path_pts: Vec<V3>,
    pulled: Vec<V3>,
}

impl Field {
    pub fn new(scene: Scene, grid: Grid) -> Field {
        let dirs = trace::sphere_directions(DIRS);
        let basis = dirs.iter().map(|&d| sh9(d)).collect();
        Field {
            scene,
            grid,
            bricks: HashMap::new(),
            dirs,
            basis,
            queue: Vec::new(),
            queued_focus: None,
            baked_new: Vec::new(),
            baked: 0,
            trace: ListenerTrace::default(),
            flood_dist: Vec::new(),
            flood_parent: Vec::new(),
            flood_reached: 0,
            path_pts: Vec::new(),
            pulled: Vec::new(),
        }
    }

    fn brick_of(&self, idx: usize) -> (usize, usize) {
        let c = self.grid.coords(idx);
        let bn = [self.grid.n[0].div_ceil(BRICK), self.grid.n[1].div_ceil(BRICK)];
        let b = [c[0] as usize / BRICK, c[1] as usize / BRICK, c[2] as usize / BRICK];
        let local = ((c[2] as usize % BRICK) * BRICK + (c[1] as usize % BRICK)) * BRICK + (c[0] as usize % BRICK);
        ((b[2] * bn[1] + b[1]) * bn[0] + b[0], local)
    }

    pub fn cell(&self, idx: usize) -> Option<&Cell> {
        let (b, l) = self.brick_of(idx);
        self.bricks.get(&b).map(|br| &br[l]).filter(|c| c.state != 0)
    }

    pub fn set_cell(&mut self, idx: usize, cell: Cell) {
        let (b, l) = self.brick_of(idx);
        let br = self.bricks.entry(b).or_insert_with(|| Box::new([Cell::default(); BRICK * BRICK * BRICK]));
        if br[l].state == 0 && cell.state != 0 {
            self.baked += 1;
        }
        br[l] = cell;
    }

    /// Bytes held by the baked bricks.
    pub fn bytes(&self) -> usize {
        self.bricks.len() * core::mem::size_of::<[Cell; BRICK * BRICK * BRICK]>()
    }

    pub fn bricks(&self) -> usize {
        self.bricks.len()
    }

    /// Probe one cell: solid test, 64 distances and escapes, a 64 x 4 bounce trace, 26 links.
    pub fn bake_cell(&mut self, idx: usize) -> Cell {
        let c = self.grid.centre(idx);
        let mut cell = Cell::default();
        if let Some(h) = self.scene.cast(c, v3(0.0, 1.0, 0.0), 1e-3)
            && h.toi <= 0.0
        {
            cell.state = 2;
            return cell;
        }
        cell.state = 1;
        let w = 4.0 * core::f64::consts::PI / DIRS as f64;
        let mut dist = [0.0f64; SH];
        let mut open = [0.0f64; SH];
        // Reflecting planes: per surface (id) and orientation, the nearest hit.
        let mut planes: Vec<(f64, V3, f64, f64)> = Vec::new();
        let mut keys: Vec<(u32, i32, i32, i32)> = Vec::new();
        for i in 0..DIRS {
            let d = self.dirs[i];
            let hit = self.scene.cast(c, d, ESCAPE);
            let (r, o) = match hit {
                Some(h) => (h.toi.min(FAR), 0.0),
                None => (FAR, 1.0),
            };
            for k in 0..SH {
                dist[k] += w * r * self.basis[i][k];
                open[k] += w * o * self.basis[i][k];
            }
            if let Some(h) = hit
                && h.toi > 0.05
                && h.toi < FAR
            {
                let mut n = h.normal.normalize();
                if n.dot(c - h.point) < 0.0 {
                    n = -n;
                }
                let key = (h.id, (n.x * 4.0).round() as i32, (n.y * 4.0).round() as i32, (n.z * 4.0).round() as i32);
                let plane = (h.toi, n, n.dot(h.point), h.absorption);
                match keys.iter().position(|k| *k == key) {
                    Some(j) => {
                        if h.toi < planes[j].0 {
                            planes[j] = plane;
                        }
                    }
                    None => {
                        keys.push(key);
                        planes.push(plane);
                    }
                }
            }
        }
        planes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        for (j, pl) in planes.iter().take(PLANES).enumerate() {
            cell.planes[j] = [pl.1.x as f32, pl.1.y as f32, pl.1.z as f32, pl.2 as f32, pl.3 as f32];
            cell.nplanes = (j + 1) as u8;
        }
        for k in 0..SH {
            cell.dist[k] = dist[k] as f32;
            cell.open[k] = open[k] as f32;
        }
        trace::trace_listener(&mut self.scene, &mut self.trace, c, 64, 6, ESCAPE, 0.35);
        let sp = self.trace.space;
        // haa-trace clamps its reverb time at 4.5 s; the large buildings ring longer (the bank goes
        // to 8 s), so the decay is recomputed here from the energy kept per bounce.
        cell.rt60 = rt60(sp.decay_per_bounce, sp.mean_free_path) as f32;
        cell.mfp = sp.mean_free_path as f32;
        cell.volume = sp.volume as f32;
        cell.absorption = sp.absorption as f32;
        cell.escape = sp.escape as f32;
        let ci = self.grid.coords(idx);
        for (k, o) in OFFSETS.iter().enumerate() {
            if self.grid.index([ci[0] + o.0 as i64, ci[1] + o.1 as i64, ci[2] + o.2 as i64]).is_none() {
                continue;
            }
            let off = v3(o.0 as f64, o.1 as f64, o.2 as f64) * self.grid.cell;
            let len = off.length();
            if self.scene.cast(c, off * (1.0 / len), len).is_none() {
                cell.edges |= 1 << k;
            }
        }
        cell
    }

    /// Bake up to `budget` cells, nearest to `focus` first. Returns the cells left.
    pub fn bake(&mut self, budget: usize, focus: V3) -> usize {
        let refocus = match self.queued_focus {
            None => true,
            Some(f) => (f - focus).length() > 2.0 * self.grid.cell,
        };
        if refocus || self.queue.is_empty() {
            let mut q: Vec<(f64, u32)> = (0..self.grid.len())
                .filter(|&i| self.cell(i).is_none())
                .map(|i| ((self.grid.centre(i) - focus).length(), i as u32))
                .collect();
            q.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
            self.queue = q.into_iter().map(|x| x.1).collect();
            self.queued_focus = Some(focus);
        }
        let mut done = 0;
        while done < budget {
            let Some(i) = self.queue.pop() else { break };
            if self.cell(i as usize).is_some() {
                continue;
            }
            let cell = self.bake_cell(i as usize);
            self.set_cell(i as usize, cell);
            self.baked_new.push(i);
            done += 1;
        }
        self.queue.len()
    }

    /// Forget the cells whose centre lies within the box (static geometry changed there).
    pub fn dirty(&mut self, lo: V3, hi: V3) -> usize {
        let pad = self.grid.cell;
        let mut n = 0;
        for i in 0..self.grid.len() {
            let c = self.grid.centre(i);
            if c.x >= lo.x - pad && c.x <= hi.x + pad && c.y >= lo.y - pad && c.y <= hi.y + pad && c.z >= lo.z - pad && c.z <= hi.z + pad && self.cell(i).is_some() {
                let (b, l) = self.brick_of(i);
                if let Some(br) = self.bricks.get_mut(&b) {
                    br[l].state = 0;
                    self.baked -= 1;
                    n += 1;
                }
            }
        }
        self.queued_focus = None;
        n
    }

    /// Trilinear read over the baked air cells around `p` (solid and unbaked cells left out).
    pub fn sample(&self, p: V3) -> Sample {
        let (b, f) = self.grid.base(p);
        let mut s = Sample::default();
        for corner in 0..8 {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let wgt = (0..3).map(|a| if o[a] == 1 { f[a] } else { 1.0 - f[a] }).product::<f64>();
            if wgt <= 0.0 {
                continue;
            }
            let Some(idx) = self.grid.index([b[0] + o[0] as i64, b[1] + o[1] as i64, b[2] + o[2] as i64]) else { continue };
            let Some(c) = self.cell(idx) else { continue };
            if c.state != 1 {
                continue;
            }
            s.weight += wgt;
            for k in 0..SH {
                s.dist[k] += (wgt * f64::from(c.dist[k])) as f32;
                s.open[k] += (wgt * f64::from(c.open[k])) as f32;
            }
            s.rt60 += wgt * f64::from(c.rt60).ln();
            s.mfp += wgt * f64::from(c.mfp);
            s.volume += wgt * f64::from(c.volume);
            s.absorption += wgt * f64::from(c.absorption);
            s.escape += wgt * f64::from(c.escape);
        }
        if s.weight > 0.0 {
            let k = 1.0 / s.weight;
            for i in 0..SH {
                s.dist[i] = (f64::from(s.dist[i]) * k) as f32;
                s.open[i] = (f64::from(s.open[i]) * k) as f32;
            }
            s.rt60 = (s.rt60 * k).exp();
            s.mfp *= k;
            s.volume *= k;
            s.absorption *= k;
            s.escape *= k;
        }
        s
    }

    pub fn clear(&mut self, a: V3, b: V3) -> bool {
        let d = b - a;
        let len = d.length();
        len < 1e-6 || self.scene.cast(a, d * (1.0 / len), len).is_none()
    }

    /// The 8 cells around `p` whose centre `p` sees, with their distance.
    fn seeds(&mut self, p: V3) -> Vec<(usize, f64)> {
        let (b, _) = self.grid.base(p);
        let mut out = Vec::new();
        for corner in 0..8 {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let Some(idx) = self.grid.index([b[0] + o[0] as i64, b[1] + o[1] as i64, b[2] + o[2] as i64]) else { continue };
            if self.cell(idx).map(|c| c.state) != Some(1) {
                continue;
            }
            let c = self.grid.centre(idx);
            if self.clear(p, c) {
                out.push((idx, (c - p).length()));
            }
        }
        out
    }

    /// Flood from the listener: shortest distance through the links to every baked air cell.
    pub fn flood(&mut self, l: V3, max: f64) -> usize {
        let n = self.grid.len();
        self.flood_dist.clear();
        self.flood_dist.resize(n, f32::INFINITY);
        self.flood_parent.clear();
        self.flood_parent.resize(n, -1);
        let mut heap = BinaryHeap::new();
        for (idx, d) in self.seeds(l) {
            self.flood_dist[idx] = d as f32;
            heap.push(Item(d as f32, idx as u32));
        }
        let lens: Vec<f32> = OFFSETS.iter().map(|o| (((o.0 * o.0 + o.1 * o.1 + o.2 * o.2) as f64).sqrt() * self.grid.cell) as f32).collect();
        let mut reached = 0;
        while let Some(Item(d, i)) = heap.pop() {
            let i = i as usize;
            if d > self.flood_dist[i] {
                continue;
            }
            // Beyond the reach no source is heard through the field: stop there (a full flood of the
            // 50 000 cells of a 1 m grid took 41 ms on the main thread).
            if f64::from(d) > max {
                break;
            }
            reached += 1;
            let Some(cell) = self.cell(i) else { continue };
            let edges = cell.edges;
            let ci = self.grid.coords(i);
            for (k, o) in OFFSETS.iter().enumerate() {
                if edges & (1 << k) == 0 {
                    continue;
                }
                let Some(j) = self.grid.index([ci[0] + o.0 as i64, ci[1] + o.1 as i64, ci[2] + o.2 as i64]) else { continue };
                if self.cell(j).map(|c| c.state) != Some(1) {
                    continue;
                }
                let nd = d + lens[k];
                if nd < self.flood_dist[j] {
                    self.flood_dist[j] = nd;
                    self.flood_parent[j] = i as i32;
                    heap.push(Item(nd, j as u32));
                }
            }
        }
        self.flood_reached = reached;
        reached
    }

    /// The path from the listener to a source through the flooded field, string-pulled into
    /// straight legs (each leg checked by one ray), its excess over the straight line and its first
    /// corner (where the sound arrives from).
    pub fn path(&mut self, s: V3, l: V3, want_aperture: bool) -> PathResult {
        let direct = (s - l).length();
        let mut r = PathResult { via: s, length: direct, lattice: direct, ..Default::default() };
        if self.clear(l, s) {
            r.found = true;
            r.visible = true;
            if want_aperture {
                r.aperture = self.aperture(&[l, s]);
            }
            return r;
        }
        if self.flood_dist.is_empty() {
            return r;
        }
        let mut best: Option<(usize, f64)> = None;
        for (idx, d) in self.seeds(s) {
            let total = f64::from(self.flood_dist[idx]) + d;
            if total.is_finite() && best.is_none_or(|b| total < b.1) {
                best = Some((idx, total));
            }
        }
        let Some((start, total)) = best else { return r };
        // Points from the source back to the listener.
        self.path_pts.clear();
        self.path_pts.push(s);
        let mut i = start as i32;
        let mut guard = 0;
        while i >= 0 && guard < 4096 {
            self.path_pts.push(self.grid.centre(i as usize));
            i = self.flood_parent[i as usize];
            guard += 1;
        }
        self.path_pts.push(l);
        self.path_pts.reverse();
        // String-pull from the listener: keep the farthest point the current corner sees.
        let pts = std::mem::take(&mut self.path_pts);
        let mut pulled = std::mem::take(&mut self.pulled);
        pulled.clear();
        pulled.push(pts[0]);
        let mut cur = pts[0];
        let mut len = 0.0;
        let mut first: Option<V3> = None;
        let mut corners = 0;
        let mut j = 1;
        while j < pts.len() {
            if !self.clear(cur, pts[j]) {
                let corner = pts[j - 1];
                if (corner - cur).length() < 1e-9 {
                    // Not even the next lattice point is visible from here: take it anyway.
                    len += (pts[j] - cur).length();
                    cur = pts[j];
                    j += 1;
                    continue;
                }
                len += (corner - cur).length();
                if first.is_none() {
                    first = Some(corner);
                }
                cur = corner;
                pulled.push(corner);
                corners += 1;
                continue;
            }
            j += 1;
        }
        len += (pts[pts.len() - 1] - cur).length();
        pulled.push(pts[pts.len() - 1]);
        self.path_pts = pts;
        if want_aperture {
            r.aperture = self.aperture(&pulled);
        }
        self.pulled = pulled;
        r.found = true;
        r.lattice = total;
        r.length = len;
        r.excess = (len - direct).max(0.0);
        r.via = first.unwrap_or(s);
        r.corners = corners;
        r
    }

    /// Free cross-section at `p`: the narrowest of 4 horizontal chords through it (0, 45, 90, 135
    /// degrees) times the vertical chord; 10 casts of at most 10 m. In a doorway the chord along the
    /// door's plane is the door's width; across a room, the room's.
    fn section(&mut self, p: V3, _d: V3) -> f64 {
        let mut reach = |dir: V3| self.scene.cast(p, dir, 10.0).map_or(10.0, |h| h.toi);
        let h = reach(v3(0.0, 1.0, 0.0)) + reach(v3(0.0, -1.0, 0.0));
        let mut w = f64::INFINITY;
        for k in 0..4 {
            let a = k as f64 * core::f64::consts::FRAC_PI_4;
            let u = v3(a.cos(), 0.0, a.sin());
            w = w.min(reach(u) + reach(-u));
        }
        w * h
    }

    /// The narrowest cross-section along a polyline, and at its two ends. A wall with a door is
    /// 0.2-0.4 m thick, so the line is scanned every 0.25 m with the vertical chord only (2 casts:
    /// a lintel lowers it), then the full section (10 casts) is taken at the 3 lowest points and at
    /// the corners of the polyline (the edges the path bends around).
    pub fn aperture(&mut self, pts: &[V3]) -> Aperture {
        let mut ap = Aperture { area: f64::INFINITY, ..Default::default() };
        if pts.len() < 2 {
            return ap;
        }
        let mut scan: Vec<(f64, V3)> = Vec::new();
        for w in pts.windows(2) {
            let seg = w[1] - w[0];
            let len = seg.length();
            if len < 1e-6 {
                continue;
            }
            let d = seg * (1.0 / len);
            let steps = (len / 0.25).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let p = w[0] + d * (len * k as f64 / steps as f64);
                let up = self.scene.cast(p, v3(0.0, 1.0, 0.0), 10.0).map_or(10.0, |h| h.toi);
                let down = self.scene.cast(p, v3(0.0, -1.0, 0.0), 10.0).map_or(10.0, |h| h.toi);
                scan.push((up + down, p));
            }
        }
        scan.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        let mut probe: Vec<V3> = scan.iter().take(3).map(|x| x.1).collect();
        probe.extend_from_slice(&pts[1..pts.len() - 1]);
        for p in probe {
            let a = self.section(p, v3(1.0, 0.0, 0.0));
            if a < ap.area {
                ap.area = a;
                ap.point = p;
            }
        }
        let n = pts.len();
        ap.at_listener = self.section(pts[0], v3(1.0, 0.0, 0.0));
        ap.at_source = self.section(pts[n - 1], v3(1.0, 0.0, 0.0));
        ap
    }

    /// The reflecting planes of the air cell nearest `p` among the 8 around it.
    pub fn planes_at(&self, p: V3) -> Option<&Cell> {
        let (b, _) = self.grid.base(p);
        let mut best: Option<(f64, &Cell)> = None;
        for corner in 0..8 {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let Some(idx) = self.grid.index([b[0] + o[0] as i64, b[1] + o[1] as i64, b[2] + o[2] as i64]) else { continue };
            let Some(c) = self.cell(idx) else { continue };
            if c.state != 1 {
                continue;
            }
            let d = (self.grid.centre(idx) - p).length();
            if best.is_none_or(|x| d < x.0) {
                best = Some((d, c));
            }
        }
        best.map(|x| x.1)
    }

    /// The flood as flat arrays (distance, parent as f32) for another instance of the same grid.
    pub fn flood_export(&self, out: &mut Vec<f32>) {
        out.clear();
        out.extend_from_slice(&self.flood_dist);
        out.extend(self.flood_parent.iter().map(|&p| p as f32));
    }

    pub fn flood_import(&mut self, data: &[f32]) {
        let n = self.grid.len();
        if data.len() < 2 * n {
            return;
        }
        self.flood_dist.clear();
        self.flood_dist.extend_from_slice(&data[..n]);
        self.flood_parent.clear();
        self.flood_parent.extend(data[n..2 * n].iter().map(|&p| p as i32));
    }

    /// Flooded distance at a cell (debug).
    pub fn flood_at(&self, idx: usize) -> f32 {
        self.flood_dist.get(idx).copied().unwrap_or(f32::INFINITY)
    }

    pub fn record(&self, idx: usize, out: &mut Vec<f32>) {
        let Some(c) = self.cell(idx) else { return };
        out.push(idx as f32);
        out.push(f32::from(c.state));
        out.extend_from_slice(&c.dist);
        out.extend_from_slice(&c.open);
        out.extend_from_slice(&[c.rt60, c.mfp, c.volume, c.absorption, c.escape]);
        out.push((c.edges & 0xffff) as f32);
        out.push((c.edges >> 16) as f32);
        out.push(f32::from(c.nplanes));
        for pl in &c.planes {
            out.extend_from_slice(pl);
        }
    }

    pub fn ingest(&mut self, r: &[f32]) {
        for rec in r.chunks_exact(RECORD) {
            let mut c = Cell { state: rec[1] as u8, ..Default::default() };
            let mut k = 2;
            c.dist.copy_from_slice(&rec[k..k + SH]);
            k += SH;
            c.open.copy_from_slice(&rec[k..k + SH]);
            k += SH;
            c.rt60 = rec[k];
            c.mfp = rec[k + 1];
            c.volume = rec[k + 2];
            c.absorption = rec[k + 3];
            c.escape = rec[k + 4];
            k += 5;
            c.edges = (rec[k] as u32) | ((rec[k + 1] as u32) << 16);
            k += 2;
            c.nplanes = rec[k] as u8;
            k += 1;
            for j in 0..PLANES {
                c.planes[j].copy_from_slice(&rec[k + j * 5..k + j * 5 + 5]);
            }
            self.set_cell(rec[0] as usize, c);
        }
    }
}

// --- wasm ABI --------------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod abi {
    use super::*;
    use std::cell::RefCell;

    struct State {
        field: Option<Field>,
        input: Vec<f32>,
        boxes: Vec<f32>,
        tris: Vec<f32>,
        out: Vec<f64>,
        records: Vec<f32>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State { field: None, input: Vec::new(), boxes: Vec::new(), tris: Vec::new(), out: vec![0.0; 64], records: Vec::new() });
    }

    /// The flood (distance then parent, one f32 each per cell) into `field_records`; returns the floats.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_flood_export() -> u32 {
        with(|s| {
            let State { field, records, .. } = s;
            let Some(f) = field.as_ref() else { return 0 };
            f.flood_export(records);
            records.len() as u32
        })
    }

    /// A flood computed elsewhere (the worker), from the scratch area.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_flood_import(floats: u32) {
        with(|s| {
            let State { field, input, .. } = s;
            if let Some(f) = field.as_mut() {
                let n = (floats as usize).min(input.len());
                f.flood_import(&input[..n]);
            }
        })
    }

    /// out = [n, then per plane nx, ny, nz, offset, absorption] of the air cell nearest the point.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_planes(x: f64, y: f64, z: f64) {
        with(|s| {
            let State { field, out, .. } = s;
            out[0] = 0.0;
            let Some(f) = field.as_ref() else { return };
            let Some(c) = f.planes_at(v3(x, y, z)) else { return };
            out[0] = f64::from(c.nplanes);
            for j in 0..c.nplanes as usize {
                for k in 0..5 {
                    out[1 + j * 5 + k] = f64::from(c.planes[j][k]);
                }
            }
        })
    }

    fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
        STATE.with(|s| f(&mut s.borrow_mut()))
    }

    /// Scratch area of `floats` f32 (scene records or cell records); valid until the next call.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_alloc(floats: u32) -> u32 {
        with(|s| {
            s.input.clear();
            s.input.resize(floats as usize, 0.0);
            s.input.as_mut_ptr() as u32
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_out() -> u32 {
        with(|s| s.out.as_mut_ptr() as u32)
    }

    /// Pointer to the records written by the last `field_take` / `field_debug`.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_records() -> u32 {
        with(|s| s.records.as_ptr() as u32)
    }

    /// Scene from the scratch area (haa-trace box and triangle records). `rebuild` 0 only stores it
    /// for the next `field_grid` (one BVH build instead of two for a new level).
    #[unsafe(no_mangle)]
    pub extern "C" fn field_scene(n_boxes: u32, n_tris: u32, rebuild: u32) {
        with(|s| {
            let nb = n_boxes as usize * haa_trace::geom::BOX_STRIDE;
            let nt = n_tris as usize * haa_trace::geom::TRI_STRIDE;
            s.boxes = s.input[..nb.min(s.input.len())].to_vec();
            s.tris = s.input[nb.min(s.input.len())..(nb + nt).min(s.input.len())].to_vec();
            if rebuild != 0
                && let Some(f) = s.field.as_mut()
            {
                f.scene = Scene::new(&s.boxes, &s.tris);
            }
        })
    }

    /// The grid (after `field_scene`): origin, cell size, cell counts. Forgets every baked cell.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_grid(ox: f64, oy: f64, oz: f64, cell: f64, nx: u32, ny: u32, nz: u32) {
        with(|s| {
            let scene = Scene::new(&s.boxes, &s.tris);
            s.field = Some(Field::new(scene, Grid { origin: v3(ox, oy, oz), cell, n: [nx as usize, ny as usize, nz as usize] }));
        })
    }

    /// Bake up to `budget` cells nearest to the focus; returns the cells left.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_bake(budget: u32, fx: f64, fy: f64, fz: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(0, |f| f.bake(budget as usize, v3(fx, fy, fz)) as u32))
    }

    /// Records of the cells baked since the last call (`RECORD` floats each); returns their count.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_take() -> u32 {
        with(|s| {
            let State { field, records, .. } = s;
            records.clear();
            let Some(f) = field.as_mut() else { return 0 };
            let list = std::mem::take(&mut f.baked_new);
            for &i in &list {
                f.record(i as usize, records);
            }
            (records.len() / RECORD) as u32
        })
    }

    /// Cell records from the scratch area (a copy baked elsewhere).
    #[unsafe(no_mangle)]
    pub extern "C" fn field_ingest(n: u32) {
        with(|s| {
            let State { field, input, .. } = s;
            if let Some(f) = field.as_mut() {
                let end = (n as usize * RECORD).min(input.len());
                f.ingest(&input[..end]);
            }
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_dirty(lx: f64, ly: f64, lz: f64, hx: f64, hy: f64, hz: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(0, |f| f.dirty(v3(lx, ly, lz), v3(hx, hy, hz)) as u32))
    }

    /// out = [weight, rt60, mfp, volume, absorption, escape, dist sh, open sh] (SH coefficients each).
    #[unsafe(no_mangle)]
    pub extern "C" fn field_sample(x: f64, y: f64, z: f64) {
        with(|s| {
            let State { field, out, .. } = s;
            let Some(f) = field.as_ref() else { return };
            let p = f.sample(v3(x, y, z));
            out[0] = p.weight;
            out[1] = p.rt60;
            out[2] = p.mfp;
            out[3] = p.volume;
            out[4] = p.absorption;
            out[5] = p.escape;
            for k in 0..SH {
                out[6 + k] = f64::from(p.dist[k]);
                out[6 + SH + k] = f64::from(p.open[k]);
            }
        })
    }

    /// 1 when the segment a-b crosses no geometry (one ray; the bounce points of the reflections).
    #[unsafe(no_mangle)]
    pub extern "C" fn field_clear(ax: f64, ay: f64, az: f64, bx: f64, by: f64, bz: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(1, |f| u32::from(f.clear(v3(ax, ay, az), v3(bx, by, bz)))))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_flood(x: f64, y: f64, z: f64, max: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(0, |f| f.flood(v3(x, y, z), max) as u32))
    }

    /// out = [found, visible, lattice, length, excess, via x, y, z, corners, aperture area, its point x y z, section at listener, at source].
    #[unsafe(no_mangle)]
    pub extern "C" fn field_path(sx: f64, sy: f64, sz: f64, lx: f64, ly: f64, lz: f64, want_aperture: u32) {
        with(|s| {
            let State { field, out, .. } = s;
            let Some(f) = field.as_mut() else { return };
            let r = f.path(v3(sx, sy, sz), v3(lx, ly, lz), want_aperture != 0);
            out[0] = f64::from(u8::from(r.found));
            out[1] = f64::from(u8::from(r.visible));
            out[2] = r.lattice;
            out[3] = r.length;
            out[4] = r.excess;
            out[5] = r.via.x;
            out[6] = r.via.y;
            out[7] = r.via.z;
            out[8] = r.corners as f64;
            out[9] = r.aperture.area;
            out[10] = r.aperture.point.x;
            out[11] = r.aperture.point.y;
            out[12] = r.aperture.point.z;
            out[13] = r.aperture.at_listener;
            out[14] = r.aperture.at_source;
        })
    }

    /// Debug: per baked air cell [x, y, z, escape, rt60, flooded distance, up openness, dist mean]; returns the count.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_debug() -> u32 {
        with(|s| {
            let State { field, records, .. } = s;
            records.clear();
            let Some(f) = field.as_ref() else { return 0 };
            let mut n = 0;
            for i in 0..f.grid.len() {
                let Some(c) = f.cell(i) else { continue };
                if c.state != 1 {
                    continue;
                }
                let p = f.grid.centre(i);
                let up = sh_eval(&c.open, v3(0.0, 1.0, 0.0), 2);
                records.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32, c.escape, c.rt60, f.flood_at(i), up as f32, f32::from(c.nplanes)]);
                n += 1;
            }
            n
        })
    }

    /// out = [baked cells, bricks, bytes, flood reached, casts].
    #[unsafe(no_mangle)]
    pub extern "C" fn field_stats() {
        with(|s| {
            let State { field, out, .. } = s;
            let Some(f) = field.as_ref() else { return };
            out[0] = f.baked as f64;
            out[1] = f.bricks() as f64;
            out[2] = f.bytes() as f64;
            out[3] = f.flood_reached as f64;
            out[4] = f.scene.casts as f64;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10 x 10 x 4 m box room (walls 0.3 m), a 2 m door in its +x wall, open ground y < 0.
    fn room() -> Scene {
        let mut b: Vec<f32> = Vec::new();
        let mut boxed = |c: [f32; 3], h: [f32; 3], id: f32| b.extend_from_slice(&[c[0], c[1], c[2], h[0], h[1], h[2], 0.0, 0.0, 0.0, 1.0, 0.05, id]);
        boxed([0.0, -0.5, 0.0], [40.0, 0.5, 40.0], 0.0);
        boxed([0.0, 4.15, 0.0], [5.3, 0.15, 5.3], 1.0);
        boxed([-5.15, 2.0, 0.0], [0.15, 2.0, 5.3], 2.0);
        boxed([0.0, 2.0, -5.15], [5.3, 2.0, 0.15], 3.0);
        boxed([0.0, 2.0, 5.15], [5.3, 2.0, 0.15], 4.0);
        boxed([5.15, 2.0, -3.0], [0.15, 2.0, 2.0], 5.0);
        boxed([5.15, 2.0, 3.0], [0.15, 2.0, 2.0], 6.0);
        boxed([5.15, 3.0, 0.0], [0.15, 1.0, 1.0], 7.0);
        Scene::new(&b, &[])
    }

    #[test]
    fn the_field_sees_the_room_and_the_flood_goes_through_the_door() {
        let grid = Grid { origin: v3(-8.0, 0.0, -8.0), cell: 1.0, n: [20, 4, 16] };
        let mut f = Field::new(room(), grid);
        while f.bake(500, v3(0.0, 1.5, 0.0)) > 0 {}
        // Inside: enclosed, a long decay; outside: open sky.
        let inside = f.sample(v3(0.0, 1.5, 0.0));
        let outside = f.sample(v3(9.0, 1.5, 0.0));
        assert!(inside.weight > 0.9 && inside.escape < 0.1, "inside escape {}", inside.escape);
        assert!(outside.escape > 0.3, "outside escape {}", outside.escape);
        assert!(inside.rt60 > outside.rt60 * 2.0, "rt60 {} vs {}", inside.rt60, outside.rt60);
        // The wall at -x (x = -5) is among the stored planes of the centre cell.
        let cell = f.planes_at(v3(0.0, 1.5, 0.0)).unwrap();
        let wall = (0..cell.nplanes as usize).map(|j| cell.planes[j]).find(|p| p[0] > 0.9);
        assert!(wall.is_some_and(|p| (f64::from(p[3]) + 5.0).abs() < 0.05), "planes {:?}", &cell.planes[..cell.nplanes as usize]);
        // A source inside behind the +x wall, the listener outside off the door: the sound goes
        // around through the door, a few metres longer than the straight line.
        let l = v3(8.0, 1.5, -4.0);
        f.flood(l, 100.0);
        let p = f.path(v3(3.0, 1.5, -3.5), l, true);
        assert!(p.found && !p.visible, "found {} visible {}", p.found, p.visible);
        assert!(p.excess > 0.5 && p.excess < 6.0, "excess {}", p.excess);
        assert!(p.via.z > -1.6 && p.via.z < 1.6, "arrives through the door: via {:?}", (p.via.x, p.via.y, p.via.z));
        // The narrowest section on the way is the 2 x 3 m door.
        assert!(p.aperture.area > 3.0 && p.aperture.area < 9.0 && (p.aperture.point.x - 5.15).abs() < 1.0, "aperture {} at {:?}", p.aperture.area, (p.aperture.point.x, p.aperture.point.z));
    }
}
