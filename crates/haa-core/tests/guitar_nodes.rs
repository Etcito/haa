//! Guitar sources and amp chain: `pluck` (tuning, gate, pickups, extremes), `amp` (bounds,
//! harmonics, aliasing, DC), `cabinet` (roll-off) and the guitar example patches.

use std::{fs, path::PathBuf};

use haa_core::catalog::{NodeTypeDef, ParamKind, find, param_index};
use haa_core::node::{Node, NodeInit, SampleCtx};
use haa_core::patch::GraphMode;
use haa_core::{Voice, issues::has_errors, load_patch, validate};

const SR: f64 = 48_000.0;

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

fn make(def: &NodeTypeDef, rate: f64, p: &[f64]) -> Box<dyn Node> {
    let texts: Vec<Option<String>> = vec![None; p.len()];
    (def.create)(&NodeInit {
        id: "n1",
        seed: 7,
        sample_rate: rate,
        params: p,
        texts: &texts,
        curve: None,
        inputs: &[],
        duration: 2.0,
    })
}

fn ctx(frame: u64, rate: f64) -> SampleCtx<'static> {
    SampleCtx {
        sample_rate: rate,
        dt: 1.0 / rate,
        t: 0.0,
        seconds: frame as f64 / rate,
        duration: 2.0,
        mode: GraphMode::Live,
        live_frames: frame,
        looped: false,
        perf_rebase: false,
        inputs: &[],
        triggers: &[],
    }
}

/// Runs a source node for `frames` samples; `p_at` may change params per frame.
fn run_source(name: &str, rate: f64, p: &[f64], frames: usize) -> Vec<f64> {
    let d = def(name);
    let mut node = make(d, rate, p);
    (0..frames).map(|i| node.process_sample(&ctx(i as u64, rate), (0.0, 0.0), p).l).collect()
}

fn run_effect(name: &str, rate: f64, p: &[f64], input: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let d = def(name);
    let mut node = make(d, rate, p);
    input
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let out = node.process_sample(&ctx(i as u64, rate), *x, p);
            (out.l, out.r)
        })
        .collect()
}

/// YIN fundamental estimate (cumulative mean normalized difference, parabolic refinement).
fn yin(signal: &[f64], rate: f64, min_hz: f64, max_hz: f64) -> f64 {
    let max_lag = (rate / min_hz) as usize + 2;
    let min_lag = ((rate / max_hz) as usize).max(2);
    let window = signal.len() - max_lag - 2;
    let mut d = vec![0.0; max_lag + 2];
    for (lag, slot) in d.iter_mut().enumerate().skip(1) {
        *slot = (0..window).map(|i| (signal[i] - signal[i + lag]).powi(2)).sum();
    }
    let mut cmnd = vec![1.0; max_lag + 2];
    let mut running = 0.0;
    for lag in 1..max_lag + 2 {
        running += d[lag];
        cmnd[lag] = if running > 0.0 { d[lag] * lag as f64 / running } else { 1.0 };
    }
    let mut best = min_lag;
    for lag in min_lag..max_lag {
        if cmnd[lag] < 0.15 {
            best = lag;
            while best + 1 < max_lag && cmnd[best + 1] < cmnd[best] {
                best += 1;
            }
            break;
        }
        if cmnd[lag] < cmnd[best] {
            best = lag;
        }
    }
    let (a, b, c) = (cmnd[best - 1], cmnd[best], cmnd[best + 1]);
    let shift = if (a - 2.0 * b + c).abs() > 1e-12 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
    rate / (best as f64 + shift)
}

fn cents(measured: f64, wanted: f64) -> f64 {
    1200.0 * (measured / wanted).log2()
}

fn goertzel_power(signal: &[f64], rate: f64, hz: f64) -> f64 {
    let w = std::f64::consts::TAU * hz / rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for x in signal {
        let s = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (signal.len() as f64).powi(2)
}

fn centroid(signal: &[f64], rate: f64) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    let mut hz = 50.0;
    while hz < 12_000.0 {
        let power = goertzel_power(signal, rate, hz);
        num += hz * power;
        den += power;
        hz *= 1.06;
    }
    num / den
}

fn rms(signal: &[f64]) -> f64 {
    (signal.iter().map(|x| x * x).sum::<f64>() / signal.len() as f64).sqrt()
}

#[test]
fn pluck_holds_pitch_within_fifteen_cents_from_bass_to_high_frets() {
    for (hz, stiffness) in [(41.2, 0.0), (41.2, 0.3), (82.41, 0.1), (110.0, 0.1), (196.0, 0.1), (329.63, 0.1), (659.26, 0.1), (1_200.0, 0.1)] {
        let p = params(def("pluck"), &[("frequency", hz), ("stiffness", stiffness), ("body", 0.0), ("decay", 6.0)]);
        let out = run_source("pluck", SR, &p, (SR * 0.8) as usize);
        let segment = &out[(SR * 0.25) as usize..];
        let f0 = yin(segment, SR, 30.0, 1_600.0);
        let error = cents(f0, hz);
        assert!(error.abs() < 15.0, "{hz} Hz (stiffness {stiffness}) measured {f0:.3} Hz, {error:.2} cents");
        assert!(out.iter().all(|x| x.is_finite()));
    }
}

#[test]
fn pluck_without_a_pluck_is_silent_and_a_gate_edge_plucks_then_damps() {
    let silent = params(def("pluck"), &[("autoStrike", 0.0)]);
    assert!(run_source("pluck", SR, &silent, 4_800).iter().all(|x| *x == 0.0));

    // Gate held 0.1..0.3 s, then released: the release damps the string far below a held one.
    let d = def("pluck");
    let gate = param_index(d, "gate").unwrap();
    let mut released = params(d, &[("autoStrike", 0.0), ("frequency", 196.0), ("decay", 5.0), ("release", 0.05)]);
    let mut held = released.clone();
    let mut a = make(d, SR, &released);
    let mut b = make(d, SR, &held);
    let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
    for i in 0..(SR * 0.6) as usize {
        let t = i as f64 / SR;
        released[gate] = if (0.1..0.3).contains(&t) { 0.9 } else { 0.0 };
        held[gate] = if t >= 0.1 { 0.9 } else { 0.0 };
        out_a.push(a.process_sample(&ctx(i as u64, SR), (0.0, 0.0), &released).l);
        out_b.push(b.process_sample(&ctx(i as u64, SR), (0.0, 0.0), &held).l);
    }
    let before = (SR * 0.1) as usize;
    assert!(out_a[..before].iter().all(|x| *x == 0.0), "nothing sounds before the gate");
    let early = (SR * 0.15) as usize..(SR * 0.25) as usize;
    assert!(rms(&out_a[early.clone()]) > 0.03, "gate edge plucks: rms {}", rms(&out_a[early]));
    let late = (SR * 0.45) as usize..(SR * 0.6) as usize;
    let ratio = rms(&out_a[late.clone()]) / rms(&out_b[late]);
    assert!(ratio < 0.05, "released/held level ratio {ratio}");
}

#[test]
fn pluck_pickups_shape_the_tone() {
    let d = def("pluck");
    let tone = |pickup: f64, position: f64| {
        let p = params(d, &[("frequency", 110.0), ("body", 0.0), ("pickup", pickup), ("pickupPosition", position), ("pickHardness", 0.8)]);
        let out = run_source("pluck", SR, &p, (SR * 0.5) as usize);
        centroid(&out[(SR * 0.05) as usize..(SR * 0.3) as usize], SR)
    };
    let none = tone(0.0, 0.2);
    let single_bridge = tone(1.0, 0.08);
    let single_neck = tone(1.0, 0.3);
    let humbucker_bridge = tone(2.0, 0.08);
    println!("centroid none {none:.0} single bridge {single_bridge:.0} single neck {single_neck:.0} humbucker bridge {humbucker_bridge:.0}");
    assert!(single_neck < single_bridge, "neck pickup darker than bridge");
    assert!(humbucker_bridge < single_bridge, "humbucker darker than single coil");
}

#[test]
fn pluck_stays_finite_at_every_param_extreme_and_rate() {
    let d = def("pluck");
    let gate = param_index(d, "gate").unwrap();
    for rate in [8_000.0, 48_000.0, 192_000.0] {
        for (hz, decay, damping, stiffness, position, hardness, pickup) in [
            (20.0, 30.0, 0.0, 1.0, 0.02, 0.0, 0.0),
            (4_000.0, 30.0, 0.0, 1.0, 0.5, 1.0, 1.0),
            (4_000.0, 0.05, 1.0, 0.0, 0.02, 1.0, 2.0),
            (41.2, 30.0, 0.0, 1.0, 0.5, 1.0, 2.0),
        ] {
            let mut p = params(
                d,
                &[
                    ("frequency", hz),
                    ("decay", decay),
                    ("damping", damping),
                    ("stiffness", stiffness),
                    ("pickPosition", position),
                    ("pickHardness", hardness),
                    ("pickup", pickup),
                    ("body", 1.0),
                    ("gain", 2.0),
                    ("velocity", 1.0),
                ],
            );
            let mut node = make(d, rate, &p);
            for i in 0..(rate * 0.4) as usize {
                // Re-pluck every 25 ms to exercise the choke and overlapping excitations.
                p[gate] = if (i / (rate * 0.0125) as usize) % 2 == 1 { 1.0 } else { 0.0 };
                let out = node.process_sample(&ctx(i as u64, rate), (0.0, 0.0), &p);
                assert!(out.l.is_finite() && out.l.abs() <= 2.0, "rate {rate} hz {hz}: {}", out.l);
            }
        }
    }
}

fn sine(hz: f64, amplitude: f64, frames: usize) -> Vec<(f64, f64)> {
    (0..frames)
        .map(|i| {
            let x = amplitude * (std::f64::consts::TAU * hz * i as f64 / SR).sin();
            (x, x)
        })
        .collect()
}

#[test]
fn amp_is_silent_for_silence_bounded_at_full_gain_and_dc_free() {
    let d = def("amp");
    let p = params(d, &[("gain", 48.0), ("bias", 1.0), ("bass", 12.0), ("mid", 12.0), ("treble", 12.0), ("presence", 12.0), ("level", 12.0)]);
    let silent = run_effect("amp", SR, &p, &vec![(0.0, 0.0); 2_000]);
    assert!(silent.iter().all(|(l, r)| *l == 0.0 && *r == 0.0));

    let loud: Vec<(f64, f64)> = (0..(SR * 0.5) as usize)
        .map(|i| if (i / 40) % 2 == 0 { (8.0, -8.0) } else { (-8.0, 8.0) })
        .collect();
    let out = run_effect("amp", SR, &p, &loud);
    let peak = out.iter().fold(0.0f64, |m, (l, r)| m.max(l.abs()).max(r.abs()));
    // Bounded by construction: stage output <= 1, makeup <= 1/2, tone stack <= +30 dB, level +12 dB.
    assert!(out.iter().all(|(l, r)| l.is_finite() && r.is_finite()) && peak < 64.0, "peak {peak}");
    let neutral = params(d, &[("gain", 48.0), ("bias", 1.0)]);
    let out = run_effect("amp", SR, &neutral, &loud);
    let peak = out.iter().fold(0.0f64, |m, (l, r)| m.max(l.abs()).max(r.abs()));
    assert!(peak < 1.0, "full gain with a flat tone stack peaks at {peak}");

    let flat = params(d, &[("gain", 40.0), ("bias", 1.0)]);
    let out = run_effect("amp", SR, &flat, &sine(220.0, 0.5, (SR * 1.0) as usize));
    let tail: Vec<f64> = out[(SR * 0.5) as usize..].iter().map(|(l, _)| *l).collect();
    let mean = tail.iter().sum::<f64>() / tail.len() as f64;
    assert!(mean.abs() < 0.01 * rms(&tail), "dc {mean} vs rms {}", rms(&tail));
}

#[test]
fn amp_gain_adds_upper_harmonics_and_keeps_aliasing_low() {
    let d = def("amp");
    let upper_ratio = |gain: f64| {
        let p = params(d, &[("gain", gain)]);
        let out = run_effect("amp", SR, &p, &sine(220.0, 0.3, 9_600));
        let x: Vec<f64> = out[4_800..].iter().map(|(l, _)| *l).collect();
        let low: f64 = (1..=5).map(|k| goertzel_power(&x, SR, 220.0 * k as f64)).sum();
        let high: f64 = (6..=40).map(|k| goertzel_power(&x, SR, 220.0 * k as f64)).sum();
        high / low
    };
    let clean = upper_ratio(0.0);
    let crunch = upper_ratio(24.0);
    let lead = upper_ratio(44.0);
    println!("upper/lower harmonic power: clean {clean:.2e} crunch {crunch:.2e} lead {lead:.2e}");
    assert!(crunch > 10.0 * clean && lead > crunch);

    // 3110 Hz is bin-exact over 4800 samples (10 Hz bins): every alias also lands on a bin, so the
    // power off the harmonic bins is the aliasing.
    let p = params(d, &[("gain", 48.0), ("presence", 0.0)]);
    let out = run_effect("amp", SR, &p, &sine(3_110.0, 0.5, 9_600));
    let x: Vec<f64> = out[4_800..].iter().map(|(l, _)| *l).collect();
    let (mut harmonic, mut other) = (0.0, 0.0);
    for bin in 1..2_400 {
        let hz = bin as f64 * 10.0;
        let power = goertzel_power(&x, SR, hz);
        if bin % 311 == 0 { harmonic += power } else { other += power }
    }
    let alias_db = 10.0 * (other / harmonic).log10();
    println!("amp alias power at 48 dB gain, 3110 Hz: {alias_db:.1} dB");
    assert!(alias_db < -35.0, "alias {alias_db:.1} dB");
}

#[test]
fn cabinet_rolls_off_above_five_kilohertz_for_every_voicing() {
    let d = def("cabinet");
    for voicing in 0..3 {
        let p = params(d, &[("voicing", voicing as f64)]);
        let level = |hz: f64| {
            let out = run_effect("cabinet", SR, &p, &sine(hz, 0.5, 9_600));
            let x: Vec<f64> = out[4_800..].iter().map(|(l, _)| *l).collect();
            rms(&x)
        };
        let mid = level(1_000.0);
        let high = level(8_000.0);
        let drop_db = 20.0 * (high / mid).log10();
        println!("cabinet voicing {voicing}: 8 kHz vs 1 kHz {drop_db:.1} dB");
        assert!(drop_db < -18.0, "voicing {voicing}: {drop_db:.1} dB");
        let silent = run_effect("cabinet", SR, &p, &vec![(0.0, 0.0); 1_000]);
        assert!(silent.iter().all(|(l, r)| *l == 0.0 && *r == 0.0));
    }
}

fn guitar_patches() -> Vec<PathBuf> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../patches/examples");
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            (name.starts_with("guitar-") || name.starts_with("bass-guitar")) && name.ends_with(".json")
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 5, "five guitar example patches");
    files
}

#[test]
fn guitar_example_patches_validate_and_play_finite_audio() {
    for path in guitar_patches() {
        let json = fs::read_to_string(&path).unwrap();
        let (patch, issues) = load_patch(&json).unwrap();
        assert!(!has_errors(&issues), "{}: {issues:?}", path.display());
        let (_, issues) = validate(&patch).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        assert!(issues.is_empty(), "{}: {issues:?}", path.display());
        let mut voice = Voice::new(SR, 0);
        assert!(voice.load(&json, false).playable);
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        let mut peak = 0.0f32;
        for _ in 0..(2.0 * SR / 128.0) as usize {
            voice.process(&mut l, &mut r);
            for x in l.iter().chain(r.iter()) {
                assert!(x.is_finite());
                peak = peak.max(x.abs());
            }
        }
        assert_eq!(voice.guard_trips(), 0, "{}", path.display());
        assert!(peak > 0.05, "{}: peak {peak}", path.display());
    }
}
