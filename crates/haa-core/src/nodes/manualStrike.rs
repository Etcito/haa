#![allow(non_snake_case)]
//! Manual impact controller: named outputs are real graph sources, never UI-only wires.
use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
const fn number(
    key: &'static str,
    max: f64,
    default: f64,
    unit: &'static str,
    en: &'static str,
    fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min: 0.,
            max,
            default,
            step: 0.001,
            scale: Scale::Lin,
        },
        unit,
        label_en: en,
        label_fr: fr,
        modulatable: true,
        smooth: false,
    }
}
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "manualStrike",
    category: Category::Control,
    label_en: "Manual strike",
    label_fr: "Frappe manuelle",
    audio_in: false,
    audio_out: false,
    control_out: true,
    control_outputs: &["trigger", "radius", "angle", "strength", "A", "B", "C", "D"],
    uses_curve: false,
    hidden: false,
    create,
    params: &[
        number("radius", 0.95, 0., "R", "Impact radius", "Rayon de frappe"),
        number("angle", 360., 0., "°", "Impact angle", "Angle de frappe"),
        number("strength", 0.01, 0.005, "N·s", "Strength", "Intensité"),
        number("strike", 1., 0., "", "Strike gate", "Déclenchement"),
        ParamDef {
            key: "autoStrike",
            kind: ParamKind::Bool { default: true },
            unit: "",
            label_en: "Strike on Play / loop",
            label_fr: "Frappe au Play / boucle",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "triggerInput",
            kind: ParamKind::Text { default: "strike" },
            unit: "",
            label_en: "Published trigger input",
            label_fr: "Entrée de déclenchement publiée",
            modulatable: false,
            smooth: false,
        },
    ],
};
fn resolve(init: &NodeInit<'_>) -> Option<usize> {
    let name = init.texts.get(5)?.as_deref()?;
    init.inputs
        .iter()
        .position(|input| input.name == name && input.kind == crate::patch::InputKind::Trigger)
}
fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(ManualStrike {
        outputs: [0.; 8],
        started: false,
        gate: false,
        input: resolve(init),
    })
}
struct ManualStrike {
    outputs: [f64; 8],
    started: bool,
    gate: bool,
    input: Option<usize>,
}
/// B/A/C/D angular sectors; C1 boundaries and quintic radial fade from the normal centre.
fn zones(radius: f64, angle: f64) -> [f64; 4] {
    let angle = angle.rem_euclid(360.);
    let sector = (angle / 90.).floor() as usize;
    let progress = angle / 90. - sector as f64;
    let r = (radius / 0.7).clamp(0., 1.);
    let radial = r * r * r * (r * (r * 6. - 15.) + 10.);
    let weight = radial * (std::f64::consts::PI * progress).sin().powi(2);
    let mut out = [0.; 4];
    out[[1, 0, 2, 3][sector]] = weight;
    out
}
impl Node for ManualStrike {
    fn control_output(&self, port: usize) -> f64 {
        self.outputs.get(port).copied().unwrap_or(0.)
    }
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        let gate = p[3] > 0.5;
        let external = self
            .input
            .and_then(|i| ctx.triggers.get(i))
            .copied()
            .unwrap_or(false);
        let trigger =
            external || (gate && !self.gate) || ((!self.started || ctx.looped) && p[4] > 0.5);
        self.started = true;
        self.gate = gate;
        self.outputs[0] = f64::from(trigger);
        self.outputs[1] = p[0];
        self.outputs[2] = p[1].rem_euclid(360.);
        self.outputs[3] = p[2];
        self.outputs[4..].copy_from_slice(&zones(p[0], p[1]));
        NodeOut {
            l: 0.,
            r: 0.,
            value: self.outputs[0],
        }
    }
    fn configure(&mut self, init: &NodeInit<'_>) {
        self.input = resolve(init);
    }
    fn reset(&mut self) {
        self.outputs.fill(0.);
        self.started = false;
        self.gate = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Voice, patch::GraphMode};
    fn ctx<'a>(triggers: &'a [bool]) -> SampleCtx<'a> {
        SampleCtx {
            sample_rate: 48000.,
            dt: 1. / 48000.,
            t: 0.,
            seconds: 0.,
            duration: 4.,
            mode: GraphMode::Live,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers,
        }
    }
    #[test]
    fn centre_and_sector_edges_are_normal_and_continuous() {
        for a in 0..360 {
            assert_eq!(zones(0., a as f64), [0.; 4]);
        }
        for a in [0., 90., 180., 270., 360.] {
            for eps in [-0.00001, 0., 0.00001] {
                assert!(zones(0.9, a + eps).iter().sum::<f64>() < 1e-12);
            }
        }
        assert!((zones(0.7, 45.)[1] - 1.).abs() < 1e-14);
        assert!((zones(0.35, 135.)[0] - 0.5).abs() < 1e-14);
    }
    #[test]
    fn trigger_is_one_sample_and_reset_replays() {
        let mut n = ManualStrike {
            outputs: [0.; 8],
            started: false,
            gate: false,
            input: Some(0),
        };
        let p = [0.3, 45., 0.005, 1., 1., 0.];
        assert_eq!(n.process_sample(&ctx(&[false]), (0., 0.), &p).value, 1.);
        assert_eq!(n.process_sample(&ctx(&[false]), (0., 0.), &p).value, 0.);
        assert_eq!(n.process_sample(&ctx(&[true]), (0., 0.), &p).value, 1.);
        n.reset();
        assert_eq!(n.process_sample(&ctx(&[false]), (0., 0.), &p).value, 1.);
    }
    fn patch(port: Option<&str>, muted: bool) -> String {
        let mut edge =
            serde_json::json!({"id":"c","from":"hit","to":"read","target":"a","amount":1});
        if let Some(p) = port {
            edge["fromPort"] = p.into();
        }
        serde_json::json!({"haa":1,"id":"named-test","title":"Named output","inputs":[],"graph":{"duration":1,"mode":"live","nodes":[{"id":"hit","type":"manualStrike","muted":muted,"params":{"radius":0.7,"angle":45,"strength":0.005}},{"id":"read","type":"math","params":{"a":0,"b":1,"operation":"multiply"}},{"id":"out","type":"output"}],"connections":[edge]}}).to_string()
    }
    #[test]
    fn real_graph_routes_each_named_output_and_mute() {
        for (port, want) in [
            ("radius", 0.7),
            ("angle", 45.),
            ("strength", 0.005),
            ("A", 0.),
            ("B", 1.),
            ("C", 0.),
            ("D", 0.),
        ] {
            let mut v = Voice::new(48000., 0);
            assert!(v.load(&patch(Some(port), false), false).playable);
            v.process(&mut [0.; 128], &mut [0.; 128]);
            let i = v.node_ids().iter().position(|id| id == "read").unwrap();
            assert!((v.trace()[i] as f64 - want).abs() < 1e-6, "{port}");
        }
        let mut v = Voice::new(48000., 0);
        assert!(v.load(&patch(Some("B"), true), false).playable);
        v.process(&mut [0.; 128], &mut [0.; 128]);
        let i = v.node_ids().iter().position(|id| id == "read").unwrap();
        assert_eq!(v.trace()[i], 0.);
    }
    #[test]
    fn source_port_roundtrips_and_unknown_is_rejected() {
        let text = patch(Some("B"), false);
        let (p, _) = crate::load_patch(&text).unwrap();
        assert_eq!(p.graph.connections[0].from_port.as_deref(), Some("B"));
        let saved = serde_json::to_value(p).unwrap();
        assert_eq!(saved["graph"]["connections"][0]["fromPort"], "B");
        let mut v = Voice::new(48000., 0);
        assert!(!v.load(&patch(Some("missing"), false), false).playable);
        assert!(v.load(&patch(None, false), false).playable);
        assert!(v.load(&patch(Some("value"), false), false).playable);
    }

    #[test]
    fn published_trigger_routes_on_exactly_one_sample() {
        let mut p: serde_json::Value =
            serde_json::from_str(&patch(Some("trigger"), false)).unwrap();
        p["inputs"] =
            serde_json::json!([{"name":"strike","kind":"trigger","min":0,"max":1,"default":0}]);
        p["graph"]["nodes"][0]["params"]["autoStrike"] = false.into();
        let mut v = Voice::new(48000., 0);
        assert!(v.load(&p.to_string(), false).playable);
        let i = v.node_ids().iter().position(|id| id == "read").unwrap();
        let input = v.input_index("strike").unwrap();
        v.process(&mut [0.; 1], &mut [0.; 1]);
        assert_eq!(v.trace()[i], 0.);
        v.trigger(input);
        v.process(&mut [0.; 1], &mut [0.; 1]);
        assert_eq!(v.trace()[i], 1.);
        v.process(&mut [0.; 1], &mut [0.; 1]);
        assert_eq!(v.trace()[i], 0.);
    }
    #[test]
    fn implicit_value_and_explicit_value_are_identical() {
        let mut a = Voice::new(48000., 0);
        let mut b = Voice::new(48000., 0);
        assert!(a.load(&patch(None, false), false).playable);
        assert!(b.load(&patch(Some("value"), false), false).playable);
        for _ in 0..256 {
            a.process(&mut [0.; 1], &mut [0.; 1]);
            b.process(&mut [0.; 1], &mut [0.; 1]);
            assert_eq!(a.trace(), b.trace());
        }
    }
}
