//! Words spelled alike and pronounced differently (dizione `homographs.py`): hand rules from
//! `homograph_rules.txt`, then a small linear classifier over the neighbouring words (`homographs.json`).

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::pyre;
use crate::tables;

const CLASSES: [(&str, &str); 13] = [
    ("TO", "to"),
    ("DET", "the a an this that these those every each no some any another such what which whose"),
    ("POSS", "my your his her its our their"),
    ("SUBJ", "i you he she it we they who"),
    ("OBJ", "me him us them"),
    ("MODAL", "will would shall should can could may might must 'll 'd won't can't cannot couldn't shouldn't wouldn't let's please"),
    ("DO", "do does did don't doesn't didn't"),
    ("BE", "am is are was were be been being isn't aren't wasn't weren't i'm you're we're they're he's she's it's that's there's"),
    ("HAVE", "have has had having haven't hasn't hadn't i've you've we've they've"),
    ("NEG", "not never"),
    (
        "PREP",
        "of in on at by for from with into onto over under about after before between through during without within against among around across behind beside near upon toward towards as than like",
    ),
    ("CONJ", "and or but nor so yet if because although though while when where unless until since whether"),
    ("ADV", "very too quite rather more most less least also just only even still always often sometimes usually again now then here there soon already almost"),
];
const SENTENCE_END: [&str; 3] = [".", "?", "!"];
const WINDOW: usize = 6;

fn tags() -> &'static HashMap<&'static str, &'static str> {
    static T: std::sync::LazyLock<HashMap<&'static str, &'static str>> = std::sync::LazyLock::new(|| {
        let mut m = HashMap::new();
        for (name, words) in CLASSES {
            for w in words.split(' ') {
                m.insert(w, name);
            }
        }
        m
    });
    &T
}

fn first_char(token: &str) -> char {
    token.chars().next().unwrap_or(' ')
}

/// Coarse grammatical class of a neighbouring token.
pub fn tag(token: &str) -> String {
    if token == "<s>" || token == "</s>" {
        return token.into();
    }
    let c = first_char(token);
    if !pyre::c_isalpha(c) {
        return if pyre::c_isdigit(c) { "NUM" } else { "PUNCT" }.into();
    }
    let lower = token.to_lowercase();
    if let Some(name) = tags().get(lower.as_str()) {
        return name.to_string();
    }
    let n = lower.chars().count();
    for (ending, name) in [("'s", "POS"), ("ly", "LY"), ("ing", "ING"), ("ed", "ED")] {
        if lower.ends_with(ending) && n > ending.len() + 1 {
            return name.into();
        }
    }
    if lower.ends_with('s') && !lower.ends_with("ss") && n > 3 {
        return "S".into();
    }
    if pyre::c_isupper(c) { "CAP" } else { "W" }.into()
}

/// Names of the features that are on for `tokens[index]` (one sentence).
pub fn features(tokens: &[String], index: usize) -> Vec<String> {
    let at = |offset: isize| -> String {
        let position = index as isize + offset;
        if position >= 0 && (position as usize) < tokens.len() {
            tokens[position as usize].clone()
        } else if offset < 0 {
            "<s>".into()
        } else {
            "</s>".into()
        }
    };
    let (p2, p1, n1, n2) = (at(-2), at(-1), at(1), at(2));
    let (tp2, tp1, tn1, tn2) = (tag(&p2), tag(&p1), tag(&n1), tag(&n2));
    let mut out = vec![
        format!("tp1={tp1}"),
        format!("tp2={tp2}"),
        format!("tn1={tn1}"),
        format!("tn2={tn2}"),
        format!("tp2p1={tp2}+{tp1}"),
        format!("tp1n1={tp1}+{tn1}"),
        format!("tn1n2={tn1}+{tn2}"),
    ];
    if index == 0 {
        out.push("first".into());
    } else if pyre::c_isupper(first_char(&tokens[index])) {
        out.push("cap".into());
    }
    out.push(format!("p1={}", p1.to_lowercase()));
    out.push(format!("p2={}", p2.to_lowercase()));
    out.push(format!("n1={}", n1.to_lowercase()));
    out.push(format!("n2={}", n2.to_lowercase()));
    let target = tokens[index].to_lowercase();
    let lo = index.saturating_sub(WINDOW);
    let hi = (index + WINDOW + 1).min(tokens.len());
    // Python sorts by code point; a BTreeSet of Strings orders by bytes, the same order for UTF-8.
    let nearby: BTreeSet<String> = tokens[lo..hi]
        .iter()
        .filter(|t| pyre::c_isalpha(first_char(t)))
        .map(|t| t.to_lowercase())
        .filter(|t| *t != target)
        .collect();
    out.extend(nearby.into_iter().map(|w| format!("w={w}")));
    out
}

fn shared(names: &[String]) -> Vec<&String> {
    names.iter().filter(|n| n.starts_with('t') || *n == "first" || *n == "cap").collect()
}

/// The sentence around `tokens[index]` and the index inside it.
fn sentence_of(tokens: &[String], index: usize) -> (&[String], usize) {
    let mut start = index;
    while start > 0 && !SENTENCE_END.contains(&tokens[start - 1].as_str()) {
        start -= 1;
    }
    let mut end = index;
    while end < tokens.len() && !SENTENCE_END.contains(&tokens[end].as_str()) {
        end += 1;
    }
    (&tokens[start..(end + 1).min(tokens.len())], index - start)
}

const SKIP_FOR_BEFORE: [&str; 3] = ["ADV", "NEG", "LY"];
const SKIP_FOR_AUX: [&str; 7] = ["ADV", "NEG", "LY", "SUBJ", "OBJ", "DET", "POSS"];

fn left(tokens: &[String], index: usize, skip: &[&str]) -> String {
    let stop = index as isize - 6;
    let mut position = index as isize - 1;
    while position > stop.max(-1) {
        let token = &tokens[position as usize];
        position -= 1;
        if skip.contains(&tag(token).as_str()) {
            continue;
        }
        let lower = token.to_lowercase();
        return if lower.ends_with("'ll") {
            "will".into()
        } else if lower.ends_with("'ve") {
            "have".into()
        } else {
            lower
        };
    }
    "<s>".into()
}

type Rules = HashMap<String, Vec<(String, Vec<(String, HashSet<String>)>)>>;

fn read_rules(text: &str) -> Result<Rules, String> {
    let mut rules: Rules = HashMap::new();
    for (number, columns) in tables::rows(text) {
        if columns.len() != 3 {
            return Err(format!("homograph_rules.txt line {number}: expected word, sense, conditions"));
        }
        let mut conditions = Vec::new();
        for part in columns[2].split(';') {
            let part = pyre::strip(part);
            let (name, words) = part.split_once(':').unwrap_or((part, ""));
            let ok = ["aux", "before", "next", "near", "first", "joined"].contains(&name);
            if !ok || (name == "first") == !pyre::strip(words).is_empty() {
                return Err(format!("homograph_rules.txt line {number}: bad condition {part:?}"));
            }
            conditions.push((name.to_string(), pyre::words(words).into_iter().map(String::from).collect()));
        }
        rules.entry(columns[0].clone()).or_default().push((columns[1].clone(), conditions));
    }
    Ok(rules)
}

fn holds(condition: &str, words: &HashSet<String>, tokens: &[String], index: usize, joined: bool) -> bool {
    match condition {
        "first" => index == 0,
        "joined" => joined && index > 0 && words.contains(&tokens[index - 1].to_lowercase()),
        "aux" => words.contains(&left(tokens, index, &SKIP_FOR_AUX)),
        "before" => words.contains(&left(tokens, index, &SKIP_FOR_BEFORE)),
        "next" => words.contains(&if index + 1 < tokens.len() { tokens[index + 1].to_lowercase() } else { "</s>".into() }),
        _ => {
            let lo = index.saturating_sub(WINDOW);
            let hi = (index + WINDOW + 1).min(tokens.len());
            tokens[lo..hi].iter().any(|t| words.contains(&t.to_lowercase()))
        }
    }
}

/// Python's `sum()` of floats (3.12 and later): Neumaier's compensated summation, as CPython's `builtin_sum`.
fn py_sum(items: impl Iterator<Item = f64>) -> f64 {
    let (mut f, mut c) = (0.0f64, 0.0f64);
    for x in items {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

struct Sense {
    id: String,
    role: String,
    phones: String,
}

enum Model {
    NounVerb { bias: f64, weights: HashMap<String, f64> },
    Own { bias: Vec<f64>, weights: Vec<HashMap<String, f64>> },
}

struct Entry {
    senses: Vec<Sense>,
    model: Model,
}

pub struct Homographs {
    words: HashMap<String, Entry>,
    pooled_bias: f64,
    pooled: HashMap<String, f64>,
    plural_bases: HashSet<String>,
    rules: Rules,
}

fn weights(v: &Value) -> HashMap<String, f64> {
    v.as_object().map(|o| o.iter().map(|(k, x)| (k.clone(), x.as_f64().unwrap_or(0.0))).collect()).unwrap_or_default()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Homographs {
    pub fn from_json(text: &str, rules_table: &str) -> Result<Homographs, String> {
        let data: Value = serde_json::from_str(text).map_err(|e| format!("homographs.json: {e}"))?;
        let mut words = HashMap::new();
        for (word, entry) in data["words"].as_object().ok_or("homographs.json: no words")? {
            let senses = entry["senses"]
                .as_array()
                .ok_or("homographs.json: no senses")?
                .iter()
                .map(|x| Sense { id: s(x, "id"), role: s(x, "role"), phones: s(x, "phones") })
                .collect();
            let m = &entry["model"];
            let model = if m["kind"].as_str() == Some("noun-verb") {
                Model::NounVerb { bias: m["bias"].as_f64().unwrap_or(0.0), weights: weights(&m["weights"]) }
            } else {
                Model::Own {
                    bias: m["bias"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default(),
                    weights: m["weights"].as_array().map(|a| a.iter().map(weights).collect()).unwrap_or_default(),
                }
            };
            words.insert(word.clone(), Entry { senses, model });
        }
        let plural_bases =
            data["plural_bases"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let rules = read_rules(rules_table)?;
        for (word, word_rules) in &rules {
            for (sense, _) in word_rules {
                let known = words.get(word).is_some_and(|e: &Entry| e.senses.iter().any(|s| &s.id == sense));
                if !known {
                    return Err(format!("homograph rule for {word:?} names the unknown sense {sense:?}"));
                }
            }
        }
        Ok(Homographs {
            words,
            pooled_bias: data["pooled"]["bias"].as_f64().unwrap_or(0.0),
            pooled: weights(&data["pooled"]["weights"]),
            plural_bases,
            rules,
        })
    }

    /// True for a homograph and for the regular -s form of a noun-or-verb one.
    pub fn covers(&self, word: &str) -> bool {
        self.words.contains_key(word) || (word.ends_with('s') && self.plural_bases.contains(&word[..word.len() - 1]))
    }

    /// `(phones as text, needs the -s ending)` for `tokens[index]`.
    pub fn pronounce(&self, word: &str, tokens: &[String], index: usize, joined: bool) -> (String, bool) {
        if self.words.contains_key(word) {
            return (self.choose(word, tokens, index, joined).phones.clone(), false);
        }
        let base = &word[..word.len() - 1];
        let entry = &self.words[base];
        let adjective = entry.senses.iter().find(|s| s.role == "n");
        if adjective.is_some_and(|a| a.id == "adj") {
            return (entry.senses.iter().find(|s| s.role == "v").unwrap().phones.clone(), true);
        }
        (self.choose(base, tokens, index, joined).phones.clone(), true)
    }

    fn scores(&self, word: &str, tokens: &[String], index: usize) -> Vec<f64> {
        let entry = &self.words[word];
        let (sentence, position) = sentence_of(tokens, index);
        let names = features(sentence, position);
        match &entry.model {
            Model::NounVerb { bias, weights } => {
                let pooled = shared(&names).into_iter().map(|n| self.pooled.get(n).copied().unwrap_or(0.0));
                let mut verb = self.pooled_bias + py_sum(pooled);
                verb += bias + py_sum(names.iter().map(|n| weights.get(n).copied().unwrap_or(0.0)));
                entry.senses.iter().map(|s| if s.role == "v" { verb } else { -verb }).collect()
            }
            Model::Own { bias, weights } => bias
                .iter()
                .zip(weights)
                .map(|(b, w)| b + py_sum(names.iter().map(|n| w.get(n).copied().unwrap_or(0.0))))
                .collect(),
        }
    }

    fn choose(&self, word: &str, tokens: &[String], index: usize, joined: bool) -> &Sense {
        let entry = &self.words[word];
        let (sentence, position) = sentence_of(tokens, index);
        if let Some(rules) = self.rules.get(word) {
            for (sense, conditions) in rules {
                if conditions.iter().all(|(name, words)| holds(name, words, sentence, position, joined)) {
                    return entry.senses.iter().find(|s| &s.id == sense).unwrap();
                }
            }
        }
        let scores = self.scores(word, tokens, index);
        let mut best = 0;
        for k in 1..scores.len() {
            if scores[k] > scores[best] {
                best = k;
            }
        }
        &entry.senses[best]
    }
}
