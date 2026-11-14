//! `blast` source node: a pressure wave, the dry part of a gunshot or a detonation. HAA-only node.
//!
//! A muzzle blast or an explosion reaches a microphone as a Friedlander wave: an instant rise to
//! the peak overpressure, a decay through zero in `duration`, then a longer negative phase
//! (`negative`, its depth). `p(t) = (1 - t/T)·e^(-b·t/T)`, the instant rise being the crack that a
//! filtered-noise burst cannot make. On top of it rides the turbulence of the expanding gas: noise
//! with an instant attack decaying in `noiseDecay` (T60), band-limited by `tone` (a one-pole
//! low-pass) and scaled by `noise`. The wave fires once after `delay`, and again on each restart,
//! with a slight random spread of duration and noise per take (the noise stream is not replayed
//! by a reset). Mono.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;
use core::f64::consts::TAU;

const DURATION: usize = 0;
const NEGATIVE: usize = 1;
const NOISE: usize = 2;
const NOISE_DECAY: usize = 3;
const TONE: usize = 4;
const DELAY: usize = 5;
const LEVEL: usize = 6;

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
    ParamDef { key, kind: ParamKind::Number { min, max, default, step, scale }, unit, label_en, label_fr, modulatable: true, smooth: true }
}

/// The `blast` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "blast",
    category: Category::Source,
    label_en: "Blast (pressure wave)",
    label_fr: "Souffle (onde de pression)",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        number("duration", 0.0001, 0.05, 0.0015, 0.0001, Scale::Log, "s", "Positive phase", "Phase positive"),
        number("negative", 0.0, 1.0, 0.4, 0.01, Scale::Lin, "", "Negative phase", "Phase négative"),
        number("noise", 0.0, 2.0, 0.5, 0.01, Scale::Lin, "", "Turbulence", "Turbulence"),
        number("noiseDecay", 0.001, 2.0, 0.03, 0.001, Scale::Log, "s", "Turbulence decay", "Décroissance de la turbulence"),
        number("tone", 200.0, 20_000.0, 9_000.0, 1.0, Scale::Log, "Hz", "Turbulence tone", "Couleur de la turbulence"),
        number("delay", 0.0, 2.0, 0.006, 0.001, Scale::Lin, "s", "Start delay", "Délai initial"),
        number("level", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "", "Level", "Niveau"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Blast::new(init.sample_rate, init.seed))
}

struct Blast {
    sr: f64,
    seed: u32,
    rng: Rng,
    frame: u64,
    /// Per-take spread of the duration and of the noise level.
    take_t: f64,
    take_n: f64,
    lp: f64,
    dc_x: f64,
    dc_y: f64,
    dc_r: f64,
}

impl Blast {
    fn new(sample_rate: f64, seed: u32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate > 1.0 { sample_rate } else { 48_000.0 };
        let mut b = Blast {
            sr,
            seed,
            rng: Rng::new(seed ^ 0x51ed_270b),
            frame: 0,
            take_t: 1.0,
            take_n: 1.0,
            lp: 0.0,
            dc_x: 0.0,
            dc_y: 0.0,
            dc_r: (-TAU * 12.0 / sr).exp(),
        };
        b.draw_take();
        b
    }

    fn draw_take(&mut self) {
        self.take_t = 0.85 + 0.3 * self.rng.next_f64();
        self.take_n = 0.8 + 0.4 * self.rng.next_f64();
    }
}

impl Node for Blast {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        let t = self.frame as f64 / self.sr - p[DELAY].clamp(0.0, 2.0);
        self.frame += 1;
        if t < 0.0 {
            return NodeOut { l: 0.0, r: 0.0, value: 0.0 };
        }
        let dur = p[DURATION].clamp(0.0001, 0.05) * self.take_t;
        // Friedlander: b sets how deep and long the negative phase is (b 1 deep and long, 4 shallow).
        let b = 4.0 - 3.0 * p[NEGATIVE].clamp(0.0, 1.0);
        let x = t / dur;
        let wave = if x < 30.0 { (1.0 - x) * (-b * x).exp() } else { 0.0 };
        let decay = p[NOISE_DECAY].clamp(0.001, 2.0);
        let env = (-6.907_755 * t / decay).exp();
        let mut out = wave;
        if env > 1e-5 {
            let white = self.rng.next_f64() * 2.0 - 1.0;
            let a = (-TAU * p[TONE].clamp(200.0, 20_000.0) / self.sr).exp();
            self.lp = white + a * (self.lp - white);
            out += p[NOISE].clamp(0.0, 2.0) * self.take_n * env * self.lp * 1.5;
        }
        let out = out * p[LEVEL].clamp(0.0, 4.0);
        // Air carries no DC: a 12 Hz blocker leaves the wave's shape and removes the net push of a shallow
        // negative phase (which otherwise reads as sub-bass energy).
        let y = out - self.dc_x + self.dc_r * self.dc_y;
        self.dc_x = out;
        self.dc_y = y;
        let out = if y.is_finite() { y } else { 0.0 };
        NodeOut { l: out, r: out, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sr).abs() > 0.5 || init.seed != self.seed {
            *self = Blast::new(init.sample_rate, init.seed);
        }
    }

    /// The wave again on a restart, with a new take (the random stream carries on).
    fn reset(&mut self) {
        self.frame = 0;
        self.lp = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
        self.draw_take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 44_100.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: SR,
            dt: 1.0 / SR,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn render(node: &mut dyn Node, p: &[f64], frames: usize) -> Vec<f64> {
        let c = ctx();
        (0..frames).map(|_| node.process_sample(&c, (0.0, 0.0), p).l).collect()
    }

    #[test]
    fn the_wave_rises_at_once_turns_negative_after_its_phase_and_fires_again_on_restart() {
        // duration 2 ms, negative 0.5, no turbulence, no delay, level 1
        let p = [0.002, 0.5, 0.0, 0.03, 9_000.0, 0.0, 1.0];
        let mut node = (DEF.create)(&NodeInit {
            id: "blast1", seed: 3, sample_rate: SR, params: &p, texts: &[], curve: None, inputs: &[], duration: 1.0,
        });
        let a = render(node.as_mut(), &p, 4410);
        // the peak is the first sample: an instant rise
        let peak = a.iter().cloned().fold(f64::MIN, f64::max);
        assert!((a[0] - peak).abs() < 1e-9 && peak > 0.9, "peak {peak} first {}", a[0]);
        // positive for the phase (1.7..2.3 ms with the take spread), negative after, and no net push
        assert!(a[(0.0012 * SR) as usize] > 0.0);
        assert!(a[(0.004 * SR) as usize] < 0.0);
        let mean: f64 = a.iter().sum::<f64>() / a.len() as f64;
        assert!(mean.abs() < 0.01, "DC {mean}");
        node.reset();
        let b = render(node.as_mut(), &p, 4410);
        assert!(b[0] > 0.9, "a restart fires the wave again");
        assert!(a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 1e-6), "every take differs");
    }
}
