//! Loss kernel: the FFT is right, identical sounds score zero, detune and level differences score
//! more, and the f0 term switches on for pitched targets only.

use haa_fit::fft::RealFft;
use haa_fit::loss::{F0Mode, LossConfig, TargetFeatures};

const RATE: f64 = 16_000.0;

fn sine(freq: f64, amp: f64, seconds: f64) -> Vec<f32> {
    let n = (seconds * RATE) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE;
            (amp * (2.0 * core::f64::consts::PI * freq * t).sin() * (-t * 3.0).exp()) as f32
        })
        .collect()
}

fn noise(seconds: f64) -> Vec<f32> {
    let mut rng = haa_core::rng::Rng::new(7);
    (0..(seconds * RATE) as usize).map(|_| (rng.next_f64() * 2.0 - 1.0) as f32 * 0.3).collect()
}

#[test]
fn real_fft_magnitudes_match_a_direct_dft() {
    let n = 64;
    let frame: Vec<f64> = (0..n).map(|i| ((i * 7 % 5) as f64 - 2.0) * 0.3 + (i as f64 * 0.4).sin()).collect();
    let mut fft = RealFft::new(n);
    let mut mags = vec![0.0; n / 2 + 1];
    fft.magnitudes(&frame, &mut mags);
    for (k, mag) in mags.iter().enumerate() {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, x) in frame.iter().enumerate() {
            let a = -2.0 * core::f64::consts::PI * (k * i) as f64 / n as f64;
            re += x * a.cos();
            im += x * a.sin();
        }
        assert!((mag - (re * re + im * im).sqrt()).abs() < 1e-9, "bin {k}");
    }
}

#[test]
fn identical_sounds_score_zero_and_detune_scores_more() {
    let target = sine(440.0, 0.5, 0.5);
    let mut features = TargetFeatures::new(&target, RATE, &LossConfig::default());
    let same = features.loss(&target);
    assert!(same.total.abs() < 1e-9, "{same:?}");
    let near = features.loss(&sine(445.0, 0.5, 0.5)).total;
    let far = features.loss(&sine(560.0, 0.5, 0.5)).total;
    assert!(near > 0.0 && far > near, "near {near} far {far}");
}

#[test]
fn level_is_compared_as_it_is() {
    let target = sine(440.0, 0.5, 0.5);
    let mut features = TargetFeatures::new(&target, RATE, &LossConfig::default());
    let half = features.loss(&sine(440.0, 0.25, 0.5));
    // −6 dB everywhere reads as about 6/20 on the envelope term: nothing is normalised away.
    assert!((half.env - 6.02 / 20.0).abs() < 0.03, "{half:?}");
}

#[test]
fn f0_term_turns_on_for_a_tone_and_stays_off_for_noise() {
    let cfg = LossConfig { f0: F0Mode::Auto, ..LossConfig::default() };
    let tone = TargetFeatures::new(&sine(220.0, 0.5, 0.5), RATE, &cfg);
    assert!(tone.uses_f0());
    assert!((tone.target_f0() - 220.0).abs() < 3.0, "f0 {}", tone.target_f0());
    let hiss = TargetFeatures::new(&noise(0.5), RATE, &cfg);
    assert!(!hiss.uses_f0());
}
