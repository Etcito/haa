//! Master limiter export group of the wasm ABI: the sfx mix's last stage (haa-core
//! `dsp::master_limiter`), run by the `haa-sfx-master` AudioWorklet processor.
//!
//! One limiter per module instance (the worklet instantiates its own). The host writes a stereo block
//! into the two `BLOCK`-long buffers, calls `haa_master_process(n)` and reads them back in place.
//! `haa_master_stats` writes (overrun samples, input true peak dBFS, lowest gain dB) since its last
//! call into a 3-f32 buffer and returns its address.

use std::sync::Mutex;

use haa_core::consts::BLOCK;
use haa_core::dsp::master_limiter::MasterLimiter;

use crate::MemPtr;

struct Master {
    limiter: MasterLimiter,
    left: Vec<f32>,
    right: Vec<f32>,
    stats: [f32; 3],
}

static MASTER: Mutex<Option<Master>> = Mutex::new(None);

fn with<R>(fallback: R, f: impl FnOnce(&mut Master) -> R) -> R {
    let mut m = MASTER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match m.as_mut() {
        Some(master) => f(master),
        None => fallback,
    }
}

/// (Re)creates the limiter for a stream at `sample_rate`; returns its latency in samples.
#[unsafe(no_mangle)]
pub extern "C" fn haa_master_new(sample_rate: f32) -> u32 {
    let limiter = MasterLimiter::new(sample_rate as f64);
    let latency = limiter.latency() as u32;
    let mut m = MASTER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *m = Some(Master { limiter, left: vec![0.0; BLOCK], right: vec![0.0; BLOCK], stats: [0.0; 3] });
    latency
}

#[unsafe(no_mangle)]
pub extern "C" fn haa_master_left() -> MemPtr {
    with(0, |m| m.left.as_mut_ptr() as MemPtr)
}

#[unsafe(no_mangle)]
pub extern "C" fn haa_master_right() -> MemPtr {
    with(0, |m| m.right.as_mut_ptr() as MemPtr)
}

/// Limits the first `n` (at most `BLOCK`) frames of the two buffers in place.
#[unsafe(no_mangle)]
pub extern "C" fn haa_master_process(n: u32) {
    with((), |m| {
        let n = (n as usize).min(BLOCK);
        let Master { limiter, left, right, .. } = m;
        limiter.process(&mut left[..n], &mut right[..n]);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn haa_master_stats() -> MemPtr {
    with(0, |m| {
        let (over, peak, gain) = m.limiter.take_stats();
        m.stats = [over as f32, peak, gain];
        m.stats.as_mut_ptr() as MemPtr
    })
}
