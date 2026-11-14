//! `pan` effect node: places its input between the left and right channels (constant power).
//! `pan` -1 is hard left, 0 leaves the input as it is, 1 is hard right. A stereo input is balanced, not summed.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

const PAN: usize = 0;

/// The `pan` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "pan",
    category: Category::Effect,
    label_en: "Pan",
    label_fr: "Panoramique",
    audio_in: true,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[ParamDef {
        key: "pan",
        kind: ParamKind::Number {
            min: -1.0,
            max: 1.0,
            default: 0.0,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: "Pan",
        label_fr: "Panoramique",
        modulatable: true,
        smooth: true,
    }],
    create,
};

fn create(_init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Pan)
}

struct Pan;

impl Node for Pan {
    fn process_sample(
        &mut self,
        _ctx: &SampleCtx<'_>,
        input: (f64, f64),
        params: &[f64],
    ) -> NodeOut {
        // constant power, unity at centre
        let angle = (params[PAN].clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
        let left = input.0 * angle.cos() * std::f64::consts::SQRT_2;
        let right = input.1 * angle.sin() * std::f64::consts::SQRT_2;
        let left = if left.is_finite() { left } else { 0.0 };
        let right = if right.is_finite() { right } else { 0.0 };
        NodeOut {
            l: left,
            r: right,
            value: (left + right) * 0.5,
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const RATE: f64 = 48_000.0;

    fn make_node(params: &[f64]) -> Box<dyn Node> {
        (DEF.create)(&NodeInit {
            id: "pan1",
            seed: 0,
            sample_rate: RATE,
            params,
            texts: &[],
            curve: None,
            inputs: &[],
            duration: 1.0,
        })
    }

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: RATE,
            dt: 1.0 / RATE,
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

    fn run(pan: f64, input: (f64, f64)) -> (f64, f64) {
        let params = [pan];
        let out = make_node(&params).process_sample(&ctx(), input, &params);
        (out.l, out.r)
    }

    #[test]
    fn centre_leaves_the_input_unchanged() {
        for input in [(0.5, -0.25), (1.0, 1.0), (-0.8, 0.3), (0.0, 0.7)] {
            let (l, r) = run(0.0, input);
            assert!((l - input.0).abs() < 1e-12, "left {l} for {input:?}");
            assert!((r - input.1).abs() < 1e-12, "right {r} for {input:?}");
        }
    }

    #[test]
    fn hard_left_and_right_silence_the_other_channel() {
        let (l, r) = run(-1.0, (0.6, 0.6));
        assert!(r.abs() < 1e-12, "hard left leaves right {r}");
        assert!((l - 0.6 * std::f64::consts::SQRT_2).abs() < 1e-12, "left {l}");
        let (l, r) = run(1.0, (0.6, 0.6));
        assert!(l.abs() < 1e-12, "hard right leaves left {l}");
        assert!((r - 0.6 * std::f64::consts::SQRT_2).abs() < 1e-12, "right {r}");
    }

    #[test]
    fn mono_input_keeps_constant_power() {
        let x = 0.5;
        let centre = x * x * 2.0;
        for step in 0..=40 {
            let pan = -1.0 + step as f64 * 0.05;
            let (l, r) = run(pan, (x, x));
            let power = l * l + r * r;
            assert!((power - centre).abs() < 1e-12, "power {power} at pan {pan}");
        }
    }
}
