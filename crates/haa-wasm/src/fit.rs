//! Fit export group of the wasm ABI: the brute-force graph fitter (`haa-fit`) for a Worker.
//!
//! One global fit per instance (a Worker owns its instance). The host copies the mono target into
//! an f32 buffer from `haa_fit_alloc_f32`, starts a fit with a JSON config, then calls
//! `haa_fit_step(n)` (at most `n` renders per call) until it returns 0, reading progress from the
//! result buffer between calls; cancelling is simply not calling `step` again (`haa_fit_free`).
//!
//! Exports (pointers and lengths are `u32` offsets into `memory`, text is UTF-8 JSON in the result):
//! - `haa_fit_alloc_f32(frames) -> ptr`, `haa_fit_free_f32(ptr, frames)`: 4-byte aligned f32 buffers;
//! - `haa_fit_start(pcm_ptr, frames, sample_rate, cfg_ptr, cfg_len) -> i32`: 1 started, 0 error
//!   (result `{"error": ...}`). Config: `{"templates": [patch, ...], ...FitOptions keys}` where a
//!   template is a patch object or a patch JSON string. The target is copied, so the host may free it;
//! - `haa_fit_step(n) -> i32`: 1 running, 0 done, −1 no fit; result = progress JSON;
//! - `haa_fit_progress() -> u32`, `haa_fit_result() -> u32`, `haa_fit_patch() -> u32`: progress,
//!   full result, best patch text (pretty HAA patch JSON) into the result; return its length
//!   (0 when there is no fit / no patch yet);
//! - `haa_fit_free()`: drops the fit;
//! - `haa_fit_describe(ptr, len, max_dims) -> i32`: 1 and the fit-able params JSON, or 0 and `{"error"}`;
//! - `haa_fit_compare(a_ptr, a_frames, b_ptr, b_frames, sample_rate) -> u32`: loss terms plus level
//!   and brightness summaries of two mono buffers (target first), JSON length;
//! - `haa_spec_target(pcm_ptr, frames, sample_rate, notes_ptr, notes_len) -> i32`: analyses a stem
//!   and its notes (`[{"start","end","midi"}]`) once for the spectral judge (1 ok, 0 error);
//! - `haa_spec_score(pcm_ptr, frames) -> u32`: spectral terms of a render against that stem, JSON
//!   length (0 without a target); `haa_spec_free()` drops the target;
//! - `haa_phone_per(ref_ptr, ref_len, hyp_ptr, hyp_len, vocab_ptr, vocab_len) -> u32`: the diction judge. Two CTC
//!   phoneme recognisers' logits (flat f32 rows of the vocabulary's size, the stem's first) and the recogniser's
//!   `vocab.json`: `{"reference": [[phone, start, end], ...], "hypothesis": [...], "per": {...}}` (frames, end
//!   exclusive), JSON length (0 when the vocabulary is not a `{"token": id}` object).
//! - `haa_phone_align(logits_ptr, logits_len, vocab_ptr, vocab_len, req_ptr, req_len) -> u32`: a line's English
//!   words timed on its audio from the recogniser's logits (G2P, forced alignment, the heard phones winning), JSON
//!   length (0 on a bad vocabulary or request).
//! - `haa_artic_curves(ema_ptr, ema_len, req_ptr, req_len) -> u32`: a phrase's articulator curves for the chimera
//!   `articulate` mode from SPARC's EMA (flat f32 rows of 12 at 50 Hz), with no text, JSON length (0 on a bad request).

use std::sync::Mutex;

use haa_core::nodes::chimera::{ipa_sayable, lyric_g2p};
use haa_fit::articulatory::{ARTIC_INPUTS, artic_curves};
use haa_fit::phonemes::{Origin, Phone, align_lyrics, ctc_greedy, parse_vocab, phone_error_rate, render_lead, tokens};
use haa_fit::{FitOptions, Fitter, SpectralTarget, SpectralWeights, parse_notes};
use serde_json::Value;

use crate::{MemPtr, set_result};

static FIT: Mutex<Option<Fitter>> = Mutex::new(None);
static SPEC: Mutex<Option<SpectralTarget>> = Mutex::new(None);

fn with_spec<R>(f: impl FnOnce(&mut Option<SpectralTarget>) -> R) -> R {
    let mut spec = SPEC.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut spec)
}

fn with_fit<R>(f: impl FnOnce(&mut Option<Fitter>) -> R) -> R {
    let mut fit = FIT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut fit)
}

fn set_len(s: String) -> u32 {
    let len = s.len() as u32;
    set_result(s);
    len
}

fn error(message: &str) -> i32 {
    set_result(serde_json::json!({ "error": message }).to_string());
    0
}

/// # Safety
/// `ptr`/`frames` must describe a live f32 buffer the host owns, or be 0/0.
unsafe fn f32_at<'a>(ptr: MemPtr, frames: u32) -> &'a [f32] {
    if ptr == 0 || frames == 0 {
        return &[];
    }
    // SAFETY: the host passes a buffer from `haa_fit_alloc_f32` (aligned, `frames` long).
    unsafe { std::slice::from_raw_parts(ptr as *const f32, frames as usize) }
}

/// # Safety
/// `ptr`/`len` must describe a live byte buffer the host owns, or be 0/0.
unsafe fn bytes_at<'a>(ptr: MemPtr, len: u32) -> &'a [u8] {
    if ptr == 0 || len == 0 {
        return &[];
    }
    // SAFETY: the host passes a buffer from `haa_alloc` that it filled.
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
}

/// Allocates a zeroed, 4-byte aligned buffer of `frames` f32 samples (0 for 0 frames).
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_alloc_f32(frames: u32) -> MemPtr {
    if frames == 0 {
        return 0;
    }
    let buffer = vec![0.0f32; frames as usize].into_boxed_slice();
    Box::into_raw(buffer) as *mut f32 as MemPtr
}

/// Frees a buffer from `haa_fit_alloc_f32`; `ptr`/`frames` must be the pair it was made with.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_free_f32(ptr: MemPtr, frames: u32) {
    if ptr == 0 || frames == 0 {
        return;
    }
    // SAFETY: exactly the pointer and length `haa_fit_alloc_f32` returned, freed once.
    unsafe {
        let slice = std::slice::from_raw_parts_mut(ptr as *mut f32, frames as usize);
        drop(Box::from_raw(slice as *mut [f32]));
    }
}

/// Starts a fit (replacing any previous one). 1 started, 0 error (`{"error"}` in the result).
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_start(pcm_ptr: MemPtr, frames: u32, sample_rate: f64, cfg_ptr: MemPtr, cfg_len: u32) -> i32 {
    // SAFETY: host buffers per the ABI.
    let pcm = unsafe { f32_at(pcm_ptr, frames) };
    let cfg_bytes = unsafe { bytes_at(cfg_ptr, cfg_len) };
    let Ok(cfg) = serde_json::from_slice::<Value>(cfg_bytes) else {
        return error("config is not JSON");
    };
    let templates: Vec<String> = match cfg.get("templates").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .map(|t| match t {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        None => return error("config needs a templates array"),
    };
    let options = FitOptions::from_json(&cfg);
    match Fitter::new(pcm, sample_rate, &templates, options) {
        Ok(fitter) => {
            let progress = fitter.progress_json();
            with_fit(|fit| *fit = Some(fitter));
            set_result(progress);
            1
        }
        Err(reason) => {
            with_fit(|fit| *fit = None);
            error(&reason)
        }
    }
}

/// Runs at most `n` evaluations. 1 running, 0 done, −1 no fit; the result holds the progress JSON.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_step(n: u32) -> i32 {
    let outcome = with_fit(|fit| fit.as_mut().map(|f| (f.step(n as usize), f.progress_json())));
    match outcome {
        Some((running, progress)) => {
            set_result(progress);
            i32::from(running)
        }
        None => -1,
    }
}

/// Writes the progress JSON into the result; 0 when there is no fit.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_progress() -> u32 {
    with_fit(|fit| fit.as_ref().map(Fitter::progress_json)).map_or(0, set_len)
}

/// Writes the full result JSON (best patch, scores, report) into the result; 0 when there is no fit.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_result() -> u32 {
    with_fit(|fit| fit.as_mut().map(Fitter::result_json)).map_or(0, set_len)
}

/// Writes the best patch (pretty HAA patch JSON) into the result; 0 before the first render.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_patch() -> u32 {
    with_fit(|fit| fit.as_ref().and_then(Fitter::best_patch_json)).map_or(0, set_len)
}

/// Drops the current fit.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_free() {
    with_fit(|fit| *fit = None);
}

/// Writes the fit-able params of a patch (`max_dims` marked selected). 1 ok, 0 error.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_describe(ptr: MemPtr, len: u32, max_dims: u32) -> i32 {
    // SAFETY: host buffer per the ABI.
    let bytes = unsafe { bytes_at(ptr, len) };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return error("patch is not UTF-8");
    };
    match haa_fit::describe_params(text, max_dims as usize) {
        Ok(json) => {
            set_result(json);
            1
        }
        Err(reason) => error(&reason),
    }
}

/// Compares two mono buffers at one rate (target first); writes the JSON and returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn haa_fit_compare(a_ptr: MemPtr, a_frames: u32, b_ptr: MemPtr, b_frames: u32, sample_rate: f64) -> u32 {
    // SAFETY: host buffers per the ABI.
    let (a, b) = unsafe { (f32_at(a_ptr, a_frames), f32_at(b_ptr, b_frames)) };
    if a.is_empty() || !(sample_rate.is_finite() && sample_rate > 0.0) {
        return 0;
    }
    set_len(haa_fit::compare(a, b, sample_rate, 16).to_string())
}

/// Analyses a stem and its notes for `haa_spec_score`. 1 ok, 0 error (`{"error"}` in the result).
#[unsafe(no_mangle)]
pub extern "C" fn haa_spec_target(pcm_ptr: MemPtr, frames: u32, sample_rate: f64, notes_ptr: MemPtr, notes_len: u32) -> i32 {
    // SAFETY: host buffers per the ABI.
    let (pcm, notes) = unsafe { (f32_at(pcm_ptr, frames), bytes_at(notes_ptr, notes_len)) };
    if pcm.is_empty() || !(sample_rate.is_finite() && sample_rate > 0.0) {
        return error("empty target or bad sample rate");
    }
    let notes = parse_notes(std::str::from_utf8(notes).unwrap_or("[]"));
    let target = SpectralTarget::new(pcm, sample_rate, &notes, SpectralWeights::default());
    set_result(serde_json::json!({ "notes": target.note_count() }).to_string());
    with_spec(|spec| *spec = Some(target));
    1
}

/// Scores a render against the stem of `haa_spec_target`; JSON length, 0 without a target.
#[unsafe(no_mangle)]
pub extern "C" fn haa_spec_score(pcm_ptr: MemPtr, frames: u32) -> u32 {
    // SAFETY: host buffer per the ABI.
    let pcm = unsafe { f32_at(pcm_ptr, frames) };
    with_spec(|spec| spec.as_mut().map(|t| t.score(pcm).to_json().to_string())).map_or(0, set_len)
}

/// Drops the spectral judge's target.
#[unsafe(no_mangle)]
pub extern "C" fn haa_spec_free() {
    with_spec(|spec| *spec = None);
}

/// The diction judge: the phones a CTC recogniser heard in a stem and in a render (greedy decoding of their logits,
/// blank `<pad>` = 0) and the render's phoneme error rate against the stem (haa-fit `phonemes`). JSON length, 0 when
/// the vocabulary is not a `{"token": id}` object.
#[unsafe(no_mangle)]
pub extern "C" fn haa_phone_per(ref_ptr: MemPtr, ref_len: u32, hyp_ptr: MemPtr, hyp_len: u32, vocab_ptr: MemPtr, vocab_len: u32) -> u32 {
    // SAFETY: host buffers per the ABI.
    let (reference, hyp, vocab) = unsafe { (f32_at(ref_ptr, ref_len), f32_at(hyp_ptr, hyp_len), bytes_at(vocab_ptr, vocab_len)) };
    let Some(vocab) = std::str::from_utf8(vocab).ok().and_then(parse_vocab) else { return 0 };
    let (r, h) = (ctc_greedy(reference, &vocab, 0), ctc_greedy(hyp, &vocab, 0));
    let list = |p: &[Phone]| p.iter().map(|x| serde_json::json!([x.token, x.start, x.end])).collect::<Vec<_>>();
    let per = phone_error_rate(&tokens(&r), &tokens(&h)).to_json();
    set_len(serde_json::json!({ "reference": list(&r), "hypothesis": list(&h), "per": per }).to_string())
}

/// The lyrics timed on the audio: a CTC phoneme recogniser's logits of one sung or spoken line (flat f32 rows of the
/// vocabulary's size), its `vocab.json`, and a request `{"words": "english words", "frame": seconds per logits row
/// (0.02), "offset": seconds of row 0}`. The words go through the lyrics voice's G2P, are force-aligned on the
/// posteriors and matched with what the recogniser heard, the audio deciding each phone (haa-fit
/// `phonemes::align_lyrics`, phones the chimera voice can say). JSON length, 0 when the vocabulary or the request is
/// not valid. The result: `{"words": [IPA per word as the voice will say it], "said": [english words], "phones":
/// [{"ipa", "word", "start", "end", "origin": "kept"|"substituted"|"inserted", "written"?}], "dropped": [{"ipa",
/// "word"}], "heard": [[phone, start, end]], "written": [[phone, start, end]]}` in seconds (-1 where a written phone
/// could not be placed).
///
/// With `"phones": [[ipa, stem start s], ...]` (a render of phones the stem says at those times) and `"lead"` (seconds
/// the line is played early so far) in place of `words`: those phones force-aligned on the render's logits, and
/// `{"written", "lead": the line's new lead, "lag": mean |render - stem| start (s) of the phones the render is heard saying
/// as written, "heard": how many, "score": the mean lag with each other phone counted 0.12 s}` (haa-fit `render_lead`,
/// leads within 0.12 s).
#[unsafe(no_mangle)]
pub extern "C" fn haa_phone_align(logits_ptr: MemPtr, logits_len: u32, vocab_ptr: MemPtr, vocab_len: u32, req_ptr: MemPtr, req_len: u32) -> u32 {
    // SAFETY: host buffers per the ABI.
    let (logits, vocab, req) = unsafe { (f32_at(logits_ptr, logits_len), bytes_at(vocab_ptr, vocab_len), bytes_at(req_ptr, req_len)) };
    let Some(vocab) = std::str::from_utf8(vocab).ok().and_then(parse_vocab) else { return 0 };
    let Some(req) = serde_json::from_slice::<Value>(req).ok() else { return 0 };
    let frame = req.get("frame").and_then(Value::as_f64).filter(|f| *f > 0.0).unwrap_or(0.02);
    let offset = req.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
    let sec = |f: usize| ((offset + f as f64 * frame) * 1000.0).round() / 1000.0;
    if let Some(phones) = req.get("phones").and_then(Value::as_array) {
        let given: Vec<(String, f64)> = phones.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_f64()?))).collect();
        let g2p: Vec<(String, usize)> = given.iter().map(|(p, _)| (p.clone(), 0)).collect();
        let a = align_lyrics(logits, &vocab, &g2p, 0, ipa_sayable);
        let lead = req.get("lead").and_then(Value::as_f64).unwrap_or(0.0);
        // only the phones the render is heard saying as written time it (a phone it garbles has no time)
        let mut at: Vec<Option<f64>> = vec![None; given.len()];
        for p in &a.phones {
            if let (Origin::Kept, Some(k)) = (&p.origin, p.source) {
                at[k] = Some(offset + p.start as f64 * frame);
            }
        }
        let stem: Vec<f64> = given.iter().map(|g| g.1).collect();
        let (lead, lag) = render_lead(&stem, &at, lead, 0.12);
        let written: Vec<Value> = given
            .iter()
            .zip(&a.written)
            .map(|((p, _), s)| match s {
                Some((s0, s1)) => serde_json::json!([p, sec(*s0), sec(*s1)]),
                None => serde_json::json!([p, -1, -1]),
            })
            .collect();
        let r = |x: f64| (x * 1e4).round() / 1e4;
        let heard = at.iter().flatten().count();
        // the render's score: its mean lag with a phone it garbles counted as the largest lead (0.12 s)
        let score = (lag * heard as f64 + 0.12 * (given.len() - heard) as f64) / given.len().max(1) as f64;
        let out = serde_json::json!({ "written": written, "lead": r(lead), "lag": r(lag), "heard": heard, "score": r(score) });
        return set_len(out.to_string());
    }
    let Some(words) = req.get("words").and_then(Value::as_str) else { return 0 };
    let (g2p, said) = lyric_g2p(words);
    let a = align_lyrics(logits, &vocab, &g2p, 0, ipa_sayable);
    // the words as the voice will say them: each word's phones in order (a word the audio drops entirely is left out)
    let mut by_word: Vec<(usize, String)> = Vec::new();
    for p in &a.phones {
        match by_word.last_mut() {
            Some((w, text)) if *w == p.word => text.push_str(&p.token),
            _ => by_word.push((p.word, p.token.clone())),
        }
    }
    let phones: Vec<Value> = a
        .phones
        .iter()
        .map(|p| {
            let (origin, written) = match &p.origin {
                Origin::Kept => ("kept", None),
                Origin::Substituted(w) => ("substituted", Some(w.clone())),
                Origin::Inserted => ("inserted", None),
            };
            let mut o = serde_json::json!({ "ipa": p.token, "word": p.word, "start": sec(p.start), "end": sec(p.end), "origin": origin });
            if let Some(w) = written {
                o["written"] = Value::String(w);
            }
            o
        })
        .collect();
    let heard: Vec<Value> = a.heard.iter().map(|h: &Phone| serde_json::json!([h.token, sec(h.start), sec(h.end)])).collect();
    let written: Vec<Value> = g2p
        .iter()
        .zip(&a.written)
        .map(|((p, _), s)| match s {
            Some((s0, s1)) => serde_json::json!([p, sec(*s0), sec(*s1)]),
            None => serde_json::json!([p, -1, -1]),
        })
        .collect();
    let dropped: Vec<Value> = a.dropped.iter().map(|(p, w)| serde_json::json!({ "ipa": p, "word": w })).collect();
    set_len(
        serde_json::json!({
            "words": by_word.into_iter().map(|(_, t)| t).collect::<Vec<_>>(), "said": said, "phones": phones,
            "dropped": dropped, "heard": heard, "written": written,
        })
        .to_string(),
    )
}

/// A phrase's articulator curves with no text: SPARC's raw EMA of a clip (flat f32 rows of 12, 50 Hz, row 0 at
/// `offset` s) and a request `{"offset", "start", "end": the phrase (patch s), "hop": the curves' frame (s), "level":
/// [amplitude 0..1 per hop frame from start], "maxPerSyllable": 6, "lead": s the articulators are played early}`.
/// SPARC's lowpass, the calibrated map to the chimera `articulate` inputs, the phrase's syllables from its sonority, and
/// per input at most `maxPerSyllable` keyframes per syllable (haa-fit `articulatory::artic_curves`). JSON length, 0 on a
/// bad request: `{"syllables": [cut s], "inputs": {name: [[s, value], ...]}, "frames": {name: [value per hop frame]},
/// "opening": [per hop frame], "points": total keyframes}`.
#[unsafe(no_mangle)]
pub extern "C" fn haa_artic_curves(ema_ptr: MemPtr, ema_len: u32, req_ptr: MemPtr, req_len: u32) -> u32 {
    // SAFETY: host buffers per the ABI.
    let (ema, req) = unsafe { (f32_at(ema_ptr, ema_len), bytes_at(req_ptr, req_len)) };
    let Some(req) = serde_json::from_slice::<Value>(req).ok() else { return 0 };
    let num = |k: &str, d: f64| req.get(k).and_then(Value::as_f64).unwrap_or(d);
    let level: Vec<f32> = req.get("level").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect());
    let (start, end, hop) = (num("start", 0.0), num("end", 0.0), num("hop", 0.01));
    if !(hop > 0.0) || !(end > start) || ema.len() < 12 {
        return 0;
    }
    let c = artic_curves(ema, num("offset", 0.0), start, end, hop, &level, num("maxPerSyllable", 6.0).max(1.0) as usize, num("lead", 0.0));
    let r = |x: f64| (x * 1e4).round() / 1e4;
    let mut inputs = serde_json::Map::new();
    let mut frames = serde_json::Map::new();
    for (i, (name, _, _)) in ARTIC_INPUTS.iter().enumerate() {
        inputs.insert(name.to_string(), Value::Array(c.keys[i].iter().map(|&k| serde_json::json!([r(start + k as f64 * hop), r(c.frames[i][k])])).collect()));
        frames.insert(name.to_string(), Value::Array(c.frames[i].iter().map(|&v| serde_json::json!(r(v))).collect()));
    }
    let out = serde_json::json!({
        "syllables": c.bounds.iter().map(|&b| r(start + b as f64 * hop)).collect::<Vec<_>>(),
        "inputs": inputs, "frames": frames,
        "opening": c.opening.iter().map(|&v| r(v)).collect::<Vec<_>>(),
        "points": c.keys.iter().map(Vec::len).sum::<usize>(),
    });
    set_len(out.to_string())
}
