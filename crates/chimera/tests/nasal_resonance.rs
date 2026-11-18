//! The resonances of the nasal branch: a lowered velum puts the nose's first resonance under 300 Hz, below a male F1.
//! Measured on the airway's impulse response (folds still and closed, lungs off, a puff of flow into the pharynx,
//! the flow radiated at the lips and nostrils).

use chimera::anatomy::human::{ArticKey, BodyMix};
use chimera::engine::Engine;
use chimera::phonetics::inventory::phoneme;
use chimera::phonetics::planner::vowel_artic;
use chimera::voice::human_voice::SourcePatch;

const SR: f64 = 44_100.0;

/// Resonance peaks (Hz) between 150 and 900 Hz of the airway posed as `ipa` (a vowel, or a consonant on a schwa) with
/// the velum at `velum` cm².
fn low_peaks(ipa: &str, velum: f64) -> Vec<f64> {
    let body = BodyMix::MALE;
    let respond = |kick: bool| -> Vec<f64> {
        let mut e = Engine::new(SR);
        let mut artic = vowel_artic(ipa, body).unwrap_or_else(|| {
            let mut a = vowel_artic("ə", body).unwrap();
            a.merge(&phoneme(ipa, body).unwrap().targets);
            a
        });
        artic.set(ArticKey::Velum, velum);
        e.set_artic(&artic, Some(true));
        e.set_source(&SourcePatch { pressure: Some(0.0), breathiness: Some(0.0), voiced: Some(true), ..SourcePatch::default() });
        e.voice_mut().glottis.params.amplitude = 0.0;
        e.voice_mut().glottis.params.chink = 0.0;
        e.set_gate(0.0);
        let mut buf = [0f32; 1];
        for _ in 0..(0.4 * SR) as usize {
            e.render(&mut buf);
        }
        let node = e.voice().airway.pharynx.first_node();
        (0..(0.2 * SR) as usize)
            .map(|i| {
                if i == 0 && kick {
                    e.voice_mut().airway.net.ext_flow[node] += 1.0e4;
                }
                e.render(&mut buf);
                let a = &e.voice().airway;
                a.net.radiated_flow(a.rad_lips) + a.net.radiated_flow(a.rad_nose)
            })
            .collect()
    };
    let (on, off) = (respond(true), respond(false));
    let x: Vec<f64> = on.iter().zip(&off).map(|(a, b)| a - b).collect();
    let freqs: Vec<f64> = (0..=375).map(|i| 150.0 + 2.0 * i as f64).collect();
    let db: Vec<f64> = freqs
        .iter()
        .map(|f| {
            let w = core::f64::consts::TAU * f / SR;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in x.iter().enumerate() {
                re += v * (w * i as f64).cos();
                im -= v * (w * i as f64).sin();
            }
            10.0 * (re * re + im * im).max(1e-30).log10()
        })
        .collect();
    (1..db.len() - 1).filter(|&i| db[i] > db[i - 1] && db[i] >= db[i + 1]).map(|i| freqs[i]).collect()
}

#[test]
fn the_m_murmur_resonates_first_under_300_hz() {
    let peaks = low_peaks("m", 0.45);
    assert!(peaks.first().is_some_and(|f| (220.0..300.0).contains(f)), "/m/ murmur peaks {peaks:?}");
}

#[test]
fn a_nasalised_schwa_gets_its_nasal_pole_under_350_hz_and_its_oral_f1_stays() {
    let oral = low_peaks("ə", 0.0);
    assert!(oral.first().is_some_and(|f| (480.0..530.0).contains(f)), "oral /ə/ peaks {oral:?}");
    let nasal = low_peaks("ə", 0.3);
    assert!(nasal.first().is_some_and(|f| (250.0..350.0).contains(f)), "nasalised /ə/ peaks {nasal:?}");
    assert!(!nasal.iter().any(|f| (440.0..540.0).contains(f)), "nasalised /ə/ keeps a pole inside a male F1's band: {nasal:?}");
}
