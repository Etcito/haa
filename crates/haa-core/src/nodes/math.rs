//! Stateless arithmetic and value shaping for modulation signals.
//! Ported from Sona v11 control-math.js (`mathSample`, `MATH_PARAMS`, `MATH_ENUMS`);
//! params and labels come from catalog.js, ui-translations.js and content-translations.js.
//! Deviations:
//! - Operations arrive as rounded enum indices instead of Sona's strings.
//! - Control output uses `value` with silent audio channels, per the Node contract.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const A: usize = 0;
const B: usize = 1;
const OPERATION: usize = 2;
const EDGE0: usize = 3;
const EDGE1: usize = 4;

/// The `math` control node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "math",
    category: Category::Control,
    label_en: "Math",
    label_fr: "Calcul",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "a",
            kind: ParamKind::Number {
                min: -20000.0,
                max: 20000.0,
                default: 0.0,
                step: 0.001,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Value A",
            label_fr: "Valeur A",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "b",
            kind: ParamKind::Number {
                min: -20000.0,
                max: 20000.0,
                default: 1.0,
                step: 0.000001,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Value B",
            label_fr: "Valeur B",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "operation",
            kind: ParamKind::Enum {
                values: &[
                    "multiply",
                    "add",
                    "subtract",
                    "divide",
                    "min",
                    "max",
                    "smoothstep",
                ],
                default: 0,
            },
            unit: "",
            label_en: "Operation",
            label_fr: "Opération",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "edge0",
            kind: ParamKind::Number {
                min: -20000.0,
                max: 20000.0,
                default: 0.0,
                step: 0.001,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "Start edge",
            label_fr: "Seuil de départ",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "edge1",
            kind: ParamKind::Number {
                min: -20000.0,
                max: 20000.0,
                default: 1.0,
                step: 0.001,
                scale: Scale::Lin,
            },
            unit: "",
            label_en: "End edge",
            label_fr: "Seuil d’arrivée",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(_init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Math)
}

struct Math;

impl Node for Math {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        _input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        let a = params[A];
        let b = params[B];
        // sona: the enum is a string in Sona; here its index is rounded, as in oscillator.
        // Keep it signed so unknown negative indices also fall through to max.
        let operation = params[OPERATION].round() as i64;
        let value = match operation {
            0 => a * b,
            1 => a + b,
            2 => a - b,
            3 => {
                if b.abs() > 1e-8 {
                    a / b
                } else {
                    0.0
                }
            }
            4 => a.min(b),
            6 => {
                let range = params[EDGE1] - params[EDGE0];
                if range == 0.0 {
                    if a < params[EDGE0] { 0.0 } else { 1.0 }
                } else {
                    let t = ((a - params[EDGE0]) / range).max(0.0).min(1.0);
                    t * t * (3.0 - 2.0 * t)
                }
            }
            _ => a.max(b),
        };
        NodeOut {
            l: 0.0,
            r: 0.0,
            value,
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    fn sample(params: [f64; 5]) -> f64 {
        let init = NodeInit {
            id: "math1",
            seed: 0,
            sample_rate: 48000.0,
            params: &params,
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        };
        let ctx = SampleCtx {
            sample_rate: 48000.0,
            dt: 1.0 / 48000.0,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        };
        let mut node = (DEF.create)(&init);
        node.reset();
        let out = node.process_sample(&ctx, (1.0, -1.0), &params);
        assert_eq!((out.l, out.r), (0.0, 0.0));
        out.value
    }

    #[test]
    fn division_by_zero_gives_zero_like_sona() {
        for b in [0.0, 1e-9, -1e-9, 1e-8, -1e-8] {
            assert_eq!(sample([12.0, b, 3.0, 0.0, 1.0]), 0.0);
        }
        assert_eq!(sample([12.0, 2.0, 3.0, 0.0, 1.0]), 6.0);
        assert_eq!(sample([12.0, -2.0, 3.0, 0.0, 1.0]), -6.0);
    }

    #[test]
    fn smoothstep_handles_coincident_and_reversed_edges() {
        for (a, expected) in [(-1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            assert_eq!(sample([a, 0.0, 6.0, 0.0, 0.0]), expected);
        }
        for (a, expected) in [
            (-1.0, 1.0),
            (0.0, 1.0),
            (0.25, 0.84375),
            (0.5, 0.5),
            (0.75, 0.15625),
            (1.0, 0.0),
            (2.0, 0.0),
        ] {
            assert_eq!(sample([a, 0.0, 6.0, 1.0, 0.0]), expected);
            assert_eq!(sample([a, 0.0, 6.0, 0.0, 1.0]), 1.0 - expected);
        }
    }

    #[test]
    fn every_operation_stays_finite_at_param_extremes() {
        for operation in 0..=6 {
            for a in [-20000.0, 20000.0] {
                for b in [-20000.0, 20000.0] {
                    for edge0 in [-20000.0, 20000.0] {
                        for edge1 in [-20000.0, 20000.0] {
                            assert!(sample([a, b, operation as f64, edge0, edge1]).is_finite());
                        }
                    }
                }
            }
        }
        for (operation, expected) in [
            (0.0, 8.0),
            (1.0, 6.0),
            (2.0, 2.0),
            (3.0, 2.0),
            (4.0, 2.0),
            (5.0, 4.0),
            (6.0, 1.0),
            (7.0, 4.0),
            (-1.0, 4.0),
            (0.6, 6.0),
        ] {
            assert_eq!(sample([4.0, 2.0, operation, 0.0, 1.0]), expected);
        }
    }
}
