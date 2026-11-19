//! The phone inventory and the token sequence built from it (dizione `symbols.py`).
//!
//! 59 phones of American English, 12 punctuation marks and 5 special tokens. Stress is a parallel feature (0 none,
//! 1 primary, 2 secondary). The order is append-only: tiny-voca's token embedding depends on it.

pub const PAD: &str = "<pad>";
pub const BOS: &str = "<bos>";
pub const EOS: &str = "<eos>";
pub const SP: &str = "<sp>";
pub const UNK: &str = "<unk>";
pub const SPECIALS: [&str; 5] = [PAD, BOS, EOS, SP, UNK];
pub const PUNCTUATION: [&str; 12] = [",", ".", "?", "!", ";", ":", "\"", "'", "(", ")", "—", "…"];
pub const CONSONANTS: [&str; 26] = [
    "p", "b", "t", "d", "k", "ɡ", "f", "v", "θ", "ð", "s", "z", "ʃ", "ʒ", "h", "m", "n", "ŋ", "l", "ɹ", "j", "w", "tʃ",
    "dʒ", "ɾ", "ʔ",
];
pub const VOWELS: [&str; 33] = [
    "ə", "ɐ", "ɚ", "ᵻ", "ɪ", "i", "iː", "ɛ", "æ", "ʌ", "ɑː", "ɔ", "ɔː", "ʊ", "uː", "ɜː", "eɪ", "aɪ", "ɔɪ", "oʊ", "aʊ",
    "iə", "oː", "ɑːɹ", "ɔːɹ", "oːɹ", "ɪɹ", "ɛɹ", "ʊɹ", "aɪɚ", "aɪə", "əl", "n̩",
];

/// One phone of the inventory and its stress (0, 1 or 2).
pub type Ph = (&'static str, u8);

/// The phone of the inventory spelled `text`, as a static string.
pub fn phone(text: &str) -> Option<&'static str> {
    CONSONANTS.iter().chain(VOWELS.iter()).find(|p| **p == text).copied()
}
pub fn is_phone(text: &str) -> bool {
    phone(text).is_some()
}
pub fn is_vowel(text: &str) -> bool {
    VOWELS.contains(&text)
}
/// All symbols in id order: specials, punctuation, consonants, vowels.
pub fn symbols() -> Vec<&'static str> {
    SPECIALS.iter().chain(PUNCTUATION.iter()).chain(CONSONANTS.iter()).chain(VOWELS.iter()).copied().collect()
}
/// The id tiny-voca's encoder takes for `symbol`.
pub fn symbol_id(symbol: &str) -> Option<u32> {
    SPECIALS
        .iter()
        .chain(PUNCTUATION.iter())
        .chain(CONSONANTS.iter())
        .chain(VOWELS.iter())
        .position(|s| *s == symbol)
        .map(|i| i as u32)
}

/// What the tokenizer reads: a punctuation mark, or the spoken words of a clause.
#[derive(Clone, Debug)]
pub enum Piece {
    Punct(String),
    Words(Vec<Vec<Ph>>),
}

/// The token sequence of one text: what tiny-voca's `synthesize(ids, stress)` takes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Phonemized {
    pub symbols: Vec<String>,
    pub ids: Vec<u32>,
    pub stress: Vec<u8>,
    /// Phones outside the inventory (never produced by the built-in rules).
    pub unknown: Vec<String>,
    /// For each spoken word, in order, the token range `[start, end)` of its phones.
    pub words: Vec<(usize, usize)>,
}

impl Phonemized {
    /// The sequence as text: stress marks on the phones (`<bos> h ə l ˈoʊ <eos>`).
    pub fn marked(&self) -> String {
        self.symbols
            .iter()
            .zip(&self.stress)
            .map(|(s, &l)| format!("{}{s}", ["", "ˈ", "ˌ"][l.min(2) as usize]))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Error of the tokenizer and the phonemizer, as dizione's `ValueError`.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueError(pub String);

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ValueError {}

/// Builds the token sequence: words separated by `<sp>`, repeated punctuation collapsed (`symbols.tokenize`).
pub fn tokenize(pieces: &[Piece]) -> Result<Phonemized, ValueError> {
    let mut symbols: Vec<String> = vec![BOS.into()];
    let mut stress = vec![0u8];
    let mut unknown = Vec::new();
    let mut words = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Punct(mark) => {
                if symbols.last().unwrap() != mark {
                    symbols.push(mark.clone());
                    stress.push(0);
                }
            }
            Piece::Words(value) => {
                for word in value {
                    let last = symbols.last().unwrap();
                    if is_phone(last) || last == UNK {
                        symbols.push(SP.into());
                        stress.push(0);
                    }
                    let start = symbols.len();
                    for &(ph, level) in word {
                        if is_phone(ph) {
                            symbols.push(ph.into());
                            stress.push(level);
                        } else {
                            unknown.push(ph.to_string());
                            symbols.push(UNK.into());
                            stress.push(0);
                        }
                    }
                    words.push((start, symbols.len()));
                }
            }
        }
    }
    symbols.push(EOS.into());
    stress.push(0);
    if !symbols.iter().any(|s| is_phone(s) || s == UNK) {
        return Err(ValueError("the text has nothing to pronounce".into()));
    }
    let ids = symbols.iter().map(|s| symbol_id(s).unwrap()).collect();
    Ok(Phonemized { symbols, ids, stress, unknown, words })
}
