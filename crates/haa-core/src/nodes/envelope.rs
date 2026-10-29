//! ADSR envelope with hold: a piecewise control over seconds.
//! Ported from Sona v11 `dsp.js` (`case 'envelope'`); the catalog entry comes from `catalog.js`.
//! Deviations: `clock` and `retrigger` are new HAA params (ARCHITECTURE §8). With
//! `clock = trigger` the envelope reads a local time in seconds that restarts on a rising
//! edge of `retrigger` and holds at the patch duration; a non-finite value is zeroed.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    step: f64,
    default: f64,
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

static PARAMS: [ParamDef; 7] = [
    number("attack", 0.001, 3.0, 0.001, 0.08, "s", "Attack", "Attaque"),
    number("decay", 0.001, 3.0, 0.001, 0.25, "s", "Decay", "Décroissance"),
    number("sustain", 0.0, 1.0, 0.01, 0.7, "", "Sustain", "Soutien"),
    number("hold", 0.01, 8.0, 0.01, 1.2, "s", "Hold", "Maintien"),
    number("release", 0.01, 4.0, 0.01, 0.5, "s", "Release", "Relâchement"),
    ParamDef {
        key: "clock",
        kind: ParamKind::Enum {
            values: &["patch", "trigger"],
            default: 0,
        },
        unit: "",
        label_en: "Clock",
        label_fr: "Horloge",
        modulatable: false,
        smooth: false,
    },
    ParamDef {
        key: "retrigger",
        kind: ParamKind::Number {
            min: 0.0,
            max: 1.0,
            default: 0.0,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: "Retrigger",
        label_fr: "Redéclenchement",
        modulatable: true,
        smooth: false,
    },
];

pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "envelope",
    category: Category::Control,
    label_en: "ADSR envelope",
    label_fr: "Enveloppe ADSR",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(EnvelopeNode::new(init))
}

struct EnvelopeNode {
    /// Local time in seconds for the trigger clock; `INFINITY` means "at the end".
    local_time: f64,
    previous_retrigger: f64,
}

impl EnvelopeNode {
    fn new(_init: &NodeInit<'_>) -> Self {
        Self {
            local_time: f64::INFINITY,
            previous_retrigger: 0.0,
        }
    }
}

impl Node for EnvelopeNode {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let p = |i: usize, d: f64| params.get(i).copied().unwrap_or(d);
        let (a, d, s, h, r) = (p(0, 0.08), p(1, 0.25), p(2, 0.7), p(3, 1.2), p(4, 0.5));
        let clock = p(5, 0.0);
        let retrigger = p(6, 0.0);

        let time = if clock >= 0.5 {
            let duration = if ctx.duration.is_finite() && ctx.duration > 0.0 {
                ctx.duration
            } else {
                1.0
            };
            let rising = self.previous_retrigger <= 0.5 && retrigger > 0.5;
            self.previous_retrigger = retrigger;
            if rising {
                self.local_time = 0.0;
            } else if self.local_time < duration {
                self.local_time += ctx.dt;
            }
            // sona: no local clock in Sona; the end sentinel and overshoot hold at duration.
            if self.local_time > duration {
                self.local_time = duration;
            }
            self.local_time
        } else {
            self.previous_retrigger = retrigger;
            ctx.seconds
        };

        let value = if time < a {
            time / a
        } else if time < a + d {
            1.0 - (1.0 - s) * (time - a) / d
        } else if time < a + d + h {
            s
        } else {
            s * (1.0 - (time - a - d - h) / r).max(0.0)
        };
        NodeOut {
            l: 0.0,
            r: 0.0,
            value: if value.is_finite() { value } else { 0.0 },
        }
    }

    fn reset(&mut self) {
        self.local_time = f64::INFINITY;
        self.previous_retrigger = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    fn make_node() -> Box<dyn Node> {
        let texts: [Option<String>; 7] = Default::default();
        let params = [0.08, 0.25, 0.7, 1.2, 0.5, 0.0, 0.0];
        let init = NodeInit {
            id: "env1",
            seed: 0,
            sample_rate: 100.0,
            params: &params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 4.0,
        };
        (DEF.create)(&init)
    }

    fn ctx(seconds: f64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: 100.0,
            dt: 0.01,
            t: seconds / 4.0,
            seconds,
            duration: 4.0,
            mode: GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn run(node: &mut Box<dyn Node>, seconds: f64, params: [f64; 7]) -> f64 {
        node.process_sample(&ctx(seconds), (0.0, 0.0), &params).value
    }

    #[test]
    fn retrigger_restarts_the_envelope() {
        let mut node = make_node();
        let mut p = [0.1, 0.2, 0.5, 1.0, 0.5, 1.0, 0.0];
        let held = run(&mut node, 0.0, p);
        assert!(held.abs() < 1e-12, "past the release before a trigger, got {held}");

        p[6] = 1.0;
        let restarted = run(&mut node, 0.0, p);
        assert!(restarted.abs() < 1e-12, "restarts at 0, got {restarted}");
        let mut last = restarted;
        for _ in 0..5 {
            let v = run(&mut node, 0.0, p);
            assert!(v > last, "rises during the attack, got {v} after {last}");
            last = v;
        }
    }

    #[test]
    fn patch_clock_reads_attack_sustain_and_release() {
        let mut node = make_node();
        let p = [0.1, 0.2, 0.5, 1.0, 0.5, 0.0, 0.0];
        let close = |x: f64, y: f64| (x - y).abs() < 1e-9;
        assert!(close(run(&mut node, 0.05, p), 0.5));
        assert!(close(run(&mut node, 0.8, p), 0.5));
        assert!(close(run(&mut node, 1.55, p), 0.25));
        assert!(close(run(&mut node, 3.0, p), 0.0));
    }

    #[test]
    fn output_stays_finite_at_param_extremes() {
        for (lo, hi) in [(true, true), (true, false), (false, true), (false, false)] {
            let mut node = make_node();
            let p = |i: usize| {
                let (min, max) = match PARAMS[i].kind {
                    ParamKind::Number { min, max, .. } => (min, max),
                    _ => (0.0, 1.0),
                };
                if (i % 2 == 0) == lo { min } else if hi { max } else { min }
            };
            for clock in [0.0, 1.0] {
                let params = [p(0), p(1), p(2), p(3), p(4), clock, p(6)];
                for step in 0..200 {
                    let v = run(&mut node, step as f64 * 0.1, params);
                    assert!(v.is_finite(), "{params:?} gave {v}");
                }
            }
        }
    }
}
