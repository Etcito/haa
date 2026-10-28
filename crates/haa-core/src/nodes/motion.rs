//! `motion` control node: inertial drive with optional threshold-reset stages.
//! Ported from Sona v11 `control-motion.js` (`MOTION_PARAMS`, `createControlMotionState`,
//! `resetMotion`, `controlMotionSample`); params and labels come from `catalog.js`
//! `NODE_TYPES.motion` and `content-translations.js`.
//! Deviations:
//! - Sona reads `params.input`; Rust receives the effective params slice in DEF order.
//! - The frame index is `ctx.live_frames` (Sona's `totalFrames`), so the motion survives
//!   timeline loops; a voice restart resets it and the node sees the backward jump.
//! - `ctx.perf_rebase` is intentionally ignored: Sona rebases `parameter.value` on a
//!   performance mode switch and keeps the control node's own response, and graph.rs
//!   `advance_performance` rebases the macro base with a smoother jump.
//! - Sona's `console.info` on state creation is not ported.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const INPUT: usize = 0;
const ACCELERATION: usize = 1;
const RESISTANCE: usize = 2;
// `return` is a Rust keyword, so only the constant name changes; the param key stays "return".
const RETURN: usize = 3;
const CEILING: usize = 4;
const RESET: usize = 5;
const STAGES: usize = 6;
const PROGRESSION: usize = 7;
const HOLD: usize = 8;
const INERTIA: usize = 9;
const REST: usize = 10;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    unit: &'static str,
    en: &'static str,
    fr: &'static str,
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
        label_en: en,
        label_fr: fr,
        modulatable: true,
        smooth: true,
    }
}

static PARAMS: [ParamDef; 11] = [
    number(
        "input",
        0.0,
        1.0,
        0.0,
        0.001,
        "",
        "Drive input",
        "Entrée de commande",
    ),
    number(
        "acceleration",
        0.0,
        4.0,
        0.36,
        0.01,
        "/s",
        "Acceleration",
        "Accélération",
    ),
    number(
        "resistance",
        0.0,
        8.0,
        0.07,
        0.01,
        "/s",
        "Resistance",
        "Résistance",
    ),
    number(
        "return",
        0.0,
        12.0,
        1.7,
        0.01,
        "/s",
        "Release drag",
        "Freinage au relâchement",
    ),
    number(
        "ceiling",
        0.05,
        1.0,
        0.98,
        0.01,
        "",
        "Upper threshold",
        "Seuil supérieur",
    ),
    number(
        "reset",
        0.0,
        0.95,
        0.64,
        0.01,
        "",
        "Reset ratio",
        "Rapport de retour",
    ),
    number(
        "stages",
        0.0,
        12.0,
        0.0,
        1.0,
        "",
        "Reset limit",
        "Nombre de retours",
    ),
    number(
        "progression",
        0.05,
        1.0,
        0.76,
        0.01,
        "",
        "Stage scaling",
        "Échelle des paliers",
    ),
    number(
        "hold",
        0.0,
        2.0,
        0.14,
        0.001,
        "s",
        "Reset pause",
        "Pause au retour",
    ),
    number("inertia", 0.0, 2.0, 0.065, 0.001, "s", "Inertia", "Inertie"),
    number(
        "rest",
        0.0,
        0.25,
        0.025,
        0.001,
        "",
        "Rest threshold",
        "Seuil de repos",
    ),
];

/// The `motion` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "motion",
    category: Category::Control,
    label_en: "Inertial motion",
    label_fr: "Mouvement inertiel",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(MotionNode::new(init))
}

struct MotionNode {
    dt: f64,
    position: f64,
    value: f64,
    stage: f64,
    hold_remaining: f64,
    last_frame: i64,
    inertia: f64,
    inertia_coefficient: f64,
    progression: f64,
    stage_scale: f64,
}

impl MotionNode {
    fn new(init: &NodeInit<'_>) -> Self {
        // sona: `clamp(Number(sampleRate) || 48000, 8000, 192000)`; zero and NaN fall back.
        let raw = if init.sample_rate.is_nan() || init.sample_rate == 0.0 {
            48_000.0
        } else {
            init.sample_rate
        };
        let rate = raw.clamp(8_000.0, 192_000.0);
        Self {
            dt: 1.0 / rate,
            position: 0.0,
            value: 0.0,
            stage: 0.0,
            hold_remaining: 0.0,
            last_frame: -1,
            inertia: -1.0,
            inertia_coefficient: 1.0,
            progression: -1.0,
            stage_scale: 1.0,
        }
    }

    /// Sona `resetMotion`; the inertia cache is kept, as in Sona.
    fn reset_motion(&mut self) {
        self.position = 0.0;
        self.value = 0.0;
        self.stage = 0.0;
        self.hold_remaining = 0.0;
        self.stage_scale = 1.0;
        self.progression = -1.0;
    }
}

impl Node for MotionNode {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let frame_index = ctx.live_frames as i64;
        if self.last_frame >= 0 && frame_index < self.last_frame {
            self.reset_motion();
        }
        self.last_frame = frame_index;

        // sona: on a performance mode switch Sona rebases `parameter.value` and keeps the
        // control node's own response; graph.rs `advance_performance` rebases the macro base
        // with a smoother jump, so `ctx.perf_rebase` leaves this state untouched.

        let drive = clamp(param(params, INPUT, 0.0), 0.0, 1.0);
        let acceleration = clamp(param(params, ACCELERATION, 0.36), 0.0, 4.0);
        let resistance = clamp(param(params, RESISTANCE, 0.07), 0.0, 8.0);
        let release = clamp(param(params, RETURN, 1.7), 0.0, 12.0);
        let ceiling = clamp(param(params, CEILING, 0.98), 0.05, 1.0);
        let reset = clamp(param(params, RESET, 0.64), 0.0, 0.95);
        let stages = clamp(param(params, STAGES, 0.0), 0.0, 12.0).round();
        let progression = clamp(param(params, PROGRESSION, 0.76), 0.05, 1.0);
        let hold = clamp(param(params, HOLD, 0.14), 0.0, 2.0);
        let inertia = clamp(param(params, INERTIA, 0.065), 0.0, 2.0);
        let rest = clamp(param(params, REST, 0.025), 0.0, 0.25);

        if progression != self.progression {
            self.progression = progression;
            self.stage_scale = progression.powf(self.stage);
        }
        if inertia != self.inertia {
            self.inertia = inertia;
            self.inertia_coefficient = if inertia == 0.0 {
                1.0
            } else {
                -((-self.dt / inertia).exp_m1())
            };
        }

        let effort = if self.hold_remaining > 0.0 {
            0.0
        } else {
            drive * acceleration * self.stage_scale
        };
        self.hold_remaining = (self.hold_remaining - self.dt).max(0.0);
        self.position = (self.position
            + (effort - (resistance + release * (1.0 - drive)) * self.position) * self.dt)
            .max(0.0);
        if self.position >= ceiling {
            if self.stage < stages && drive > 0.02 {
                self.position = ceiling * reset;
                self.stage += 1.0;
                self.stage_scale = progression.powf(self.stage);
                self.hold_remaining = hold;
            } else {
                self.position = ceiling;
            }
        }
        if drive < 0.005 && self.position <= rest {
            self.position = 0.0;
            self.stage = 0.0;
            self.stage_scale = 1.0;
            self.hold_remaining = 0.0;
        }
        self.value += (self.position - self.value) * self.inertia_coefficient;

        NodeOut {
            l: 0.0,
            r: 0.0,
            value: clamp(self.value, 0.0, 1.0),
        }
    }

    fn reset(&mut self) {
        self.reset_motion();
        self.last_frame = -1;
    }
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    max.min(min.max(value))
}

/// Sona's `Number.isFinite(params.x) ? params.x : default`.
fn param(params: &[f64], index: usize, default: f64) -> f64 {
    match params.get(index).copied() {
        Some(value) if value.is_finite() => value,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 48_000.0;
    const DEFAULTS: [f64; 11] = [
        0.0, 0.36, 0.07, 1.7, 0.98, 0.64, 0.0, 0.76, 0.14, 0.065, 0.025,
    ];

    fn make_node() -> Box<dyn Node> {
        let texts: [Option<String>; 11] = Default::default();
        (DEF.create)(&NodeInit {
            id: "motion1",
            seed: 0,
            sample_rate: RATE,
            params: &DEFAULTS,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 2.0,
        })
    }

    fn ctx(frame: u64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
            t: 0.0,
            seconds: frame as f64 / RATE,
            duration: 2.0,
            mode: GraphMode::Live,
            live_frames: frame,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn run(node: &mut Box<dyn Node>, frame: u64, params: &[f64; 11]) -> f64 {
        let out = node.process_sample(&ctx(frame), (0.0, 0.0), params);
        assert_eq!((out.l, out.r), (0.0, 0.0), "a control node is silent");
        out.value
    }

    #[test]
    fn zero_drive_settles_to_rest() {
        let mut node = make_node();
        let mut params = DEFAULTS;
        params[INPUT] = 1.0;
        params[INERTIA] = 0.0;
        let mut frame = 0;
        let mut charged = 0.0;
        for _ in 0..(RATE * 0.4) as u64 {
            charged = run(&mut node, frame, &params);
            frame += 1;
        }
        assert!(
            charged > 0.1,
            "drive should charge the motion, got {charged}"
        );

        params[INPUT] = 0.0;
        let mut settled = false;
        for _ in 0..(RATE * 1.5) as u64 {
            let out = run(&mut node, frame, &params);
            frame += 1;
            if out == 0.0 {
                settled = true;
                break;
            }
        }
        assert!(settled, "release should reach exactly 0 within 2 s");
    }

    #[test]
    fn held_drive_rises_monotonically_to_ceiling() {
        let mut node = make_node();
        let mut params = DEFAULTS;
        params[INPUT] = 1.0;
        params[ACCELERATION] = 4.0;
        params[STAGES] = 0.0;
        let ceiling = params[CEILING];
        let mut previous = f64::NEG_INFINITY;
        let mut last = 0.0;
        for frame in 0..(RATE * 2.0) as u64 {
            last = run(&mut node, frame, &params);
            assert!(
                last >= previous,
                "output decreased at frame {frame}: {last} < {previous}"
            );
            previous = last;
        }
        assert!(last <= 1.0, "output above 1: {last}");
        assert!(
            (last - ceiling).abs() < 1e-6,
            "expected ceiling {ceiling}, got {last}"
        );
    }

    #[test]
    fn output_is_finite_and_bounded_at_param_extremes() {
        let mut cases = [[0.0; 11]; 4];
        for (index, case) in cases.iter_mut().enumerate() {
            let resistance = if index % 2 == 0 { 0.0 } else { 8.0 };
            let inertia = if index < 2 { 0.0 } else { 2.0 };
            *case = DEFAULTS;
            case[ACCELERATION] = 4.0;
            case[RESISTANCE] = resistance;
            case[RETURN] = 12.0;
            case[CEILING] = if resistance == 0.0 { 1.0 } else { 0.05 };
            case[RESET] = 0.95;
            case[STAGES] = 12.0;
            case[PROGRESSION] = 0.05;
            case[HOLD] = if index == 0 { 0.0 } else { 2.0 };
            case[INERTIA] = inertia;
            case[REST] = 0.25;
        }
        for params in &cases {
            let mut node = make_node();
            for frame in 0..(RATE * 0.5) as u64 {
                let mut effective = *params;
                effective[INPUT] = if (frame / 2400) % 2 == 0 { 1.0 } else { 0.0 };
                let out = run(&mut node, frame, &effective);
                assert!(out.is_finite(), "non-finite output {out}");
                assert!((0.0..=1.0).contains(&out), "output {out} outside 0..1");
            }
        }
    }
}
