//! Direct sound and room: `room::separate` keeps the source of a recording and leaves its room tail
//! to the world's reverb.

use haa_fit::room::{find, separate};

const RATE: f64 = 48_000.0;

fn noise(seed: &mut u32) -> f64 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f64 / 8_388_608.0 - 1.0
}

/// A knock (modes with T60 0.08-0.15 s) with or without a room: diffuse noise decaying with RT60 1.5 s,
/// 25 dB under the knock's peak.
fn knock(room: bool) -> Vec<f32> {
    let n = (1.6 * RATE) as usize;
    let mut seed = 5u32;
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE;
            let src = 0.6 * (-6.9 * t / 0.15).exp() * (std::f64::consts::TAU * 220.0 * t).sin()
                + 0.3 * (-6.9 * t / 0.08).exp() * (std::f64::consts::TAU * 1300.0 * t).sin();
            let tail = if room { 0.6 * 10f64.powf(-25.0 / 20.0) * (-6.9 * t / 1.5).exp() * noise(&mut seed) } else { 0.0 };
            (src + tail + 1e-5 * noise(&mut seed)) as f32
        })
        .collect()
}

#[test]
fn a_room_tail_is_found_and_cut_where_the_source_ends() {
    let (dry, room) = separate(&knock(true), RATE);
    let room = room.expect("the 1.5 s tail is a room");
    assert!(room.tonal_share < 0.25, "a diffuse tail has no lines: {room:?}");
    assert!((room.tail_rt60_s - 1.5).abs() < 0.4, "tail RT60 {room:?}");
    // The knock falls 25 dB (its 0.15 s mode: 166 dB/s) in about 0.15 s, where the tail takes over.
    assert!((0.08..0.3).contains(&room.knee_s), "knee {room:?}");
    assert!(dry.len() as f64 / RATE < room.knee_s + 0.025, "trimmed after the fade: {} s", dry.len() as f64 / RATE);
    let last = dry[dry.len() - 10..].iter().fold(0.0f32, |a, x| a.max(x.abs()));
    assert!(last < 0.01, "the cut fades to silence: {last}");
}

#[test]
fn a_dry_knock_is_left_whole() {
    assert!(find(&knock(false), RATE).is_err(), "{:?}", find(&knock(false), RATE));
    let (dry, room) = separate(&knock(false), RATE);
    assert!(room.is_err() && dry.len() == knock(false).len());
}
