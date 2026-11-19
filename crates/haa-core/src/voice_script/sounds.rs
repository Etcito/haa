//! Inline sounds of a `speak` script: `{...}` holds phones that bypass G2P, or a named vocal event.
//!
//! - Phones: tiny-voca / dizione IPA (`{h ə l ˈoʊ}`), ARPAbet (`{HH AH0 L OW1}`, dizione's inline syntax), or
//!   letters run together (`{mmm hh}`): a repeated phone is one phone held longer (`mmm` = 3 times its predicted
//!   length), `:` after a phone doubles it, `:0.8s` gives its length in seconds. Stress is 0 unless a vowel carries
//!   `ˈ` / `ˌ` (or an ARPAbet digit). ASCII `g` and `r` stand for `ɡ` and `ɹ`.
//! - Events: `{hmm}`, `{mmh}`, `{uh}`, `{um}`, `{ah}`, `{oh}`, `{sigh}`, `{breath}`, `{gasp}`, `{tsk}`, `{laugh}` (and
//!   aliases) from `vocal_events.json`: phones with seconds, forced voicing, a pitch contour, energy and breath.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use serde_json::Value;

/// dizione symbols.rs: consonants (ids 17..43) and vowels (ids 43..76), in id order.
const CONSONANTS: [&str; 26] = [
    "p", "b", "t", "d", "k", "ɡ", "f", "v", "θ", "ð", "s", "z", "ʃ", "ʒ", "h", "m", "n", "ŋ", "l", "ɹ", "j", "w", "tʃ",
    "dʒ", "ɾ", "ʔ",
];
const VOWELS: [&str; 33] = [
    "ə", "ɐ", "ɚ", "ᵻ", "ɪ", "i", "iː", "ɛ", "æ", "ʌ", "ɑː", "ɔ", "ɔː", "ʊ", "uː", "ɜː", "eɪ", "aɪ", "ɔɪ", "oʊ", "aʊ",
    "iə", "oː", "ɑːɹ", "ɔːɹ", "oːɹ", "ɪɹ", "ɛɹ", "ʊɹ", "aɪɚ", "aɪə", "əl", "n̩",
];

/// The token id of an inventory phone.
fn phone_id(symbol: &str) -> Option<u32> {
    let s = match symbol {
        "g" => "ɡ",
        "r" => "ɹ",
        other => other,
    };
    CONSONANTS.iter().position(|p| *p == s).map(|i| 17 + i as u32).or_else(|| VOWELS.iter().position(|p| *p == s).map(|i| 43 + i as u32))
}

fn is_vowel_id(id: u32) -> bool {
    (43..76).contains(&id)
}

/// ARPAbet (dizione's inline form) to the inventory; vowels by their stressed / unstressed realisation.
fn arpabet(symbol: &str) -> Option<(&'static str, u32)> {
    let (base, digit) = match symbol.as_bytes().last() {
        Some(d @ b'0'..=b'2') => (&symbol[..symbol.len() - 1], Some((d - b'0') as u32)),
        _ => (symbol, None),
    };
    let stressed = digit.is_some_and(|d| d > 0);
    let ipa = match base {
        "AA" => "ɑː",
        "AE" => "æ",
        "AH" => if stressed { "ʌ" } else { "ə" },
        "AO" => "ɔː",
        "AW" => "aʊ",
        "AY" => "aɪ",
        "EH" => "ɛ",
        "ER" => if stressed { "ɜː" } else { "ɚ" },
        "EY" => "eɪ",
        "IH" => "ɪ",
        "IY" => "i",
        "OW" => "oʊ",
        "OY" => "ɔɪ",
        "UH" => "ʊ",
        "UW" => "uː",
        "B" => "b",
        "CH" => "tʃ",
        "D" => "d",
        "DH" => "ð",
        "F" => "f",
        "G" => "ɡ",
        "HH" => "h",
        "JH" => "dʒ",
        "K" => "k",
        "L" => "l",
        "M" => "m",
        "N" => "n",
        "NG" => "ŋ",
        "P" => "p",
        "R" => "ɹ",
        "S" => "s",
        "SH" => "ʃ",
        "T" => "t",
        "TH" => "θ",
        "V" => "v",
        "W" => "w",
        "Y" => "j",
        "Z" => "z",
        "ZH" => "ʒ",
        _ => return None,
    };
    let vowel = VOWELS.contains(&ipa);
    if vowel != digit.is_some() {
        return None;
    }
    Some((ipa, digit.unwrap_or(0)))
}

/// One phone of an inline sound.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundPhone {
    pub id: u32,
    pub stress: u32,
    /// Times the model's predicted length (1 = as predicted).
    pub stretch: f64,
    /// Length in seconds, replacing the prediction.
    pub seconds: Option<f64>,
    /// Forced voicing (`None` = the model's).
    pub voiced: Option<bool>,
}

/// The prosody of a named event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    /// Pitch contour in semitones, spread evenly over the event, relative to the voice's pitch around it.
    pub f0_st: Vec<f64>,
    pub energy_db: f64,
    pub breath_db: f64,
}

/// An inline sound: its phones and, for a named event, its name and shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub phones: Vec<SoundPhone>,
    pub event: Option<String>,
    pub shape: Option<Shape>,
}

/// `vocal_events.json` v1.
#[derive(Clone, Debug, Default)]
pub struct Events {
    aliases: HashMap<String, String>,
    events: HashMap<String, Sound>,
}

impl Events {
    pub fn parse(text: &str) -> Result<Events, String> {
        let doc: Value = serde_json::from_str(text).map_err(|e| format!("vocal_events.json: {e}"))?;
        let aliases = doc
            .get("aliases")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.to_lowercase(), v.to_lowercase()))).collect())
            .unwrap_or_default();
        let mut events = HashMap::new();
        for (name, e) in doc.get("events").and_then(Value::as_object).ok_or("vocal_events.json: no events")? {
            let bad = |what: &str| format!("vocal_events.json: {name}: {what}");
            let mut warnings = Vec::new();
            let phones = parse_phones(e.get("phones").and_then(Value::as_str).ok_or_else(|| bad("no phones"))?, &mut warnings);
            if !warnings.is_empty() || phones.is_empty() {
                return Err(bad(&warnings.join("; ")));
            }
            let list = |k: &str| -> Vec<Value> { e.get(k).and_then(Value::as_array).cloned().unwrap_or_default() };
            let seconds = list("seconds");
            let voiced = list("voiced");
            if seconds.len() != phones.len() || (!voiced.is_empty() && voiced.len() != phones.len()) {
                return Err(bad("seconds (and voiced) need one value per phone"));
            }
            let phones = phones
                .into_iter()
                .enumerate()
                .map(|(i, mut p)| {
                    p.seconds = seconds[i].as_f64().filter(|s| *s > 0.0);
                    p.voiced = voiced.get(i).and_then(Value::as_f64).map(|v| v > 0.5);
                    p
                })
                .collect();
            let num = |k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let shape = Shape {
                f0_st: list("f0_st").iter().filter_map(Value::as_f64).collect(),
                energy_db: num("energy_db"),
                breath_db: num("breath_db"),
            };
            events.insert(name.to_lowercase(), Sound { phones, event: Some(name.to_lowercase()), shape: Some(shape) });
        }
        Ok(Events { aliases, events })
    }

    /// The event named `name` (case-insensitive, aliases folded).
    pub fn get(&self, name: &str) -> Option<&Sound> {
        let n = name.trim().to_lowercase();
        let n = self.aliases.get(&n).cloned().unwrap_or(n);
        self.events.get(&n)
    }

    /// The event names, sorted.
    pub fn names(&self) -> Vec<&str> {
        let mut k: Vec<&str> = self.events.keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }
}

/// The vocal events this crate ships.
pub const BUILTIN_JSON: &str = include_str!("vocal_events.json");

static INSTALLED: LazyLock<RwLock<Events>> =
    LazyLock::new(|| RwLock::new(Events::parse(BUILTIN_JSON).expect("the shipped vocal_events.json parses")));

/// Replaces the vocal events every `speak` node uses.
pub fn install(events: Events) {
    *INSTALLED.write().unwrap_or_else(|p| p.into_inner()) = events;
}

/// Runs `f` on the vocal events in use.
pub fn with_events<R>(f: impl FnOnce(&Events) -> R) -> R {
    f(&INSTALLED.read().unwrap_or_else(|p| p.into_inner()))
}

/// The phones of `text` (see the module comment); problems go to `warnings` and the phone is skipped.
pub fn parse_phones(text: &str, warnings: &mut Vec<String>) -> Vec<SoundPhone> {
    let mut out: Vec<SoundPhone> = Vec::new();
    for unit in text.split_whitespace() {
        if unit.len() > 1 && unit.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
            match arpabet(unit).and_then(|(ipa, s)| phone_id(ipa).map(|id| (id, s))) {
                Some((id, stress)) => out.push(SoundPhone { id, stress, stretch: 1.0, seconds: None, voiced: None }),
                None => warnings.push(format!("{{{text}}}: {unit} is not an ARPAbet phone")),
            }
            continue;
        }
        let mut rest = unit;
        let mut last_in_unit: Option<usize> = None;
        while !rest.is_empty() {
            let mut stress = 0;
            if let Some(r) = rest.strip_prefix('ˈ') {
                stress = 1;
                rest = r;
            } else if let Some(r) = rest.strip_prefix('ˌ') {
                stress = 2;
                rest = r;
            }
            if let Some(r) = rest.strip_prefix(':') {
                // `:` doubles the phone before; `:0.8s` / `:0.8` gives its length
                let digits = r.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(r.len());
                rest = &r[digits..];
                if let Some(r2) = rest.strip_prefix('s') {
                    rest = r2;
                }
                match last_in_unit.and_then(|i| out.get_mut(i)) {
                    Some(p) if digits > 0 => match r[..digits].parse::<f64>() {
                        Ok(s) if s > 0.0 && s <= 10.0 => p.seconds = Some(s),
                        _ => warnings.push(format!("{{{text}}}: bad length :{}", &r[..digits])),
                    },
                    Some(p) => p.stretch *= 2.0,
                    None => warnings.push(format!("{{{text}}}: ':' with no phone before it")),
                }
                continue;
            }
            // the longest inventory symbol at the start of `rest`
            let found = rest
                .char_indices()
                .map(|(i, c)| i + c.len_utf8())
                .take(4)
                .filter_map(|end| phone_id(&rest[..end]).map(|id| (end, id)))
                .last();
            let Some((end, id)) = found else {
                let c = rest.chars().next().unwrap_or(' ');
                warnings.push(format!("{{{text}}}: '{c}' is not a phone of the inventory"));
                rest = &rest[c.len_utf8()..];
                continue;
            };
            rest = &rest[end..];
            if stress > 0 && !is_vowel_id(id) {
                warnings.push(format!("{{{text}}}: a stress mark on a consonant is ignored"));
                stress = 0;
            }
            match last_in_unit.and_then(|i| out.get_mut(i)) {
                Some(p) if p.id == id && p.seconds.is_none() => p.stretch += 1.0,
                _ => {
                    out.push(SoundPhone { id, stress, stretch: 1.0, seconds: None, voiced: None });
                    last_in_unit = Some(out.len() - 1);
                }
            }
        }
    }
    out
}

/// The sound inside `{...}`: a named event, else phones. `None` (with a warning) when nothing is usable.
pub fn parse_sound(inside: &str, events: &Events, warnings: &mut Vec<String>) -> Option<Sound> {
    if let Some(e) = events.get(inside) {
        return Some(e.clone());
    }
    let mut w = Vec::new();
    let phones = parse_phones(inside, &mut w);
    if phones.is_empty() {
        warnings.push(format!("{{{inside}}} is neither a vocal event ({}) nor phones, and is ignored", events.names().join(", ")));
        return None;
    }
    warnings.extend(w);
    Some(Sound { phones, event: None, shape: None })
}
