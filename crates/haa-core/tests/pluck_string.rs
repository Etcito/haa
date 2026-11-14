//! The `pluck` string as an instrument: score mode (a polyphonic part in one node) and the decay
//! shape of the two polarizations and the palm mute.

use haa_core::catalog::{NodeTypeDef, ParamKind, find, param_index};
use haa_core::node::{Node, NodeInit, SampleCtx};
use haa_core::patch::GraphMode;

const SR: f64 = 48_000.0;
const SECONDS: f64 = 2.0;

fn def() -> &'static NodeTypeDef {
    find("pluck").expect("pluck in the catalog")
}

fn params(overrides: &[(&str, f64)]) -> Vec<f64> {
    let d = def();
    let mut values: Vec<f64> = d
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
        values[param_index(d, key).expect("pluck param")] = *value;
    }
    values
}

/// Renders the node for SECONDS in timeline time, with an optional score text.
fn render(p: &[f64], score: Option<&str>) -> Vec<f64> {
    let d = def();
    let mut texts: Vec<Option<String>> = vec![None; p.len()];
    if let Some(s) = score {
        texts[param_index(d, "score").expect("score param")] = Some(s.to_string());
    }
    let mut node: Box<dyn Node> = (d.create)(&NodeInit {
        id: "s",
        seed: 3,
        sample_rate: SR,
        params: p,
        texts: &texts,
        curve: None,
        inputs: &[],
        duration: SECONDS,
    });
    let frames = (SR * SECONDS) as usize;
    (0..frames)
        .map(|i| {
            let ctx = SampleCtx {
                sample_rate: SR,
                dt: 1.0 / SR,
                t: i as f64 / frames as f64,
                seconds: i as f64 / SR,
                duration: SECONDS,
                mode: GraphMode::Timeline,
                live_frames: i as u64,
                looped: false,
                perf_rebase: false,
                inputs: &[],
                triggers: &[],
            };
            node.process_sample(&ctx, (0.0, 0.0), p).l
        })
        .collect()
}

fn goertzel(signal: &[f64], hz: f64) -> f64 {
    let w = std::f64::consts::TAU * hz / SR;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for x in signal {
        let s = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (signal.len() as f64).powi(2)
}

fn rms(x: &[f64]) -> f64 {
    (x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

#[test]
fn score_mode_plays_an_overlapping_chord_on_separate_strings() {
    // A2, E3 and C#4 struck 50 ms apart and held together: three strings ring at once.
    let score = r#"[{"start":0.1,"duration":1.5,"midi":45,"level":1},{"start":0.15,"duration":1.5,"midi":52,"level":1},{"start":0.2,"duration":1.5,"midi":61,"level":1}]"#;
    let out = render(&params(&[("stiffness", 0.0), ("attackBend", 0.0)]), Some(score));
    assert!(out.iter().all(|x| x.is_finite()));
    assert!(out[..(0.09 * SR) as usize].iter().all(|x| *x == 0.0), "silent before the first note");
    let held = &out[(0.5 * SR) as usize..(1.0 * SR) as usize];
    let hz = |m: f64| 440.0 * 2f64.powf((m - 69.0) / 12.0);
    let off = goertzel(held, hz(48.5)).max(goertzel(held, hz(56.5)));
    for midi in [45.0, 52.0, 61.0] {
        let on = goertzel(held, hz(midi));
        assert!(on > 30.0 * off, "MIDI {midi}: {on:e} vs off-pitch {off:e}");
    }
    // Released at 1.6-1.7 s: the strings are damped.
    let after = &out[(1.85 * SR) as usize..];
    assert!(rms(after) < 0.1 * rms(held), "released {} vs held {}", rms(after), rms(held));
}

#[test]
fn polarization_lengthens_the_tail_and_a_palm_mute_shortens_it() {
    let score = r#"[{"start":0.0,"duration":1.95,"midi":45,"level":1}]"#;
    let tail_ratio = |overrides: &[(&str, f64)]| {
        let out = render(&params(overrides), Some(score));
        rms(&out[(1.2 * SR) as usize..(1.6 * SR) as usize]) / rms(&out[(0.05 * SR) as usize..(0.25 * SR) as usize])
    };
    let base = [("decay", 1.5), ("decayTilt", 0.0)];
    let single = tail_ratio(&[base[0], base[1], ("polarization", 0.0)]);
    let double = tail_ratio(&[base[0], base[1], ("polarization", 0.5)]);
    let muted = tail_ratio(&[base[0], base[1], ("polarization", 0.0), ("mute", 0.8)]);
    assert!(double > 1.5 * single, "two polarizations decay in two stages: tail {double} vs {single}");
    assert!(muted < 0.1 * single, "palm mute: tail {muted} vs {single}");
}

#[test]
fn a_score_note_carries_its_own_decay_and_brightness() {
    // the same A2 twice: the second note sustains a quarter as long and is darker
    let tail_and_tone = |note: &str| {
        let out = render(&params(&[("decay", 2.0), ("decayTilt", 0.0), ("polarization", 0.0)]), Some(&format!("[{note}]")));
        let head = &out[(0.05 * SR) as usize..(0.25 * SR) as usize];
        let tail = rms(&out[(1.2 * SR) as usize..(1.6 * SR) as usize]) / rms(head);
        let hz = 110.0;
        (tail, goertzel(head, 8.0 * hz) / goertzel(head, hz))
    };
    let (tail, tone) = tail_and_tone(r#"{"start":0.0,"duration":1.95,"midi":45,"level":1}"#);
    let (tail_short, tone_dark) = tail_and_tone(r#"{"start":0.0,"duration":1.95,"midi":45,"level":1,"decay":0.25,"brightness":-0.4}"#);
    assert!(tail_short < 0.2 * tail, "decay x0.25: tail {tail_short} vs {tail}");
    assert!(tone_dark < 0.5 * tone, "brightness -0.4: 8th harmonic share {tone_dark} vs {tone}");
}
