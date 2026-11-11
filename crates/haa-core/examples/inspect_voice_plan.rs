use std::io::Read;
use chimera::anatomy::human::BodyMix;
use chimera::phonetics::english::EnglishG2P;
use chimera::phonetics::planner::{plan_utterance,PlanOptions};
fn main() {
    chimera::init();
    let mut text=String::new();
    flate2::read::GzDecoder::new(&include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/data/cmudict.txt.gz"))[..]).read_to_string(&mut text).unwrap();
    let mut g2p=EnglishG2P::new();g2p.load_dict(&text);
    let phrases=g2p.to_phrases("The little girl had been asleep, but she heard the raps and opened the door.");
    let mut t=0.0;let mut rows=Vec::new();
    for p in phrases {
        let plan=plan_utterance(&p.segments,&PlanOptions{body:BodyMix::MALE,t0:t,rate:Some(0.87),f0:Some(105.0),accent:Some(0.32),intonation:Some(p.intonation),..Default::default()});
        rows.push(serde_json::json!({"input":p.segments,"pause":p.pause,"plan":plan}));
        t=plan.end+p.pause;
    }
    println!("{}",serde_json::to_string_pretty(&rows).unwrap());
}
