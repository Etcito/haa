//! The `pluck` string as an instrument: score mode (a polyphonic part in one node) and the decay
//! shape of the two polarizations and the palm mute; a partial release, split air/bridge outputs and
//! sympathetic open strings.

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

/// Renders the node for SECONDS in timeline time, with an optional score text (left channel).
fn render(p: &[f64], score: Option<&str>) -> Vec<f64> {
    render_stereo(p, score).into_iter().map(|(l, _)| l).collect()
}

/// Both channels (they differ only with `split`).
fn render_stereo(p: &[f64], score: Option<&str>) -> Vec<(f64, f64)> {
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
            let out = node.process_sample(&ctx, (0.0, 0.0), p);
            (out.l, out.r)
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

fn span(x: &[f64], a: f64, b: f64) -> &[f64] {
    &x[(a * SR) as usize..(b * SR) as usize]
}

#[test]
fn a_partial_release_damps_by_its_depth_then_rings_on() {
    // G2 held 0.3 s, released with a 0.1 s T60: a 20 dB release leaves the string ringing about
    // 20 dB down at its fundamental, a full release (the default) silences it.
    let score = r#"[{"start":0.05,"duration":0.3,"midi":43,"level":1}]"#;
    let base = [("decay", 6.0), ("decayTilt", 0.0), ("polarization", 0.0), ("release", 0.1), ("pickNoise", 0.0)];
    let drop = |depth: f64, from: f64| {
        let mut o = base.to_vec();
        o.push(("releaseDepth", depth));
        let out = render(&params(&o), Some(score));
        10.0 * (goertzel(span(&out, from, from + 0.15), 98.0) / goertzel(span(&out, 0.18, 0.33), 98.0)).log10()
    };
    let partial = drop(20.0, 0.45);
    let full = drop(120.0, 0.9);
    assert!((-28.0..-14.0).contains(&partial), "20 dB release: {partial} dB");
    assert!(full < -60.0, "full release: {full} dB");
}

#[test]
fn split_outputs_send_the_fast_polarization_to_the_bridge() {
    // With fastDecay the right (bridge) side loses its loud attack quickly; the left (air) side
    // hears the slow polarization and keeps ringing.
    let score = r#"[{"start":0.0,"duration":1.9,"midi":43,"level":1}]"#;
    let p = params(&[("split", 1.0), ("fastDecay", 0.25), ("polarization", 0.2), ("decay", 6.0), ("decayTilt", 0.0), ("pickNoise", 0.0)]);
    let (l, r): (Vec<f64>, Vec<f64>) = render_stereo(&p, Some(score)).into_iter().unzip();
    let fall = |x: &[f64]| 20.0 * (rms(span(x, 0.02, 0.1)) / rms(span(x, 0.8, 1.0))).log10();
    assert!(fall(&r) > fall(&l) + 10.0, "bridge falls {} dB, air {} dB", fall(&r), fall(&l));
    // Without split both channels are the same signal.
    let mono = render_stereo(&params(&[("fastDecay", 0.25)]), Some(score));
    assert!(mono.iter().all(|(a, b)| a == b));
}

#[test]
fn sympathetic_open_strings_ring_after_the_played_note_is_damped() {
    // D3 (the open D string's pitch) played 0.3 s then fully damped: only the sympathetic
    // strings keep sounding at 146.8 Hz.
    let score = r#"[{"start":0.05,"duration":0.3,"midi":50,"level":1}]"#;
    let ring = |amount: f64| {
        let out = render(&params(&[("release", 0.02), ("sympathetic", amount), ("pickNoise", 0.0)]), Some(score));
        goertzel(span(&out, 0.8, 1.2), 146.83)
    };
    let (with, without) = (ring(1.0), ring(0.0));
    assert!(with > 1e-10 && with > 1e4 * without, "with {with:e} without {without:e}");
}
