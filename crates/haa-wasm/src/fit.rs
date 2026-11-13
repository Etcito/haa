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
//!   and brightness summaries of two mono buffers (target first), JSON length.

use std::sync::Mutex;

use haa_fit::{FitOptions, Fitter};
use serde_json::Value;

use crate::{MemPtr, set_result};

static FIT: Mutex<Option<Fitter>> = Mutex::new(None);

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
