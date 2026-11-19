//! dizione in Rust: English text to the phoneme tokens and stress of tiny-voca.
//!
//! A port of dizione (github n4ze3m/dizione, Apache-2.0, Muhammed Nazeem), the front end tiny-voca's JS page uses:
//! CMUdict with rules for flaps, reduced vowels and r-coloured vowels, regular endings and compounds, Wikipedia
//! homographs, text rules for numbers, money, dates, units, abbreviations and chat forms, and a small model for
//! unknown words. It gives exactly the tokens dizione gives (`symbols.rs`: 5 specials, 12 marks, 59 phones; stress
//! 0/1/2 as a parallel array), so tiny-voca's encoder takes them as they are.
//!
//! The built data (dictionary, alignment table, homograph weights, unknown-word model, 5.4 MB) is never bundled: the
//! host loads it at run time (`Data::from_files` over the `phonemizer/` folder of the tiny-voca Hugging Face repo,
//! or `Data::from_gguf` over tiny-voca's one-file `tiny-voca-js.gguf`). The hand-written tables (31 KB) are compiled
//! in from `data/`, copied unchanged from dizione.
//!
//! Script markup (`[angry,5]`, `*word*`, `|`) is handled before this crate; `g2p` returns, for every spoken word,
//! its token range and where it is written in the input, so emphasis can be mapped onto tokens.

pub mod align;
pub mod convert;
pub mod data;
pub mod engine;
pub mod g2p;
pub mod homographs;
pub mod morphology;
pub mod normalize;
pub mod pyre;
pub mod sentence;
pub mod symbols;
pub mod tables;
pub mod utterances;

use std::collections::HashMap;
use std::sync::Mutex;

pub use data::Data;
pub use engine::{Part, Phonemizer};
pub use symbols::{Phonemized, ValueError};

/// The phonemizer the host loaded for this process (haa-wasm's `haa_dizione_*` or `haa_tinyvoca_load` with a
/// one-file GGUF), shared by every `speak` node given text. Not `Sync` (it caches words), hence the lock.
static SHARED: Mutex<Option<Phonemizer>> = Mutex::new(None);

/// Installs `phonemizer` as the shared one (replacing any earlier one).
pub fn install(phonemizer: Phonemizer) {
    *SHARED.lock().unwrap_or_else(|p| p.into_inner()) = Some(phonemizer);
}

/// Runs `f` on the shared phonemizer; `None` when the host loaded none.
pub fn with_shared<R>(f: impl FnOnce(&Phonemizer) -> R) -> Option<R> {
    SHARED.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map(f)
}

/// True once a shared phonemizer is installed.
pub fn installed() -> bool {
    SHARED.lock().unwrap_or_else(|p| p.into_inner()).is_some()
}

/// One spoken word: the phones of a word or of words said as one (a hyphenated compound, a phrase).
#[derive(Clone, Debug, PartialEq)]
pub struct SpokenWord {
    /// Spelling in lower case; the parts of a compound joined by `-`, the words of a phrase by a space.
    pub word: String,
    /// As written in the normalised text (digits for a number, the letters of an initialism...).
    pub raw: String,
    pub phones: Vec<&'static str>,
    pub stress: Vec<u8>,
    /// Where the pronunciation came from: dictionary, ending, model, function, phrase, homograph, letters, name,
    /// inline, override (several, joined by `+`, for a compound).
    pub source: String,
    /// Token range `[start, end)` of the phones in `Phonemized::ids`.
    pub tokens: (usize, usize),
    /// Byte range of the word in the input text, when it is written there as it is said (not for "$3", "Dr."...).
    pub span: Option<(usize, usize)>,
}

/// `g2p` result: the spoken words and the token sequence they make.
#[derive(Clone, Debug, PartialEq)]
pub struct G2p {
    pub words: Vec<SpokenWord>,
    pub tokens: Phonemized,
}

impl Phonemizer {
    /// From the files of the `phonemizer/` folder (`lexicon.tsv`, `align.json`, `g2p.json`, `g2p.npz`,
    /// `homographs.json`), by file name.
    pub fn from_files(files: &HashMap<String, Vec<u8>>) -> Result<Phonemizer, String> {
        Phonemizer::new(Data::from_files(files)?)
    }

    /// From tiny-voca's one-file GGUF (`gguf/tiny-voca-js.gguf`).
    pub fn from_gguf(bytes: &[u8]) -> Result<Phonemizer, String> {
        Phonemizer::new(Data::from_gguf(bytes)?)
    }

    /// Text -> spoken words with their phones and stress, and the exact token and stress arrays tiny-voca takes.
    pub fn g2p(&self, text: &str) -> Result<G2p, ValueError> {
        if pyre::strip(text).is_empty() {
            return Err(ValueError("text must not be empty".into()));
        }
        let parts = self.analyse(text);
        let mut pieces = Vec::new();
        let mut words = Vec::new();
        let mut cursor = 0;
        for part in &parts {
            match part {
                Part::Punct(m) => pieces.push(symbols::Piece::Punct(m.clone())),
                Part::Clause(units) => {
                    let groups = sentence::groups(units);
                    for (phones, members) in &groups {
                        let mut span: Option<(usize, usize)> = None;
                        for &k in members {
                            if let Some((a, b)) = locate(text, &units[k].raw, &mut cursor) {
                                span = Some(span.map_or((a, b), |(s, e)| (s.min(a), e.max(b))));
                            }
                        }
                        let join = |f: &dyn Fn(&sentence::Unit) -> &str, sep: &str| members.iter().map(|&k| f(&units[k])).collect::<Vec<_>>().join(sep);
                        words.push(SpokenWord {
                            word: join(&|u| u.word.as_str(), "-"),
                            raw: join(&|u| u.raw.as_str(), ""),
                            phones: phones.iter().map(|p| p.0).collect(),
                            stress: phones.iter().map(|p| p.1).collect(),
                            source: join(&|u| u.source.as_str(), "+"),
                            tokens: (0, 0),
                            span,
                        });
                    }
                    pieces.push(symbols::Piece::Words(groups.into_iter().map(|(g, _)| g).collect()));
                }
            }
        }
        let tokens = symbols::tokenize(&pieces)?;
        for (word, range) in words.iter_mut().zip(&tokens.words) {
            word.tokens = *range;
        }
        Ok(G2p { words, tokens })
    }
}

/// The next place at or after `cursor` where `raw` is written as a whole word (ASCII case ignored).
fn locate(text: &str, raw: &str, cursor: &mut usize) -> Option<(usize, usize)> {
    if raw.is_empty() || raw.contains(' ') {
        return None;
    }
    let hay = text.get(*cursor..)?.to_ascii_lowercase();
    let needle = raw.to_ascii_lowercase();
    let word_char = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '\'');
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        let (a, b) = (*cursor + from + i, *cursor + from + i + needle.len());
        if !word_char(text[..a].chars().next_back()) && !word_char(text[b..].chars().next()) {
            *cursor = b;
            return Some((a, b));
        }
        from += i + needle.len().max(1);
    }
    None
}
