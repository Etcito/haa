//! The phonemizer (dizione `engine.py`): text -> spoken words of phones, clause by clause.
//!
//! Order of authority for one word: an inline pronunciation, the user's overrides, the built-in exceptions (and
//! names of several words), function words and phrases, the dictionary, regular endings and compounds of dictionary
//! words, "-leigh" names and "-virus" words from their parts, then the unknown-word model.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::align::Aligner;
use crate::convert::{convert, from_text, settle_stress};
use crate::data::Data;
use crate::g2p::Model;
use crate::homographs::Homographs;
use crate::morphology::{self, Lookup};
use crate::normalize::{self, Normalizer, Token};
use crate::rx;
use crate::sentence::{self, Unit};
use crate::symbols::{tokenize, Ph, Phonemized, Piece, ValueError};
use crate::tables::{self, Function, Phrase, Pron};

pub struct Phonemizer {
    lexicon: HashMap<String, String>,
    aligner: Aligner,
    model: Option<Model>,
    homographs: Option<Homographs>,
    functions: HashMap<String, Function>,
    phrases: HashMap<Vec<String>, Phrase>,
    phrase_length: usize,
    exceptions: HashMap<String, Pron>,
    names: HashMap<Vec<String>, Vec<Pron>>,
    name_starts: HashSet<String>,
    name_length: usize,
    user_names: HashSet<Vec<String>>,
    overrides: HashMap<String, Pron>,
    norm: normalize::Tables,
    letter_names: HashMap<String, Vec<Ph>>,
    cache: RefCell<HashMap<String, (Vec<Ph>, &'static str)>>,
}

/// What `analyse` gives: punctuation marks and clauses of units, in order.
#[derive(Clone, Debug)]
pub enum Part {
    Punct(String),
    Clause(Vec<Unit>),
}

impl Phonemizer {
    /// The phonemizer over loaded data. Tables present in `data.tables` replace the built-in copies.
    pub fn new(data: Data) -> Result<Phonemizer, String> {
        let table = |name: &str, default: &'static str| -> String { data.tables.get(name).cloned().unwrap_or_else(|| default.to_string()) };
        let exceptions_text = table("exceptions.txt", tables::EXCEPTIONS);
        let functions = tables::function_words(&table("function_words.txt", tables::FUNCTION_WORDS));
        let phrases = tables::phrases(&table("phrases.txt", tables::PHRASES));
        let phrase_length = phrases.keys().map(Vec::len).max().unwrap_or(1).max(1);
        let exceptions = tables::pronunciations(&exceptions_text).map_err(|e| e.0)?;
        let names = tables::names(&exceptions_text).map_err(|e| e.0)?;
        let model = match &data.g2p_json {
            Some(settings) if !data.g2p_tensors.is_empty() => Some(Model::new(settings, data.g2p_tensors)?),
            _ => None,
        };
        let homographs = match &data.homographs_json {
            Some(text) => Some(Homographs::from_json(text, &table("homograph_rules.txt", tables::HOMOGRAPH_RULES))?),
            None => None,
        };
        let norm = normalize::Tables::new(
            &table("abbreviations.txt", tables::ABBREVIATIONS),
            &table("units.txt", tables::UNITS),
            &table("chat.txt", tables::CHAT),
            &table("endings.txt", tables::ENDINGS),
        )?;
        let letter_names = tables::mapping(&table("letters.txt", tables::LETTERS))
            .into_iter()
            .map(|(k, v)| from_text(&v).map(|p| (k, p)))
            .collect::<Result<_, _>>()
            .map_err(|e| e.0)?;
        let mut p = Phonemizer {
            lexicon: data.lexicon,
            aligner: Aligner::from_json(&data.align_json)?,
            model,
            homographs,
            functions,
            phrases,
            phrase_length,
            exceptions,
            names,
            name_starts: HashSet::new(),
            name_length: 1,
            user_names: HashSet::new(),
            overrides: HashMap::new(),
            norm,
            letter_names,
            cache: RefCell::new(HashMap::new()),
        };
        p.count_names();
        Ok(p)
    }

    fn count_names(&mut self) {
        self.name_starts = self.names.keys().map(|k| k[0].clone()).collect();
        self.name_length = self.names.keys().map(Vec::len).max().unwrap_or(1).max(1);
    }

    /// Pronunciations that win over everything else: a table text, one word and its pronunciation (CMUdict notation
    /// or phones with stress marks) per line; a name of several words takes one pronunciation per word, split by `|`.
    pub fn add_overrides(&mut self, text: &str) -> Result<(), ValueError> {
        self.overrides.extend(tables::pronunciations(text)?);
        let taught = tables::names(text)?;
        self.user_names.extend(taught.keys().cloned());
        self.names.extend(taught);
        self.count_names();
        self.cache.borrow_mut().clear();
        Ok(())
    }

    fn entry(&self, word: &str, entry: &Pron) -> Vec<Ph> {
        match entry {
            Pron::Phones(p) => p.clone(),
            Pron::Arpabet(symbols) => convert(word, symbols, self.aligner.align(word, symbols)).unwrap_or_default(),
        }
    }

    /// Pronunciation from the overrides, the exceptions or the dictionary.
    pub fn known(&self, word: &str) -> Option<Vec<Ph>> {
        if let Some(e) = self.overrides.get(word) {
            return Some(self.entry(word, e));
        }
        if let Some(e) = self.exceptions.get(word) {
            return Some(self.entry(word, e));
        }
        self.lexicon.get(word).map(|t| from_text(t).unwrap_or_default())
    }

    fn ordinary(&self, word: &str) -> Option<Vec<Ph>> {
        if word.chars().count() < 3 || !["s", "es", "ed", "ing"].iter().any(|e| self.lexicon.contains_key(&format!("{word}{e}"))) {
            return None;
        }
        self.known(word)
    }

    /// The unknown-word model, then the conversion rules, on one spelling.
    fn guess(&self, word: &str) -> Vec<Ph> {
        let Some(model) = &self.model else { return self.spell(word) };
        let spelling = model.spellable(word);
        let mut symbols = model.predict(&spelling);
        let voiced = |s: &str| s.ends_with(['0', '1', '2']);
        for k in 0..symbols.len() {
            if symbols[k] == "R" {
                let near = [k.wrapping_sub(1), k + 1];
                if !near.iter().any(|&j| j < symbols.len() && voiced(&symbols[j])) {
                    symbols[k] = "ER0".into();
                }
            }
        }
        if !symbols.iter().any(|s| voiced(s)) {
            return self.spell(word);
        }
        let runs = self.aligner.align(&spelling, &symbols);
        match convert(&spelling, &symbols, runs) {
            Ok(phones) => sentence::promote(&phones),
            Err(_) => self.spell(word),
        }
    }

    /// Letter names, joined: the last letter carries the main stress.
    pub fn spell(&self, letters: &str) -> Vec<Ph> {
        let lower = letters.to_lowercase();
        let possessive = lower.ends_with("'s");
        let body: String = if possessive { letters.chars().take(letters.chars().count() - 2).collect() } else { letters.to_string() };
        let names: Vec<&Vec<Ph>> = body.to_lowercase().chars().filter_map(|c| self.letter_names.get(&c.to_string())).collect();
        let mut out = Vec::new();
        for (index, name) in names.iter().enumerate() {
            for &(phone, stress) in name.iter() {
                out.push((phone, if stress != 0 && index < names.len() - 1 { 2 } else { stress }));
            }
        }
        if possessive && !out.is_empty() { morphology::plural(&out) } else { out }
    }

    /// True for a spelling the tables or the dictionary know.
    pub fn is_word(&self, word: &str) -> bool {
        self.lexicon.contains_key(word) || self.functions.contains_key(word) || self.exceptions.contains_key(word) || self.overrides.contains_key(word)
    }

    fn from_parts(&self, word: &str) -> Option<(Vec<Ph>, &'static str)> {
        let n = word.chars().count();
        if word.ends_with("'s") && n > 3 {
            let (phones, source) = self.from_parts(&word[..word.len() - 2])?;
            return Some((plural_or_same(&phones), source));
        }
        let leigh = rx!(r"(?<=[a-z]{2})leigh(?=s?$)");
        if leigh.is_match(word) {
            return Some(self.word(&leigh.sub(word, |_| "ley".into())));
        }
        for tail in ["viruses", "virus"] {
            if word.ends_with(tail) && n - tail.len() > 2 {
                if let Some(second) = self.known(tail) {
                    let (first, source) = self.word(&word[..word.len() - tail.len()]);
                    let phones = sentence::promote(&settle_stress(&morphology::compound(&first, &second)));
                    return Some((phones, if source == "model" { "model" } else { "ending" }));
                }
            }
        }
        None
    }

    /// `(phones, source)` for one spelling in lower case; source is dictionary, ending, model.
    pub fn word(&self, word: &str) -> (Vec<Ph>, &'static str) {
        if let Some(hit) = self.cache.borrow().get(word) {
            return hit.clone();
        }
        let mut found = self.known(word).map(|p| (p, "dictionary"));
        if found.is_none() {
            found = morphology::derive(word, self, 0).map(|p| (settle_stress(&p), "ending"));
        }
        if found.is_none() {
            found = self.from_parts(word);
        }
        if found.is_none() && word.ends_with("'s") && word.chars().count() > 3 {
            found = Some((plural_or_same(&self.guess(&word[..word.len() - 2])), "model"));
        }
        let result = found.unwrap_or_else(|| (self.guess(word), "model"));
        self.cache.borrow_mut().insert(word.to_string(), result.clone());
        result
    }

    fn units(&self, words: &[&normalize::Word], places: &[usize], written: &[String]) -> Vec<Unit> {
        let mut units = Vec::new();
        let mut index = 0;
        while index < words.len() {
            let token = words[index];
            if let Some(phones) = &token.phones {
                units.push(Unit::new(&token.text, phones.clone(), "p", None, token.attach, "inline", &token.raw));
            } else if token.spell {
                units.push(Unit::new(&token.text, self.spell(&token.text), "p", None, token.attach, "letters", &token.raw));
            } else {
                let size = if self.name_starts.contains(&token.text) { self.name_size(words, index) } else { 0 };
                if size > 0 {
                    let key: Vec<String> = words[index..index + size].iter().map(|w| w.text.clone()).collect();
                    for (position, (text, entry)) in key.iter().zip(&self.names[&key]).enumerate() {
                        let w = words[index + position];
                        units.push(Unit::new(text, self.entry(text, entry), "p", None, token.attach && position == 0, "name", &w.raw));
                    }
                    index += size;
                    continue;
                }
                let size = self.phrase_size(words, index);
                if size > 0 {
                    let key: Vec<String> = words[index..index + size].iter().map(|w| w.text.clone()).collect();
                    let (kind, forms, final_) = &self.phrases[&key];
                    if forms.len() == 1 {
                        let raw = words[index..index + size].iter().map(|w| w.raw.as_str()).collect::<Vec<_>>().join(" ");
                        units.push(Unit::new(&key.join(" "), forms[0].clone(), kind, final_.clone(), token.attach, "phrase", &raw));
                    } else {
                        for (position, (text, form)) in key.iter().zip(forms).enumerate() {
                            let w = words[index + position];
                            units.push(Unit::new(text, form.clone(), kind, None, token.attach && position == 0, "phrase", &w.raw));
                        }
                    }
                    index += size;
                    continue;
                }
                let homographs = self.homographs.as_ref().filter(|h| !written.is_empty() && h.covers(&token.text));
                if self.overrides.contains_key(&token.text) {
                    let phones = self.known(&token.text).unwrap_or_default();
                    units.push(Unit::new(&token.text, phones, "c", None, token.attach, "override", &token.raw));
                } else if let Some(h) = homographs.filter(|_| !self.exceptions.contains_key(&token.text)) {
                    let (text, ending) = h.pronounce(&token.text, written, places[index], token.attach);
                    let phones = from_text(&text).unwrap_or_default();
                    let phones = if ending { plural_or_same(&phones) } else { phones };
                    units.push(Unit::new(&token.text, phones, "c", None, token.attach, "homograph", &token.raw));
                } else if let Some((kind, form, final_)) = self.functions.get(&token.text) {
                    units.push(Unit::new(&token.text, form.clone(), kind, final_.clone(), token.attach, "function", &token.raw));
                } else {
                    let (phones, source) = self.word(&token.text);
                    units.push(Unit::new(&token.text, phones, "c", None, token.attach, source, &token.raw));
                }
            }
            index += 1;
        }
        units
    }

    fn name_size(&self, words: &[&normalize::Word], index: usize) -> usize {
        for size in (2..=self.name_length.min(words.len() - index)).rev() {
            let span = &words[index..index + size];
            let key: Vec<String> = span.iter().map(|w| w.text.clone()).collect();
            if self.names.contains_key(&key)
                && span.iter().all(|w| w.phones.is_none() && !w.spell)
                && !span[1..].iter().any(|w| w.attach)
                && (self.user_names.contains(&key) || !span.iter().any(|w| self.overrides.contains_key(&w.text)))
            {
                return size;
            }
        }
        0
    }

    fn phrase_size(&self, words: &[&normalize::Word], index: usize) -> usize {
        for size in (2..=self.phrase_length.min(words.len() - index)).rev() {
            let span = &words[index..index + size];
            if span[1..].iter().all(|w| w.phones.is_none() && !w.spell && !w.attach)
                && span[0].phones.is_none()
                && !span[0].spell
                && self.phrases.contains_key(&span.iter().map(|w| w.text.clone()).collect::<Vec<_>>())
                && !span.iter().any(|w| self.overrides.contains_key(&w.text))
            {
                return size;
            }
        }
        0
    }

    /// The words and marks of the normalised text.
    pub fn tokens(&self, text: &str) -> Vec<Token> {
        let is_word = |w: &str| self.is_word(w);
        Normalizer::new(&self.norm, &is_word).tokens(text)
    }

    /// Clauses and marks in order.
    pub fn analyse(&self, text: &str) -> Vec<Part> {
        let stream = self.tokens(text);
        let written: Vec<String> = stream
            .iter()
            .map(|t| match t {
                Token::Punct(m) => m.clone(),
                Token::Word(w) => {
                    if !w.raw.is_empty() {
                        w.raw.clone()
                    } else if !w.text.is_empty() {
                        w.text.clone()
                    } else {
                        "x".into()
                    }
                }
            })
            .collect();
        let mut out = Vec::new();
        let mut words: Vec<&normalize::Word> = Vec::new();
        let mut places = Vec::new();
        let flush = |words: &mut Vec<&normalize::Word>, places: &mut Vec<usize>, out: &mut Vec<Part>| {
            if words.is_empty() {
                return;
            }
            let mut units = self.units(words, places, &written);
            sentence::stress_clause(&mut units);
            sentence::join_clause(&mut units);
            out.push(Part::Clause(units));
            words.clear();
            places.clear();
        };
        for (place, token) in stream.iter().enumerate() {
            match token {
                Token::Punct(m) => {
                    flush(&mut words, &mut places, &mut out);
                    out.push(Part::Punct(m.clone()));
                }
                Token::Word(w) => {
                    words.push(w);
                    places.push(place);
                }
            }
        }
        flush(&mut words, &mut places, &mut out);
        out
    }

    /// What the tokenizer reads.
    pub fn pieces(&self, text: &str) -> Vec<Piece> {
        self.analyse(text)
            .into_iter()
            .map(|part| match part {
                Part::Punct(m) => Piece::Punct(m),
                Part::Clause(units) => Piece::Words(sentence::groups(&units).into_iter().map(|(g, _)| g).collect()),
            })
            .collect()
    }

    /// Text -> tiny-voca's token ids and stress (dizione's `phonemize`).
    pub fn phonemize(&self, text: &str) -> Result<Phonemized, ValueError> {
        if crate::pyre::strip(text).is_empty() {
            return Err(ValueError("text must not be empty".into()));
        }
        tokenize(&self.pieces(text))
    }
}

fn plural_or_same(phones: &[Ph]) -> Vec<Ph> {
    if phones.is_empty() { Vec::new() } else { morphology::plural(phones) }
}

impl Lookup for Phonemizer {
    fn lookup(&self, spelling: &str) -> Option<Vec<Ph>> {
        self.known(spelling)
    }
    fn part(&self, spelling: &str) -> Option<Vec<Ph>> {
        self.ordinary(spelling)
    }
}
