//! A text of any length cut into utterances tiny-voca takes: a port of tiny-voca `js/text.js` (`utterancesOf`).
//!
//! The text gets a final full stop when it ends on a bare word, is cut into sentences after `.`, `!`, `?` or `…`
//! (optionally closed by a quote or bracket) before a capital or a digit, never after a known abbreviation
//! ("Dr.", "e.g.", a single initial). A sentence of more than `limit` tokens is cut after `,`, `;`, `:` or `—`,
//! else in two halves by words, until every piece fits. tiny-voca slows down past about 20 s of speech, and
//! upstream synthesizes one utterance per call, which is what this cut is for.

use std::sync::LazyLock;

use regex::Regex;

use crate::{Phonemized, Phonemizer, ValueError};

static ABBREVIATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(?:mr|mrs|ms|dr|prof|sr|jr|st|vs|etc|no|inc|ltd|co|e\.g|i\.e|a\.m|p\.m)|\b[a-z])\.$").expect("abbreviation pattern")
});

const STOPS: [char; 4] = ['.', '!', '?', '…'];

fn has_letter_or_digit(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

/// text.js `finished`: a text whose last word is a bare word gets a final full stop.
pub fn finished(text: &str) -> String {
    let body = text.trim_end();
    if body.is_empty() {
        return text.to_string();
    }
    let last = body.split_whitespace().next_back().unwrap_or("");
    let marked = if has_letter_or_digit(last) {
        // /[.!?…,;:]["'”’)\]»]*$/
        let core = last.trim_end_matches(['"', '\'', '”', '’', ')', ']', '»']);
        core.ends_with(['.', '!', '?', '…', ',', ';', ':'])
    } else {
        last.contains(STOPS) || matches!(last, "," | ";" | ":")
    };
    if marked { text.to_string() } else { format!("{body}.") }
}

/// text.js `sentences`: spaces collapsed, cut after a sentence stop before a capital or a digit, abbreviations
/// kept with what follows; pieces with no letter or digit are dropped.
pub fn sentences(text: &str) -> Vec<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    // a cut at a space: /(?<=[.!?…]["')\]]?)\s+(?=["'(\[]?[A-Z0-9])/
    let cut = |i: usize| -> bool {
        let before = |k: usize| i.checked_sub(k).map(|j| chars[j]);
        let stop_before = match before(1) {
            Some(c) if STOPS.contains(&c) => true,
            Some('"' | '\'' | ')' | ']') => before(2).is_some_and(|c| STOPS.contains(&c)),
            _ => false,
        };
        let capital = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        let after = chars.get(i + 1);
        let starts = capital(after) || (matches!(after, Some('"' | '\'' | '(' | '[')) && capital(chars.get(i + 2)));
        stop_before && starts
    };
    let mut pieces = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        if chars[i] == ' ' && cut(i) {
            pieces.push(chars[start..i].iter().collect::<String>());
            start = i + 1;
        }
    }
    pieces.push(chars[start..].iter().collect::<String>());

    let mut parts = Vec::new();
    let mut current = String::new();
    for piece in pieces {
        current = if current.is_empty() { piece } else { format!("{current} {piece}") };
        if !ABBREVIATION.is_match(&current) {
            parts.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts.into_iter().filter(|p| has_letter_or_digit(p)).collect()
}

impl Phonemizer {
    /// text.js `utterancesOf`: the text as utterances of at most `limit` tokens each (tiny-voca's page uses
    /// `max_tokens - 12`), in order. A sentence with nothing to pronounce gives none.
    pub fn utterances(&self, text: &str, limit: usize) -> Result<Vec<Phonemized>, ValueError> {
        let mut out = Vec::new();
        for sentence in sentences(&finished(text)) {
            self.fit(&sentence, limit, &mut out)?;
        }
        Ok(out)
    }

    fn fit(&self, sentence: &str, limit: usize, out: &mut Vec<Phonemized>) -> Result<(), ValueError> {
        let result = match self.phonemize(sentence) {
            Ok(r) => r,
            Err(e) if e.0 == "the text has nothing to pronounce" || e.0 == "text must not be empty" => return Ok(()),
            Err(e) => return Err(e),
        };
        if result.ids.len() <= limit {
            out.push(result);
            return Ok(());
        }
        // /(?<=[,;:—])\s+/
        let mut pieces = Vec::new();
        let mut piece = String::new();
        for word in sentence.split(' ') {
            if !piece.is_empty() {
                if piece.ends_with([',', ';', ':', '—']) {
                    pieces.push(std::mem::take(&mut piece));
                } else {
                    piece.push(' ');
                }
            }
            piece.push_str(word);
        }
        pieces.push(piece);
        if pieces.len() > 1 {
            for p in &pieces {
                self.fit(p, limit, out)?;
            }
            return Ok(());
        }
        let words: Vec<&str> = sentence.split(' ').collect();
        if words.len() < 2 {
            return Ok(());
        }
        let half = words.len().div_ceil(2);
        self.fit(&words[..half].join(" "), limit, out)?;
        self.fit(&words[half..].join(" "), limit, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_and_final_stop_follow_text_js() {
        assert_eq!(finished("Hello there"), "Hello there.");
        assert_eq!(finished("Is it \"done?\""), "Is it \"done?\"");
        assert_eq!(finished("Wait ,"), "Wait ,");
        assert_eq!(
            sentences("Dr. Smith  arrived at 9 a.m. Then he left!   \"Why?\" she asked. 3 cats... and e.g. dogs. ok."),
            ["Dr. Smith arrived at 9 a.m. Then he left!", "\"Why?\" she asked.", "3 cats... and e.g. dogs. ok."]
        );
        assert_eq!(sentences("... !"), Vec::<String>::new());
    }
}
