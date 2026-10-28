//! Automation curve: breakpoint control over normalized time.
//! Ported from Sona v11 `dsp.js` (`curveAt`); the catalog entry comes from `catalog.js`.
//! Deviations: `clock` and `retrigger` are new HAA params (ARCHITECTURE §8); missing or
//! malformed curve data falls back to a flat zero curve instead of Sona's default shape.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

/// Sona rejects curves outside 2..=128 points.
const MAX_POINTS: usize = 128;

static PARAMS: [ParamDef; 3] = [
    ParamDef {
        key: "amount",
        kind: ParamKind::Number {
            min: 0.0,
            max: 1.0,
            default: 1.0,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: "Amplitude",
        label_fr: "Amplitude",
        modulatable: true,
        smooth: true,
    },
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
    type_name: "curve",
    category: Category::Control,
    label_en: "Automation curve",
    label_fr: "Courbe temporelle",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: true,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(CurveNode::new(init))
}

struct CurveNode {
    points: Vec<(f64, f64)>,
    local_time: f64,
    previous_retrigger: f64,
}

impl CurveNode {
    fn new(init: &NodeInit<'_>) -> Self {
        let mut points = Vec::with_capacity(MAX_POINTS);
        read_points(init.curve, &mut points);
        Self {
            points,
            local_time: 1.0,
            previous_retrigger: 0.0,
        }
    }
}

impl Node for CurveNode {
    fn process_sample(
        &mut self,
        ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let amount = params.first().copied().unwrap_or(1.0);
        let clock = params.get(1).copied().unwrap_or(0.0);
        let retrigger = params.get(2).copied().unwrap_or(0.0);

        let time = if clock >= 0.5 {
            let rising = self.previous_retrigger <= 0.5 && retrigger > 0.5;
            self.previous_retrigger = retrigger;
            if rising {
                self.local_time = 0.0;
            } else if self.local_time < 1.0 {
                let duration = if ctx.duration.is_finite() && ctx.duration > 0.0 {
                    ctx.duration
                } else {
                    1.0
                };
                self.local_time += ctx.dt / duration;
                if self.local_time > 1.0 {
                    self.local_time = 1.0;
                }
            }
            self.local_time
        } else {
            self.previous_retrigger = retrigger;
            ctx.t
        };

        let value = curve_at(&self.points, time) * amount;
        NodeOut {
            l: 0.0,
            r: 0.0,
            value: if value.is_finite() { value } else { 0.0 },
        }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        read_points(init.curve, &mut self.points);
    }

    fn reset(&mut self) {
        self.local_time = 1.0;
        self.previous_retrigger = 0.0;
    }
}

/// Sona `validateGraph` clamps points to 0..1 and sorts them by time; a bad curve is flat.
fn read_points(curve: Option<&serde_json::Value>, out: &mut Vec<(f64, f64)>) {
    out.clear();
    if let Some(points) = curve.and_then(serde_json::Value::as_array) {
        if (2..=MAX_POINTS).contains(&points.len()) {
            let mut valid = true;
            for point in points {
                let Some(pair) = point.as_array() else {
                    valid = false;
                    break;
                };
                if pair.len() < 2 {
                    valid = false;
                    break;
                }
                let (Some(x), Some(y)) = (pair[0].as_f64(), pair[1].as_f64()) else {
                    valid = false;
                    break;
                };
                if !x.is_finite() || !y.is_finite() {
                    valid = false;
                    break;
                }
                out.push((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
            }
            if valid {
                out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                return;
            }
        }
    }
    out.clear();
    out.push((0.0, 0.0));
    out.push((1.0, 0.0));
}

/// Sona `curveAt`: linear interpolation between sorted breakpoints.
fn curve_at(points: &[(f64, f64)], t: f64) -> f64 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    if t <= first.0 {
        return first.1;
    }
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.0 {
            let u = (t - a.0) / (b.0 - a.0).max(0.000001);
            return a.1 + (b.1 - a.1) * u;
        }
    }
    points[points.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;
    use serde_json::json;

    fn make_node(curve: &serde_json::Value) -> Box<dyn Node> {
        let texts: [Option<String>; 3] = [None, None, None];
        let params = [1.0, 0.0, 0.0];
        let init = NodeInit {
            id: "curve1",
            seed: 0,
            sample_rate: 48000.0,
            params: &params,
            texts: &texts,
            curve: Some(curve),
            inputs: &[],
            duration: 3.0,
        };
        (DEF.create)(&init)
    }

    fn ctx(t: f64) -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: 100.0,
            dt: 0.01,
            t,
            seconds: t,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    fn run(node: &mut Box<dyn Node>, t: f64, params: [f64; 3]) -> f64 {
        node.process_sample(&ctx(t), (0.0, 0.0), &params).value
    }

    #[test]
    fn trigger_clock_holds_end_then_restarts_at_first_breakpoint() {
        let curve = json!([[0.0, 0.25], [0.5, 1.0], [1.0, 0.75]]);
        let mut node = make_node(&curve);

        let held = run(&mut node, 0.0, [1.0, 1.0, 0.0]);
        assert!(
            (held - 0.75).abs() < 1e-12,
            "holds the end value, got {held}"
        );

        let restarted = run(&mut node, 0.0, [1.0, 1.0, 1.0]);
        assert!(
            (restarted - 0.25).abs() < 1e-12,
            "restarts at the first breakpoint, got {restarted}"
        );

        let advanced = run(&mut node, 0.0, [1.0, 1.0, 1.0]);
        assert!(
            advanced > restarted,
            "local time advances after the trigger, got {advanced}"
        );
    }

    #[test]
    fn output_stays_finite_with_bad_and_extreme_curves() {
        let curves = [
            json!([]),
            json!([[0.5, 0.5]]),
            json!([[1e300, -1e300], [-1e300, 1e300]]),
            json!("not a curve"),
        ];
        for curve in &curves {
            let mut node = make_node(curve);
            for step in 0..40 {
                let value = run(&mut node, step as f64 / 39.0, [1.0, 0.0, 0.0]);
                assert!(value.is_finite(), "curve {curve} gave {value}");
            }
        }
    }

    #[test]
    fn patch_clock_reads_first_and_last_breakpoints() {
        let curve = json!([[0.0, 0.2], [0.4, 0.9], [1.0, 0.6]]);
        let mut node = make_node(&curve);
        let first = run(&mut node, 0.0, [1.0, 0.0, 0.0]);
        let last = run(&mut node, 1.0, [1.0, 0.0, 0.0]);
        assert!(
            (first - 0.2).abs() < 1e-12,
            "t=0 is the first breakpoint, got {first}"
        );
        assert!(
            (last - 0.6).abs() < 1e-12,
            "t=1 is the last breakpoint, got {last}"
        );
    }
}
