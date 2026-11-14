//! `gate` effect node: a noise gate / downward expander with a side-chain key. HAA-only (not a Sona type).
//!
//! The detector follows a level (`detector`): the audio input's peak (`input`, a classic gate), the `key` param
//! (`key`: any control wired into it, e.g. a `notes` node's gate output or a `curve` holding an envelope, so the gate
//! opens where that control says something sounds), or the lower of both (`both`). Above `threshold` the gain is unity;
//! below it the signal is pushed down by (threshold - level) x (`ratio` - 1) dB, at most `range` dB (`ratio` 20 is a
//! hard gate, 2 a gentle expander). The gain opens in `attack`, stays open `hold` seconds after the level falls under
//! the threshold, then closes in `release`.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const THRESHOLD: usize = 0;
const RATIO: usize = 1;
const RANGE: usize = 2;
const ATTACK: usize = 3;
const HOLD: usize = 4;
const RELEASE: usize = 5;
const KEY: usize = 6;
const DETECTOR: usize = 7;

const DETECT_INPUT: usize = 0;
const DETECT_KEY: usize = 1;
/// Release of the input peak follower (s): short, so the gain computer sees the envelope, not each cycle.
const FOLLOW_RELEASE: f64 = 0.02;

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
    smooth: bool,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth,
    }
}

static PARAMS: [ParamDef; 8] = [
    number("threshold", -100.0, 0.0, -50.0, 0.5, Scale::Lin, "dB", "Threshold", "Seuil", true),
    number("ratio", 1.0, 20.0, 20.0, 0.1, Scale::Lin, ":1", "Expansion ratio", "Taux d'expansion", true),
    number("range", 0.0, 100.0, 80.0, 0.5, Scale::Lin, "dB", "Range (most attenuation)", "Plage (atténuation max.)", true),
    number("attack", 0.0005, 0.2, 0.002, 0.0005, Scale::Log, "s", "Attack", "Attaque", true),
    number("hold", 0.0, 2.0, 0.03, 0.01, Scale::Lin, "s", "Hold", "Maintien", true),
    number("release", 0.005, 3.0, 0.1, 0.005, Scale::Log, "s", "Release", "Retour", true),
    number("key", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Side-chain key level", "Niveau de la clé (side-chain)", false),
    ParamDef {
        key: "detector",
        kind: ParamKind::Enum { values: &["input", "key", "both"], default: 0 },
        unit: "",
        label_en: "Detector",
        label_fr: "Détecteur",
        modulatable: false,
        smooth: false,
    },
];

/// The `gate` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "gate",
    category: Category::Effect,
    label_en: "Gate / expander",
    label_fr: "Gate / expandeur",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Gate { sample_rate: init.sample_rate.max(1.0), follow: 0.0, gain: 0.0, held: 0.0 })
}

struct Gate {
    sample_rate: f64,
    /// Input peak follower (linear).
    follow: f64,
    /// Current gain (linear, 0..1).
    gain: f64,
    /// Seconds the gate stays open after the level fell under the threshold.
    held: f64,
}

impl Gate {
    /// Gain (linear) for a detector level (linear), before attack / hold / release.
    fn target(level: f64, p: &[f64]) -> f64 {
        let db = 20.0 * level.max(1e-10).log10();
        let under = p[THRESHOLD] - db;
        if under <= 0.0 {
            return 1.0;
        }
        let cut = (under * (p[RATIO].clamp(1.0, 20.0) - 1.0)).min(p[RANGE].clamp(0.0, 100.0));
        10f64.powf(-cut / 20.0)
    }
}

impl Node for Gate {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, input: (f64, f64), p: &[f64]) -> NodeOut {
        let sr = self.sample_rate;
        let peak = input.0.abs().max(input.1.abs());
        let fall = (-1.0 / (FOLLOW_RELEASE * sr)).exp();
        self.follow = if peak > self.follow { peak } else { peak + fall * (self.follow - peak) };
        let key = p[KEY].clamp(0.0, 1.0);
        let level = match p[DETECTOR].round() as usize {
            DETECT_INPUT => self.follow,
            DETECT_KEY => key,
            _ => self.follow.min(key),
        };
        let mut target = Gate::target(level, p);
        if target >= 1.0 {
            self.held = p[HOLD].clamp(0.0, 2.0);
        } else if self.held > 0.0 {
            self.held -= ctx.dt;
            target = 1.0;
        }
        let time = if target > self.gain { p[ATTACK].clamp(0.0005, 0.2) } else { p[RELEASE].clamp(0.005, 3.0) };
        let coef = (-1.0 / (time * sr)).exp();
        self.gain = target + coef * (self.gain - target);
        let (l, r) = (input.0 * self.gain, input.1 * self.gain);
        let (l, r) = (if l.is_finite() { l } else { 0.0 }, if r.is_finite() { r } else { 0.0 });
        NodeOut { l, r, value: (l + r) * 0.5 }
    }

    fn reset(&mut self) {
        self.follow = 0.0;
        self.gain = 0.0;
        self.held = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;
    use std::f64::consts::TAU;

    const RATE: f64 = 48_000.0;
    // threshold, ratio, range, attack, hold, release, key, detector (DEF order)
    const DEFAULTS: [f64; 8] = [-50.0, 20.0, 80.0, 0.002, 0.03, 0.1, 0.0, 0.0];

    fn ctx(frame: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: frame as f64 / RATE,
            seconds: frame as f64 / RATE,
            duration: 2.0,
            mode: GraphMode::Timeline,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    /// RMS of the output over [from, to) s of a 220 Hz tone of amplitude `amp`; `key(t)` drives the key param.
    fn run(params: [f64; 8], amp: f64, key: impl Fn(f64) -> f64, from: f64, to: f64) -> f64 {
        let mut node = create(&NodeInit { id: "g", seed: 0, sample_rate: RATE, params: &params, texts: &[], curve: None, inputs: &[], duration: 2.0 });
        let (mut sum, mut n) = (0.0, 0usize);
        for frame in 0..(to * RATE) as u64 {
            let t = frame as f64 / RATE;
            let mut p = params;
            p[KEY] = key(t);
            let s = amp * (TAU * 220.0 * t).sin();
            let out = node.process_sample(&ctx(frame), (s, s), &p);
            if t >= from {
                sum += out.l * out.l;
                n += 1;
            }
        }
        (sum / n.max(1) as f64).sqrt()
    }

    #[test]
    fn input_gate_passes_loud_and_cuts_quiet() {
        let loud = run(DEFAULTS, 0.5, |_| 0.0, 0.5, 1.0);
        assert!((loud - 0.5 / 2f64.sqrt()).abs() < 1e-3, "a tone over the threshold passes unchanged: {loud}");
        // -66 dB peak, 16 dB under the threshold: cut by the full range
        let quiet = run(DEFAULTS, 0.0005, |_| 0.0, 0.5, 1.0);
        assert!(quiet < 0.0005 * 1e-3, "a tone under the threshold is cut by ~80 dB: {quiet}");
    }

    #[test]
    fn key_opens_and_closes_the_gate_whatever_the_input_level() {
        let mut p = DEFAULTS;
        p[DETECTOR] = DETECT_KEY as f64;
        // the key is on for 0..1 s: a loud tone passes there and is gone 0.9 s after the key falls
        let open = run(p, 0.5, |t| if t < 1.0 { 1.0 } else { 0.0 }, 0.5, 1.0);
        let closed = run(p, 0.5, |t| if t < 1.0 { 1.0 } else { 0.0 }, 1.9, 2.0);
        assert!((open - 0.5 / 2f64.sqrt()).abs() < 1e-3, "{open}");
        assert!(closed < 0.5 * 1e-3, "{closed}");
    }
}
