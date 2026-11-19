//! Parity of the Rust port against PyTorch stage dumps (made offline by a reference script; never committed).
//! `cargo run --release -p tinyvoca --example parity -- <model.gguf> <dump dir>`
//! A dump `<stem>` has `<stem>.json` (ids, stress, durations, voice) and f32 files `log_f0 voiced energy h n noise
//! wave`; dumps of voices that are not stock speakers of the GGUF are skipped. Each stage is fed the reference's inputs, so one
//! stage's error does not hide behind another's; `end-to-end` runs free with the reference noise.
//! With `PARITY_DUMP=<dir>` the free-running audio of each stem is written there as `<stem>.wave.f32`.

use std::fs;
use std::time::Instant;
use tinyvoca::{Cepstra, Model, Options};

fn floats(path: &str) -> Vec<f32> {
    fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}")).chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn snr(reference: &[f32], ours: &[f32]) -> f64 {
    let s: f64 = reference.iter().map(|&x| (x as f64).powi(2)).sum();
    let e: f64 = reference.iter().zip(ours).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum();
    10.0 * (s / e.max(1e-30)).log10()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (gguf, dir) = (&args[1], &args[2]);
    let started = Instant::now();
    let model = Model::from_gguf(&fs::read(gguf).unwrap()).unwrap();
    println!("loaded {gguf} in {:.0} ms", started.elapsed().as_secs_f64() * 1e3);
    let mut stems: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".json").filter(|s| !s.ends_with(".tvv")).map(str::to_string))
        .collect();
    stems.sort();
    let (mut audio_s, mut work_s) = (0.0, 0.0);
    let mut worst_snr = f64::INFINITY;
    println!("stem      | dur diff (tokens, max) | log_f0   voiced   energy   | harm cep  noise cep | head SNR | end-to-end SNR | x RT");
    for stem in stems {
        let p = format!("{dir}/{stem}");
        let meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(format!("{p}.json")).unwrap()).unwrap();
        let ids: Vec<u32> = meta["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        let stress: Vec<u32> = meta["stress"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        let ref_dur: Vec<u32> = meta["durations"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        let name = meta["voice"].as_str().unwrap();
        // stock speakers only: a z voice needs a tvms GGUF (voice_in, frame_voice_in, voice_film)
        let Ok(voice) = model.base_voice(name) else { continue };
        let (log_f0, voiced, energy) = (floats(&format!("{p}.log_f0.f32")), floats(&format!("{p}.voiced.f32")), floats(&format!("{p}.energy.f32")));
        let reference = Cepstra { harmonic: floats(&format!("{p}.h.f32")), noise: floats(&format!("{p}.n.f32")) };
        let noise = floats(&format!("{p}.noise.f32"));
        let wave = floats(&format!("{p}.wave.f32"));

        let enc = model.encode(&ids, &stress, &voice).unwrap();
        let other = enc.durations.iter().zip(&ref_dur).filter(|(a, b)| a != b).count();
        let dmax = enc.durations.iter().zip(&ref_dur).map(|(&a, &b)| (a as i64 - b as i64).abs()).max().unwrap_or(0);
        let frames = model.expand(&enc.hidden, &ref_dur, &voice).unwrap();
        let pros = model.prosody(&frames, &ids);
        let cep = model.decode(&pros.hidden, &log_f0, &voiced, &energy).unwrap();
        let head = model.vocode(&reference, &log_f0, None, Some(&noise), 0).unwrap();

        let t0 = Instant::now();
        let free = model.synthesize(&ids, &stress, &voice, &Options::default(), if other == 0 { Some(&noise) } else { None }).unwrap();
        let took = t0.elapsed().as_secs_f64();
        let seconds = free.audio.len() as f64 / 24_000.0;
        audio_s += seconds;
        work_s += took;
        if let Ok(out) = std::env::var("PARITY_DUMP") {
            let bytes: Vec<u8> = free.audio.iter().flat_map(|v| v.to_le_bytes()).collect();
            fs::write(format!("{out}/{stem}.wave.f32"), bytes).unwrap();
            // the envelope edit of a voice post (PARITY_POST = a voice.tvv.json v3) on the reference cepstra
            if let Ok(post) = std::env::var("PARITY_POST") {
                let post = tinyvoca::VoiceFile::parse(&post).unwrap().post;
                let mut edited = reference.clone();
                model.edit_envelope(&mut edited, &log_f0, &post);
                let bytes = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
                fs::write(format!("{out}/{stem}.edited.h.f32"), bytes(&edited.harmonic)).unwrap();
                fs::write(format!("{out}/{stem}.edited.n.f32"), bytes(&edited.noise)).unwrap();
            }
        }
        let e2e = if free.audio.len() == wave.len() { snr(&wave, &free.audio) } else { f64::NAN };
        if e2e.is_finite() {
            worst_snr = worst_snr.min(e2e);
        } else {
            worst_snr = f64::NAN;
        }
        println!(
            "{stem:9} | {other:3} of {:3}, max {dmax}  | {:.2e} {:.2e} {:.2e} | {:.2e}  {:.2e}  | {:6.1} dB | {:8.1} dB    | {:.1}",
            ids.len(),
            max_diff(&pros.log_f0, &log_f0),
            max_diff(&pros.voiced, &voiced),
            max_diff(&pros.energy, &energy),
            max_diff(&cep.harmonic, &reference.harmonic),
            max_diff(&cep.noise, &reference.noise),
            snr(&wave, &head),
            e2e,
            seconds / took
        );
    }
    println!("worst end-to-end SNR {worst_snr:.1} dB; speed {audio_s:.1} s of audio in {work_s:.2} s = {:.1} x real time", audio_s / work_s);
}
