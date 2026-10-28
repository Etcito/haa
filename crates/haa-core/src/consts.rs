//! Engine constants (ARCHITECTURE §5). Values copied from Sona v11 `dist/dsp.js` name the Sona source in a comment.

/// Haa header value: the patch format this engine reads and writes.
pub const PATCH_FORMAT: u32 = 1;
/// Sample rate of offline renders (CLI, Node, Worker), like Sona's export.
pub const OFFLINE_RATE: u32 = 48_000;
/// Maximum frames per `process` call.
pub const BLOCK: usize = 128;
/// Maximum nodes per graph (Sona: 64).
pub const MAX_NODES: usize = 256;
/// Maximum connections per graph (Sona: 256).
pub const MAX_CONNECTIONS: usize = 1024;
/// One-pole smoothing time of node parameters. Sona `dsp.js` GraphDSP `paramSmoothing`: `1 - exp(-1 / (.005 * sr))`.
pub const PARAM_SMOOTH_S: f64 = 0.005;
/// Peak level of the output limiter.
pub const LIMIT_PEAK: f64 = 0.92;
/// Rate of control trace packets.
pub const TRACE_HZ: f64 = 25.0;
/// Default smoothing time of a published continuous input.
pub const DEFAULT_INPUT_SMOOTH_S: f64 = 0.05;
/// Length of a live patch's render when no seconds are given.
pub const LIVE_DEFAULT_SECONDS: f64 = 10.0;
/// Sona sound revision this engine reproduces.
pub const SONA_SOUND_REVISION: u32 = 8;
/// Node types of the v0.1 engine, rejected with `legacy-type`.
pub const LEGACY_TYPES: [&str; 3] = ["engine", "reed", "bubble"];
/// Reserved inputs, always present after the header inputs, with their defaults.
pub const RESERVED_INPUTS: [(&str, f64); 3] = [("_dist", 0.0), ("_pan", 0.0), ("_doppler", 1.0)];

/// Smoothing time of audio and control edge amounts. Sona `dsp.js` GraphDSP `paramSmoothing`, the same coefficient as `PARAM_SMOOTH_S`.
pub const EDGE_SMOOTH_S: f64 = 0.005;
/// Cutoff of the DC blocker. Sona `dsp.js` GraphDSP `dcCoefficient`: `exp(-TAU * 5 / sr)`.
pub const DC_BLOCK_HZ: f64 = 5.0;
/// Start fade of the timeline and of a looping live voice, in seconds of `startFrames`. Sona `dsp.js` `startFrames / (sr * .005)`.
pub const START_FADE_S: f64 = 0.005;
/// Start fade of the timeline measured from the clock. Sona `dsp.js` `frame / (sr * .003)`.
pub const TIMELINE_START_FADE_S: f64 = 0.003;
/// End fade of a timeline. Sona `dsp.js` `(durationFrames - frame) / (sr * .006)`.
pub const END_FADE_S: f64 = 0.006;

/// DC blocker coefficient for a sample rate. Sona `dsp.js` GraphDSP `dcCoefficient`.
pub fn dc_blocker_coefficient(sample_rate: f64) -> f64 {
    (-core::f64::consts::TAU * DC_BLOCK_HZ / sample_rate).exp()
}
