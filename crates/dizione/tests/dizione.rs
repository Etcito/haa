//! The crate's checks: tokens and text rules without data, and, when `DIZIONE_DATA` names the `phonemizer/` folder
//! of tiny-voca, whole sentences against dizione's Python (the full parity run is `examples/dizione.rs`).

use std::collections::HashMap;

use dizione::normalize::{Normalizer, Tables, Token};
use dizione::symbols::{tokenize, Piece};
use dizione::{convert, tables, Phonemizer};

#[test]
fn tokens_and_text_rules_without_data() {
    let hello = convert::from_text("h ə l ˈoʊ").unwrap();
    let r = tokenize(&[Piece::Words(vec![hello.clone(), hello]), Piece::Punct(".".into())]).unwrap();
    assert_eq!(r.ids, [1, 31, 43, 35, 62, 3, 31, 43, 35, 62, 6, 2]);
    assert_eq!(r.stress, [0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0]);
    assert_eq!(r.words, [(1, 5), (6, 10)]);

    let t = Tables::new(tables::ABBREVIATIONS, tables::UNITS, tables::CHAT, tables::ENDINGS).unwrap();
    let no_words = |_: &str| false;
    let said: Vec<String> = Normalizer::new(&t, &no_words)
        .tokens("Dr. Smith paid $3.50 on 12/25/2023, e.g. at 9:30 pm.")
        .iter()
        .map(|t| match t {
            Token::Word(w) => w.text.clone(),
            Token::Punct(m) => m.clone(),
        })
        .collect();
    assert_eq!(
        said.join(" "),
        "doctor smith paid three dollars and fifty cents on december twenty fifth twenty twenty three , for example at nine thirty pm ."
    );
}

#[test]
fn sentences_match_dizione() {
    let Ok(dir) = std::env::var("DIZIONE_DATA") else {
        eprintln!("DIZIONE_DATA not set: skipped");
        return;
    };
    let mut files = HashMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        files.insert(path.file_name().unwrap().to_string_lossy().to_string(), std::fs::read(&path).unwrap());
    }
    let p = Phonemizer::from_files(&files).unwrap();
    let cases: [(&str, &[u32], &[u8]); 2] = [
        (
            "I will read it; I read it yesterday.",
            &[1, 60, 3, 38, 47, 35, 3, 36, 49, 20, 3, 47, 19, 9, 60, 3, 36, 50, 20, 3, 47, 19, 3, 37, 50, 27, 19, 45, 20, 59, 6, 2],
            &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 2, 0, 0],
        ),
        (
            "Kayleigh met Zorblax at 9:30 pm.",
            &[1, 21, 59, 35, 48, 3, 32, 50, 19, 3, 28, 67, 18, 35, 51, 21, 27, 3, 51, 19, 3, 33, 60, 33, 3, 25, 58, 41, 48, 3, 17, 49, 50, 32, 6, 2],
            &[0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 2, 1, 0, 0, 0],
        ),
    ];
    for (text, ids, stress) in cases {
        let r = p.phonemize(text).unwrap();
        assert_eq!((r.ids.as_slice(), r.stress.as_slice()), (ids, stress), "{text}");
    }
    // word boundaries for emphasis: the token range and the place in the text of each spoken word
    let g = p.g2p("Kayleigh met Zorblax at 9:30 pm.").unwrap();
    let k = &g.words[0];
    assert_eq!((k.word.as_str(), k.source.as_str(), k.tokens, k.span), ("kayleigh", "model", (1, 5), Some((0, 8))));
    assert_eq!(g.words[2].span, Some((13, 20)));
    assert_eq!(g.words[4].word, "nine");
    assert_eq!(g.words[4].span, None);
}
