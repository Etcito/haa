//! The spectral judge: zero for the stem itself, and its note terms see a wrong harmonic balance
//! and a wrong decay that the band levels barely show.

use haa_fit::{NoteSpan, SpectralTarget, SpectralWeights};

const SR: f64 = 44_100.0;

/// A plucked-like tone: harmonics 1..10 with amplitude k^-tilt, decaying at `t60` seconds.
fn tone(f0: f64, tilt: f64, t60: f64, seconds: f64) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let env = 10f64.powf(-3.0 * t / t60);
            let s: f64 = (1..=10).map(|k| (k as f64).powf(-tilt) * (std::f64::consts::TAU * k as f64 * f0 * t).sin()).sum();
            (0.2 * env * s) as f32
        })
        .collect()
}

#[test]
fn judge_is_zero_on_the_stem_and_sees_harmonic_balance_and_decay() {
    let notes = [NoteSpan { start: 0.0, end: 1.5, midi: 45.0 }];
    let stem = tone(110.0, 1.0, 1.2, 1.5);
    let mut judge = SpectralTarget::new(&stem, SR, &notes, SpectralWeights::default());
    let same = judge.score(&stem);
    assert!(same.total < 1e-6, "identical render: {same:?}");

    let bright = judge.score(&tone(110.0, 0.2, 1.2, 1.5));
    let long = judge.score(&tone(110.0, 1.0, 6.0, 1.5));
    assert!(bright.harm > 0.2, "flatter harmonics: harm {}", bright.harm);
    assert!(bright.centroid > 0.2, "brighter: centroid {} octaves", bright.centroid);
    assert!(long.env > 0.2, "slower decay: env {}", long.env);
    assert!(long.harm < bright.harm, "same harmonic balance: harm {} vs {}", long.harm, bright.harm);
    assert_eq!(bright.notes.len(), 1);
}
