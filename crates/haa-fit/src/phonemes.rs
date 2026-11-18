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
//!
//! The same posteriors time a text on the audio ([`align_lyrics`]): the text's phones force-aligned (CTC Viterbi,
//! [`ctc_align`]) and matched with what the recogniser heard, the audio deciding each phone. The lyrics voice takes
//! its phone times from there, not from a speech-to-text model's word guesses.

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

/// Rows of `logits` (`v` scores per frame) as log-probabilities (a log-softmax per frame).
pub fn log_softmax(logits: &[f32], v: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; logits.len()];
    if v == 0 {
        return out;
    }
    for (row, o) in logits.chunks_exact(v).zip(out.chunks_exact_mut(v)) {
        let m = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let z = (row.iter().map(|x| f64::from(x - m).exp()).sum::<f64>().ln() as f32) + m;
        for (a, b) in o.iter_mut().zip(row) {
            *a = b - z;
        }
    }
    out
}

/// CTC forced alignment (Viterbi): the most likely path through `logp` (log-probabilities, `v` per frame) that emits
/// exactly `targets` (vocabulary ids, in order), blanks (`blank`) between and around them and a blank between two equal
/// tokens. Per target, the frames it is emitted on (first, last + 1). None when no target is given or the frames
/// cannot hold the sequence.
pub fn ctc_align(logp: &[f32], v: usize, targets: &[usize], blank: usize) -> Option<Vec<(usize, usize)>> {
    if v == 0 || targets.is_empty() || targets.iter().any(|&t| t >= v) {
        return None;
    }
    let frames = logp.len() / v;
    let states = 2 * targets.len() + 1;
    let tok = |s: usize| if s % 2 == 0 { blank } else { targets[s / 2] };
    let neg = f32::NEG_INFINITY;
    if frames == 0 {
        return None;
    }
    let (mut prev, mut cur) = (vec![neg; states], vec![neg; states]);
    // per frame and state, how many states back the best predecessor sits (0 stay, 1, 2 skipping a blank)
    let mut back = vec![0u8; frames * states];
    prev[0] = logp[tok(0)];
    prev[1] = logp[tok(1)];
    for t in 1..frames {
        let row = &logp[t * v..(t + 1) * v];
        for s in 0..states {
            let (mut best, mut b) = (prev[s], 0u8);
            if s >= 1 && prev[s - 1] > best {
                best = prev[s - 1];
                b = 1;
            }
            if s >= 2 && s % 2 == 1 && tok(s) != tok(s - 2) && prev[s - 2] > best {
                best = prev[s - 2];
                b = 2;
            }
            cur[s] = best + row[tok(s)];
            back[t * states + s] = b;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let mut s = if prev[states - 2] > prev[states - 1] { states - 2 } else { states - 1 };
    if prev[s] == neg {
        return None;
    }
    let mut spans: Vec<Option<(usize, usize)>> = vec![None; targets.len()];
    for t in (0..frames).rev() {
        if s % 2 == 1 {
            let k = s / 2;
            spans[k] = Some(match spans[k] {
                None => (t, t + 1),
                Some((_, e)) => (t, e),
            });
        }
        if t > 0 {
            s -= back[t * states + s] as usize;
        }
    }
    spans.into_iter().collect()
}

/// The vowel letters of IPA (a phone is a vowel when its first letter, marks folded away, is one).
const VOWEL_LETTERS: &str = "iyɨʉɯuɪʏʊeøɘɵɤoəɛœɜɞʌɔæɐaɶɑɒɝɚᵻ";

/// Whether an IPA phone is a vowel (a diphthong or an r-coloured vowel included).
pub fn is_vowel_phone(token: &str) -> bool {
    fold_phone(token).chars().next().is_some_and(|c| VOWEL_LETTERS.contains(c))
}

/// Spellings of one sound that a G2P and a recogniser write differently.
const ALIASES: [(&str, &str); 9] = [("g", "ɡ"), ("r", "ɹ"), ("ɫ", "l"), ("ɝ", "ɚ"), ("ʍ", "w"), ("ɐ", "ə"), ("ʧ", "tʃ"), ("ʤ", "dʒ"), ("ɒ", "ɑ")];

/// The vocabulary id of an IPA phone: the same token, its alias, or the first token equal once both are folded
/// ([`fold_phone`]); None when the recogniser has no such phone.
pub fn vocab_id(vocab: &[String], ipa: &str) -> Option<usize> {
    let alias = ALIASES.iter().find(|(a, _)| *a == ipa).map(|(_, b)| *b);
    let exact = |t: &str| vocab.iter().position(|v| v == t);
    let folded = |t: &str| {
        let f = fold_phone(t);
        vocab.iter().position(|v| !special(v) && fold_phone(v) == f)
    };
    exact(ipa).or_else(|| alias.and_then(exact)).or_else(|| folded(ipa)).or_else(|| alias.and_then(folded))
}

/// What the audio did to a phone of the text: heard as written, heard as another phone (the written one kept here),
/// or heard where the text has none.
#[derive(Clone, Debug, PartialEq)]
pub enum Origin {
    Kept,
    Substituted(String),
    Inserted,
}

/// One phone of a line as the audio says it: its token (what the voice will say), the recogniser id it is aligned
/// with, the word of the text it belongs to, where it came from, and its frames (start, end exclusive: up to the
/// next phone's start, the last one up to its own last frame).
#[derive(Clone, Debug, PartialEq)]
pub struct AlignedPhone {
    pub token: String,
    pub id: usize,
    pub word: usize,
    pub origin: Origin,
    pub start: usize,
    pub end: usize,
}

/// A line's phones timed on the audio (see [`align_lyrics`]).
#[derive(Clone, Debug, Default)]
pub struct LyricAlignment {
    pub phones: Vec<AlignedPhone>,
    /// the text's phones the audio does not say: (phone, word)
    pub dropped: Vec<(String, usize)>,
    /// what the recogniser heard (greedy decoding)
    pub heard: Vec<Phone>,
    /// the text's phones force-aligned as written (None where the recogniser has no such phone or the alignment failed)
    pub written: Vec<Option<(usize, usize)>>,
}

/// The cost of saying `b` where `a` is written: 0 for the same phone once folded, less between two vowels or two
/// consonants than across.
fn substitution_cost(a: &str, b: &str) -> f64 {
    let (fa, fb) = (fold_phone(a), fold_phone(b));
    if fa == fb || ALIASES.iter().any(|&(x, y)| (fold_phone(x) == fa && fold_phone(y) == fb) || (fold_phone(x) == fb && fold_phone(y) == fa)) {
        return 0.0;
    }
    match (is_vowel_phone(a), is_vowel_phone(b)) {
        (true, true) => 0.45,
        (false, false) => 0.55,
        _ => 1.2,
    }
}

/// The text's phones (G2P: each with its word index) timed on the audio, the audio deciding which phones are said.
///
/// 1. The written phones are force-aligned ([`ctc_align`]) on the recogniser's posteriors, which places each one.
/// 2. The recogniser's own reading (greedy CTC) is matched with them by a weighted edit distance: a substitution
///    costs by phone class ([`substitution_cost`]) plus how far apart the two sit in time; a deletion or an
///    insertion 0.7. Where the two disagree, the audio wins: a written phone heard as another is replaced by it, a
///    written phone not heard is dropped, a heard phone not written is inserted (into the nearer word). A heard phone
///    the voice cannot say (`sayable` false, even folded) keeps the written one there, and is never inserted.
/// 3. That sequence is force-aligned on the posteriors: each phone starts on its first emitted frame and lasts
///    until the next phone starts.
pub fn align_lyrics(logits: &[f32], vocab: &[String], g2p: &[(String, usize)], blank: usize, sayable: impl Fn(&str) -> bool) -> LyricAlignment {
    let v = vocab.len();
    let heard = ctc_greedy(logits, vocab, blank);
    let logp = log_softmax(logits, v);
    // 1. the written phones where the posteriors put them
    let ids: Vec<Option<usize>> = g2p.iter().map(|(p, _)| vocab_id(vocab, p)).collect();
    let known: Vec<usize> = ids.iter().flatten().copied().collect();
    let mut written = vec![None; g2p.len()];
    if let Some(spans) = ctc_align(&logp, v, &known, blank) {
        let mut q = spans.into_iter();
        for (w, id) in written.iter_mut().zip(&ids) {
            if id.is_some() {
                *w = q.next();
            }
        }
    }
    // what the voice says for a heard phone: the token, else its folded form, else nothing
    let said = |t: &str| {
        if sayable(t) {
            Some(t.to_string())
        } else {
            let f = fold_phone(t);
            (!f.is_empty() && sayable(&f)).then_some(f)
        }
    };
    let heard_said: Vec<Option<String>> = heard.iter().map(|h| said(&h.token)).collect();
    // 2. weighted edit distance, written (rows) against heard (columns)
    let (n, m) = (g2p.len(), heard.len());
    let w = m + 1;
    const GAP: f64 = 0.7;
    let centre = |s: (usize, usize)| (s.0 + s.1) as f64 * 0.5;
    let sub = |i: usize, j: usize| {
        let base = match &heard_said[j] {
            Some(t) => substitution_cost(&g2p[i].0, t),
            None => 0.3,
        };
        let dt = written[i].map_or(0.0, |s| (centre(s) - centre((heard[j].start, heard[j].end))).abs());
        base + (0.015 * dt).min(0.6)
    };
    let mut d = vec![0.0f64; (n + 1) * w];
    for i in 0..=n {
        d[i * w] = i as f64 * GAP;
    }
    for j in 0..=m {
        d[j] = j as f64 * GAP;
    }
    for i in 1..=n {
        for j in 1..=m {
            d[i * w + j] = (d[(i - 1) * w + j - 1] + sub(i - 1, j - 1)).min(d[(i - 1) * w + j] + GAP).min(d[i * w + j - 1] + GAP);
        }
    }
    // the path, back to front: (written index, heard index)
    let mut ops: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && (d[i * w + j] - (d[(i - 1) * w + j - 1] + sub(i - 1, j - 1))).abs() < 1e-9 {
            ops.push((Some(i - 1), Some(j - 1)));
            i -= 1;
            j -= 1;
        } else if i > 0 && (d[i * w + j] - (d[(i - 1) * w + j] + GAP)).abs() < 1e-9 {
            ops.push((Some(i - 1), None));
            i -= 1;
        } else {
            ops.push((None, Some(j - 1)));
            j -= 1;
        }
    }
    ops.reverse();
    // the sequence the voice will say; an inserted phone's word is decided once its neighbours are known
    let mut seq: Vec<AlignedPhone> = Vec::new();
    let mut dropped = Vec::new();
    for (k, &(gi, hj)) in ops.iter().enumerate() {
        match (gi, hj) {
            (Some(gi), Some(hj)) => {
                let (gt, word) = (&g2p[gi].0, g2p[gi].1);
                let (token, origin) = match &heard_said[hj] {
                    Some(t) if substitution_cost(gt, t) == 0.0 => (t.clone(), Origin::Kept),
                    Some(t) => (t.clone(), Origin::Substituted(gt.clone())),
                    None => (gt.clone(), Origin::Kept),
                };
                seq.push(AlignedPhone { token, id: heard[hj].id, word, origin, start: heard[hj].start, end: heard[hj].end });
            }
            (Some(gi), None) => dropped.push((g2p[gi].0.clone(), g2p[gi].1)),
            (None, Some(hj)) => {
                let Some(t) = &heard_said[hj] else { continue };
                let h = &heard[hj];
                // the nearer word: the written neighbours on each side, by the gap in frames
                let before = ops[..k].iter().rev().find_map(|&(g, hh)| g.map(|g| (g2p[g].1, hh.map_or(h.start, |x| heard[x].end))));
                let after = ops[k + 1..].iter().find_map(|&(g, hh)| g.map(|g| (g2p[g].1, hh.map_or(h.end, |x| heard[x].start))));
                let word = match (before, after) {
                    (Some((a, ea)), Some((b, sb))) => {
                        if a == b || h.start.saturating_sub(ea) <= sb.saturating_sub(h.end) { a } else { b }
                    }
                    (Some((a, _)), None) => a,
                    (None, Some((b, _))) => b,
                    (None, None) => 0,
                };
                seq.push(AlignedPhone { token: t.clone(), id: h.id, word, origin: Origin::Inserted, start: h.start, end: h.end });
            }
            (None, None) => {}
        }
    }
    // 3. the said sequence force-aligned (the greedy frames stay if it cannot be); each phone up to the next one
    let said_ids: Vec<usize> = seq.iter().map(|p| p.id).collect();
    if let Some(spans) = ctc_align(&logp, v, &said_ids, blank) {
        for (p, s) in seq.iter_mut().zip(spans) {
            (p.start, p.end) = s;
        }
    }
    for k in 0..seq.len().saturating_sub(1) {
        seq[k].end = seq[k + 1].start.max(seq[k].start + 1);
    }
    LyricAlignment { phones: seq, dropped, heard, written }
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

    #[test]
    fn lyrics_aligned_on_the_audio_which_decides_each_phone() {
        let vocab = s(&["<pad>", "a", "b", "d", "dʒ", "æ", "ə", "ɡ"]);
        // heard: æ d d b (a blank between the two d), the text says ə dʒ | b
        let ids = [0usize, 0, 5, 5, 0, 3, 0, 3, 0, 2, 2, 0];
        let mut logits = vec![0.0f32; ids.len() * vocab.len()];
        for (f, id) in ids.iter().enumerate() {
            logits[f * vocab.len() + id] = 5.0;
        }
        // forced alignment: each token on its frames, the blank compulsory between the equal ones
        let lp = log_softmax(&logits, vocab.len());
        assert_eq!(ctc_align(&lp, vocab.len(), &[5, 3, 3, 2], 0), Some(vec![(2, 4), (5, 6), (7, 8), (9, 11)]));
        assert_eq!(ctc_align(&lp, vocab.len(), &[3; 7], 0), None);
        assert_eq!(vocab_id(&vocab, "g"), Some(7));
        let g2p = vec![("ə".to_string(), 0), ("dʒ".to_string(), 0), ("b".to_string(), 1)];
        let a = align_lyrics(&logits, &vocab, &g2p, 0, |_| true);
        let got: Vec<(&str, usize, Origin, usize, usize)> = a.phones.iter().map(|p| (p.token.as_str(), p.word, p.origin.clone(), p.start, p.end)).collect();
        assert_eq!(got, vec![
            ("æ", 0, Origin::Substituted("ə".into()), 2, 5),
            ("d", 0, Origin::Substituted("dʒ".into()), 5, 7),
            ("d", 0, Origin::Inserted, 7, 9),
            ("b", 1, Origin::Kept, 9, 11),
        ]);
        assert!(a.dropped.is_empty());
        // a heard phone the voice cannot say keeps the written one placed there (a, not ə: by time); a written phone
        // not heard is dropped
        let g2p = vec![("ə".to_string(), 0), ("a".to_string(), 0), ("dʒ".to_string(), 1), ("d".to_string(), 1), ("b".to_string(), 1)];
        let a = align_lyrics(&logits, &vocab, &g2p, 0, |t| t != "æ");
        assert_eq!(a.phones.iter().map(|p| p.token.as_str()).collect::<Vec<_>>(), vec!["a", "d", "d", "b"]);
        assert_eq!(a.dropped, vec![("ə".to_string(), 0)]);
    }
}
