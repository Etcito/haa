//! haa-field: a baked, sparse acoustic field of the level (docs/physics-sound.md, "Acoustic field").
//!
//! Instead of casting rays for every sound event, the level is probed once on a coarse voxel grid
//! (1-2 m cells), nearest to the listener first, in the background. Per air cell:
//! - the diffuse field a sound made there returns to that point, traced like the per-voice rays do
//!   (a 48-ray, 6-bounce trace from the cell and next-event estimation back to it): its energy, its
//!   arrival direction and its pre-delay; the decay per bounce and the mean free path, hence RT60;
//!   absorption and escape share;
//! - up to 6 reflecting planes (the nearest distinct surfaces 64 probe rays hit) for the early
//!   reflections;
//! - the 26 neighbour links a straight segment can cross (the flood's graph).
//!
//! A propagation field flooded from the listener's cell through the links (Dijkstra) gives every cell
//! how far sound travels to it around the obstacles; a source reads its path, string-pulled into a few
//! corners, and the narrowest opening along it.
//!
//! Cells are stored quantised (about 56 bytes each). Geometry and ray casts come from haa-trace
//! (oriented boxes, triangles, a BVH). RAA's world is an SDF: a bake could sample the SDF at the cell
//! centres (solid test), sphere-trace it for the probes, and step it for the links.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use haa_trace::trace::{self, ListenerTrace, ReverbOptions};
use haa_trace::{Scene, V3, v3};

/// Probe directions per cell (planes).
pub const DIRS: usize = 64;
/// Rays and bounces of the per-cell trace.
pub const TRACE_RAYS: usize = 48;
pub const TRACE_BOUNCES: usize = 6;
/// A ray that hits nothing within this (m) has escaped.
pub const ESCAPE: f64 = 60.0;
/// Reflecting planes stored per cell.
pub const PLANES: usize = 6;
const BRICK: usize = 4;
/// Floats per exported cell record: index, state, rt60, mfp, absorption, escape, energy, dir xyz,
/// predelay, 2 edge halves, plane count, planes (normal xyz, distance, absorption).
pub const RECORD: usize = 14 + PLANES * 5;

/// A reflecting plane, quantised: octahedral normal (toward the air), distance from the cell centre
/// (cm), absorption (/255).
#[derive(Clone, Copy, Default)]
pub struct Plane {
    pub oct: [i8; 2],
    pub dist_cm: u16,
    pub alpha: u8,
}

/// One cell, quantised.
#[derive(Clone, Copy, Default)]
pub struct Cell {
    /// 0 not baked, 1 air, 2 solid.
    pub state: u8,
    pub nplanes: u8,
    /// Bit k: the segment to neighbour k (`OFFSETS`) is clear.
    pub edges: u32,
    /// rt60 (0-8.5 s), mfp (0-60 m), absorption, escape, energy (dB -80..+40), dir x y z (-1..1), predelay (0-0.25 s).
    q: [u16; 9],
    pub planes: [Plane; PLANES],
}

const Q_RANGES: [(f64, f64); 9] = [(0.0, 8.5), (0.0, 60.0), (0.0, 1.0), (0.0, 1.0), (-80.0, 40.0), (-1.0, 1.0), (-1.0, 1.0), (-1.0, 1.0), (0.0, 0.25)];

fn quant(v: f64, (lo, hi): (f64, f64)) -> u16 {
    (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * 65535.0).round() as u16
}
fn dequant(q: u16, (lo, hi): (f64, f64)) -> f64 {
    lo + (hi - lo) * f64::from(q) / 65535.0
}

/// Octahedral encoding of a unit vector into two i8.
pub fn oct_encode(n: V3) -> [i8; 2] {
    let l1 = n.x.abs() + n.y.abs() + n.z.abs();
    let (mut x, mut y) = (n.x / l1, n.y / l1);
    if n.z < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * ox.signum();
        y = (1.0 - ox.abs()) * oy.signum();
    }
    [(x * 127.0).round() as i8, (y * 127.0).round() as i8]
}

pub fn oct_decode(o: [i8; 2]) -> V3 {
    let (mut x, mut y) = (f64::from(o[0]) / 127.0, f64::from(o[1]) / 127.0);
    let z = 1.0 - x.abs() - y.abs();
    if z < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * ox.signum();
        y = (1.0 - ox.abs()) * oy.signum();
    }
    v3(x, y, z).normalize()
}

impl Cell {
    pub fn rt60(&self) -> f64 {
        dequant(self.q[0], Q_RANGES[0])
    }
    pub fn mfp(&self) -> f64 {
        dequant(self.q[1], Q_RANGES[1])
    }
    pub fn absorption(&self) -> f64 {
        dequant(self.q[2], Q_RANGES[2])
    }
    pub fn escape(&self) -> f64 {
        dequant(self.q[3], Q_RANGES[3])
    }
    /// Diffuse energy of a sound made here, back here, relative to the direct sound at 2 m (dB).
    pub fn energy_db(&self) -> f64 {
        dequant(self.q[4], Q_RANGES[4])
    }
    pub fn dir(&self) -> V3 {
        v3(dequant(self.q[5], Q_RANGES[5]), dequant(self.q[6], Q_RANGES[6]), dequant(self.q[7], Q_RANGES[7]))
    }
    pub fn predelay(&self) -> f64 {
        dequant(self.q[8], Q_RANGES[8])
    }
    #[allow(clippy::too_many_arguments)]
    pub fn set(&mut self, rt60: f64, mfp: f64, absorption: f64, escape: f64, energy_db: f64, dir: V3, predelay: f64) {
        let v = [rt60, mfp, absorption, escape, energy_db, dir.x, dir.y, dir.z, predelay];
        for k in 0..9 {
            self.q[k] = quant(v[k], Q_RANGES[k]);
        }
    }
    /// Plane `j`: normal toward the air and offset dot(n, point on the plane), for a cell centred at `c`.
    pub fn plane(&self, j: usize, c: V3) -> (V3, f64, f64) {
        let p = self.planes[j];
        let n = oct_decode(p.oct);
        (n, n.dot(c) - f64::from(p.dist_cm) / 100.0, f64::from(p.alpha) / 255.0)
    }
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

/// Index in `OFFSETS` of a neighbour offset (each component -1, 0 or 1, not all 0).
pub fn offset_index(d: [i32; 3]) -> usize {
    let k = ((d[0] + 1) * 9 + (d[1] + 1) * 3 + (d[2] + 1)) as usize;
    if k > 13 { k - 1 } else { k }
}

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
    /// The lower corner of the 8 cell centres around `p`, and the fractions (trilinear lookups).
    pub fn base(&self, p: V3) -> ([i64; 3], [f64; 3]) {
        let f = |v: f64, o: f64| (v - o) / self.cell - 0.5;
        let r = [f(p.x, self.origin.x), f(p.y, self.origin.y), f(p.z, self.origin.z)];
        let b = [r[0].floor() as i64, r[1].floor() as i64, r[2].floor() as i64];
        (b, [r[0] - b[0] as f64, r[1] - b[1] as f64, r[2] - b[2] as f64])
    }
}

/// What a point reads (trilinear over the baked air cells around it).
#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub weight: f64,
    pub rt60: f64,
    pub mfp: f64,
    pub absorption: f64,
    pub escape: f64,
    pub energy_db: f64,
    pub dir: V3,
    pub predelay: f64,
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
    queue: Vec<u32>,
    queued_focus: Option<V3>,
    pub baked_new: Vec<u32>,
    pub baked: usize,
    trace: ListenerTrace,
    arrivals: Vec<(f64, f64)>,
    flood_dist: Vec<f32>,
    flood_parent: Vec<i32>,
    pub flood_reached: usize,
    path_pts: Vec<V3>,
    pulled: Vec<V3>,
}

impl Field {
    pub fn new(scene: Scene, grid: Grid) -> Field {
        Field {
            scene,
            grid,
            bricks: HashMap::new(),
            dirs: trace::sphere_directions(DIRS),
            queue: Vec::new(),
            queued_focus: None,
            baked_new: Vec::new(),
            baked: 0,
            trace: ListenerTrace::default(),
            arrivals: Vec::new(),
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

    /// Probe one cell: solid test; 64 plane probes; a 48 x 6 trace and its next-event estimate back to
    /// the centre (the diffuse field of a sound made here); 26 links.
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
        // Reflecting planes: per surface (id) and orientation, the nearest hit.
        let mut planes: Vec<(f64, V3, f64)> = Vec::new();
        let mut keys: Vec<(u32, i32, i32, i32)> = Vec::new();
        for i in 0..DIRS {
            let d = self.dirs[i];
            let Some(h) = self.scene.cast(c, d, 30.0) else { continue };
            if h.toi <= 0.05 {
                continue;
            }
            let mut n = h.normal.normalize();
            if n.dot(c - h.point) < 0.0 {
                n = -n;
            }
            let key = (h.id, (n.x * 4.0).round() as i32, (n.y * 4.0).round() as i32, (n.z * 4.0).round() as i32);
            let dist = n.dot(c - h.point);
            match keys.iter().position(|k| *k == key) {
                Some(j) => {
                    if dist < planes[j].0 {
                        planes[j] = (dist, n, h.absorption);
                    }
                }
                None => {
                    keys.push(key);
                    planes.push((dist, n, h.absorption));
                }
            }
        }
        planes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        for (j, pl) in planes.iter().take(PLANES).enumerate() {
            cell.planes[j] = Plane { oct: oct_encode(pl.1), dist_cm: (pl.0 * 100.0).clamp(0.0, 65535.0) as u16, alpha: (pl.2 * 255.0).round().clamp(0.0, 255.0) as u8 };
            cell.nplanes = (j + 1) as u8;
        }
        trace::trace_listener(&mut self.scene, &mut self.trace, c, TRACE_RAYS, TRACE_BOUNCES, ESCAPE, 0.35);
        let sp = self.trace.space;
        // The source sits on the listener here; in play it stands 1.5 m or more away. The first
        // bounces (the floor at 8.7 ms from 1.5 m up) and the paths under 3.4 m (a corner next to the
        // cell) are then part of the direct sound and the taps: orders >= 2, 10 ms on (they made the
        // doorway and 4 m outside 4-7 dB too wet).
        let o = ReverbOptions { stride: 1, max_order: 99, reference: 2.0, max_decay: 0.985, min_delay: 0.01, first_share: 0.0 };
        let vr = trace::voice_reverb(&mut self.scene, &self.trace, c, &o, &mut self.arrivals);
        let energy_db = if vr.energy > 0.0 { 10.0 * vr.energy.log10() } else { -80.0 };
        // haa-trace clamps its reverb time at 4.5 s; the large buildings ring longer.
        cell.set(rt60(sp.decay_per_bounce, sp.mean_free_path), sp.mean_free_path, sp.absorption, sp.escape, energy_db, vr.direction, vr.predelay);
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

    /// Trilinear read over the baked air cells around `p` (solid and unbaked cells left out), and
    /// only over those linked to the nearest one: a wall thinner than a cell (a 0.3 m wall, a
    /// triangle shell) has air cells on both sides, and the room behind it must not leak in.
    pub fn sample(&self, p: V3) -> Sample {
        let (b, f) = self.grid.base(p);
        let mut s = Sample::default();
        let mut ln_rt = 0.0;
        let mut corners: [(f64, Option<&Cell>); 8] = [(0.0, None); 8];
        let mut near: Option<(usize, f64)> = None;
        for (corner, slot) in corners.iter_mut().enumerate() {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let wgt = (0..3).map(|a| if o[a] == 1 { f[a] } else { 1.0 - f[a] }).product::<f64>();
            let Some(idx) = self.grid.index([b[0] + o[0] as i64, b[1] + o[1] as i64, b[2] + o[2] as i64]) else { continue };
            let Some(c) = self.cell(idx) else { continue };
            if c.state != 1 {
                continue;
            }
            *slot = (wgt, Some(c));
            if near.is_none_or(|n| wgt > n.1) {
                near = Some((corner, wgt));
            }
        }
        let Some((n, _)) = near else { return s };
        let edges = corners[n].1.map_or(0, |c| c.edges);
        let on = [n & 1, (n >> 1) & 1, (n >> 2) & 1];
        for (corner, &(wgt, c)) in corners.iter().enumerate() {
            let Some(c) = c else { continue };
            if wgt <= 0.0 {
                continue;
            }
            if corner != n {
                let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
                let d = [o[0] as i32 - on[0] as i32, o[1] as i32 - on[1] as i32, o[2] as i32 - on[2] as i32];
                if edges & (1 << offset_index(d)) == 0 {
                    continue;
                }
            }
            s.weight += wgt;
            ln_rt += wgt * c.rt60().max(0.05).ln();
            s.mfp += wgt * c.mfp();
            s.absorption += wgt * c.absorption();
            s.escape += wgt * c.escape();
            s.energy_db += wgt * c.energy_db();
            s.dir = s.dir + c.dir() * wgt;
            s.predelay += wgt * c.predelay();
        }
        if s.weight > 0.0 {
            let k = 1.0 / s.weight;
            s.rt60 = (ln_rt * k).exp();
            s.mfp *= k;
            s.absorption *= k;
            s.escape *= k;
            s.energy_db *= k;
            s.dir = s.dir * k;
            s.predelay *= k;
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

    /// Flood from the listener: shortest distance through the links to every baked air cell within `max`.
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
    /// straight legs (each leg checked by one ray), its excess, its first corner and, on request, the
    /// narrowest opening along it.
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

    /// Free cross-section at `p`: the narrowest of 4 horizontal chords through it times the vertical chord.
    fn section(&mut self, p: V3) -> f64 {
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

    /// The narrowest cross-section along a polyline: the vertical chord every 0.25 m (a lintel lowers
    /// it), then the full section at the 3 lowest points and at the corners; and at the two ends.
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
            let a = self.section(p);
            if a < ap.area {
                ap.area = a;
                ap.point = p;
            }
        }
        let n = pts.len();
        ap.at_listener = self.section(pts[0]);
        ap.at_source = self.section(pts[n - 1]);
        ap
    }

    /// The air cell nearest `p` among the 8 around it, with its centre.
    pub fn nearest(&self, p: V3) -> Option<(&Cell, V3)> {
        let (b, _) = self.grid.base(p);
        let mut best: Option<(f64, &Cell, V3)> = None;
        for corner in 0..8 {
            let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let Some(idx) = self.grid.index([b[0] + o[0] as i64, b[1] + o[1] as i64, b[2] + o[2] as i64]) else { continue };
            let Some(c) = self.cell(idx) else { continue };
            if c.state != 1 {
                continue;
            }
            let centre = self.grid.centre(idx);
            let d = (centre - p).length();
            if best.is_none_or(|x| d < x.0) {
                best = Some((d, c, centre));
            }
        }
        best.map(|x| (x.1, x.2))
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

    pub fn flood_at(&self, idx: usize) -> f32 {
        self.flood_dist.get(idx).copied().unwrap_or(f32::INFINITY)
    }

    pub fn record(&self, idx: usize, out: &mut Vec<f32>) {
        let Some(c) = self.cell(idx) else { return };
        let d = c.dir();
        out.extend_from_slice(&[idx as f32, f32::from(c.state), c.rt60() as f32, c.mfp() as f32, c.absorption() as f32, c.escape() as f32, c.energy_db() as f32, d.x as f32, d.y as f32, d.z as f32, c.predelay() as f32]);
        out.push((c.edges & 0xffff) as f32);
        out.push((c.edges >> 16) as f32);
        out.push(f32::from(c.nplanes));
        for p in &c.planes {
            let n = oct_decode(p.oct);
            out.extend_from_slice(&[n.x as f32, n.y as f32, n.z as f32, f32::from(p.dist_cm) / 100.0, f32::from(p.alpha) / 255.0]);
        }
    }

    pub fn ingest(&mut self, r: &[f32]) {
        for rec in r.chunks_exact(RECORD) {
            let mut c = Cell { state: rec[1] as u8, ..Default::default() };
            let f = |i: usize| f64::from(rec[i]);
            c.set(f(2), f(3), f(4), f(5), f(6), v3(f(7), f(8), f(9)), f(10));
            c.edges = (rec[11] as u32) | ((rec[12] as u32) << 16);
            c.nplanes = rec[13] as u8;
            for j in 0..PLANES {
                let k = 14 + j * 5;
                c.planes[j] = Plane { oct: oct_encode(v3(f(k), f(k + 1), f(k + 2))), dist_cm: (f(k + 3) * 100.0).round().clamp(0.0, 65535.0) as u16, alpha: (f(k + 4) * 255.0).round() as u8 };
            }
            self.set_cell(rec[0] as usize, c);
        }
    }

    /// Every baked cell as bytes (to persist a bake, or hand it to another instance of the grid):
    /// index u32, state u8, and for air nplanes u8, edges u32, 9 u16, 6 planes of 5 bytes (little endian).
    pub fn pack(&self, out: &mut Vec<u8>) {
        out.clear();
        for i in 0..self.grid.len() {
            let Some(c) = self.cell(i) else { continue };
            out.extend_from_slice(&(i as u32).to_le_bytes());
            out.push(c.state);
            if c.state != 1 {
                continue;
            }
            out.push(c.nplanes);
            out.extend_from_slice(&c.edges.to_le_bytes());
            for q in c.q {
                out.extend_from_slice(&q.to_le_bytes());
            }
            for p in &c.planes {
                out.extend_from_slice(&[p.oct[0] as u8, p.oct[1] as u8]);
                out.extend_from_slice(&p.dist_cm.to_le_bytes());
                out.push(p.alpha);
            }
        }
    }

    /// Cells from ; returns how many (stops at the first malformed one).
    pub fn unpack(&mut self, b: &[u8]) -> usize {
        let mut k = 0;
        let mut n = 0;
        let u16at = |b: &[u8], k: usize| u16::from_le_bytes([b[k], b[k + 1]]);
        while k + 5 <= b.len() {
            let idx = u32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]) as usize;
            let mut c = Cell { state: b[k + 4], ..Default::default() };
            k += 5;
            if idx >= self.grid.len() {
                break;
            }
            if c.state == 1 {
                if k + 53 > b.len() {
                    break;
                }
                c.nplanes = b[k];
                c.edges = u32::from_le_bytes([b[k + 1], b[k + 2], b[k + 3], b[k + 4]]);
                k += 5;
                for q in 0..9 {
                    c.q[q] = u16at(b, k);
                    k += 2;
                }
                for p in 0..PLANES {
                    c.planes[p] = Plane { oct: [b[k] as i8, b[k + 1] as i8], dist_cm: u16at(b, k + 2), alpha: b[k + 4] };
                    k += 5;
                }
            }
            self.set_cell(idx, c);
            n += 1;
        }
        n
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
        bytes: Vec<u8>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State { field: None, input: Vec::new(), boxes: Vec::new(), tris: Vec::new(), out: vec![0.0; 64], records: Vec::new(), bytes: Vec::new() });
    }

    fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
        STATE.with(|s| f(&mut s.borrow_mut()))
    }

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

    #[unsafe(no_mangle)]
    pub extern "C" fn field_records() -> u32 {
        with(|s| s.records.as_ptr() as u32)
    }

    /// Scene from the scratch area; `rebuild` 0 only stores it for the next `field_grid`.
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

    #[unsafe(no_mangle)]
    pub extern "C" fn field_grid(ox: f64, oy: f64, oz: f64, cell: f64, nx: u32, ny: u32, nz: u32) {
        with(|s| {
            let scene = Scene::new(&s.boxes, &s.tris);
            s.field = Some(Field::new(scene, Grid { origin: v3(ox, oy, oz), cell, n: [nx as usize, ny as usize, nz as usize] }));
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_bake(budget: u32, fx: f64, fy: f64, fz: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(0, |f| f.bake(budget as usize, v3(fx, fy, fz)) as u32))
    }

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

    /// Pack every baked cell into the byte area; returns its length ( points at it).
    #[unsafe(no_mangle)]
    pub extern "C" fn field_pack() -> u32 {
        with(|s| {
            let State { field, bytes, .. } = s;
            let Some(f) = field.as_ref() else { return 0 };
            f.pack(bytes);
            bytes.len() as u32
        })
    }

    /// A byte area of  bytes (for ).
    #[unsafe(no_mangle)]
    pub extern "C" fn field_alloc_bytes(n: u32) -> u32 {
        with(|s| {
            s.bytes.clear();
            s.bytes.resize(n as usize, 0);
            s.bytes.as_mut_ptr() as u32
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_bytes() -> u32 {
        with(|s| s.bytes.as_ptr() as u32)
    }

    /// Cells from the byte area ( of the same grid); returns how many.
    #[unsafe(no_mangle)]
    pub extern "C" fn field_unpack(n: u32) -> u32 {
        with(|s| {
            let State { field, bytes, .. } = s;
            let Some(f) = field.as_mut() else { return 0 };
            let end = (n as usize).min(bytes.len());
            f.unpack(&bytes[..end]) as u32
        })
    }

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

    /// out = [weight, rt60, mfp, absorption, escape, energy dB, dir x y z, predelay].
    #[unsafe(no_mangle)]
    pub extern "C" fn field_sample(x: f64, y: f64, z: f64) {
        with(|s| {
            let State { field, out, .. } = s;
            let Some(f) = field.as_ref() else { return };
            let p = f.sample(v3(x, y, z));
            out[..10].copy_from_slice(&[p.weight, p.rt60, p.mfp, p.absorption, p.escape, p.energy_db, p.dir.x, p.dir.y, p.dir.z, p.predelay]);
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_clear(ax: f64, ay: f64, az: f64, bx: f64, by: f64, bz: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(1, |f| u32::from(f.clear(v3(ax, ay, az), v3(bx, by, bz)))))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_flood(x: f64, y: f64, z: f64, max: f64) -> u32 {
        with(|s| s.field.as_mut().map_or(0, |f| f.flood(v3(x, y, z), max) as u32))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn field_flood_export() -> u32 {
        with(|s| {
            let State { field, records, .. } = s;
            let Some(f) = field.as_ref() else { return 0 };
            f.flood_export(records);
            records.len() as u32
        })
    }

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
            let Some((c, centre)) = f.nearest(v3(x, y, z)) else { return };
            out[0] = f64::from(c.nplanes);
            for j in 0..c.nplanes as usize {
                let (n, offset, alpha) = c.plane(j, centre);
                out[1 + j * 5..1 + j * 5 + 5].copy_from_slice(&[n.x, n.y, n.z, offset, alpha]);
            }
        })
    }

    /// out = [found, visible, lattice, length, excess, via x, y, z, corners, aperture area, its point x y z, section at listener, at source].
    #[unsafe(no_mangle)]
    pub extern "C" fn field_path(sx: f64, sy: f64, sz: f64, lx: f64, ly: f64, lz: f64, want_aperture: u32) {
        with(|s| {
            let State { field, out, .. } = s;
            let Some(f) = field.as_mut() else { return };
            let r = f.path(v3(sx, sy, sz), v3(lx, ly, lz), want_aperture != 0);
            let a = r.aperture;
            out[..15].copy_from_slice(&[
                f64::from(u8::from(r.found)), f64::from(u8::from(r.visible)), r.lattice, r.length, r.excess,
                r.via.x, r.via.y, r.via.z, r.corners as f64, a.area, a.point.x, a.point.y, a.point.z, a.at_listener, a.at_source,
            ]);
        })
    }

    /// Debug: per baked air cell [x, y, z, escape, rt60, flooded distance, energy dB, plane count]; returns the count.
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
                records.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32, c.escape() as f32, c.rt60() as f32, f.flood_at(i), c.energy_db() as f32, f32::from(c.nplanes)]);
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
            out[..5].copy_from_slice(&[f.baked as f64, f.bricks() as f64, f.bytes() as f64, f.flood_reached as f64, f.scene.casts as f64]);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(b: &mut Vec<f32>, c: [f32; 3], h: [f32; 3], id: f32) {
        b.extend_from_slice(&[c[0], c[1], c[2], h[0], h[1], h[2], 0.0, 0.0, 0.0, 1.0, 0.05, id]);
    }

    /// A 10 x 10 x 4 m box room (walls 0.3 m), a 2 m door in its +x wall, open ground y < 0.
    fn room() -> Scene {
        let mut b: Vec<f32> = Vec::new();
        boxed(&mut b, [0.0, -0.5, 0.0], [40.0, 0.5, 40.0], 0.0);
        boxed(&mut b, [0.0, 4.15, 0.0], [5.3, 0.15, 5.3], 1.0);
        boxed(&mut b, [-5.15, 2.0, 0.0], [0.15, 2.0, 5.3], 2.0);
        boxed(&mut b, [0.0, 2.0, -5.15], [5.3, 2.0, 0.15], 3.0);
        boxed(&mut b, [0.0, 2.0, 5.15], [5.3, 2.0, 0.15], 4.0);
        boxed(&mut b, [5.15, 2.0, -3.0], [0.15, 2.0, 2.0], 5.0);
        boxed(&mut b, [5.15, 2.0, 3.0], [0.15, 2.0, 2.0], 6.0);
        boxed(&mut b, [5.15, 3.0, 0.0], [0.15, 1.0, 1.0], 7.0);
        Scene::new(&b, &[])
    }

    #[test]
    fn the_field_sees_the_room_and_the_flood_goes_through_the_door() {
        let grid = Grid { origin: v3(-8.0, 0.0, -8.0), cell: 1.0, n: [20, 4, 16] };
        let mut f = Field::new(room(), grid);
        while f.bake(500, v3(0.0, 1.5, 0.0)) > 0 {}
        let inside = f.sample(v3(0.0, 1.5, 0.0));
        let outside = f.sample(v3(9.0, 1.5, 0.0));
        assert!(inside.weight > 0.9 && inside.escape < 0.1, "inside escape {}", inside.escape);
        assert!(outside.escape > 0.3, "outside escape {}", outside.escape);
        assert!(inside.rt60 > outside.rt60 * 2.0, "rt60 {} vs {}", inside.rt60, outside.rt60);
        // The traced diffuse field is much stronger inside than outside.
        assert!(inside.energy_db > outside.energy_db + 10.0, "energy {} vs {}", inside.energy_db, outside.energy_db);
        // The wall at -x (x = -5) is among the stored planes of the centre cell (within the quantisation).
        let (cell, centre) = f.nearest(v3(0.0, 1.5, 0.0)).unwrap();
        let wall = (0..cell.nplanes as usize).map(|j| cell.plane(j, centre)).find(|p| p.0.x > 0.9);
        assert!(wall.is_some_and(|p| (p.1 + 5.0).abs() < 0.05), "no -x wall plane");
        // A packed bake read back into a fresh field gives the same samples.
        let mut bytes = Vec::new();
        f.pack(&mut bytes);
        let mut g = Field::new(room(), grid);
        assert_eq!(g.unpack(&bytes), f.baked);
        let again = g.sample(v3(0.0, 1.5, 0.0));
        assert!((again.energy_db - inside.energy_db).abs() < 1e-9 && (again.rt60 - inside.rt60).abs() < 1e-9);
        let l = v3(8.0, 1.5, -4.0);
        f.flood(l, 100.0);
        let p = f.path(v3(3.0, 1.5, -3.5), l, true);
        assert!(p.found && !p.visible, "found {} visible {}", p.found, p.visible);
        assert!(p.excess > 0.5 && p.excess < 6.0, "excess {}", p.excess);
        assert!(p.via.z > -1.6 && p.via.z < 1.6, "arrives through the door: via {:?}", (p.via.x, p.via.y, p.via.z));
        assert!(p.aperture.area > 3.0 && p.aperture.area < 9.0 && (p.aperture.point.x - 5.15).abs() < 1.0, "aperture {} at {:?}", p.aperture.area, (p.aperture.point.x, p.aperture.point.z));
    }

    #[test]
    fn a_room_made_of_triangles_is_a_room() {
        // A closed 8 x 3 x 6 m room of 12 triangles (no boxes but the ground under it).
        let (lo, hi) = ([-4.0f32, 0.0, -3.0], [4.0f32, 3.0, 3.0]);
        let c = |i: usize| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
        let quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        let mut t: Vec<f32> = Vec::new();
        for (id, q) in quads.iter().enumerate() {
            for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                for v in tri {
                    t.extend_from_slice(&c(v));
                }
                t.extend_from_slice(&[0.05, id as f32]);
            }
        }
        let grid = Grid { origin: v3(-5.0, 0.0, -4.0), cell: 1.0, n: [10, 3, 8] };
        let mut f = Field::new(Scene::new(&[], &t), grid);
        while f.bake(500, v3(0.0, 1.5, 0.0)) > 0 {}
        let s = f.sample(v3(0.0, 1.5, 0.0));
        assert!(s.weight > 0.9 && s.escape < 0.05 && s.rt60 > 1.0, "escape {} rt60 {}", s.escape, s.rt60);
        // 6 walls seen from the middle: the planes are the triangles' (the +x wall 4 m away).
        let (cell, centre) = f.nearest(v3(0.0, 1.5, 0.0)).unwrap();
        assert!(cell.nplanes >= 5, "planes {}", cell.nplanes);
        let px = (0..cell.nplanes as usize).map(|j| cell.plane(j, centre)).find(|p| p.0.x < -0.9);
        assert!(px.is_some_and(|p| (p.1 + 4.0).abs() < 0.1), "no +x wall plane");
        // The shell has no thickness: air cells on both sides. Next to the wall inside, the open
        // cells outside (half a cell away) must not leak into the read.
        for (k, o) in OFFSETS.iter().enumerate() {
            assert_eq!(offset_index([o.0, o.1, o.2]), k);
        }
        let wall = f.sample(v3(-3.9, 1.5, 0.3));
        assert!(wall.escape < 0.05 && wall.rt60 > 1.0, "next to the shell: escape {} rt60 {}", wall.escape, wall.rt60);
    }
}
