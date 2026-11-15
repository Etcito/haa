//! Impact decomposition: the struck-object template measured from a one-shot, the scene preset made
//! from a fit, and the voice-limiter reading the fitter penalises.

use haa_fit::eval::Renderer;
use haa_fit::impact::{analyze, sfx_preset, template};

const RATE: f64 = 24_000.0;

/// A knock: two decaying modes (180 Hz T60 0.3 s, 900 Hz T60 0.1 s) after a 1 ms click.
fn knock() -> Vec<f32> {
    let n = (0.6 * RATE) as usize;
    let mut seed = 7u32;
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE;
            let m1 = 0.5 * (-6.9 * t / 0.3).exp() * (std::f64::consts::TAU * 180.0 * t).sin();
            let m2 = 0.3 * (-6.9 * t / 0.1).exp() * (std::f64::consts::TAU * 900.0 * t).sin();
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let click = if t < 0.001 { 0.4 * ((seed >> 8) as f64 / 8_388_608.0 - 1.0) } else { 0.0 };
            (m1 + m2 + click) as f32 * 0.25
        })
        .collect()
}

fn loudest(pcm: &[f32]) -> f64 {
    let n = (0.05 * RATE) as usize;
    pcm.windows(n).step_by(64).map(|w| w.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).fold(0.0, f64::max)
}

#[test]
fn analysis_finds_the_modes_of_a_knock_and_their_decay_order() {
    let a = analyze(&knock(), RATE);
    let near = |hz: f64| a.modes.iter().find(|m| (m.hz / hz).ln().abs() < 0.03).copied();
    let (low, high) = (near(180.0).expect("180 Hz mode"), near(900.0).expect("900 Hz mode"));
    assert!(low.db > high.db, "the 180 Hz mode is the stronger: {low:?} {high:?}");
    assert!(low.t60 > high.t60, "the 180 Hz mode rings longer: {low:?} {high:?}");
}

#[test]
fn template_is_a_playable_patch_that_renders_the_knock() {
    let (text, report) = template(&knock(), RATE, "knock");
    let (patch, issues) = haa_core::load_patch(&text).expect("loads");
    assert!(!haa_core::issues::has_errors(&issues), "{issues:?}");
    let mut r = Renderer::new(RATE, knock().len());
    let mono = r.render(&patch).expect("playable").to_vec();
    let db = 10.0 * (loudest(&mono) / loudest(&knock())).log10();
    assert!(db.abs() < 6.0, "the corrected template is within 6 dB of the knock: {db} ({report})");
}

#[test]
fn scene_preset_level_follows_energy_linearly() {
    let (text, _) = template(&knock(), RATE, "knock");
    let preset = sfx_preset(&text, "knock-sfx", "Knock", 0.5).expect("wraps");
    let render = |e: f64| {
        let o = haa_core::render::RenderOptions {
            sample_rate: RATE as u32,
            inputs: vec![("energy".to_string(), haa_core::render::InputValue::Number(e))],
            ..Default::default()
        };
        let r = haa_core::render::render(&preset, &o).expect("renders");
        loudest(&r.left)
    };
    let db = 10.0 * (render(0.25) / render(1.0)).log10();
    assert!((db + 12.04).abs() < 0.5, "energy 0.25 is 12 dB under energy 1: {db}");
}

#[test]
fn renderer_reports_the_voice_limiter() {
    let noise = |gain: f64| {
        format!(
            r#"{{"haa":1,"id":"n","title":"n","graph":{{"duration":0.3,"loop":false,"mode":"timeline","nodes":[
            {{"id":"n","type":"noise","x":0,"y":0,"label":"n","params":{{"color":"white","level":1}}}},
            {{"id":"g","type":"gain","x":0,"y":0,"label":"g","params":{{"gain":{gain}}}}},
            {{"id":"out","type":"output","x":0,"y":0,"label":"o","params":{{"gain":1}}}}],
            "connections":[{{"id":"c1","from":"n","to":"g","target":"audio","amount":1}},{{"id":"c2","from":"g","to":"out","target":"audio","amount":1}}]}}}}"#
        )
    };
    let mut r = Renderer::new(RATE, (0.3 * RATE) as usize);
    let (loud, _) = haa_core::load_patch(&noise(2.0)).unwrap();
    r.render(&loud).unwrap();
    assert!(r.limited_db() > 1.0, "a doubled full-scale noise is limited: {}", r.limited_db());
    let (quiet, _) = haa_core::load_patch(&noise(0.05)).unwrap();
    r.render(&quiet).unwrap();
    assert_eq!(r.limited_db(), 0.0);
}
