//! The `Node` trait and the per-sample context every node reads.
//! Ported from Sona v11 `dsp.js` node processing; the contract is in ARCHITECTURE §4 and §10.

use crate::patch::{GraphMode, InputDef};

/// Clock, inputs and triggers for one sample.
pub struct SampleCtx<'a> {
    pub sample_rate: f64,
    pub dt: f64,
    /// Patch clock, normalized 0..1 (live: (seconds mod duration) / duration).
    pub t: f64,
    /// Patch clock in seconds since start or the last wrap.
    pub seconds: f64,
    pub duration: f64,
    pub mode: GraphMode,
    /// Frames since the voice started, never reset by loops (Sona totalFrames).
    pub live_frames: u64,
    /// True on the sample the timeline wrapped.
    pub looped: bool,
    /// True for one sample when performance.mode switches.
    pub perf_rebase: bool,
    /// Smoothed input values: header inputs, then reserved inputs.
    pub inputs: &'a [f64],
    /// Trigger pulses on this sample, same indexing as `inputs`.
    pub triggers: &'a [bool],
}

/// One sample of node output. A source returns `value = 0`; a control returns `l = r = 0`.
pub struct NodeOut {
    pub l: f64,
    pub r: f64,
    pub value: f64,
}

/// Everything a node needs to build its state.
pub struct NodeInit<'a> {
    pub id: &'a str,
    pub seed: u32,
    pub sample_rate: f64,
    /// Base values in `DEF.params` order (enum = index, bool = 0/1, text = 0).
    pub params: &'a [f64],
    /// Text param values, same indexing as `params`.
    pub texts: &'a [Option<String>],
    pub curve: Option<&'a serde_json::Value>,
    pub inputs: &'a [InputDef],
    pub duration: f64,
}

pub trait Node {
    /// Processes one sample. `params` holds the effective (smoothed, modulated, clamped) values.
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, input: (f64, f64), params: &[f64]) -> NodeOut;

    /// Live swap: takes new params and keeps the state.
    fn configure(&mut self, init: &NodeInit<'_>) {
        let _ = init;
    }

    fn reset(&mut self);
}
