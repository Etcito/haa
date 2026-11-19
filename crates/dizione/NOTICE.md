# Provenance of this crate

A Rust port of **dizione** (https://github.com/n4ze3m/dizione), "English text to phonemes", Apache License 2.0,
by Muhammed Nazeem (see LICENSE in this folder). This crate is under the same licence; the rest of haa is MIT.

- Source revision: 0d39f7cf06dfcbbc2ed5617cb95bebf1fd9723d3 (branch `fix/dotted-acronyms`, 2026-11-17: letters with
  full stops read as one acronym), which is what tiny-voca's JS port (`js/dizione/`) implements, plus the JS port's one
  difference: "p." after a space and before a number is a page, not pence (`normalize::Normalizer::measure`).
- Ported: every module of `src/dizione` (normalize, engine, sentence, convert, morphology, align, homographs, g2p,
  symbols, tables) except the CLI and the build scripts that make the data. Regular expressions keep Python's text,
  translated at start-up (`pyre.rs`) and run by `fancy-regex` (look-behind and back-references, which `regex` lacks).
  `normalize.py`'s one conditional group (`_GROUPED`) is written as the equivalent alternation.
- Copied unchanged: the hand-written tables in `data/` (abbreviations, chat, endings, exceptions, function_words,
  homograph_rules, letters, phrases, units; 31 KB), compiled in. A table file handed to `Data` replaces its copy.
- Not here: the built data (CMUdict lexicon, alignment table, homograph weights, unknown-word model, 5.4 MB). The host
  loads it at run time from the `phonemizer/` folder of the tiny-voca Hugging Face repo, or from tiny-voca's one-file
  `tiny-voca-js.gguf` (its embedded files and `g2p/*` tensors).
- Parity: `examples/dizione.rs` writes the same lines as `js/test/dizione.mjs` (`ids`, `units`, `words`); on 3,189
  texts (dizione's tricky sentences and test strings, tiny-voca's sentence vectors, generated numbers, money, dates,
  times, units, abbreviations, codes, homographs, unknown words, NPC lines, and 2,000 random splices of them) the ids
  and stress equal dizione's Python token for token.
