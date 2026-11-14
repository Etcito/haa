#![allow(non_snake_case)]
//! `rainOnFace` source node: rain falling on one face of the level (a roof, a plate, the ground).
//! HAA-only (docs/physics-sound.md, "Rain on faces"). The runtime measures the face and passes:
//! - `intensity`: how hard it rains (0..1);
//! - `flux`: the share of the rain the face receives, max(0, −n·d) × its exposed share (a vertical
//!   wall under vertical rain gets 0, a roof facing wind-driven rain 1);
//! - `area`: the exposed area (m²): the drop rate is flux × area, so a small face gives sparse
//!   separate drops and a large one a dense wash;
//! - `hardness`, `shape`, `dim`, `thickness`: the face's material and its hand-chosen resonator
//!   (contract `SoundBody`): a metal sheet pings and rings, tiles clink, wood knocks, concrete
//!   ticks dully, snow and earth are nearly silent.
//!
//! Each drop is a short noise splash (shorter and brighter on harder surfaces, bigger drops longer
//! and louder) that drives a 6-mode resonator bank of the face's shape. Faces over 2 m² alternate
//! two banks tuned ±7 % apart (many panels, not one bell). Drops are Poisson; above 4000 drops/s the
//! drops scale up by √(rate/4000) instead (incoherent drops add in power). The area beyond 25 m²
//! adds density, not level: the far parts of a large face are the spatial chain's distance.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;

const INTENSITY: usize = 0;
const FLUX: usize = 1;
const AREA: usize = 2;
const HARDNESS: usize = 3;
const SHAPE: usize = 4;
const DIM: usize = 5;
const THICKNESS: usize = 6;
const LEVEL: usize = 7;

const MODES: usize = 6;
const SPLASHES: usize = 48;
/// Drops per m² per second at full intensity.
const DROPS_PER_M2: f64 = 600.0;
/// Area (m²) beyond which a face only gets denser.
const LOUD_AREA: f64 = 25.0;
/// Most drops scheduled per second; beyond, each drop stands for several.
const MAX_RATE: f64 = 4000.0;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    scale: Scale,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

const PARAMS: [ParamDef; 8] = [
    number("intensity", 0.0, 1.0, 0.6, 0.01, Scale::Lin, "", "Rain intensity", "Intensité de la pluie"),
    number("flux", 0.0, 1.0, 1.0, 0.01, Scale::Lin, "", "Rain received (normal · direction)", "Pluie reçue (normale · direction)"),
    number("area", 0.01, 10000.0, 4.0, 0.01, Scale::Log, "m²", "Face area", "Surface de la face"),
    number("hardness", 0.0, 1.0, 0.8, 0.01, Scale::Lin, "", "Hardness", "Dureté"),
    number("shape", 0.0, 6.0, 1.0, 1.0, Scale::Lin, "", "Shape (block, plate, sheet, bar, box, shell, bottle)", "Forme (bloc, plaque, tôle, barre, caisse, coque, bouteille)"),
    number("dim", 0.01, 10.0, 0.3, 0.01, Scale::Log, "m", "Size", "Taille"),
    number("thickness", 0.0005, 0.5, 0.01, 0.0001, Scale::Log, "m", "Thickness", "Épaisseur"),
    number("level", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Level", "Niveau"),
];

/// The `rainOnFace` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "rainOnFace",
    category: Category::Source,
    label_en: "Rain on a face",
    label_fr: "Pluie sur une face",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let sr = if init.sample_rate.is_finite() && init.sample_rate > 0.0 { init.sample_rate } else { 48000.0 };
    Box::new(RainOnFace::new(init.seed, sr))
}

/// Mode frequency ratios of each shape (block, plate, sheet, bar, box, shell, bottle).
const RATIOS: [[f64; MODES]; 7] = [
    [1.0, 1.48, 2.1, 2.9, 3.7, 4.6],
    [1.0, 1.59, 2.14, 2.3, 2.65, 2.92],
    [1.0, 1.31, 1.73, 2.05, 2.42, 2.87],
    [1.0, 2.76, 5.4, 8.93, 13.34, 18.64],
    [1.0, 2.3, 3.1, 3.8, 4.6, 5.9],
    [1.0, 2.32, 4.25, 6.63, 9.38, 12.5],
    [1.0, 3.0, 4.9, 6.8, 8.6, 10.7],
];
/// Base frequency (Hz) of each shape at 0.3 m, before hardness.
const BASE_HZ: [f64; 7] = [600.0, 900.0, 1400.0, 700.0, 250.0, 1200.0, 1000.0];
/// Ring time (s, 60 dB) of each shape's first mode on the hardest material.
const RING_S: [f64; 7] = [0.012, 0.06, 0.25, 0.4, 0.05, 0.5, 0.3];
/// How much of each drop goes into the modes (the rest is the splash itself).
const RING_MIX: [f64; 7] = [0.15, 0.5, 0.8, 0.6, 0.55, 0.75, 0.6];

struct Bank {
    a1: [f64; MODES],
    a2: [f64; MODES],
    g: [f64; MODES],
    y1: [f64; MODES],
    y2: [f64; MODES],
}

impl Bank {
    fn new() -> Bank {
        Bank { a1: [0.0; MODES], a2: [0.0; MODES], g: [0.0; MODES], y1: [0.0; MODES], y2: [0.0; MODES] }
    }

    fn tune(&mut self, f0: f64, ratios: &[f64; MODES], ring: f64, sr: f64) {
        for k in 0..MODES {
            let f = (f0 * ratios[k]).min(sr * 0.45);
            let t60 = ring / (1.0 + 0.6 * k as f64);
            let r = (-6.9078 / (t60.max(0.001) * sr)).exp();
            let w = std::f64::consts::TAU * f / sr;
            self.a1[k] = 2.0 * r * w.cos();
            self.a2[k] = -r * r;
            // Equal peak energy per mode, higher modes a little weaker.
            self.g[k] = (1.0 - r * r).sqrt() / (1.0 + 0.35 * k as f64);
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let mut out = 0.0;
        for k in 0..MODES {
            let y = self.a1[k] * self.y1[k] + self.a2[k] * self.y2[k] + self.g[k] * x;
            self.y2[k] = self.y1[k];
            self.y1[k] = y;
            out += y;
        }
        out
    }
}

struct RainOnFace {
    seed: u32,
    rng: Rng,
    sr: f64,
    next_drop: f64,
    env: [f64; SPLASHES],
    decay: [f64; SPLASHES],
    amp: [f64; SPLASHES],
    pan: [f64; SPLASHES],
    bank_of: [usize; SPLASHES],
    noise: u32,
    lp: [f64; 2],
    banks: [Bank; 2],
    tuned: [f64; 5],
    counter: u32,
}

impl RainOnFace {
    fn new(seed: u32, sr: f64) -> RainOnFace {
        RainOnFace {
            seed,
            rng: Rng::new(seed ^ 0x5eed_7a1e),
            sr,
            next_drop: 0.0,
            env: [0.0; SPLASHES],
            decay: [0.0; SPLASHES],
            amp: [0.0; SPLASHES],
            pan: [0.5; SPLASHES],
            bank_of: [0; SPLASHES],
            noise: 0x1234_5678 ^ seed.max(1),
            lp: [0.0; 2],
            banks: [Bank::new(), Bank::new()],
            tuned: [-1.0; 5],
            counter: 0,
        }
    }

    fn retune(&mut self, p: &[f64]) {
        let shape = (p[SHAPE].round() as usize).min(6);
        let h = p[HARDNESS].clamp(0.0, 1.0);
        let key = [shape as f64, h, p[DIM], p[THICKNESS], p[AREA].min(2.5)];
        if key.iter().zip(self.tuned.iter()).all(|(a, b)| (a - b).abs() < 1e-3) {
            return;
        }
        self.tuned = key;
        // A drop excites a large face locally: the patch of a sheet rings, at most half a metre of
        // it, and a slab (the ground, a concrete roof) answers with its surface, not its whole size.
        let dim = match shape {
            0 => p[DIM].min(0.3),
            2 => p[DIM].min(0.5),
            _ => p[DIM],
        };
        // Stiffer materials ring higher (sqrt(E/rho)): wood well under tiles, metal above both.
        let stiff = 0.35 + 1.3 * h * h;
        let f0 = (BASE_HZ[shape] * (0.3 / dim.max(0.01)).powf(0.7) * stiff).clamp(60.0, 9000.0);
        // Thin plates and sheets ring longer; soft materials barely ring at all.
        let thin = if shape == 1 || shape == 2 { (0.003 / p[THICKNESS].max(0.0005)).powf(0.3).clamp(0.5, 2.0) } else { 1.0 };
        let ring = RING_S[shape] * thin * (0.05 + 0.95 * h * h);
        self.banks[0].tune(f0, &RATIOS[shape], ring, self.sr);
        let spread = if p[AREA] > 2.0 { 1.07 } else { 1.0 };
        self.banks[1].tune(f0 * spread, &RATIOS[shape], ring, self.sr);
    }

    fn white(&mut self) -> f64 {
        let mut s = self.noise;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.noise = s;
        f64::from(s) / 2_147_483_648.0 - 1.0
    }

    fn drop(&mut self, p: &[f64], weight: f64) {
        let mut slot = 0;
        let mut quietest = f64::INFINITY;
        for i in 0..SPLASHES {
            let e = self.env[i] * self.amp[i];
            if e < quietest {
                quietest = e;
                slot = i;
            }
        }
        let h = p[HARDNESS].clamp(0.0, 1.0);
        // Drop size: mostly small, a few big ones (heavier rain, bigger drops).
        let u = self.rng.next_f64();
        let size = (0.25 + 0.75 * u * u) * (0.7 + 0.3 * p[INTENSITY]);
        // A big drop on a soft surface splashes longer; a hard surface makes a short tick.
        let contact_s = (0.00015 + 0.0009 * size) * (1.6 - h);
        self.env[slot] = 1.0;
        self.decay[slot] = (-1.0 / (contact_s * self.sr)).exp();
        self.amp[slot] = size.powf(1.5) * weight;
        self.pan[slot] = self.rng.next_f64();
        self.bank_of[slot] = (self.rng.next_u32() & 1) as usize;
    }
}

impl Node for RainOnFace {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        if self.counter % 64 == 0 {
            self.retune(p);
        }
        self.counter = self.counter.wrapping_add(1);
        let flux = p[FLUX].clamp(0.0, 1.0);
        let intensity = p[INTENSITY].clamp(0.0, 1.0);
        let rate = DROPS_PER_M2 * intensity.powf(1.3) * flux * p[AREA].clamp(0.0, LOUD_AREA);
        let density = if p[AREA] > LOUD_AREA { (p[AREA] / LOUD_AREA).sqrt().min(4.0) } else { 1.0 };
        let scheduled = (rate * density).min(MAX_RATE);
        if scheduled > 0.0 {
            // Each scheduled drop carries the power of rate/scheduled real ones.
            let weight = (rate / scheduled).sqrt();
            self.next_drop -= 1.0;
            let mut n = 0;
            while self.next_drop <= 0.0 && n < 16 {
                n += 1;
                self.drop(p, weight);
                let u = self.rng.next_f64().max(1e-9);
                self.next_drop += -u.ln() * self.sr / scheduled;
            }
        } else {
            self.next_drop = 0.0;
        }
        let h = p[HARDNESS].clamp(0.0, 1.0);
        let mut splash = [0.0; 2];
        let mut excite = [0.0; 2];
        for i in 0..SPLASHES {
            if self.env[i] < 1e-4 {
                continue;
            }
            let v = self.white() * self.env[i] * self.amp[i];
            self.env[i] *= self.decay[i];
            splash[0] += v * (1.0 - self.pan[i]).sqrt();
            splash[1] += v * self.pan[i].sqrt();
            excite[self.bank_of[i]] += v;
        }
        // Harder surfaces tick brighter: a one-pole low-pass from 1.2 kHz (snow) to 11 kHz (metal).
        let cutoff = 1200.0 + 9800.0 * h;
        let a = (-std::f64::consts::TAU * cutoff / self.sr).exp();
        for c in 0..2 {
            self.lp[c] = splash[c] + a * (self.lp[c] - splash[c]);
        }
        let shape = (p[SHAPE].round() as usize).min(6);
        let ring_mix = RING_MIX[shape] * h;
        let m0 = self.banks[0].process(excite[0]);
        let m1 = self.banks[1].process(excite[1]);
        // Soft ground is nearly silent; the modes carry the ring of hard faces.
        let loud = 0.08 + 0.92 * h.powf(1.5);
        let gain = p[LEVEL] * loud * 0.35;
        let l = (self.lp[0] * (1.0 - ring_mix) + (m0 * 0.8 + m1 * 0.2) * ring_mix * 4.0) * gain;
        let r = (self.lp[1] * (1.0 - ring_mix) + (m0 * 0.2 + m1 * 0.8) * ring_mix * 4.0) * gain;
        NodeOut { l: if l.is_finite() { l } else { 0.0 }, r: if r.is_finite() { r } else { 0.0 }, value: 0.0 }
    }

    fn reset(&mut self) {
        *self = RainOnFace::new(self.seed, self.sr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 48000.0;

    fn render(params: &[f64; 8], seconds: f64) -> Vec<f64> {
        let mut node = (DEF.create)(&NodeInit {
            id: "rain1",
            seed: 7,
            sample_rate: SR,
            params,
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        let n = (seconds * SR) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let ctx = SampleCtx {
                sample_rate: SR,
                dt: 1.0 / SR,
                t: 0.0,
                seconds: i as f64 / SR,
                duration: 1.0,
                mode: GraphMode::Live,
                live_frames: i as u64,
                looped: false,
                perf_rebase: false,
                inputs: &[],
                triggers: &[],
            };
            let o = node.process_sample(&ctx, (0.0, 0.0), params);
            out.push((o.l * o.l + o.r * o.r) / 2.0);
        }
        out
    }

    fn db(x: &[f64]) -> f64 {
        10.0 * (x.iter().sum::<f64>() / x.len() as f64 + 1e-30).log10()
    }

    #[test]
    fn flux_and_area_set_the_rain_and_the_shape_sets_its_ring() {
        // intensity, flux, area, hardness, shape, dim, thickness, level
        let wall = render(&[0.6, 0.0, 10.0, 0.9, 0.0, 2.0, 0.3, 0.5], 0.5);
        assert!(db(&wall) < -150.0, "a face that receives no rain is silent: {}", db(&wall));
        let plate = render(&[0.6, 1.0, 0.3, 0.9, 1.0, 0.3, 0.01, 0.5], 1.0);
        let roof = render(&[0.6, 1.0, 20.0, 0.9, 1.0, 0.3, 0.01, 0.5], 1.0);
        assert!(db(&roof) > db(&plate) + 6.0, "a large face is louder: roof {} plate {}", db(&roof), db(&plate));
        // Sparse drops on the small plate: most 5 ms windows are near silence, unlike the roof's wash.
        let quiet = |x: &[f64]| {
            let w: Vec<f64> = x.chunks(240).map(|c| c.iter().sum::<f64>() / 240.0).collect();
            let mean = w.iter().sum::<f64>() / w.len() as f64;
            w.iter().filter(|&&v| v < mean * 0.1).count() as f64 / w.len() as f64
        };
        assert!(quiet(&plate) > quiet(&roof) + 0.2, "plate {} roof {}", quiet(&plate), quiet(&roof));
        // The same rain on a thin metal sheet rings on; on a concrete block it is dead within milliseconds.
        let tail = |x: &[f64]| db(&x[x.len() / 2..]);
        let sheet = render(&[0.15, 1.0, 0.05, 1.0, 2.0, 1.0, 0.001, 0.5], 1.0);
        let block = render(&[0.15, 1.0, 0.05, 0.9, 0.0, 1.0, 0.3, 0.5], 1.0);
        let silence = |x: &[f64]| x.iter().filter(|&&v| v < 1e-10).count() as f64 / x.len() as f64;
        assert!(silence(&sheet) < silence(&block), "sheet {} block {} ({} {})", silence(&sheet), silence(&block), tail(&sheet), tail(&block));
    }
}
