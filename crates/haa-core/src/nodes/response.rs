//! Causal follow, rise and fall envelopes for control signals.
//! Ported from Sona v11 `control-response.js`, with catalog.js metadata and translations.
//! Deviations: enum strings become indices, missing slice entries use Sona's fallbacks,
//! logging is omitted, and SampleCtx supplies the control clock (see below).

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::patch::GraphMode;

const RESPONSE_ENUMS: &[&str] = &["follow", "rise", "fall"];

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min,
            max,
            default,
            step,
            scale: Scale::Lin,
        },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

// Catalog order, including the mode between input and attack.
const RESPONSE_PARAMS: [ParamDef; 8] = [
    number(
        "input",
        0.0,
        1.0,
        0.0,
        0.001,
        "",
        "Control input",
        "Entrée de contrôle",
    ),
    ParamDef {
        key: "mode",
        kind: ParamKind::Enum {
            values: RESPONSE_ENUMS,
            default: 0,
        },
        unit: "",
        label_en: "Response mode",
        label_fr: "Mode de réponse",
        modulatable: false,
        smooth: false,
    },
    number("attack", 0.0, 2.0, 0.003, 0.0001, "s", "Attack", "Attaque"),
    number(
        "release",
        0.0,
        3.0,
        0.035,
        0.0001,
        "s",
        "Release",
        "Relâchement",
    ),
    number(
        "delay",
        0.0,
        2.0,
        0.0,
        0.0001,
        "s",
        "Response delay",
        "Délai de réponse",
    ),
    number(
        "shape",
        0.25,
        4.0,
        1.0,
        0.01,
        "×",
        "Curve power",
        "Courbure",
    ),
    number(
        "gain",
        0.0,
        4.0,
        1.0,
        0.01,
        "×",
        "Response gain",
        "Gain de réponse",
    ),
    number(
        "sensitivity",
        0.001,
        1.0,
        0.04,
        0.001,
        "s",
        "Edge window",
        "Fenêtre de variation",
    ),
];

/// Sona's reusable control response node.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "response",
    category: Category::Control,
    label_en: "Control response",
    label_fr: "Réponse de contrôle",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &RESPONSE_PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    // sona: NodeInit already contains an f64, so only Number(...)'s || fallback remains.
    let sample_rate = if init.sample_rate.is_nan() || init.sample_rate == 0.0 {
        48000.0
    } else {
        init.sample_rate
    }
    .clamp(8000.0, 192000.0);
    Box::new(ResponseNode {
        sample_rate,
        history: vec![0.0; (sample_rate * 2.0).ceil() as usize + 2],
        write: 0,
        available: 0,
        previous: 0.0,
        envelope: 0.0,
        last_time: -1.0,
        attack: -1.0,
        release: -1.0,
        attack_coefficient: 1.0,
        release_coefficient: 1.0,
    })
}

struct ResponseNode {
    sample_rate: f64,
    history: Vec<f64>,
    write: usize,
    available: usize,
    previous: f64,
    envelope: f64,
    last_time: f64,
    attack: f64,
    release: f64,
    attack_coefficient: f64,
    release_coefficient: f64,
}

impl ResponseNode {
    fn reset_response(&mut self) {
        self.write = 0;
        self.available = 0;
        self.previous = 0.0;
        self.envelope = 0.0;
    }
}

impl Node for ResponseNode {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let sr = self.sample_rate;
        let time_seconds = match ctx.mode {
            GraphMode::Live => ctx.live_frames as f64 / sr,
            GraphMode::Timeline => ctx.seconds,
        };
        if ctx.perf_rebase {
            self.last_time = time_seconds - 1.0 / sr;
        }
        // sona: SampleCtx has no performance mode; a timeline graph with performance.mode live resets at the loop wrap here
        if self.last_time >= 0.0
            && (time_seconds < self.last_time || time_seconds - self.last_time > 2.0 / sr)
        {
            self.reset_response();
        }
        self.last_time = time_seconds;

        // sona: safe slice access substitutes for JS's missing object properties.
        let p = |index: usize, fallback: f64| {
            params
                .get(index)
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
        };
        let input = p(0, 0.0).clamp(0.0, 1.0);
        let delay = p(4, 0.0).clamp(0.0, 2.0) * sr;
        let frames = delay.floor() as usize;
        let fraction = delay - frames as f64;
        let size = self.history.len();
        self.history[self.write] = input;
        // sona: unsigned indices require branching before subtracting the delay.
        let recent = if self.write >= frames {
            self.write - frames
        } else {
            size + self.write - frames
        };
        let older = if recent != 0 { recent - 1 } else { size - 1 };
        let a = if frames <= self.available {
            self.history[recent]
        } else {
            0.0
        };
        let b = if frames + 1 <= self.available {
            self.history[older]
        } else {
            0.0
        };
        let delayed = a + (b - a) * fraction;
        self.write = (self.write + 1) % size;
        self.available = (self.available + 1).min(size - 1);

        let mut target = delayed;
        let mode = p(1, 0.0);
        if mode == 1.0 || mode == 2.0 {
            let sensitivity = p(7, 0.04).clamp(0.001, 1.0);
            let slope = (delayed - self.previous) * sr * sensitivity;
            target = (if mode == 1.0 { slope } else { -slope }).clamp(0.0, 1.0);
        }
        self.previous = delayed;
        let power = p(5, 1.0).clamp(0.25, 4.0);
        let gain = p(6, 1.0).clamp(0.0, 4.0);
        if power != 1.0 {
            target = target.powf(power);
        }
        target = (target * gain).clamp(0.0, 1.0);
        let attack = p(2, 0.003).clamp(0.0, 2.0);
        let release = p(3, 0.035).clamp(0.0, 3.0);
        if attack != self.attack {
            self.attack = attack;
            self.attack_coefficient = if attack == 0.0 {
                1.0
            } else {
                -(-1.0 / (attack * sr)).exp_m1()
            };
        }
        if release != self.release {
            self.release = release;
            self.release_coefficient = if release == 0.0 {
                1.0
            } else {
                -(-1.0 / (release * sr)).exp_m1()
            };
        }
        let coefficient = if target > self.envelope {
            self.attack_coefficient
        } else {
            self.release_coefficient
        };
        self.envelope = if coefficient == 1.0 {
            target
        } else {
            self.envelope + (target - self.envelope) * coefficient
        };
        NodeOut {
            l: 0.0,
            r: 0.0,
            value: self.envelope.clamp(0.0, 1.0),
        }
    }

    fn reset(&mut self) {
        self.reset_response();
        self.last_time = -1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48000.0;
    const DEFAULTS: [f64; 8] = [0.0, 0.0, 0.003, 0.035, 0.0, 1.0, 1.0, 0.04];

    fn make_node(sample_rate: f64) -> Box<dyn Node> {
        (DEF.create)(&NodeInit {
            id: "response1",
            seed: 0,
            sample_rate,
            params: &DEFAULTS,
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    fn ctx(frame: u64, sample_rate: f64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate,
            dt: 1.0 / sample_rate,
            t: frame as f64 / sample_rate,
            seconds: frame as f64 / sample_rate,
            duration: 1.0,
            mode: GraphMode::Live,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn run(node: &mut dyn Node, ctx: &SampleCtx<'_>, params: &[f64]) -> f64 {
        let out = node.process_sample(ctx, (0.0, 0.0), params);
        assert_eq!((out.l, out.r), (0.0, 0.0));
        assert!(out.value.is_finite() && (0.0..=1.0).contains(&out.value));
        out.value
    }

    #[test]
    fn follow_mode_tracks_a_constant_input() {
        let mut node = make_node(RATE);
        let mut params = DEFAULTS;
        params[0] = 0.7;
        let mut value = 0.0;
        for frame in 0..4800 {
            value = run(node.as_mut(), &ctx(frame, RATE), &params);
        }
        assert!((value - 0.7).abs() < 1e-12);

        params[2] = 0.0;
        params[3] = 0.0;
        for (offset, input) in [0.7_f64, 0.0, 0.1, 1.0, -1.0, 2.0].into_iter().enumerate() {
            params[0] = input;
            assert_eq!(
                run(node.as_mut(), &ctx(4800 + offset as u64, RATE), &params),
                input.clamp(0.0, 1.0)
            );
        }

        // Half a sample of delay interpolates with zero before history is available.
        node.reset();
        params[0] = 0.7;
        params[4] = 0.5 / RATE;
        assert_eq!(run(node.as_mut(), &ctx(0, RATE), &params), 0.35);
        assert_eq!(run(node.as_mut(), &ctx(1, RATE), &params), 0.7);

        // Timeline wraps and forward seeks invalidate history, but rebasing preserves it.
        let mut clock = ctx(2, RATE);
        clock.mode = GraphMode::Timeline;
        clock.seconds = 0.0;
        clock.looped = true;
        assert_eq!(run(node.as_mut(), &clock, &params), 0.35);
        clock.seconds = 10.0;
        clock.perf_rebase = true;
        assert_eq!(run(node.as_mut(), &clock, &params), 0.7);
        clock.seconds = 20.0;
        clock.perf_rebase = false;
        assert_eq!(run(node.as_mut(), &clock, &params), 0.35);
        clock.seconds = 0.0;
        clock.perf_rebase = true;
        assert_eq!(run(node.as_mut(), &clock, &params), 0.7);

        // Live time follows live_frames even when the patch seconds wrap.
        node.reset();
        assert_eq!(run(node.as_mut(), &ctx(0, RATE), &params), 0.35);
        clock = ctx(1, RATE);
        clock.seconds = 0.0;
        clock.looped = true;
        assert_eq!(run(node.as_mut(), &clock, &params), 0.7);
    }

    #[test]
    fn rise_mode_never_decreases_while_input_is_held() {
        let mut node = make_node(RATE);
        let mut params = DEFAULTS;
        params[1] = 1.0;
        params[7] = 0.25;
        let mut previous = 0.0;
        // Binary fractions give an exactly constant slope, avoiding ramp rounding noise.
        for frame in 0..32768 {
            params[0] = frame.min(16384) as f64 / 32768.0;
            let value = run(node.as_mut(), &ctx(frame, RATE), &params);
            if (1440..=16384).contains(&frame) {
                assert!(value >= previous, "constant slope fell at frame {frame}");
            }
            if frame > 16384 {
                assert!(value <= previous, "held input rose at frame {frame}");
            }
            previous = value;
        }
        assert!(previous < 0.001);
    }

    #[test]
    fn outputs_stay_finite_at_param_extremes() {
        let rate = 8000.0;
        // 12 cases of 1200 samples total 1.8 seconds; no render exceeds two seconds.
        for mode in [0.0, 1.0, 2.0] {
            for shape in [0.25, 4.0] {
                for sensitivity in [0.001, 1.0] {
                    let mut node = make_node(rate);
                    for frame in 0..1200 {
                        let input = if frame % 40 < 20 { 0.0 } else { 1.0 };
                        let attack = if frame % 200 < 100 { 0.0 } else { 2.0 };
                        let release = if frame % 200 < 100 { 3.0 } else { 0.0 };
                        let delay = match frame {
                            0..400 => 2.0,
                            400..800 => 0.5 / rate,
                            _ => 0.0,
                        };
                        let params = [input, mode, attack, release, delay, shape, 4.0, sensitivity];
                        let value = run(node.as_mut(), &ctx(frame, rate), &params);
                        if frame < 400 {
                            assert_eq!(value, 0.0);
                        }
                    }
                    // All numeric params clamp safely or use finite defaults.
                    for (frame, bad) in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -100.0, 100.0]
                        .into_iter()
                        .enumerate()
                    {
                        let params = [bad, mode, bad, bad, bad, bad, bad, bad];
                        run(node.as_mut(), &ctx(1200 + frame as u64, rate), &params);
                    }
                    node.reset();
                    for frame in 0..20 {
                        let params = [0.0, mode, 0.0, 0.0, 0.0, shape, 4.0, sensitivity];
                        assert_eq!(run(node.as_mut(), &ctx(frame, rate), &params), 0.0);
                    }
                }
            }
        }
    }
}
