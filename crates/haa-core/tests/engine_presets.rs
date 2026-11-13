//! Engine presets built only from general primitives (pulse, waveguide, harmonics, shaper,
//! crossover, bandNoise, ring, sampleHold, ...): every `patches/examples/engine-*.json` validates
//! and renders finite audio, and the 45° V-twin idle fires unevenly at 405° / 315° of its
//! 720° cycle, measured from the pulse onsets of the rendered sound.

use std::{fs, path::PathBuf};

use haa_core::render::{RenderOptions, render};
use haa_core::{issues::has_errors, load_patch, validate};

fn examples() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../patches/examples"))
}

fn engine_presets() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(examples())
        .expect("read examples")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("engine-") && n.ends_with(".json")))
        .collect();
    files.sort();
    files
}

#[test]
fn every_engine_preset_validates_and_renders_finite_audio() {
    let files = engine_presets();
    assert_eq!(files.len(), 5, "{files:?}");
    for path in files {
        let json = fs::read_to_string(&path).unwrap();
        let (patch, issues) = load_patch(&json).unwrap_or_else(|i| panic!("{}: {}", path.display(), i.message));
        assert!(!has_errors(&issues), "{}: {issues:?}", path.display());
        let types = ["engine", "combustion", "exhaust"];
        assert!(patch.graph.nodes.iter().all(|n| !types.contains(&n.kind.as_str())), "{}", path.display());
        validate(&patch).unwrap_or_else(|i| panic!("{}: {i:?}", path.display()));
        let out = render(&json, &RenderOptions { seconds: Some(1.0), ..RenderOptions::default() }).unwrap();
        let peak = out.left.iter().chain(&out.right).fold(0.0_f32, |m, v| m.max(v.abs()));
        assert!(out.left.iter().chain(&out.right).all(|v| v.is_finite()), "{}", path.display());
        assert!((0.05..=0.92).contains(&peak), "{} peak {peak}", path.display());
    }
}

/// One-pole lowpass of `x` at `hz`.
fn lowpass(x: &[f64], hz: f64, sr: f64) -> Vec<f64> {
    let a = 1.0 - (-std::f64::consts::TAU * hz / sr).exp();
    let mut y = 0.0;
    x.iter().map(|v| { y += (v - y) * a; y }).collect()
}

#[test]
fn harley_idle_fires_at_405_and_315_degrees_of_the_cycle() {
    let json = fs::read_to_string(examples().join("engine-harley-vtwin-idle.json")).unwrap();
    let sr = 48_000.0;
    let out = render(&json, &RenderOptions { seconds: Some(2.0), ..RenderOptions::default() }).unwrap();
    let mono: Vec<f64> = out.left.iter().zip(&out.right).map(|(l, r)| f64::from(*l + *r) * 0.5).collect();
    // 30..500 Hz band, rectified, 3 ms envelope: the exhaust thumps dominate it.
    let low = lowpass(&mono, 500.0, sr);
    let slow = lowpass(&low, 30.0, sr);
    let band: Vec<f64> = low.iter().zip(&slow).map(|(a, b)| (a - b).abs()).collect();
    let env = lowpass(&band, 1.0 / 0.003 / std::f64::consts::TAU, sr);
    // At 850 rpm the 720° cycle lasts 120/850 s; onsets are at least a quarter cycle apart.
    let cycle = 120.0 / 850.0;
    let gap = (0.25 * cycle * sr) as usize;
    let skip = (0.3 * sr) as usize;
    let threshold = 0.3 * env[skip..].iter().cloned().fold(0.0, f64::max);
    let mut onsets = Vec::new();
    let mut i = skip;
    while i < env.len() {
        if env[i] > threshold {
            let end = (i + gap).min(env.len());
            let peak = (i..end).max_by(|a, b| env[*a].total_cmp(&env[*b])).unwrap();
            onsets.push(peak);
            i = peak + gap;
        } else {
            i += 1;
        }
    }
    let intervals: Vec<f64> = onsets.windows(2).map(|w| (w[1] - w[0]) as f64 / sr).collect();
    let mut ratios: Vec<f64> = intervals
        .chunks_exact(2)
        .map(|p| p[0].min(p[1]) / (p[0] + p[1]))
        .collect();
    assert!(ratios.len() >= 8, "{} interval pairs from {} onsets", ratios.len(), onsets.len());
    ratios.sort_by(f64::total_cmp);
    let median = ratios[ratios.len() / 2];
    // 315 / 720 = 0.4375 of the cycle between rear and front firing.
    assert!((median - 0.4375).abs() < 0.02, "median short-interval ratio {median}");
    let mut cycles: Vec<f64> = intervals.chunks_exact(2).map(|p| p[0] + p[1]).collect();
    cycles.sort_by(f64::total_cmp);
    let mean_cycle = cycles[cycles.len() / 2];
    assert!((mean_cycle / cycle - 1.0).abs() < 0.1, "cycle {mean_cycle} s vs {cycle} s");
}
