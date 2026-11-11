//! Optional measured delivery for the physical voice. No recorded audio is replayed.
//! Planning/validation happens outside the audio loop; the result is a bounded gesture score.
use chimera::anatomy::human::BodyMix;
use chimera::control::score::{ControlKey, Gesture, Targets};
use chimera::engine::MAX_SCHEDULE;
use chimera::phonetics::english::{EnglishG2P, arpabet_to_segments};
use chimera::phonetics::planner::{plan_utterance, vowel_artic, PlanOptions};
use chimera::phonetics::{Intonation, Plan};
use serde::Deserialize;

#[derive(Deserialize)]
struct Performance {
    phrases: Vec<Phrase>,
    pitch: Vec<[f64; 2]>,
    #[serde(default)]
    vowel_overlap_seconds: f64,
}
#[derive(Deserialize)]
struct Phrase { start: f64, words: Vec<Word> }
#[derive(Deserialize)]
struct Word { word: String, phones: Vec<Phone> }
#[derive(Deserialize)]
struct Phone { start: f64, end: f64, #[serde(default)] phone: Option<String> }

/// Parse an explicit phonetic timing/pitch plan. Invalid input is rejected, never truncated.
pub fn plan_score(json: &str, g2p: &EnglishG2P, body: BodyMix) -> Result<Plan, String> {
    if json.len() > 262_144 { return Err("performance exceeds 256 KiB".into()); }
    let score: Performance = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if score.phrases.is_empty() || score.phrases.len() > 64 { return Err("expected 1..64 phrases".into()); }
    if !score.vowel_overlap_seconds.is_finite() || !(0.0..=0.08).contains(&score.vowel_overlap_seconds) { return Err("invalid vowel overlap".into()); }
    if !(2..=2048).contains(&score.pitch.len()) { return Err("expected 2..2048 pitch points".into()); }
    let mut previous = -1.0;
    for &[time, hz] in &score.pitch {
        if !time.is_finite() || !(0.0..=120.0).contains(&time) || time <= previous || !hz.is_finite() || !(60.0..=1100.0).contains(&hz) { return Err("invalid pitch trajectory".into()); }
        previous = time;
    }
    let mut result = Plan::default();
    let mut total_phones = 0;
    let mut previous_end = 0.0;
    for phrase in score.phrases {
        if !phrase.start.is_finite() || !(0.06..=120.0).contains(&phrase.start) || phrase.start < previous_end || phrase.words.is_empty() || phrase.words.len() > 128 { return Err("invalid phrase timing or length".into()); }
        let mut segments = Vec::new();
        let mut expected = phrase.start;
        for word in phrase.words {
            if word.word.is_empty() || word.word.len() > 64 || word.word.chars().any(char::is_whitespace) { return Err("expected a single word".into()); }
            let mut planned = g2p.to_phrases(&word.word);
            if planned.len() != 1 { return Err("invalid word pronunciation".into()); }
            let mut phones = planned.remove(0).segments;
            if phones.len() != word.phones.len() || phones.is_empty() { return Err(format!("phone count mismatch for {}", word.word)); }
            total_phones += phones.len();
            if total_phones > 1024 { return Err("too many phones".into()); }
            for (phone, timing) in phones.iter_mut().zip(word.phones) {
                let duration = timing.end - timing.start;
                if !timing.start.is_finite() || !timing.end.is_finite() || !(0.009..=2.0).contains(&duration) || (timing.start - expected).abs() > 0.001 || timing.end > 120.0 { return Err("non-contiguous or invalid phone timing".into()); }
                // Explicit reference phones are authoritative. Previously only their times
                // were read, so function-word reduction could silently replace a reference vowel.
                // Keep contextual tap/dark-L realizations when they denote the same consonant.
                if let Some(label) = timing.phone {
                    if label.is_empty() || label.len() > 4 || !label.bytes().all(|c| c.is_ascii_uppercase() || matches!(c, b'0'..=b'2')) {
                        return Err("invalid ARPAbet phone".into());
                    }
                    let has_stress = label.as_bytes().last().is_some_and(u8::is_ascii_digit);
                    let tagged = if has_stress { label } else { format!("{}{}", label, phone.stress.unwrap_or(0)) };
                    let explicit = arpabet_to_segments(&tagged);
                    if explicit.len() != 1 { return Err("unknown ARPAbet phone".into()); }
                    let wanted = &explicit[0];
                    let same_allophone = (phone.ipa == "ɾ" && matches!(wanted.ipa.as_str(), "t" | "d"))
                        || (phone.ipa == "ɫ" && wanted.ipa == "l");
                    if !same_allophone { phone.ipa = wanted.ipa.clone(); }
                    phone.stress = wanted.stress;
                }
                phone.dur = Some(duration);
                expected = timing.end;
            }
            if let Some(phone) = phones.last_mut() { phone.word_end = Some(true); }
            segments.extend(phones);
        }
        let mut plan = plan_utterance(&segments, &PlanOptions {
            body, t0: phrase.start - 0.06, rate: Some(1.0), f0: Some(score.pitch[0][1]),
            intonation: Some(Intonation::Flat), accent: Some(0.0), ..Default::default()
        });
        plan.gestures.retain(|g| !g.targets.has(ControlKey::F0));
        for gesture in &mut plan.gestures {
            if vowel_artic(gesture.label_str(), body).is_some() {
                gesture.ramp_in += score.vowel_overlap_seconds;
                gesture.ramp_out += score.vowel_overlap_seconds;
            }
        }
        previous_end = expected;
        result.end = result.end.max(plan.end);
        result.segments.extend(plan.segments);
        result.gestures.extend(plan.gestures);
    }
    for pair in score.pitch.windows(2) {
        result.gestures.push(Gesture::new(pair[0][0], pair[1][0], 0.015, 0.015, 8.0,
            Targets::default().with(ControlKey::F0, pair[0][1])).label("measured pitch"));
    }
    if result.gestures.len() > MAX_SCHEDULE { return Err("performance exceeds gesture capacity".into()); }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (EnglishG2P, serde_json::Value, usize) {
        chimera::init();
        let g2p = EnglishG2P::new();
        let count = g2p.to_phrases("hello")[0].segments.len();
        let phones: Vec<_> = (0..count).map(|i| serde_json::json!({"start":0.3+i as f64*0.1,"end":0.4+i as f64*0.1})).collect();
        let score = serde_json::json!({"phrases":[{"start":0.3,"words":[{"word":"hello","phones":phones}]}],"pitch":[[0.2,140],[2.0,180]]});
        (g2p, score, count)
    }
    #[test]
    fn measured_timing_is_preserved_and_score_is_bounded() {
        let (g2p, score, count) = fixture();
        let plan = plan_score(&score.to_string(), &g2p, BodyMix::MALE).unwrap();
        assert!((plan.segments[0].t0 - 0.3).abs() < 1e-9);
        assert!((plan.end - (0.3 + count as f64 * 0.1)).abs() < 1e-9);
        assert!(plan.gestures.len() <= MAX_SCHEDULE);
    }
    #[test]
    fn explicit_reference_phone_overrides_lexical_vowel_and_rejects_unknown() {
        let (g2p, mut score, _) = fixture();
        // An explicit identity must affect the planned articulation, not only duration.
        score["phrases"][0]["words"][0]["phones"][0]["phone"] = serde_json::json!("AE1");
        let plan = plan_score(&score.to_string(), &g2p, BodyMix::MALE).unwrap();
        assert_eq!(plan.segments[0].ipa, "æ");
        assert_eq!(plan.segments[0].stress, Some(1));
        score["phrases"][0]["words"][0]["phones"][0]["phone"] = serde_json::json!("BAD");
        assert!(plan_score(&score.to_string(), &g2p, BodyMix::MALE).is_err());
    }
    #[test]
    fn malformed_timings_and_pitch_are_rejected() {
        let (g2p, mut score, _) = fixture();
        score["pitch"][1][0] = serde_json::json!(0.1);
        assert!(plan_score(&score.to_string(), &g2p, BodyMix::MALE).is_err());
        let (_, mut score, _) = fixture();
        score["phrases"][0]["words"][0]["phones"][0]["end"] = serde_json::json!(0.2);
        assert!(plan_score(&score.to_string(), &g2p, BodyMix::MALE).is_err());
        assert!(plan_score("{}", &g2p, BodyMix::MALE).is_err());
    }
}
