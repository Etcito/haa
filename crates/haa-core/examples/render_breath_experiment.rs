use std::io::{Read,Write};
use chimera::anatomy::human::BodyMix;
use chimera::phonetics::english::EnglishG2P;
use chimera::phonetics::planner::{plan_utterance,PlanOptions,vowel_artic};
use chimera::phonetics::Intonation;
use chimera::control::score::{Gesture,Targets,ControlKey};
use chimera::voice::human_voice::SourcePatch;
use chimera::engine::Engine;
fn main() {
    chimera::init();
    let args:Vec<String>=std::env::args().collect();
    let score:serde_json::Value=serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let mut dictionary=String::new();
    flate2::read::GzDecoder::new(&include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/data/cmudict.txt.gz"))[..]).read_to_string(&mut dictionary).unwrap();
    let mut g2p=EnglishG2P::new();g2p.load_dict(&dictionary);
    let mut gestures=Vec::new();let mut segment_report=Vec::new();
    for phrase in score["phrases"].as_array().unwrap() {
        let mut segments=Vec::new();
        for word in phrase["words"].as_array().unwrap() {
            let spelling=word["word"].as_str().unwrap();
            let mut phones=g2p.to_phrases(spelling).remove(0).segments;
            let measured=word["phones"].as_array().unwrap();
            assert_eq!(phones.len(),measured.len(),"phone count for {spelling}");
            for (p,m) in phones.iter_mut().zip(measured) {
                let duration=m["end"].as_f64().unwrap()-m["start"].as_f64().unwrap();
                assert!((0.009..=2.0).contains(&duration));
                p.dur=Some(duration);
            }
            if let Some(p)=phones.last_mut(){p.word_end=Some(true);}
            segments.extend(phones);
        }
        let start=phrase["start"].as_f64().unwrap();
        let mut plan=plan_utterance(&segments,&PlanOptions{body:BodyMix::MALE,t0:start-0.06,rate:Some(1.0),f0:Some(105.0),intonation:Some(Intonation::Flat),accent:Some(0.0),..Default::default()});
        plan.gestures.retain(|g| !g.targets.has(ControlKey::F0));
        // Candidate coarticulation: overlap vowel postures, leaving consonant constraints intact.
        let link=score["vowel_overlap_seconds"].as_f64().unwrap_or(0.0);
        for g in &mut plan.gestures {
            if vowel_artic(g.label_str(),BodyMix::MALE).is_some() {
                g.ramp_in+=link;g.ramp_out+=link;
            }
        }
        segment_report.extend(plan.segments);gestures.extend(plan.gestures);
    }
    let pitch=score["pitch"].as_array().unwrap();
    for pair in pitch.windows(2) {
        let t0=pair[0][0].as_f64().unwrap();let t1=pair[1][0].as_f64().unwrap();
        let hz=pair[0][1].as_f64().unwrap();
        assert!(hz.is_finite() && (60.0..=350.0).contains(&hz));
        gestures.push(Gesture::new(t0,t1,0.015,0.015,8.0,Targets::default().with(ControlKey::F0,hz)).label("measured pitch"));
    }
    if let Some(groups) = score["breath_experiment"].as_array().filter(|v| !v.is_empty()) {
        gestures.retain(|g| !g.targets.has(ControlKey::BREATH));
        for group in groups {
            let points = group.as_array().unwrap();
            for pair in points.windows(2) {
                let t0 = pair[0][0].as_f64().unwrap();
                let t1 = pair[1][0].as_f64().unwrap();
                let value = pair[0][1].as_f64().unwrap();
                assert!((0.75..=1.25).contains(&value));
                gestures.push(Gesture::new(t0,t1,0.04,0.05,1.0,
                    Targets::default().with(ControlKey::BREATH,value)).label("breath dynamics trial"));
            }
        }
    }
    assert!(gestures.len()<4000);
    let mut engine=Engine::new(48000.0);engine.set_body(BodyMix::MALE);
    engine.set_source(&SourcePatch{f0:Some(105.0),pressure:Some(8.0),breathiness:Some(0.08),tension:Some(0.5),..Default::default()});
    engine.set_gate(0.0);engine.schedule(&gestures,0.0,true);
    let mut audio=vec![0.0f32;48000*6];engine.render(&mut audio);
    let coefficient=(-std::f64::consts::TAU*20.0/48000.0).exp();let(mut px,mut py,mut peak)=(0.0,0.0,0.0f64);
    let mut pcm=Vec::with_capacity(audio.len()*2);
    for v in audio {
        let x=v as f64*0.81;let y=x-px+coefficient*py;px=x;py=y;peak=peak.max(y.abs());
        assert!(y.is_finite() && y.abs()<1.0,"invalid sample {y}");pcm.extend_from_slice(&((y*32767.0).round() as i16).to_le_bytes());
    }
    let mut f=std::fs::File::create(&args[2]).unwrap();let size=pcm.len() as u32;
    f.write_all(b"RIFF").unwrap();f.write_all(&(36+size).to_le_bytes()).unwrap();f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();f.write_all(&1u16.to_le_bytes()).unwrap();f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&48000u32.to_le_bytes()).unwrap();f.write_all(&96000u32.to_le_bytes()).unwrap();f.write_all(&2u16.to_le_bytes()).unwrap();f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();f.write_all(&size.to_le_bytes()).unwrap();f.write_all(&pcm).unwrap();
    std::fs::write(format!("{}.segments.json",args[2]),serde_json::to_string_pretty(&segment_report).unwrap()).unwrap();
    println!("rendered {} gestures, peak {peak:.4}",gestures.len());
}
