//! Words the dictionary lacks but can be built from words it has (dizione `morphology.py`): plurals, possessives,
//! -ed, -ing, -ly and other regular endings, a few prefixes, compounds of two dictionary words, British spellings.

use crate::rx;
use crate::symbols::{is_vowel, Ph};

const SIBILANT: [&str; 6] = ["s", "z", "ʃ", "ʒ", "tʃ", "dʒ"];
const VOICELESS: [&str; 6] = ["p", "t", "k", "f", "θ", "ʔ"];
const SPLIT: [(&str, &str); 5] = [("ɑːɹ", "ɑː"), ("ɔːɹ", "ɔː"), ("oːɹ", "oː"), ("ɪɹ", "ɪ"), ("ɛɹ", "ɛ")];
const RHOTIC: [&str; 4] = ["ɚ", "ɜː", "aɪɚ", "ʊɹ"];
const CONSONANT_LETTERS: &str = "bcdfghjklmnpqrstvwxz";

fn p(phones: &[&'static str]) -> Vec<Ph> {
    phones.iter().map(|&x| (x, 0)).collect()
}

fn endings() -> Vec<(&'static str, Vec<Ph>)> {
    vec![
        ("fully", p(&["f", "əl", "i"])),
        ("lessly", p(&["l", "ə", "s", "l", "i"])),
        ("ness", p(&["n", "ə", "s"])),
        ("less", p(&["l", "ə", "s"])),
        ("ments", p(&["m", "ə", "n", "t", "s"])),
        ("ment", p(&["m", "ə", "n", "t"])),
        ("ful", p(&["f", "əl"])),
        ("ship", vec![("ʃ", 0), ("ɪ", 2), ("p", 0)]),
        ("hood", vec![("h", 0), ("ʊ", 2), ("d", 0)]),
        ("like", vec![("l", 0), ("aɪ", 2), ("k", 0)]),
        ("dom", p(&["d", "ə", "m"])),
        ("ish", p(&["ɪ", "ʃ"])),
        ("able", p(&["ə", "b", "əl"])),
        ("ism", p(&["ɪ", "z", "ə", "m"])),
        ("ist", p(&["ɪ", "s", "t"])),
        ("wards", p(&["w", "ɚ", "d", "z"])),
        ("ward", p(&["w", "ɚ", "d"])),
        ("age", p(&["ɪ", "dʒ"])),
        ("ers", p(&["ɚ", "z"])),
        ("er", p(&["ɚ"])),
        ("est", p(&["ɪ", "s", "t"])),
        ("y", p(&["i"])),
    ]
}

fn prefixes() -> Vec<(&'static str, Vec<Ph>, Vec<Ph>)> {
    let same = |s: &'static str, v: Vec<Ph>| (s, v.clone(), v);
    vec![
        same("under", vec![("ʌ", 2), ("n", 0), ("d", 0), ("ɚ", 0)]),
        same("over", vec![("oʊ", 2), ("v", 0), ("ɚ", 0)]),
        same("anti", vec![("æ", 2), ("n", 0), ("t", 0), ("i", 0)]),
        same("semi", vec![("s", 0), ("ɛ", 2), ("m", 0), ("i", 0)]),
        same("non", vec![("n", 0), ("ɑː", 2), ("n", 0)]),
        same("out", vec![("aʊ", 2), ("t", 0)]),
        ("dis", p(&["d", "ɪ", "s"]), vec![("d", 0), ("ɪ", 2), ("s", 0)]),
        ("mis", p(&["m", "ɪ", "s"]), vec![("m", 0), ("ɪ", 2), ("s", 0)]),
        ("un", p(&["ʌ", "n"]), vec![("ʌ", 2), ("n", 0)]),
        ("in", p(&["ɪ", "n"]), vec![("ɪ", 2), ("n", 0)]),
        ("re", p(&["ɹ", "ᵻ"]), vec![("ɹ", 0), ("iː", 2)]),
        same("pre", vec![("p", 0), ("ɹ", 0), ("iː", 2)]),
    ]
}

const BRITISH: [(&str, &str); 23] = [
    ("isation", "ization"), ("ising", "izing"), ("ised", "ized"), ("ises", "izes"), ("ise", "ize"), ("yse", "yze"),
    ("our", "or"), ("ourable", "orable"), ("ours", "ors"), ("oured", "ored"), ("ouring", "oring"), ("tre", "ter"),
    ("tres", "ters"), ("bre", "ber"), ("ence", "ense"), ("ogue", "og"), ("mme", "m"), ("lled", "led"), ("lling", "ling"),
    ("ller", "ler"), ("oul", "ol"), ("ae", "e"), ("enqui", "inqui"),
];

/// Stem + ending, adjusted at the seam.
pub fn join(stem: &[Ph], ending: &[Ph]) -> Vec<Ph> {
    let mut stem = stem.to_vec();
    if stem.is_empty() || ending.is_empty() {
        stem.extend_from_slice(ending);
        return stem;
    }
    let (last, stress) = *stem.last().unwrap();
    let (first, first_stress) = ending[0];
    let n = stem.len();
    if is_vowel(first) {
        if let Some((_, split)) = SPLIT.iter().find(|(a, _)| *a == last) {
            stem[n - 1] = (split, stress);
            stem.push(("ɹ", 0));
        } else if RHOTIC.contains(&last) {
            stem.push(("ɹ", 0));
        } else if last == "t"
            && n > 1
            && (is_vowel(stem[n - 2].0) || stem[n - 2].0 == "ɹ")
            && first_stress == 0
            && !(first == "ə" && ending.len() > 1 && ending[1].0 == "n")
        {
            stem[n - 1] = ("ɾ", 0);
        } else if last == "i" {
            stem[n - 1] = (if first != "ɪ" { "ɪ" } else { "i" }, stress);
        }
    } else if last == "i" && ["n", "l", "f", "m"].contains(&first) {
        stem[n - 1] = ("ɪ", stress);
    } else if last == "n" && (first == "k" || first == "ɡ") {
        stem[n - 1] = ("ŋ", 0);
    }
    stem.extend_from_slice(ending);
    stem
}

pub fn plural(stem: &[Ph]) -> Vec<Ph> {
    let last = stem.last().unwrap().0;
    let ending = if SIBILANT.contains(&last) {
        p(&["ᵻ", "z"])
    } else if VOICELESS.contains(&last) {
        p(&["s"])
    } else {
        p(&["z"])
    };
    join(stem, &ending)
}

pub fn past(stem: &[Ph]) -> Vec<Ph> {
    let last = stem.last().unwrap().0;
    if ["t", "d", "ɾ"].contains(&last) {
        return join(stem, &p(&["ᵻ", "d"]));
    }
    let voiceless = VOICELESS.contains(&last) || ["s", "ʃ", "tʃ"].contains(&last);
    join(stem, &p(&[if voiceless { "t" } else { "d" }]))
}

pub fn adverb(stem: &[Ph]) -> Vec<Ph> {
    let last = stem.last().unwrap().0;
    join(stem, &if last == "əl" || last == "l" { p(&["i"]) } else { p(&["l", "i"]) })
}

fn consonant_letter(c: char) -> bool {
    CONSONANT_LETTERS.contains(c)
}

/// Spellings the stem may have had before an ending starting with a vowel.
fn stems(body: &str) -> Vec<String> {
    let b: Vec<char> = body.chars().collect();
    let mut out = vec![body.to_string()];
    let n = b.len();
    if n > 2 && b[n - 1] == b[n - 2] && consonant_letter(b[n - 1]) {
        out.push(b[..n - 1].iter().collect());
    }
    if n > 0 && (consonant_letter(b[n - 1]) || b[n - 1] == 'u') {
        let single = rx!(r"(?:^|[^aeiou])[aeiou](?:[bcdfgklmnprstvz]|th)$").is_match(body);
        let with_e = format!("{body}e");
        if single {
            out.insert(0, with_e);
        } else {
            out.push(with_e);
        }
    }
    if body.ends_with('i') && n > 2 {
        out.push(format!("{}y", &body[..body.len() - 1]));
    }
    out
}

fn secondary(phones: &[Ph]) -> Vec<Ph> {
    phones.iter().map(|&(ph, s)| (ph, if s == 1 { 2 } else { s })).collect()
}

/// Two words said as one: the second keeps only a lesser stress.
pub fn compound(first: &[Ph], second: &[Ph]) -> Vec<Ph> {
    join(first, &secondary(second))
}

/// What `derive` looks words up with.
pub trait Lookup {
    fn lookup(&self, spelling: &str) -> Option<Vec<Ph>>;
    fn part(&self, spelling: &str) -> Option<Vec<Ph>>;
}

fn chars_len(s: &str) -> usize {
    s.chars().count()
}

/// Pronunciation of `word` from a shorter dictionary word, or `None`.
pub fn derive(word: &str, l: &dyn Lookup, depth: u32) -> Option<Vec<Ph>> {
    let stem = |spelling: &str| -> Option<Vec<Ph>> {
        if chars_len(spelling) < 2 {
            return None;
        }
        l.lookup(spelling).filter(|f| !f.is_empty()).or_else(|| if depth < 2 { derive(spelling, l, depth + 1) } else { None })
    };
    // Python's `lookup(s) or derive(...)`: an empty pronunciation counts as none.
    let first = |candidates: &[String], build: &dyn Fn(&[Ph]) -> Option<Vec<Ph>>| -> Option<Vec<Ph>> {
        for spelling in candidates {
            if let Some(found) = stem(spelling).filter(|f| !f.is_empty()) {
                return build(&found).filter(|r| !r.is_empty());
            }
        }
        None
    };
    let len = chars_len(word);
    let cut = |k: usize| -> String { word.chars().take(len - k).collect() };
    let last = |k: usize| -> char { word.chars().nth(len - k).unwrap() };

    if word.ends_with("'s") {
        return first(&[cut(2)], &|f| Some(plural(f)));
    }
    if word.ends_with("s'") {
        return first(&[cut(1)], &|f| Some(f.to_vec()));
    }
    if word.ends_with("ies") && len > 4 {
        if let Some(found) = first(&[format!("{}y", cut(3))], &|f| Some(plural(f))) {
            return Some(found);
        }
    }
    if word.ends_with("ves") && len > 4 {
        let build = |f: &[Ph]| {
            if f.last().unwrap().0 == "f" {
                let mut s = f[..f.len() - 1].to_vec();
                s.push(("v", 0));
                Some(join(&s, &p(&["z"])))
            } else {
                None
            }
        };
        if let Some(found) = first(&[format!("{}f", cut(3)), format!("{}fe", cut(3))], &build) {
            return Some(found);
        }
    }
    if word.ends_with("es") && len > 3 {
        let candidates = if "sxzh".contains(last(3)) { vec![cut(2)] } else { vec![] };
        if let Some(found) = first(&candidates, &|f| Some(plural(f))) {
            return Some(found);
        }
    }
    if word.ends_with('s') && !(word.ends_with("ss") || word.ends_with("us") || word.ends_with("is")) && len > 3 {
        if let Some(found) = first(&[cut(1)], &|f| Some(plural(f))) {
            return Some(found);
        }
    }
    if word.ends_with("ied") && len > 4 {
        if let Some(found) = first(&[format!("{}y", cut(3))], &|f| Some(past(f))) {
            return Some(found);
        }
    }
    if word.ends_with("ed") && len > 3 {
        if let Some(found) = first(&stems(&cut(2)), &|f| Some(past(f))) {
            return Some(found);
        }
    }
    if word.ends_with("ing") && len > 4 {
        if let Some(found) = first(&stems(&cut(3)), &|f| Some(join(f, &p(&["ɪ", "ŋ"])))) {
            return Some(found);
        }
    }
    if word.ends_with("ily") && len > 4 {
        if let Some(found) = first(&[format!("{}y", cut(3))], &|f| Some(adverb(f))) {
            return Some(found);
        }
    }
    if word.ends_with("ly") && len > 3 {
        let candidates = if word.ends_with("bly") || word.ends_with("ply") {
            vec![cut(2), format!("{}le", cut(2)), format!("{}e", cut(1))]
        } else {
            vec![cut(2)]
        };
        if let Some(found) = first(&candidates, &|f| Some(adverb(f))) {
            return Some(found);
        }
    }
    for (spelling, phones) in endings() {
        let sl = chars_len(spelling);
        if word.ends_with(spelling) && len - sl > 2 {
            let body = cut(sl);
            if spelling == "y" && !consonant_letter(body.chars().last().unwrap()) {
                continue;
            }
            let vowel_first = is_vowel(phones[0].0);
            let candidates = if vowel_first {
                stems(&body)
            } else if body.ends_with('i') {
                vec![format!("{}y", &body[..body.len() - 1]), body.clone()]
            } else {
                vec![body.clone()]
            };
            if let Some(found) = first(&candidates, &|f| Some(join(f, &phones))) {
                return Some(found);
            }
        }
    }
    for (spelling, plain, stressed) in prefixes() {
        let sl = chars_len(spelling);
        if word.starts_with(spelling) && len - sl > 2 {
            if let Some(found) = stem(&word[spelling.len()..]).filter(|f| !f.is_empty()) {
                let vowels: Vec<u8> = found.iter().filter(|(ph, _)| is_vowel(ph)).map(|(_, s)| *s).collect();
                let head = if !vowels.is_empty() && vowels[0] == 1 { &plain } else { &stressed };
                return Some(join(head, &found));
            }
        }
    }
    for (old, new) in BRITISH {
        if word.contains(old) {
            if let Some(found) = stem(&word.replacen(old, new, 1)).filter(|f| !f.is_empty()) {
                return Some(found);
            }
        }
    }
    if depth == 0 {
        let chars: Vec<char> = word.chars().collect();
        let mut best: Option<(usize, Vec<Ph>, Vec<Ph>)> = None;
        for c in 4..len.saturating_sub(3) {
            let left: String = chars[..c].iter().collect();
            let right: String = chars[c..].iter().collect();
            if let (Some(a), Some(b)) = (l.part(&left).filter(|x| !x.is_empty()), l.part(&right).filter(|x| !x.is_empty())) {
                let score = c.min(len - c);
                if best.as_ref().is_none_or(|bst| score > bst.0) {
                    best = Some((score, a, b));
                }
            }
        }
        if let Some((_, a, b)) = best {
            return Some(compound(&a, &b));
        }
    }
    None
}
