//! Map: HAA control node (ARCHITECTURE §8) that turns a value in game units into another
//! through Sona-shaped breakpoints drawn against the input range. HAA-only, not a Sona port.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

/// Sona's breakpoint limit (2..=128 points).
const MAX_POINTS: usize = 128;

const fn range_param(
    key: &'static str,
    default: f64,
    en: &'static str,
    fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min: -100000.0,
            max: 100000.0,
            default,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: en,
        label_fr: fr,
        modulatable: false,
        smooth: true,
    }
}

static PARAMS: [ParamDef; 6] = [
    ParamDef {
        key: "in",
        kind: ParamKind::Number {
            min: -100000.0,
            max: 100000.0,
            default: 0.0,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: "In",
        label_fr: "Entrée",
        modulatable: true,
        // The input node already smooths.
        smooth: false,
    },
    range_param("inMin", 0.0, "In min", "Entrée min"),
    range_param("inMax", 1.0, "In max", "Entrée max"),
    ParamDef {
        key: "scale",
        kind: ParamKind::Enum {
            values: &["lin", "log"],
            default: 0,
        },
        unit: "",
        label_en: "Scale",
        label_fr: "Échelle",
        modulatable: false,
        smooth: false,
    },
    range_param("outMin", 0.0, "Out min", "Sortie min"),
    range_param("outMax", 1.0, "Out max", "Sortie max"),
];

/// The `map` node definition.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "map",
    category: Category::Control,
    label_en: "Map",
    label_fr: "Courbe de réponse",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: true,
    hidden: false,
    params: &PARAMS,
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let mut points = Vec::with_capacity(MAX_POINTS);
    read_points(init.curve, &mut points);
    Box::new(MapNode { points })
}

struct MapNode {
    points: Vec<(f64, f64)>,
}

impl Node for MapNode {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let p = |i: usize, d: f64| {
            params
                .get(i)
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(d)
        };
        let value = map_value(
            &self.points,
            p(0, 0.0),
            p(1, 0.0),
            p(2, 1.0),
            p(3, 0.0) >= 0.5,
            p(4, 0.0),
            p(5, 1.0),
        );
        NodeOut {
            l: 0.0,
            r: 0.0,
            value: if value.is_finite() { value } else { 0.0 },
        }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        read_points(init.curve, &mut self.points);
    }

    fn reset(&mut self) {}
}

fn map_value(
    points: &[(f64, f64)],
    input: f64,
    in_min: f64,
    in_max: f64,
    log: bool,
    out_min: f64,
    out_max: f64,
) -> f64 {
    let norm = if in_max == in_min {
        0.0
    } else if log && in_min > 0.0 && in_max > 0.0 {
        if input > 0.0 {
            (input / in_min).ln() / (in_max / in_min).ln()
        } else {
            0.0
        }
    } else {
        (input - in_min) / (in_max - in_min)
    };
    let norm = if norm.is_finite() {
        norm.clamp(0.0, 1.0)
    } else {
        0.0
    };
    out_min + (out_max - out_min) * curve_eval(points, norm)
}

/// Sona's breakpoint shape `[[x, y], ...]`: 2..=128 finite pairs, clamped to 0..1, sorted
/// by x. Missing or malformed data leaves the list empty (identity curve).
fn read_points(curve: Option<&serde_json::Value>, out: &mut Vec<(f64, f64)>) {
    out.clear();
    let Some(points) = curve.and_then(serde_json::Value::as_array) else {
        return;
    };
    if !(2..=MAX_POINTS).contains(&points.len()) {
        return;
    }
    for point in points {
        let pair = point.as_array().filter(|p| p.len() >= 2);
        let xy = pair.and_then(|p| Some((p[0].as_f64()?, p[1].as_f64()?)));
        match xy {
            Some((x, y)) if x.is_finite() && y.is_finite() => {
                out.push((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)));
            }
            _ => {
                out.clear();
                return;
            }
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
}

/// Linear interpolation between breakpoints, holding the ends; identity without breakpoints.
fn curve_eval(points: &[(f64, f64)], t: f64) -> f64 {
    let Some(first) = points.first() else {
        return t;
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

    fn make_node(curve: Option<&serde_json::Value>) -> Box<dyn Node> {
        let texts: [Option<String>; 6] = Default::default();
        let params = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let init = NodeInit {
            id: "map1",
            seed: 0,
            sample_rate: 48000.0,
            params: &params,
            texts: &texts,
            curve,
            inputs: &[],
            duration: 1.0,
        };
        (DEF.create)(&init)
    }

    fn run(node: &mut Box<dyn Node>, params: [f64; 6]) -> f64 {
        let ctx = SampleCtx {
            sample_rate: 48000.0,
            dt: 1.0 / 48000.0,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Live,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        };
        let out = node.process_sample(&ctx, (0.0, 0.0), &params);
        assert_eq!((out.l, out.r), (0.0, 0.0));
        out.value
    }

    #[test]
    fn no_breakpoints_is_linear_and_clamps() {
        let mut node = make_node(None);
        let mid = run(&mut node, [15.0, 10.0, 20.0, 0.0, 100.0, 300.0]);
        assert!((mid - 200.0).abs() < 1e-9, "mid gave {mid}");
        assert_eq!(run(&mut node, [5.0, 10.0, 20.0, 0.0, 100.0, 300.0]), 100.0);
        assert_eq!(run(&mut node, [25.0, 10.0, 20.0, 0.0, 100.0, 300.0]), 300.0);
    }

    #[test]
    fn log_scale_maps_geometric_mean_to_half() {
        let mut node = make_node(None);
        let value = run(
            &mut node,
            [(200.0f64 * 1200.0).sqrt(), 200.0, 1200.0, 1.0, 0.0, 1.0],
        );
        assert!((value - 0.5).abs() < 1e-9, "got {value}");
    }

    #[test]
    fn breakpoints_are_interpolated_and_outputs_stay_finite_at_extremes() {
        let curve = json!([[1.0, 1.0], [0.0, 0.0], [0.5, 0.2]]);
        let mut node = make_node(Some(&curve));
        let quarter = run(&mut node, [0.25, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert!((quarter - 0.1).abs() < 1e-9, "got {quarter}");
        let three_q = run(&mut node, [0.75, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert!((three_q - 0.6).abs() < 1e-9, "got {three_q}");

        let e = 100000.0;
        for v in [-e, 0.0, e] {
            for (lo, hi) in [(-e, e), (e, -e), (0.0, 0.0), (-e, -e), (e, e)] {
                for scale in [0.0, 1.0] {
                    let out = run(&mut node, [v, lo, hi, scale, -e, e]);
                    assert!(out.is_finite(), "v {v} lo {lo} hi {hi} gave {out}");
                }
            }
        }
    }
}
