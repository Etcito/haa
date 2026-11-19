//! The hand-written tables of dizione (`src/dizione/data/*.txt`, Apache-2.0, copied unchanged into `data/`) and
//! their readers (dizione `tables.py`). One entry per line, columns separated by tabs or two or more spaces, `#`
//! starts a comment line. Pronunciations are inventory phones with stress marks or CMUdict notation.

use std::collections::HashMap;

use crate::convert::{from_text, is_arpabet, ft};
use crate::pyre;
use crate::symbols::{Ph, ValueError};

pub const ABBREVIATIONS: &str = include_str!("../data/abbreviations.txt");
pub const CHAT: &str = include_str!("../data/chat.txt");
pub const ENDINGS: &str = include_str!("../data/endings.txt");
pub const EXCEPTIONS: &str = include_str!("../data/exceptions.txt");
pub const FUNCTION_WORDS: &str = include_str!("../data/function_words.txt");
pub const HOMOGRAPH_RULES: &str = include_str!("../data/homograph_rules.txt");
pub const LETTERS: &str = include_str!("../data/letters.txt");
pub const PHRASES: &str = include_str!("../data/phrases.txt");
pub const UNITS: &str = include_str!("../data/units.txt");

const STRESS_CLASSES: [&str; 8] = ["u", "uf", "ur", "s", "ss", "si", "pf", "p"];

/// Rows of a table: line number and columns.
pub fn rows(text: &str) -> Vec<(usize, Vec<String>)> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = pyre::strip(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = crate::rx!(r"\t+| {2,}").split(line).into_iter().map(|c| pyre::strip(c).to_string()).collect();
        out.push((index + 1, columns));
    }
    out
}

/// A function word: stress class, form, final form.
pub type Function = (String, Vec<Ph>, Option<Vec<Ph>>);

pub fn function_words(text: &str) -> HashMap<String, Function> {
    let mut table = HashMap::new();
    for (number, c) in rows(text) {
        assert!(
            (c.len() == 3 || c.len() == 4) && STRESS_CLASSES.contains(&c[1].as_str()),
            "function_words.txt line {number}: expected word, class, form[, final form]"
        );
        table.insert(c[0].clone(), (c[1].clone(), ft(&c[2]), (c.len() == 4).then(|| ft(&c[3]))));
    }
    table
}

/// A phrase: stress class, one form (joined) or one per word, final form.
pub type Phrase = (String, Vec<Vec<Ph>>, Option<Vec<Ph>>);

pub fn phrases(text: &str) -> HashMap<Vec<String>, Phrase> {
    let mut table = HashMap::new();
    for (number, c) in rows(text) {
        assert!(
            (c.len() == 3 || c.len() == 4) && STRESS_CLASSES.contains(&c[1].as_str()),
            "phrases.txt line {number}: expected words, class, form[, final form]"
        );
        let forms: Vec<Vec<Ph>> = c[2].split('|').map(ft).collect();
        let key: Vec<String> = pyre::words(&c[0]).into_iter().map(String::from).collect();
        assert!(
            forms.len() == 1 || (c[1] == "p" && forms.len() == key.len()),
            "phrases.txt line {number}: a form with boundaries needs class p and one part per word"
        );
        table.insert(key, (c[1].clone(), forms, (c.len() == 4).then(|| ft(&c[3]))));
    }
    table
}

/// A pronunciation as written in a table: CMUdict symbols, or phones.
#[derive(Clone, Debug)]
pub enum Pron {
    Arpabet(Vec<String>),
    Phones(Vec<Ph>),
}

fn pronunciation(text: &str) -> Result<Pron, ValueError> {
    let symbols: Vec<String> = pyre::words(text).into_iter().map(String::from).collect();
    Ok(if is_arpabet(&symbols) { Pron::Arpabet(symbols) } else { Pron::Phones(from_text(text)?) })
}

/// word -> pronunciation, for the exception list and user overrides (rows of one word only).
pub fn pronunciations(text: &str) -> Result<HashMap<String, Pron>, ValueError> {
    let mut table = HashMap::new();
    for (where_, c) in rows(text) {
        if c.len() != 2 {
            return Err(ValueError(format!("{where_}: expected a word and its pronunciation, separated by a tab or two spaces")));
        }
        if pyre::words(&c[0]).len() == 1 {
            table.insert(c[0].to_lowercase(), pronunciation(&c[1])?);
        }
    }
    Ok(table)
}

/// (word, word, ...) -> one pronunciation per word, for names of several words.
pub fn names(text: &str) -> Result<HashMap<Vec<String>, Vec<Pron>>, ValueError> {
    let mut table = HashMap::new();
    for (where_, c) in rows(text) {
        let lower = c[0].to_lowercase();
        let words: Vec<String> = pyre::words(&lower).into_iter().map(String::from).collect();
        if c.len() != 2 || words.len() < 2 {
            continue;
        }
        let parts: Vec<&str> = c[1].split('|').map(pyre::strip).collect();
        if parts.len() != words.len() {
            return Err(ValueError(format!(
                "{where_}: {:?} has {} words and {} pronunciations; write one per word, separated by |",
                c[0],
                words.len(),
                parts.len()
            )));
        }
        table.insert(words, parts.into_iter().map(pronunciation).collect::<Result<_, _>>()?);
    }
    Ok(table)
}

/// Two-column table as a map (the second column may be empty).
pub fn mapping(text: &str) -> HashMap<String, String> {
    rows(text).into_iter().map(|(_, c)| (c[0].clone(), c.get(1).cloned().unwrap_or_default())).collect()
}
