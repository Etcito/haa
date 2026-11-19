//! Sentence rules (dizione `sentence.py`): what a word's pronunciation owes to its neighbours in one clause.
//! Sentence stress of function words, "the" and "to" before a vowel, linking r, the flap of "it" and "at".

use crate::symbols::{is_vowel, Ph};

const SPLIT: [(&str, &str); 6] = [("ɑːɹ", "ɑː"), ("ɔːɹ", "ɔː"), ("oːɹ", "oː"), ("ɪɹ", "ɪ"), ("ɛɹ", "ɛ"), ("ʊɹ", "ʊ")];
const RHOTIC: [&str; 3] = ["ɚ", "ɜː", "aɪɚ"];
const NO_LINK_BEFORE: [&str; 3] = ["and", "or", "if"];
const FLAP_BEFORE_VOWEL: [&str; 2] = ["it", "at"];

/// One unit of a clause: a word, or several words said as one (a phrase).
#[derive(Clone, Debug)]
pub struct Unit {
    /// Spelling in lower case; several words for a phrase.
    pub word: String,
    pub phones: Vec<Ph>,
    /// Stress class of `function_words.txt`; "c" = stress as given.
    pub kind: String,
    /// Form to use when the word is promoted.
    pub final_: Option<Vec<Ph>>,
    /// No word boundary before this unit (hyphenated compound).
    pub attach: bool,
    /// Where the pronunciation came from.
    pub source: String,
    /// The word as written in the text (not in dizione: for locating words).
    pub raw: String,
}

impl Unit {
    pub fn new(word: &str, phones: Vec<Ph>, kind: &str, final_: Option<Vec<Ph>>, attach: bool, source: &str, raw: &str) -> Unit {
        Unit { word: word.into(), phones, kind: kind.into(), final_, attach, source: source.into(), raw: raw.into() }
    }
    fn last_word(&self) -> &str {
        self.word.rsplit(' ').next().unwrap()
    }
    fn first_word(&self) -> &str {
        self.word.split(' ').next().unwrap()
    }
}

pub fn unstress(phones: &[Ph]) -> Vec<Ph> {
    phones.iter().map(|&(p, _)| (p, 0)).collect()
}

/// Gives the word a primary stress: on its marked vowel, else on the first vowel.
pub fn promote(phones: &[Ph]) -> Vec<Ph> {
    if phones.iter().any(|&(_, s)| s == 1) {
        return phones.to_vec();
    }
    let target = if let Some(k) = phones.iter().rposition(|&(_, s)| s == 2) {
        Some(k)
    } else {
        phones.iter().position(|&(p, _)| is_vowel(p))
    };
    phones.iter().enumerate().map(|(i, &(p, s))| (p, if Some(i) == target { 1 } else { s })).collect()
}

/// Resolves the stress class of every unit of a clause, right to left.
pub fn stress_clause(units: &mut [Unit]) {
    let mut primary_follows = false;
    let n = units.len();
    for index in (0..n).rev() {
        let unit = &units[index];
        let promoted = || unit.final_.clone().unwrap_or_else(|| promote(&unit.phones));
        let phones = match unit.kind.as_str() {
            "u" => unstress(&unit.phones),
            "uf" => {
                if index == n - 1 {
                    promoted()
                } else {
                    unstress(&unit.phones)
                }
            }
            "ur" => {
                if primary_follows {
                    unstress(&unit.phones)
                } else {
                    promoted()
                }
            }
            "s" => {
                if primary_follows {
                    unit.phones.clone()
                } else {
                    promoted()
                }
            }
            "si" => {
                if !primary_follows {
                    promoted()
                } else if index == 0 {
                    unit.phones.clone()
                } else {
                    unstress(&unit.phones)
                }
            }
            "pf" => {
                if index == n - 1 {
                    promoted()
                } else {
                    unit.phones.clone()
                }
            }
            _ => unit.phones.clone(),
        };
        units[index].phones = phones;
        primary_follows = primary_follows || units[index].phones.iter().any(|&(_, s)| s == 1);
    }
    if !units.is_empty() && !units.iter().any(|u| u.phones.iter().any(|&(_, s)| s != 0)) {
        for unit in units.iter_mut().rev() {
            if unit.phones.iter().any(|&(p, _)| is_vowel(p)) {
                unit.phones = promote(&unit.phones);
                break;
            }
        }
    }
}

/// Adjusts the end of each word to the beginning of the next.
pub fn join_clause(units: &mut [Unit]) {
    for k in 0..units.len().saturating_sub(1) {
        let (head, tail) = units.split_at_mut(k + 1);
        let unit = &mut head[k];
        let following = &tail[0];
        if unit.phones.is_empty() || following.phones.is_empty() || !is_vowel(following.phones[0].0) {
            continue;
        }
        let (last, stress) = *unit.phones.last().unwrap();
        let n = unit.phones.len();
        if unit.last_word() == "the" && last == "ə" {
            unit.phones[n - 1] = ("ɪ", stress);
        } else if unit.last_word() == "to" && last == "ə" {
            unit.phones[n - 1] = ("ʊ", stress);
        } else if NO_LINK_BEFORE.contains(&following.first_word()) {
            continue;
        } else if let Some((_, split)) = SPLIT.iter().find(|(a, _)| *a == last) {
            unit.phones[n - 1] = (split, stress);
            unit.phones.push(("ɹ", 0));
        } else if RHOTIC.contains(&last) {
            unit.phones.push(("ɹ", 0));
        } else if last == "t" && FLAP_BEFORE_VOWEL.contains(&unit.word.as_str()) && n > 1 && is_vowel(unit.phones[n - 2].0) {
            unit.phones[n - 1] = ("ɾ", 0);
        }
    }
}

/// Units of a clause as spoken words: an attached unit continues the one before. Each word comes with the indices
/// of the units it is made of.
pub fn groups(units: &[Unit]) -> Vec<(Vec<Ph>, Vec<usize>)> {
    let mut out: Vec<(Vec<Ph>, Vec<usize>)> = Vec::new();
    for (i, unit) in units.iter().enumerate() {
        if unit.attach && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.0.extend_from_slice(&unit.phones);
            last.1.push(i);
        } else {
            out.push((unit.phones.clone(), vec![i]));
        }
    }
    out.into_iter().filter(|(g, _)| !g.is_empty()).collect()
}
