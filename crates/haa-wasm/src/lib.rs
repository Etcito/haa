//! haa-wasm: the wasm ABI of the HAA engine (ARCHITECTURE §13).
//!
//! Plain `extern "C"` exports over linear memory: no wasm-bindgen glue and no
//! JS imports, so the host instantiates the module with `{}`. Text crosses the
//! boundary as UTF-8 bytes held in the global result buffer.
//!
//! Later milestones append their `mod <name>;` lines below the imports.

use std::sync::Mutex;

// Module list (append-only): later milestones add `mod <name>;` lines here.

/// A pointer as the ABI sees it: a `u32` offset into the wasm linear memory.
/// Native tests run at pointer width, so there the same value is a `usize` that
/// can be cast back to a pointer; the wasm exports are identical either way.
#[cfg(target_arch = "wasm32")]
type MemPtr = u32;
#[cfg(not(target_arch = "wasm32"))]
type MemPtr = usize;

/// The last UTF-8 JSON result (`haa_catalog` today, voice calls later).
///
/// One global buffer, behind a `Mutex` instead of `static mut` (`static mut` is
/// banned in haa-core, and edition 2024 denies references to it anyway). The
/// wasm module is single-threaded and the host calls exports one at a time, so
/// the lock is never contended there; natively it keeps the parallel `cargo
/// test` harness sound when a test writes the buffer.
static RESULT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn with_result<R>(f: impl FnOnce(&mut Vec<u8>) -> R) -> R {
    let mut buffer = RESULT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut buffer)
}

/// Replaces the global result; later export groups write their JSON through it.
pub(crate) fn set_result(s: String) {
    with_result(|buffer| *buffer = s.into_bytes());
}

/// The patch format number this build reads and writes.
#[unsafe(no_mangle)]
pub extern "C" fn haa_version() -> u32 {
    haa_core::consts::PATCH_FORMAT
}

/// Allocates `len` bytes the host fills before passing them to an export.
/// Returns 0 for an empty request; the host frees the block with `haa_free`.
#[unsafe(no_mangle)]
pub extern "C" fn haa_alloc(len: u32) -> MemPtr {
    if len == 0 {
        return 0;
    }
    let bytes = vec![0u8; len as usize].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8 as MemPtr
}

/// Frees a block from `haa_alloc`; `ptr`/`len` must be the pair it returned.
#[unsafe(no_mangle)]
pub extern "C" fn haa_free(ptr: MemPtr, len: u32) {
    if ptr == 0 || len == 0 {
        return;
    }
    let ptr = ptr as *mut u8;
    // SAFETY: the host passes back exactly the pointer and length that
    // `haa_alloc` returned, and frees it once. `Box<[u8]>` deallocates with the
    // same layout the boxed slice was built with.
    unsafe {
        let slice = std::slice::from_raw_parts_mut(ptr, len as usize);
        drop(Box::from_raw(slice as *mut [u8]));
    }
}

/// Pointer to the first byte of the last result.
#[unsafe(no_mangle)]
pub extern "C" fn haa_result_ptr() -> MemPtr {
    with_result(|buffer| buffer.as_mut_ptr() as MemPtr)
}

/// Length in bytes of the last result.
#[unsafe(no_mangle)]
pub extern "C" fn haa_result_len() -> u32 {
    with_result(|buffer| buffer.len() as u32)
}

/// Writes the §11 catalog JSON into the result and returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn haa_catalog() -> u32 {
    let json = haa_core::catalog_json();
    let len = json.len() as u32;
    set_result(json);
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_json() -> (Vec<u8>, serde_json::Value) {
        let len = haa_result_len() as usize;
        let ptr = haa_result_ptr() as *const u8;
        // SAFETY: the preceding export wrote the buffer and the pointer/len
        // pair describes it; no export runs while this test reads.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
        (bytes.clone(), serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn catalog_is_readable_through_ptr_and_len() {
        let written = haa_catalog();
        let (bytes, json) = result_json();
        assert_eq!(written as usize, bytes.len());
        assert_eq!(json["format"], haa_core::consts::PATCH_FORMAT);
        assert_eq!(json["soundRevision"], haa_core::consts::SONA_SOUND_REVISION);

        // Node cards add `curve`, `gain`, `oscillator` and `output`; the export
        // must always mirror the live registry, which the shell starts empty.
        let types = json["types"].as_array().expect("types array");
        let names: Vec<&str> = types
            .iter()
            .map(|def| def["type"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), haa_core::catalog::all().len());
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        for name in &names {
            assert!(haa_core::catalog::find(name).is_some());
        }
    }

    #[test]
    fn version_is_the_patch_format() {
        assert_eq!(haa_version(), haa_core::consts::PATCH_FORMAT);
    }

    #[test]
    fn alloc_round_trips_a_host_buffer() {
        let ptr = haa_alloc(4);
        assert_ne!(ptr, 0);
        // SAFETY: the block is four bytes and owned by this test until free.
        unsafe {
            std::slice::from_raw_parts_mut(ptr as *mut u8, 4).copy_from_slice(b"haa!");
        }
        haa_free(ptr, 4);
        assert_eq!(haa_alloc(0), 0);
    }
}
