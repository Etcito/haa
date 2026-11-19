//! English text to tiny-voca phoneme tokens (crate `dizione`, a Rust port of n4ze3m/dizione, Apache-2.0).
//!
//! The data (5.4 MB) is never in the module: the host fetches it from the tiny-voca Hugging Face repo and hands it
//! over once, either
//! * file by file, `haa_dizione_file(name_ptr, name_len, data_ptr, data_len)` for each of `lexicon.tsv` (or
//!   `lexicon.packed.gz`), `align.json`, `g2p.json`, `g2p.npz`, `homographs.json` of the `phonemizer/` folder (a
//!   table such as `exceptions.txt` given the same way replaces the built-in copy), then `haa_dizione_init()`; or
//! * `haa_dizione_gguf(ptr, len)` with tiny-voca's one-file `gguf/tiny-voca-js.gguf`.
//!
//! Both return 1 when the phonemizer is ready, 0 otherwise (the result then holds `{"error"}`).
//!
//! `haa_dizione_phonemize(ptr, len)`: request `{"text"}` (plain text, script markup already removed); result
//! `{"ids", "stress", "symbols", "words": [{"word", "raw", "phones", "stress", "source", "tokens": [start, end],
//! "span": [start, end] | null}]}`, `ids` and `stress` exactly what tiny-voca's encoder takes, `tokens` a word's
//! range in `ids`, `span` its UTF-8 byte range in the text (null when not written as said, e.g. "$3"). Returns the
//! result length.

use std::collections::HashMap;
use std::sync::Mutex;

use ::dizione::Phonemizer;
use serde_json::{Value, json};

use crate::{MemPtr, set_result};

static FILES: Mutex<Option<HashMap<String, Vec<u8>>>> = Mutex::new(None);
static PHONEMIZER: Mutex<Option<Phonemizer>> = Mutex::new(None);

fn bytes<'a>(ptr: MemPtr, len: u32) -> &'a [u8] {
    if ptr == 0 || len == 0 {
        &[]
    } else {
        // SAFETY: the host passes a buffer from `haa_alloc` that it filled, `len` bytes long.
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
    }
}

fn ready(built: Result<Phonemizer, String>) -> u32 {
    match built {
        Ok(p) => {
            *PHONEMIZER.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
            set_result(json!({ "ready": true }).to_string());
            1
        }
        Err(e) => {
            set_result(json!({ "error": e }).to_string());
            0
        }
    }
}

/// Keeps one data file until `haa_dizione_init`; returns the number of files held.
#[unsafe(no_mangle)]
pub extern "C" fn haa_dizione_file(name_ptr: MemPtr, name_len: u32, data_ptr: MemPtr, data_len: u32) -> u32 {
    let name = String::from_utf8_lossy(bytes(name_ptr, name_len)).to_string();
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let held = files.get_or_insert_with(HashMap::new);
    held.insert(name, bytes(data_ptr, data_len).to_vec());
    held.len() as u32
}

/// Builds the phonemizer from the files given so far (and lets them go).
#[unsafe(no_mangle)]
pub extern "C" fn haa_dizione_init() -> u32 {
    let files = FILES.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
    ready(Phonemizer::from_files(&files))
}

/// Builds the phonemizer from tiny-voca's one-file GGUF.
#[unsafe(no_mangle)]
pub extern "C" fn haa_dizione_gguf(ptr: MemPtr, len: u32) -> u32 {
    ready(Phonemizer::from_gguf(bytes(ptr, len)))
}

fn phonemize_json(p: &Phonemizer, v: &Value) -> Value {
    let Some(text) = v.get("text").and_then(Value::as_str) else {
        return json!({ "error": "expected {text}" });
    };
    match p.g2p(text) {
        Err(e) => json!({ "error": e.0 }),
        Ok(r) => json!({
            "ids": r.tokens.ids,
            "stress": r.tokens.stress,
            "symbols": r.tokens.symbols,
            "words": r.words.iter().map(|w| json!({
                "word": w.word,
                "raw": w.raw,
                "phones": w.phones,
                "stress": w.stress,
                "source": w.source,
                "tokens": [w.tokens.0, w.tokens.1],
                "span": w.span.map(|(a, b)| [a, b]),
            })).collect::<Vec<_>>(),
        }),
    }
}

/// Writes the tokens JSON (or `{"error"}`) into the result and returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn haa_dizione_phonemize(ptr: MemPtr, len: u32) -> u32 {
    let guard = PHONEMIZER.lock().unwrap_or_else(|e| e.into_inner());
    let out = match (guard.as_ref(), serde_json::from_slice::<Value>(bytes(ptr, len))) {
        (None, _) => json!({ "error": "no data: call haa_dizione_file + haa_dizione_init, or haa_dizione_gguf" }),
        (_, Err(_)) => json!({ "error": "expected {text}" }),
        (Some(p), Ok(v)) => phonemize_json(p, &v),
    }
    .to_string();
    let n = out.len() as u32;
    set_result(out);
    n
}
