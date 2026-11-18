//! Diction judge: what a CTC phoneme recogniser heard in a stem and in a render, and how far apart the two are.
//!
//! The recogniser itself (a wav2vec2-style CTC model, e.g. `facebook/wav2vec2-xlsr-53-espeak-cv-ft`) runs outside
//! this crate (ONNX); it hands over its logits, one row of `vocab` scores per frame. This module turns them into
//! phones (greedy CTC: argmax per frame, repeats collapsed, the blank dropped) and compares two phone sequences by
//! their edit distance: the phoneme error rate (PER) of a render against its stem is (substitutions + deletions +
//! insertions) / phones of the stem. A log-mel distance says how a render's spectrum differs; this says whether the
//! same sounds are said.
//!
//! Phones are compared after [`fold_phone`]: length, stress, tone and syllable marks are dropped (a sung vowel is held
//! long; that is not a different vowel), so `iː` and `i` count as the same phone.

use serde_json::{Value, json};

/// One recognised phone: its vocabulary index and token, and the frames it spans (end exclusive).
#[derive(Clone, Debug, PartialEq)]
pub struct Phone {
    pub id: usize,
    pub token: String,
    pub start: usize,
    pub end: usize,
}

/// A recogniser's vocabulary in index order, from its `vocab.json` (`{"token": id, ...}`). Missing ids are empty
/// strings. None when the text is not such an object.
pub fn parse_vocab(text: &str) -> Option<Vec<String>> {
    let Value::Object(map) = serde_json::from_str::<Value>(text).ok()? else { return None };
    let n = map.values().filter_map(Value::as_u64).max()? as usize + 1;
    let mut out = vec![String::new(); n];
    for (token, id) in &map {
        if let Some(i) = id.as_u64() {
            out[i as usize] = token.clone();
        }
    }
    Some(out)
}

/// Whether a token is a special symbol (`<pad>`, `<s>`, `</s>`, `<unk>`) or a word separator, not a phone.
fn special(token: &str) -> bool {
    token.is_empty() || token == "|" || (token.starts_with('<') && token.ends_with('>'))
}

/// Greedy CTC decoding of `logits` (`frames` rows of `vocab.len()` scores, row-major): per frame the best token,
/// repeats merged, the blank (`blank`, usually `<pad>` = 0) and special tokens dropped. A token repeated across a
/// blank is two phones.
pub fn ctc_greedy(logits: &[f32], vocab: &[String], blank: usize) -> Vec<Phone> {
    let v = vocab.len();
    if v == 0 {
        return Vec::new();
    }
    let frames = logits.len() / v;
    let mut out: Vec<Phone> = Vec::new();
    let mut prev = usize::MAX;
    for f in 0..frames {
        let row = &logits[f * v..(f + 1) * v];
        let mut best = 0;
        for (i, x) in row.iter().enumerate() {
            if *x > row[best] {
                best = i;
            }
        }
        if best == prev {
            if let Some(last) = out.last_mut() {
                if last.id == best && last.end == f {
                    last.end = f + 1;
                }
            }
            continue;
        }
        prev = best;
        if best != blank && !special(&vocab[best]) {
            out.push(Phone { id: best, token: vocab[best].clone(), start: f, end: f + 1 });
        }
    }
    out
}

/// A phone as compared: length (`ː`, `ˑ`, `:`), stress (`ˈ`, `ˌ`), tone digits and syllable dots dropped.
/// Empty for a token that was only such marks.
pub fn fold_phone(token: &str) -> String {
    token.chars().filter(|c| !matches!(c, 'ː' | 'ˑ' | ':' | 'ˈ' | 'ˌ' | '.' | '0'..='9')).collect()
}

/// Edit operations between a reference and a hypothesis sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditCount {
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    /// length of the reference
    pub reference: usize,
}

impl EditCount {
    pub fn errors(&self) -> usize {
        self.substitutions + self.deletions + self.insertions
    }

    /// Errors per reference phone (0 when both are empty, the hypothesis length when only the reference is).
    pub fn rate(&self) -> f64 {
        if self.reference == 0 { self.insertions as f64 } else { self.errors() as f64 / self.reference as f64 }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "per": self.rate(), "substitutions": self.substitutions, "deletions": self.deletions,
            "insertions": self.insertions, "reference": self.reference,
        })
    }
}

/// Levenshtein distance (unit costs) of `hyp` against `reference`, split into substitutions, deletions and
/// insertions along one optimal path (ties: a match or substitution first, then a deletion).
pub fn edit_count<T: PartialEq>(reference: &[T], hyp: &[T]) -> EditCount {
    let (n, m) = (reference.len(), hyp.len());
    let w = m + 1;
    let mut d = vec![0usize; (n + 1) * w];
    for i in 0..=n {
        d[i * w] = i;
    }
    for j in 0..=m {
        d[j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = d[(i - 1) * w + j - 1] + usize::from(reference[i - 1] != hyp[j - 1]);
            d[i * w + j] = sub.min(d[(i - 1) * w + j] + 1).min(d[i * w + j - 1] + 1);
        }
    }
    let mut c = EditCount { reference: n, ..EditCount::default() };
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && d[i * w + j] == d[(i - 1) * w + j - 1] + usize::from(reference[i - 1] != hyp[j - 1]) {
            if reference[i - 1] != hyp[j - 1] {
                c.substitutions += 1;
            }
            i -= 1;
            j -= 1;
        } else if i > 0 && d[i * w + j] == d[(i - 1) * w + j] + 1 {
            c.deletions += 1;
            i -= 1;
        } else {
            c.insertions += 1;
            j -= 1;
        }
    }
    c
}

/// PER of a hypothesis phone list against a reference one, both folded ([`fold_phone`]; phones that fold to nothing
/// are left out).
pub fn phone_error_rate(reference: &[String], hyp: &[String]) -> EditCount {
    let fold = |s: &[String]| s.iter().map(|t| fold_phone(t)).filter(|t| !t.is_empty()).collect::<Vec<_>>();
    edit_count(&fold(reference), &fold(hyp))
}

/// Tokens of decoded phones.
pub fn tokens(phones: &[Phone]) -> Vec<String> {
    phones.iter().map(|p| p.token.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn greedy_decoding_and_phone_error_rate() {
        let vocab = s(&["<pad>", "<s>", "a", "iː", "n"]);
        // frames: a a <pad> a n n <s> iː
        let ids = [2usize, 2, 0, 2, 4, 4, 1, 3];
        let mut logits = vec![0.0f32; ids.len() * vocab.len()];
        for (f, id) in ids.iter().enumerate() {
            logits[f * vocab.len() + id] = 5.0;
        }
        let phones = ctc_greedy(&logits, &vocab, 0);
        assert_eq!(tokens(&phones), s(&["a", "a", "n", "iː"]));
        assert_eq!((phones[0].start, phones[0].end, phones[2].start, phones[2].end), (0, 2, 4, 6));
        // a sequence against itself: no error; length marks fold away
        assert_eq!(phone_error_rate(&tokens(&phones), &s(&["a", "a", "n", "i"])).errors(), 0);
        // one substitution, one deletion, one insertion against a 4-phone reference
        let c = phone_error_rate(&s(&["a", "a", "n", "i"]), &s(&["o", "n", "i", "t"]));
        assert_eq!((c.substitutions, c.deletions, c.insertions, c.reference), (1, 1, 1, 4));
        assert!((c.rate() - 0.75).abs() < 1e-12);
        assert_eq!(parse_vocab(r#"{"<pad>": 0, "n": 2, "a": 1}"#), Some(s(&["<pad>", "a", "n"])));
    }
}
