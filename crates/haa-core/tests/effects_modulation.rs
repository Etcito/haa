//! Modulation effects: `chorus`, `flanger`, `phaser`, `tremolo` (silence, extremes, dry
//! passthrough, comb/notch placement, amplitude modulation and stereo width).

use haa_core::catalog::{NodeTypeDef, ParamKind, find, param_index};
use haa_core::node::{Node, NodeInit, SampleCtx};
use haa_core::patch::GraphMode;

const SR: f64 = 48_000.0;
const EFFECTS: [&str; 4] = ["chorus", "flanger", "phaser", "tremolo"];

fn def(name: &str) -> &'static NodeTypeDef {
    find(name).unwrap_or_else(|| panic!("node type {name} missing from the catalog"))
}

fn params(def: &NodeTypeDef, overrides: &[(&str, f64)]) -> Vec<f64> {
    let mut values: Vec<f64> = def
        .params
        .iter()
        .map(|p| match p.kind {
            ParamKind::Number { default, .. } => default,
            ParamKind::Enum { default, .. } => default as f64,
            ParamKind::Bool { default } => f64::from(u8::from(default)),
            ParamKind::Text { .. } => 0.0,
        })
        .collect();
    for (key, value) in overrides {
        let index = param_index(def, key).unwrap_or_else(|| panic!("{} has no param {key}", def.type_name));
        values[index] = *value;
    }
    values
}

fn extremes(def: &NodeTypeDef) -> Vec<Vec<f64>> {
    let mut low = params(def, &[]);
    let mut high = params(def, &[]);
    for (i, p) in def.params.iter().enumerate() {
        match p.kind {
            ParamKind::Number { min, max, .. } => {
                low[i] = min;
                high[i] = max;
            }
            ParamKind::Enum { values, .. } => high[i] = (values.len() - 1) as f64,
            _ => {}
        }
    }
    vec![low, high]
}

fn ctx(frame: u64) -> SampleCtx<'static> {
    SampleCtx {
        sample_rate: SR,
        dt: 1.0 / SR,
        t: 0.0,
        seconds: frame as f64 / SR,
        duration: 2.0,
        mode: GraphMode::Live,
        live_frames: frame,
        looped: false,
        perf_rebase: false,
        inputs: &[],
        triggers: &[],
    }
}

fn run(name: &str, p: &[f64], input: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let d = def(name);
    let texts: Vec<Option<String>> = vec![None; p.len()];
    let mut node: Box<dyn Node> = (d.create)(&NodeInit {
        id: "fx1",
        seed: 1,
        sample_rate: SR,
        params: p,
        texts: &texts,
        curve: None,
        inputs: &[],
        duration: 2.0,
    });
    input
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let out = node.process_sample(&ctx(i as u64), *x, p);
            (out.l, out.r)
        })
        .collect()
}

fn sine(hz: f64, frames: usize) -> Vec<(f64, f64)> {
    (0..frames)
        .map(|i| {
            let x = 0.5 * (std::f64::consts::TAU * hz * i as f64 / SR).sin();
            (x, x)
        })
        .collect()
}

/// Deterministic white noise (LCG), mono in both channels.
fn noise(frames: usize) -> Vec<(f64, f64)> {
    let mut state = 0x1234_5678u32;
    (0..frames)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = state as f64 / u32::MAX as f64 - 0.5;
            (x, x)
        })
        .collect()
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt()
}

fn left(x: &[(f64, f64)]) -> Vec<f64> {
    x.iter().map(|(l, _)| *l).collect()
}

#[test]
fn silence_in_gives_silence_out_and_extremes_stay_finite() {
    for name in EFFECTS {
        let d = def(name);
        for p in extremes(d).into_iter().chain([params(d, &[])]) {
            let out = run(name, &p, &vec![(0.0, 0.0); 2_000]);
            assert!(out.iter().all(|(l, r)| *l == 0.0 && *r == 0.0), "{name} silence");
            let mut input = noise((SR * 0.5) as usize);
            for x in input.iter_mut().step_by(997) {
                *x = (8.0, -8.0);
            }
            let out = run(name, &p, &input);
            let peak = out.iter().fold(0.0f64, |m, (l, r)| m.max(l.abs()).max(r.abs()));
            assert!(out.iter().all(|(l, r)| l.is_finite() && r.is_finite()) && peak < 100.0, "{name} {p:?} peak {peak}");
        }
    }
}

#[test]
fn zero_mix_or_depth_passes_the_signal_dry() {
    let input = noise(4_800);
    for (name, key) in [("chorus", "mix"), ("flanger", "mix"), ("phaser", "mix"), ("tremolo", "depth")] {
        let out = run(name, &params(def(name), &[(key, 0.0)]), &input);
        for (x, y) in input.iter().zip(&out) {
            assert!((x.0 - y.0).abs() < 1e-12 && (x.1 - y.1).abs() < 1e-12, "{name} not dry");
        }
    }
}

#[test]
fn flanger_comb_and_phaser_notches_sit_where_the_delay_and_stages_put_them() {
    // Flanger at a fixed 1 ms (48 samples), mix 0.5, no feedback: nulls at 500 Hz, peak at 1 kHz.
    let p = params(def("flanger"), &[("depth", 0.0), ("delay", 1.0), ("feedback", 0.0), ("mix", 0.5)]);
    let level = |name: &str, p: &[f64], hz: f64| rms(&left(&run(name, p, &sine(hz, 9_600))[4_800..]));
    let notch = level("flanger", &p, 500.0);
    let peak = level("flanger", &p, 1_000.0);
    let depth_db = 20.0 * (notch / peak).log10();
    println!("flanger notch 500 Hz vs 1 kHz: {depth_db:.1} dB");
    assert!(depth_db < -30.0);

    // Four stages around 1 kHz: each stage is -45° at 0.414 x fc, so the cascade inverts there.
    let p = params(def("phaser"), &[("depth", 0.0), ("center", 1_000.0), ("feedback", 0.0), ("mix", 0.5), ("stages", 0.0)]);
    let notch_hz = SR / std::f64::consts::PI * ((std::f64::consts::PI * 1_000.0 / SR).tan() * (std::f64::consts::PI / 8.0).tan()).atan();
    let notch = level("phaser", &p, notch_hz);
    let pass = level("phaser", &p, 1_000.0);
    let depth_db = 20.0 * (notch / pass).log10();
    println!("phaser notch {notch_hz:.0} Hz vs 1 kHz: {depth_db:.1} dB");
    assert!(depth_db < -30.0);
}

#[test]
fn tremolo_modulates_level_and_width_turns_it_into_auto_pan() {
    let input = sine(440.0, (SR * 1.0) as usize);
    let p = params(def("tremolo"), &[("rate", 5.0), ("depth", 1.0), ("width", 1.0)]);
    let out = run("tremolo", &p, &input);
    let block = (SR * 0.01) as usize;
    let env = |pick: fn(&(f64, f64)) -> f64| -> Vec<f64> {
        out.chunks(block).map(|c| rms(&c.iter().map(pick).collect::<Vec<_>>())).collect()
    };
    let (env_l, env_r) = (env(|x| x.0), env(|x| x.1));
    let max = env_l.iter().cloned().fold(0.0, f64::max);
    let min = env_l.iter().cloned().fold(f64::MAX, f64::min);
    assert!(max / min.max(1e-9) > 10.0, "depth 1 swings the level: {max} / {min}");
    let mean_l = env_l.iter().sum::<f64>() / env_l.len() as f64;
    let mean_r = env_r.iter().sum::<f64>() / env_r.len() as f64;
    let cov: f64 = env_l.iter().zip(&env_r).map(|(a, b)| (a - mean_l) * (b - mean_r)).sum();
    assert!(cov < 0.0, "auto-pan: left and right envelopes move in opposite directions");
}

#[test]
fn chorus_turns_mono_into_moving_stereo() {
    let input = sine(440.0, (SR * 1.0) as usize);
    let out = run("chorus", &params(def("chorus"), &[("depth", 1.0), ("width", 1.0), ("rate", 1.0)]), &input);
    let tail = &out[(SR * 0.1) as usize..];
    let side: Vec<f64> = tail.iter().map(|(l, r)| l - r).collect();
    let mid: Vec<f64> = tail.iter().map(|(l, r)| l + r).collect();
    let width = rms(&side) / rms(&mid);
    println!("chorus side/mid ratio {width:.3}");
    assert!(width > 0.2, "side/mid {width}");
    // The sweep beats the dry and wet copies: the 10 ms envelope of the left channel moves.
    let block = (SR * 0.01) as usize;
    let env: Vec<f64> = left(tail).chunks(block).map(rms).collect();
    let max = env.iter().cloned().fold(0.0, f64::max);
    let min = env.iter().cloned().fold(f64::MAX, f64::min);
    assert!(max / min > 1.2, "chorus envelope {max} / {min}");
}
