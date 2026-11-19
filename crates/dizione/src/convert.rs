//! One word: CMUdict pronunciation -> phones of the inventory (dizione `convert.py`).
//!
//! Flaps, a glottal stop before syllabic n, r-coloured vowels as single phones, syllabic l and several reduced
//! vowels, by rule, on the pronunciation aligned to the spelling. The result is the citation form of the word.

use crate::symbols::{is_phone, is_vowel, phone, Ph, ValueError};

const ARPA_VOWELS: [&str; 15] = ["AA", "AE", "AH", "AO", "AW", "AY", "EH", "ER", "EY", "IH", "IY", "OW", "OY", "UH", "UW"];
const CONSONANTS: [(&str, &str); 24] = [
    ("B", "b"), ("CH", "tʃ"), ("D", "d"), ("DH", "ð"), ("F", "f"), ("G", "ɡ"), ("HH", "h"), ("JH", "dʒ"), ("K", "k"),
    ("L", "l"), ("M", "m"), ("N", "n"), ("NG", "ŋ"), ("P", "p"), ("R", "ɹ"), ("S", "s"), ("SH", "ʃ"), ("T", "t"),
    ("TH", "θ"), ("V", "v"), ("W", "w"), ("Y", "j"), ("Z", "z"), ("ZH", "ʒ"),
];
const FULL_VOWELS: [(&str, &str); 15] = [
    ("AA", "ɑː"), ("AE", "æ"), ("AH", "ʌ"), ("AO", "ɔː"), ("AW", "aʊ"), ("AY", "aɪ"), ("EH", "ɛ"), ("ER", "ɜː"),
    ("EY", "eɪ"), ("IH", "ɪ"), ("IY", "iː"), ("OW", "oʊ"), ("OY", "ɔɪ"), ("UH", "ʊ"), ("UW", "uː"),
];
const SIBILANTS: [&str; 6] = ["S", "Z", "SH", "ZH", "CH", "JH"];
const FORCE_SPELLINGS: [&str; 10] = ["forc", "ford", "sword", "worn", "torn", "sworn", "borne", "shorn", "pork", "divorc"];
const PREFIXES: [&str; 8] = ["dis", "mis", "un", "in", "re", "non", "over", "pre"];
const WEAK_PREFIXES: [&str; 4] = ["be", "de", "re", "pre"];
const IRE_ENDINGS: [&str; 8] = ["ing", "ings", "ingly", "able", "y", "er", "ers", "est"];

fn consonant(name: &str) -> Option<&'static str> {
    CONSONANTS.iter().find(|(a, _)| *a == name).map(|(_, p)| *p)
}
fn full_vowel(name: &str) -> &'static str {
    FULL_VOWELS.iter().find(|(a, _)| *a == name).map(|(_, p)| *p).unwrap()
}
fn arpa_vowel(name: &str) -> bool {
    ARPA_VOWELS.contains(&name)
}
fn vowel_letter(c: char) -> bool {
    "aeiouy".contains(c)
}

/// ARPABET symbol without its stress digit.
pub fn base(symbol: &str) -> &str {
    symbol.trim_end_matches(['0', '1', '2'])
}

/// True when every symbol is ARPABET (consonants, vowels with a stress digit).
pub fn is_arpabet<S: AsRef<str>>(symbols: &[S]) -> bool {
    !symbols.is_empty()
        && symbols.iter().all(|s| {
            let s = s.as_ref();
            consonant(s).is_some() || (s.is_ascii() && s.len() == 3 && arpa_vowel(&s[..s.len() - 1]) && "012".contains(&s[s.len() - 1..]))
        })
}

fn vowel_letters(run: &str) -> String {
    run.chars().filter(|&c| vowel_letter(c)).collect()
}

/// `"h ə l ˈoʊ"` -> phones; an error for a symbol outside the inventory.
pub fn from_text(text: &str) -> Result<Vec<Ph>, ValueError> {
    let mut phones = Vec::new();
    for unit in crate::pyre::words(text) {
        let first = unit.chars().next().unwrap();
        let stress = if first == 'ˈ' { 1 } else if first == 'ˌ' { 2 } else { 0 };
        let name = if stress > 0 { &unit[first.len_utf8()..] } else { unit };
        let Some(p) = phone(name) else {
            return Err(ValueError(format!("'{name}' is not a phone of the inventory")));
        };
        if stress > 0 && !is_vowel(p) {
            return Err(ValueError(format!("stress mark on the consonant '{name}'")));
        }
        phones.push((p, stress));
    }
    Ok(phones)
}

/// `from_text` for tables known to be valid.
pub fn ft(text: &str) -> Vec<Ph> {
    from_text(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

/// Phones as text, stress marks on the vowels.
pub fn to_text(phones: &[Ph]) -> String {
    phones.iter().map(|(p, s)| format!("{}{p}", ["", "ˈ", "ˌ"][*s as usize])).collect::<Vec<_>>().join(" ")
}

struct Word<'a> {
    word: &'a str,
    bases: Vec<String>,
    stress: Vec<Option<u8>>,
    aligned: bool,
    runs: Vec<String>,
    starts: Vec<usize>,
    count: usize,
}

impl<'a> Word<'a> {
    fn new(word: &'a str, arpabet: &[String], runs: Option<Vec<String>>) -> Result<Word<'a>, ValueError> {
        let bases: Vec<String> = arpabet.iter().map(|s| base(s).to_string()).collect();
        let stress = arpabet
            .iter()
            .map(|s| s.chars().last().and_then(|c| c.to_digit(10)).filter(|d| *d <= 2).map(|d| d as u8))
            .collect();
        for (symbol, name) in arpabet.iter().zip(&bases) {
            if consonant(name).is_none() && !arpa_vowel(name) {
                return Err(ValueError(format!("'{symbol}' is not an ARPABET symbol")));
            }
        }
        let aligned = runs.is_some();
        let runs = runs.unwrap_or_else(|| vec![String::new(); arpabet.len()]);
        let mut starts = Vec::new();
        let mut position = 0;
        for run in &runs {
            starts.push(position);
            position += run.len();
        }
        let count = bases.len();
        Ok(Word { word, bases, stress, aligned, runs, starts, count })
    }
    fn base(&self, i: isize) -> Option<&str> {
        if i >= 0 && (i as usize) < self.count { Some(&self.bases[i as usize]) } else { None }
    }
    fn vowel(&self, i: usize) -> bool {
        i < self.count && arpa_vowel(&self.bases[i])
    }
    fn unstressed(&self, i: usize) -> bool {
        self.vowel(i) && !matches!(self.stress[i], Some(s) if s != 0)
    }
    fn letters(&self, i: usize) -> String {
        vowel_letters(&self.runs[i])
    }
    fn before(&self, i: usize) -> &str {
        if self.aligned { &self.word[..self.starts[i]] } else { "" }
    }
    fn after(&self, i: usize) -> &str {
        if self.aligned { &self.word[self.starts[i] + self.runs[i].len()..] } else { "" }
    }
    fn ends_with(&self, i: usize, symbols: &[&str]) -> bool {
        let rest = &self.bases[(i + 1).min(self.count)..];
        rest.len() == symbols.len() && rest.iter().zip(symbols).all(|(a, b)| a == b)
    }
    fn first_vowel(&self, i: usize) -> bool {
        !self.bases[..i].iter().any(|b| arpa_vowel(b))
    }
    fn next_vowel_stress(&self, i: usize) -> Option<u8> {
        for j in i + 1..self.count {
            if arpa_vowel(&self.bases[j]) {
                return Some(self.stress[j].unwrap_or(0));
            }
        }
        None
    }
    fn weak_prefix(&self, i: usize) -> bool {
        self.aligned
            && self.first_vowel(i)
            && self.letters(i) == "e"
            && WEAK_PREFIXES.iter().any(|p| self.word.starts_with(p))
            && ["b", "d", "r", "pr"].contains(&self.before(i))
    }
}

fn north_or_force(w: &Word, i: usize, before_vowel: bool) -> &'static str {
    if !w.aligned {
        return "ɔː";
    }
    let spelled = w.letters(i);
    if ["oa", "oo", "ou"].contains(&spelled.as_str()) {
        return "oː";
    }
    if spelled != "o" {
        return "ɔː";
    }
    let run = &w.runs[i];
    let tail = format!("{}{}", &run[run.find('o').unwrap() + 1..], w.after(i));
    let rest = tail.get(1..).unwrap_or("");
    if !before_vowel {
        if rest.starts_with('e') {
            return "oː";
        }
        if w.before(i).ends_with('p') && rest.starts_with('t') {
            return "oː";
        }
        return if FORCE_SPELLINGS.iter().any(|s| w.word.contains(s)) { "oː" } else { "ɔː" };
    }
    if rest.starts_with('r') {
        return "ɔː";
    }
    if ["y", "ies", "ied", "a", "as", "ah", "er", "ers"].contains(&rest)
        || ["ia", "io", "iu", "ing", "age"].iter().any(|p| rest.starts_with(p))
    {
        return "oː";
    }
    "ɔː"
}

fn weak_i(w: &Word, i: usize) -> &'static str {
    let ii = i as isize;
    let (nxt, nxt2) = (w.base(ii + 1), w.base(ii + 2));
    if nxt == Some("F") && nxt2 == Some("AH") && w.base(ii + 3) == Some("L") {
        return "i";
    }
    if (nxt == Some("T") && nxt2 == Some("IY"))
        || (nxt == Some("B") && matches!(nxt2, Some("AH") | Some("L")))
        || (nxt == Some("F") && nxt2 == Some("AY"))
    {
        return "ᵻ";
    }
    if nxt == Some("T") && w.ends_with(i, &["T"]) && matches!(w.base(ii - 1), Some("R") | Some("ER")) && w.word.ends_with("ite")
    {
        return "ᵻ";
    }
    "ɪ"
}

fn weak_e(w: &Word, i: usize) -> &'static str {
    let ii = i as isize;
    let (nxt, prev) = (w.base(ii + 1), w.base(ii - 1));
    if nxt == Some("T") && (w.ends_with(i, &["T"]) || w.ends_with(i, &["T", "S"])) {
        return if prev == Some("L") { "ə" } else if prev == Some("R") { "ᵻ" } else { "ɪ" };
    }
    if nxt == Some("S") && (w.ends_with(i, &["S", "T"]) || w.ends_with(i, &["S", "T", "L", "IY"])) {
        return "ɪ";
    }
    if nxt == Some("JH") {
        return "ɪ";
    }
    if nxt == Some("K") && w.base(ii + 2) == Some("T") {
        return "ɛ";
    }
    if nxt == Some("D") && w.ends_with(i, &["D"]) {
        return "ɪ";
    }
    "ə"
}

fn inflection(w: &Word, i: usize) -> bool {
    if i + 1 >= w.count || (w.aligned && !format!("{}{}", w.runs[i], w.runs[i + 1]).contains('e')) {
        return false;
    }
    let ii = i as isize;
    let (nxt, prev) = (w.base(ii + 1), w.base(ii - 1));
    if nxt == Some("D")
        && matches!(prev, Some("T") | Some("D"))
        && (w.ends_with(i, &["D"]) || w.ends_with(i, &["D", "L", "IY"]))
    {
        return true;
    }
    nxt == Some("Z") && prev.is_some_and(|p| SIBILANTS.contains(&p)) && w.ends_with(i, &["Z"])
}

fn first_syllable(w: &Word, i: usize) -> Option<&'static str> {
    if !w.aligned || !w.first_vowel(i) || !matches!(w.next_vowel_stress(i), Some(s) if s != 0) {
        return None;
    }
    let word = w.word.as_bytes();
    let at = |k: usize| word.get(k).map(|&b| b as char);
    if i == 0 && w.word.starts_with('e') {
        if w.word.starts_with("ex") {
            return Some("ɛ");
        }
        if matches!(at(1), Some('n') | Some('m') | Some('s'))
            && at(2).is_some_and(|c| !vowel_letter(c) && Some(c) != at(1))
        {
            return Some("ɛ");
        }
        return Some("ɪ");
    }
    if w.weak_prefix(i) {
        if w.word.starts_with("pre") || (w.word.starts_with("be") && matches!(at(2), Some('c') | Some('g'))) {
            return Some("ɪ");
        }
        return Some("ᵻ");
    }
    if w.letters(i) == "i"
        && w.word.starts_with("di")
        && word.len() > 3
        && !vowel_letter(word[2] as char)
        && word[2] != b's'
        && vowel_letter(word[3] as char)
    {
        return Some("ᵻ");
    }
    None
}

fn reduced(w: &Word, i: usize) -> &'static str {
    let ii = i as isize;
    let name = w.bases[i].as_str();
    let spelled = w.letters(i);
    let (nxt, prev) = (w.base(ii + 1), w.base(ii - 1));
    if name == "IH" && nxt == Some("NG") {
        return "ɪ";
    }
    if inflection(w, i) {
        return "ᵻ";
    }
    if name == "AH" {
        if prev == Some("Y") && format!("{}{}", w.runs[i - 1], w.runs[i]).contains('u') {
            return "ʊ";
        }
        if i == 0 && w.word.starts_with('a') {
            return "ɐ";
        }
        if i == 0 && nxt == Some("N") && w.word.starts_with("un") {
            return "ʌ";
        }
        if spelled == "a" && PREFIXES.contains(&w.before(i)) && w.runs[i].starts_with('a') {
            return "ɐ";
        }
    }
    if let Some(prefix) = first_syllable(w, i) {
        return prefix;
    }
    if spelled == "e"
        && matches!(nxt, Some("K") | Some("G"))
        && w.runs[i + 1].starts_with('x')
        && matches!(w.next_vowel_stress(i), Some(s) if s != 0)
    {
        return "ɛ";
    }
    if ["i", "ai", "ui", "y"].contains(&spelled.as_str()) {
        return weak_i(w, i);
    }
    if spelled == "e" {
        return weak_e(w, i);
    }
    if spelled == "a" {
        if nxt == Some("JH") || (nxt == Some("N") && w.base(ii + 2) == Some("JH")) {
            return "ɪ";
        }
        if nxt == Some("S") && w.ends_with(i, &["S"]) && w.word.ends_with("ace") {
            return "ɪ";
        }
        if name == "IH" && !(nxt == Some("T") && w.ends_with(i, &["T"])) {
            return "ɪ";
        }
        return "ə";
    }
    if name == "AH" && nxt == Some("M") && w.ends_with(i, &["M"]) && w.word.ends_with("ome") {
        return "ʌ";
    }
    if name == "IH" && !["o", "u", "ou"].contains(&spelled.as_str()) { "ɪ" } else { "ə" }
}

fn unstressed_iy(w: &Word, i: usize) -> (&'static str, u8) {
    let ii = i as isize;
    let nxt = w.base(ii + 1);
    if nxt.is_none() || w.ends_with(i, &["Z"]) || w.ends_with(i, &["D"]) {
        return ("i", 0);
    }
    if w.vowel(i + 1) {
        return (if nxt == Some("IH") && w.base(ii + 2) == Some("NG") { "i" } else { "ɪ" }, 0);
    }
    if w.weak_prefix(i) {
        return if w.next_vowel_stress(i) == Some(1) { ("ᵻ", 0) } else { ("iː", 2) };
    }
    ("ɪ", 0)
}

/// Citation form of `word` from a CMUdict pronunciation and the letters aligned to each symbol (or none).
pub fn convert<S: AsRef<str>>(word: &str, arpabet: &[S], runs: Option<Vec<String>>) -> Result<Vec<Ph>, ValueError> {
    let arpabet: Vec<String> = arpabet.iter().map(|s| s.as_ref().to_string()).collect();
    let w = Word::new(word, &arpabet, runs)?;
    let mut out: Vec<(&'static str, u8)> = Vec::new();
    let mut i = 0usize;
    while i < w.count {
        let ii = i as isize;
        let name = w.bases[i].as_str();
        let stress = w.stress[i].unwrap_or(0);
        let nxt = w.base(ii + 1);
        if !arpa_vowel(name) {
            if name == "Y"
                && nxt == Some("AH")
                && w.unstressed(i + 1)
                && matches!(w.base(ii - 1), Some("N") | Some("L"))
                && w.base(ii + 2) != Some("L")
                && (!w.aligned || format!("{}{}", w.runs[i], w.runs[i + 1]).contains('i'))
            {
                out.push(("iə", 0));
                i += 2;
                continue;
            }
            out.push((consonant(name).unwrap(), 0));
            i += 1;
            continue;
        }
        let before_vowel = w.vowel(i + 2);
        if name == "ER" {
            out.push((if stress != 0 { "ɜː" } else { "ɚ" }, stress));
            if nxt == Some("R") {
                i += 1;
            }
            if w.vowel(i + 1) {
                out.push(("ɹ", 0));
            }
        } else if nxt == Some("R") && name == "AW" {
            out.push(("aʊ", stress));
            out.push(("ɚ", 0));
            if before_vowel {
                out.push(("ɹ", 0));
            }
            i += 1;
        } else if nxt == Some("R") && ["AA", "AO", "IH", "IY", "EH", "UH", "UW", "AY"].contains(&name) {
            let (quality, single): (&'static str, &'static str) = match name {
                "AA" => ("ɑː", "ɑːɹ"),
                "AO" => {
                    let q = north_or_force(&w, i, before_vowel);
                    (q, if q == "oː" { "oːɹ" } else { "ɔːɹ" })
                }
                "IH" | "IY" => {
                    let l = w.letters(i);
                    (if stress != 0 && (l == "e" || l == "eo") { "iə" } else { "ɪ" }, "ɪɹ")
                }
                "EH" => {
                    let doubled = format!("{}{}", w.runs[i + 1], w.after(i + 1)).starts_with("rr")
                        || (w.runs[i].ends_with('r') && w.runs[i + 1].starts_with('r'));
                    (if w.letters(i) == "a" && doubled { "æ" } else { "ɛ" }, "ɛɹ")
                }
                "UH" | "UW" => ("ʊɹ", "ʊɹ"),
                _ => (if !w.aligned || IRE_ENDINGS.contains(&w.after(i + 1)) { "aɪɚ" } else { "aɪ" }, "aɪɚ"),
            };
            if before_vowel {
                out.push((quality, stress));
                out.push(("ɹ", 0));
            } else {
                out.push((single, stress));
            }
            i += 1;
        } else if name == "AY" && nxt == Some("ER") && w.unstressed(i + 1) && w.runs[i + 1] == "er" && w.runs[i] != "i" {
            out.push(("aɪ", stress));
        } else if name == "AY" && nxt == Some("ER") && w.unstressed(i + 1) {
            out.push(("aɪɚ", stress));
            i += 1;
            if w.vowel(i + 1) {
                out.push(("ɹ", 0));
            }
        } else if name == "AY" && nxt == Some("AH") && w.unstressed(i + 1) {
            out.push(("aɪə", stress));
            i += 1;
        } else if name == "IY" && nxt == Some("AH") && w.unstressed(i + 1) && !w.weak_prefix(i) {
            let after = w.base(ii + 2);
            if stress != 1 && (matches!(after, Some("L") | Some("D") | Some("T")) || (after == Some("N") && w.base(ii + 3) == Some("S")))
            {
                out.push(("ɪ", 0));
            } else {
                out.push(("iə", stress));
                i += 1;
            }
        } else if stress == 0
            && name == "AH"
            && nxt == Some("N")
            && w.base(ii - 1) == Some("T")
            && w.base(ii + 2) != Some("T")
            && out.len() >= 2
            && out[out.len() - 1].0 == "t"
            && is_vowel(out[out.len() - 2].0)
            && out[out.len() - 2].1 != 0
        {
            let n = out.len();
            out[n - 1].0 = "ʔ";
            out.push(("n̩", 0));
            i += 1;
        } else if stress == 0
            && name == "AH"
            && nxt == Some("W")
            && w.base(ii + 2) == Some("AH")
            && w.unstressed(i + 2)
            && format!("{}{}", w.runs[i], w.runs[i + 1]).contains('u')
        {
            out.push(("uː", 0));
            i += 1;
        } else if stress == 0 && name == "AH" && nxt == Some("L") && i > 0 && !(w.vowel(i + 2) && w.stress[i + 2].unwrap_or(0) != 0)
        {
            if w.letters(i) == "i" && w.vowel(i + 2) {
                out.push(("ɪ", 0));
            } else if w.base(ii - 1) == Some("Y") && format!("{}{}", w.runs[i - 1], w.runs[i]).contains('u') {
                out.push(("ʊ", 0));
            } else {
                out.push(("əl", 0));
                i += 1;
            }
        } else if stress == 0 && (name == "AH" || name == "IH") {
            out.push((reduced(&w, i), 0));
        } else if name == "IY" && (stress == 0 || (stress == 2 && nxt.is_none())) {
            out.push(unstressed_iy(&w, i));
        } else if stress == 0
            && name == "AE"
            && i == 0
            && (w.word.starts_with("ad") || w.word.starts_with("acc"))
            && matches!(nxt, Some("D") | Some("K"))
        {
            out.push(("ɐ", 0));
        } else if stress == 0 && name == "ER" {
            out.push(("ɚ", 0));
        } else if (name == "AA" || name == "AO")
            && w.letters(i) == "a"
            && (w.before(i).ends_with('w') || w.before(i).ends_with("qu"))
            && nxt == Some("N")
        {
            out.push(("ɔ", stress));
        } else if name == "AO" && w.letters(i) == "o" {
            out.push(("ɔ", stress));
        } else {
            out.push((full_vowel(name), stress));
        }
        i += 1;
    }
    flap(&mut out);
    for k in 0..out.len().saturating_sub(1) {
        if out[k].0 == "n" && (out[k + 1].0 == "k" || out[k + 1].0 == "ɡ") {
            out[k].0 = "ŋ";
        }
    }
    settle(&mut out);
    debug_assert!(out.iter().all(|(p, _)| is_phone(p)));
    Ok(out)
}

fn flap(out: &mut [Ph]) {
    let n = out.len();
    for k in 1..n.saturating_sub(1) {
        if out[k].0 != "t" {
            continue;
        }
        let before = out[k - 1].0;
        let (after, stress) = out[k + 1];
        if (!is_vowel(before) && before != "ɹ") || before == "n̩" || before == "əl" || !is_vowel(after) || after == "n̩" {
            continue;
        }
        let final_o = after == "oʊ" && (k + 2 == n || (k + 3 == n && out[k + 2].0 == "z"));
        if stress != 0 && !final_o {
            continue;
        }
        if (after == "ə" || after == "ɚ") && k + 2 < n && out[k + 2].0 == "n" {
            continue;
        }
        out[k].0 = "ɾ";
    }
}

/// One primary stress per word, no secondary stress next to the primary one.
pub fn settle_stress(phones: &[Ph]) -> Vec<Ph> {
    let mut out = phones.to_vec();
    settle(&mut out);
    out
}

fn settle(out: &mut [Ph]) {
    let vowels: Vec<usize> = (0..out.len()).filter(|&k| is_vowel(out[k].0)).collect();
    let primaries: Vec<usize> = vowels.iter().copied().filter(|&k| out[k].1 == 1).collect();
    if primaries.len() > 1 {
        for &k in &primaries[..primaries.len() - 1] {
            out[k].1 = 2;
        }
    }
    for position in 0..vowels.len() {
        let k = vowels[position];
        if out[k].1 != 2 {
            continue;
        }
        let near = [position.wrapping_sub(1), position + 1];
        if near.iter().filter(|&&p| p < vowels.len()).any(|&p| out[vowels[p]].1 == 1) {
            out[k].1 = 0;
        }
    }
}
