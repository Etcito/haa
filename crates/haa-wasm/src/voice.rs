//! Voice export group of the wasm ABI (ARCHITECTURE §13).
//!
//! One global table of playing voices: `haa_voice_new` allocates a slot that
//! owns a `haa_core::Voice` plus its two `BLOCK`-long f32 output buffers. Handles
//! are the slot index plus one, so handle 0 never names a voice. Every call on
//! handle 0 or on an unknown/freed handle is a no-op returning 0 or -1.
//!
//! `haa_voice_process` renders into the slot's preallocated buffers: it never
//! allocates, never formats and never touches the global result buffer.

use std::sync::Mutex;

use haa_core::Voice;
use haa_core::consts::BLOCK;
use haa_core::issues::{Issue, Severity, issues_json};

use crate::{MemPtr, set_result};

/// One live voice: the interpreter and the two f32 buffers the host reads.
struct Slot {
    voice: Voice,
    left: Vec<f32>,
    right: Vec<f32>,
}

// `haa_core::Voice` holds `Box<dyn Node>` (not `Send` because the trait has no
// bound). The wasm module is single-threaded and natively the `Mutex` serializes
// every access, so moving a slot between the test harness's threads is sound.
unsafe impl Send for Slot {}

/// The global voice table, behind a `Mutex` instead of `static mut` (banned).
/// A freed slot is `None` and `haa_voice_new` reuses the first free index.
static VOICES: Mutex<Vec<Option<Slot>>> = Mutex::new(Vec::new());

/// Locks the voice table, recovering from a poisoned lock like `with_result`.
fn with_voices<R>(f: impl FnOnce(&mut Vec<Option<Slot>>) -> R) -> R {
    let mut voices = VOICES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut voices)
}

/// Runs `f` on the slot named by `handle`, or returns `fallback` for a bad handle.
fn with_slot<R>(handle: u32, fallback: R, f: impl FnOnce(&mut Slot) -> R) -> R {
    if handle == 0 {
        return fallback;
    }
    with_voices(|voices| match voices.get_mut((handle - 1) as usize) {
        Some(Some(slot)) => f(slot),
        _ => fallback,
    })
}

/// Views the host bytes at `ptr`/`len`. Only valid for the duration of a call.
///
/// # Safety
/// `ptr`/`len` must describe a live allocation the host owns, or be 0/0.
unsafe fn bytes_at<'a>(ptr: MemPtr, len: u32) -> &'a [u8] {
    if ptr == 0 || len == 0 {
        return &[];
    }
    // SAFETY: the caller guarantees the range is a live host allocation.
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
}

/// The `not-json` error issue for patch bytes that are not UTF-8.
fn not_json_issue() -> Issue {
    Issue {
        severity: Severity::Error,
        code: "not-json".to_string(),
        path: String::new(),
        node: None,
        message: "the patch bytes are not valid UTF-8".to_string(),
    }
}

/// Creates a voice for `sample_rate` and `seed`; returns its handle (index + 1).
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_new(sample_rate: f64, seed: u32) -> u32 {
    let slot = Slot {
        voice: Voice::new(sample_rate, seed),
        left: vec![0.0; BLOCK],
        right: vec![0.0; BLOCK],
    };
    with_voices(|voices| {
        if let Some(index) = voices.iter().position(Option::is_none) {
            voices[index] = Some(slot);
            (index + 1) as u32
        } else {
            voices.push(Some(slot));
            voices.len() as u32
        }
    })
}

/// Frees a voice slot; later `haa_voice_new` calls may reuse the index.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_free(handle: u32) {
    if handle == 0 {
        return;
    }
    with_voices(|voices| {
        if let Some(slot) = voices.get_mut((handle - 1) as usize) {
            *slot = None;
        }
    });
}

/// Loads UTF-8 patch bytes: 1 = playable, 0 = not playable (silent), -1 = bad handle.
/// The issues JSON array is written into the result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_load(handle: u32, ptr: MemPtr, len: u32, keep_state: u32) -> i32 {
    with_slot(handle, -1, |slot| {
        // SAFETY: the host passes a buffer from `haa_alloc` that it owns during the call.
        let bytes = unsafe { bytes_at(ptr, len) };
        match std::str::from_utf8(bytes) {
            Ok(json) => {
                let outcome = slot.voice.load(json, keep_state != 0);
                let playable = outcome.playable;
                set_result(issues_json(&outcome.issues));
                i32::from(playable)
            }
            Err(_) => {
                set_result(issues_json(&[not_json_issue()]));
                0
            }
        }
    })
}

/// Writes the `VoiceInfo` JSON (§9) into the result; returns its length, or 0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_info(handle: u32) -> u32 {
    with_slot(handle, 0, |slot| {
        let json = slot.voice.info_json();
        let len = json.len() as u32;
        set_result(json);
        len
    })
}

/// Resolves an input name to its index (header inputs, then reserved); -1 if unknown.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_input_index(handle: u32, ptr: MemPtr, len: u32) -> i32 {
    // SAFETY: the host passes a buffer from `haa_alloc` that it owns during the call.
    let bytes = unsafe { bytes_at(ptr, len) };
    let Ok(name) = std::str::from_utf8(bytes) else {
        return -1;
    };
    with_slot(handle, -1, |slot| {
        slot.voice.input_index(name).map_or(-1, |i| i as i32)
    })
}

/// Sets an input value in game units; out-of-range indices and bad handles are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_set_input(handle: u32, index: u32, value: f64) {
    with_slot(handle, (), |slot| {
        slot.voice.set_input(index as usize, value);
    });
}

/// Raises a trigger input's one-sample pulse; out-of-range indices and bad handles are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_trigger(handle: u32, index: u32) {
    with_slot(handle, (), |slot| {
        slot.voice.trigger(index as usize);
    });
}

/// Restarts the patch clock and resets every node; a bad handle is ignored.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_restart(handle: u32) {
    with_slot(handle, (), |slot| {
        slot.voice.restart();
    });
}

/// Renders up to `BLOCK` frames into the slot buffers: 1 while playing, 0 once ended.
/// Never allocates, never formats and never touches the result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_process(handle: u32, frames: u32) -> u32 {
    with_slot(handle, 0, |slot| {
        let frames = (frames as usize).min(BLOCK);
        u32::from(
            slot.voice
                .process(&mut slot.left[..frames], &mut slot.right[..frames]),
        )
    })
}

/// Pointer to the voice's left output buffer (`BLOCK` f32 frames); 0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_left(handle: u32) -> MemPtr {
    with_slot(handle, 0, |slot| slot.left.as_ptr() as MemPtr)
}

/// Pointer to the voice's right output buffer (`BLOCK` f32 frames); 0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_right(handle: u32) -> MemPtr {
    with_slot(handle, 0, |slot| slot.right.as_ptr() as MemPtr)
}

/// Pointer to the last control value of each plan node (f32); 0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_trace(handle: u32) -> MemPtr {
    with_slot(handle, 0, |slot| slot.voice.trace().as_ptr() as MemPtr)
}

/// Number of f32 values at `haa_voice_trace`; 0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_trace_len(handle: u32) -> u32 {
    with_slot(handle, 0, |slot| slot.voice.trace().len() as u32)
}

/// Normalized patch time of the next sample; 0.0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_time(handle: u32) -> f64 {
    with_slot(handle, 0.0, |slot| slot.voice.time())
}

/// Patch clock in seconds; 0.0 for a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn haa_voice_seconds(handle: u32) -> f64 {
    with_slot(handle, 0.0, |slot| slot.voice.seconds())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_json() -> Option<serde_json::Value> {
        // Snapshot under the buffer lock: other tests write the same global result.
        let bytes = crate::with_result(|buffer| buffer.clone());
        serde_json::from_slice(&bytes).ok()
    }

    fn alloc_bytes(bytes: &[u8]) -> MemPtr {
        let ptr = crate::haa_alloc(bytes.len() as u32);
        assert_ne!(ptr, 0);
        // SAFETY: `haa_alloc` returned a block of exactly this length.
        unsafe {
            std::slice::from_raw_parts_mut(ptr as *mut u8, bytes.len()).copy_from_slice(bytes);
        }
        ptr
    }

    #[test]
    fn new_load_process_free_round_trip() {
        let handle = haa_voice_new(48_000.0, 0);
        assert!(handle >= 1);

        let json = include_str!("../../../patches/examples/osc-curve.json");
        let ptr = alloc_bytes(json.as_bytes());
        assert_eq!(
            haa_voice_load(handle, ptr, json.len() as u32, 0),
            1,
            "osc-curve must be playable"
        );
        crate::haa_free(ptr, json.len() as u32);

        let mut audible = false;
        let blocks = (0.5 * 48_000.0 / BLOCK as f64).ceil() as usize;
        for _ in 0..blocks {
            let playing = haa_voice_process(handle, BLOCK as u32);
            // SAFETY: the slot owns a BLOCK-long left buffer for the voice's life.
            let left =
                unsafe { std::slice::from_raw_parts(haa_voice_left(handle) as *const f32, BLOCK) };
            if left.iter().any(|&sample| sample != 0.0) {
                audible = true;
                break;
            }
            if playing == 0 {
                break;
            }
        }
        assert!(audible, "osc-curve is audible within its first 0.5 s");

        haa_voice_free(handle);
        let name = b"frequency";
        assert_eq!(
            haa_voice_input_index(handle, name.as_ptr() as MemPtr, name.len() as u32),
            -1
        );
    }

    #[test]
    fn unknown_handle_is_a_no_op() {
        let handle = haa_voice_new(48_000.0, 0);
        let name = b"pitch";
        let name_ptr = name.as_ptr() as MemPtr;
        let name_len = name.len() as u32;

        assert_eq!(haa_voice_load(0, name_ptr, name_len, 0), -1);
        assert_eq!(haa_voice_input_index(0, name_ptr, name_len), -1);
        assert_eq!(haa_voice_process(0, BLOCK as u32), 0);

        haa_voice_free(handle);
        assert_eq!(haa_voice_load(handle, name_ptr, name_len, 0), -1);
        assert_eq!(haa_voice_input_index(handle, name_ptr, name_len), -1);
        assert_eq!(haa_voice_process(handle, BLOCK as u32), 0);
    }

    #[test]
    fn invalid_utf8_gives_not_json() {
        let handle = haa_voice_new(48_000.0, 0);
        let bytes = [0xff_u8, 0xfe, 0x00];
        let ptr = alloc_bytes(&bytes);
        let mut json = None;
        // Other tests write the shared result buffer concurrently, so retry the
        // load+snapshot until this call's own result is observed.
        for _ in 0..32 {
            assert_eq!(haa_voice_load(handle, ptr, bytes.len() as u32, 0), 0);
            let seen = result_json();
            if seen
                .as_ref()
                .is_some_and(|value| value[0]["code"] == "not-json")
            {
                json = seen;
                break;
            }
        }
        crate::haa_free(ptr, bytes.len() as u32);

        let json = json.expect("not-json result");
        assert_eq!(json[0]["severity"], "error");
        assert_eq!(json[0]["code"], "not-json");
        assert_eq!(json[0]["path"], "");
        haa_voice_free(handle);
    }
}
