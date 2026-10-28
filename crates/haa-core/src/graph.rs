// Ported from Sona v11 dsp.js
//! The interpreter (ARCHITECTURE §4, §8): a `Voice` plays a validated plan per sample with Sona's `GraphDSP` semantics.
//! Deviations from Sona `GraphDSP`:
//! - params smooth only when `ParamDef.smooth`, and control-wire amounts are edge-smoothed like audio wires;
//! - nodes return `NodeOut`; a source's trace value is its `value` (0), not its audio sample;
//! - `master` is applied to the output sum before the DC blocker (§4), not by a gain node after the limiter;
//! - `graph.mode = "live"` runs a continuous clock with only the start fade;
//! - a non-finite output zeroes the block, resets every node and counts a guard trip.

use crate::catalog::{NodeTypeDef, ParamKind};
use crate::consts::{
    BLOCK, EDGE_SMOOTH_S, END_FADE_S, LIMIT_PEAK, PARAM_SMOOTH_S, RESERVED_INPUTS, START_FADE_S,
    TIMELINE_START_FADE_S,
};
use crate::dsp::dc_blocker::DcBlocker;
use crate::dsp::smoother::Smoother;
use crate::issues::{Issue, has_errors};
use crate::node::{Node, NodeInit, SampleCtx};
use crate::patch::{GraphMode, Patch, PerfMode, load_patch};
use crate::rng::node_seed;
use crate::validate::{Plan, validate};

/// Release time of the output limiter. Sona `dsp.js` GraphDSP `limiterRelease`: `1 - exp(-1 / (.06 * sr))`.
const LIMITER_RELEASE_S: f64 = 0.06;

/// The result of loading a patch into a voice.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadOutcome {
    pub playable: bool,
    pub issues: Vec<Issue>,
}

/// One playing patch: its plan, node instances, clock and output chain.
pub struct Voice {
    sample_rate: f64,
    voice_seed: u32,
    issues: Vec<Issue>,
    plan: Option<Plan>,
    nodes: Vec<Box<dyn Node>>,
    node_ids: Vec<String>,
    /// Base-value smoother per node param (time 0 for params that do not smooth).
    param_smoothers: Vec<Vec<Smoother>>,
    /// Amount smoother per audio wire, parallel to `PlanNode.audio_in`.
    wire_smoothers: Vec<Vec<Smoother>>,
    /// Amount smoother per control wire, parallel to `PlanNode.mods`.
    mod_smoothers: Vec<Vec<Smoother>>,
    /// Effective (smoothed, modulated, clamped) params per node.
    effective: Vec<Vec<f64>>,
    values: Vec<f64>,
    signal_l: Vec<f64>,
    signal_r: Vec<f64>,
    output_indices: Vec<usize>,
    trace: Vec<f32>,
    /// Current input values: header inputs, then reserved inputs.
    input_values: Vec<f64>,
    /// Trigger pulses for the current sample, same indexing as `input_values`.
    triggers: Vec<bool>,
    // Clock (Sona `frame`, `totalFrames`, `startFrames`, `durationFrames`, `ended`).
    frame: u64,
    live_frames: u64,
    start_frames: u64,
    duration_frames: u64,
    looped: bool,
    ended: bool,
    perf_rebase: bool,
    // Output chain.
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    limiter: f64,
    limiter_release: f64,
    guard_trips: u32,
}

impl Voice {
    /// A silent voice with no patch. Sona clamps the rate to 8000..192000 and falls back to 48000.
    pub fn new(sample_rate: f64, voice_seed: u32) -> Voice {
        let sample_rate = if sample_rate.is_finite() && sample_rate != 0.0 {
            sample_rate.clamp(8_000.0, 192_000.0)
        } else {
            48_000.0
        };
        Voice {
            sample_rate,
            voice_seed,
            issues: Vec::new(),
            plan: None,
            nodes: Vec::new(),
            node_ids: Vec::new(),
            param_smoothers: Vec::new(),
            wire_smoothers: Vec::new(),
            mod_smoothers: Vec::new(),
            effective: Vec::new(),
            values: Vec::new(),
            signal_l: Vec::new(),
            signal_r: Vec::new(),
            output_indices: Vec::new(),
            trace: Vec::new(),
            input_values: Vec::new(),
            triggers: Vec::new(),
            frame: 0,
            live_frames: 0,
            start_frames: 0,
            duration_frames: 1,
            looped: false,
            ended: false,
            perf_rebase: false,
            dc_l: DcBlocker::new(sample_rate),
            dc_r: DcBlocker::new(sample_rate),
            limiter: 1.0,
            limiter_release: 1.0 - (-1.0 / (LIMITER_RELEASE_S * sample_rate)).exp(),
            guard_trips: 0,
        }
    }

    /// Loads patch JSON (`load_patch` then `validate`). A patch that is not playable keeps its issues and makes the voice silent.
    pub fn load(&mut self, json: &str, keep_state: bool) -> LoadOutcome {
        match load_patch(json) {
            Err(issue) => self.fail(vec![issue]),
            Ok((patch, mut issues)) => {
                let outcome = self.load_patch(&patch, keep_state);
                issues.extend(outcome.issues);
                self.issues = issues.clone();
                LoadOutcome {
                    playable: outcome.playable,
                    issues,
                }
            }
        }
    }

    /// Validates a loaded patch and installs its plan. Every buffer the voice uses is allocated here.
    pub fn load_patch(&mut self, patch: &Patch, keep_state: bool) -> LoadOutcome {
        match validate(patch) {
            Err(issues) => self.fail(issues),
            Ok((_, issues)) if has_errors(&issues) => self.fail(issues),
            Ok((plan, issues)) => {
                self.install(plan, keep_state);
                self.issues = issues.clone();
                LoadOutcome {
                    playable: true,
                    issues,
                }
            }
        }
    }

    /// The issues of the last load.
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// True when the last load produced a plan that plays.
    pub fn playable(&self) -> bool {
        self.plan.is_some()
    }

    /// Node ids in plan order.
    pub fn node_ids(&self) -> &[String] {
        &self.node_ids
    }

    /// The last control value of each plan node, in plan order.
    pub fn trace(&self) -> &[f32] {
        &self.trace
    }

    /// Normalized patch time of the next sample (live: (seconds mod duration) / duration).
    pub fn time(&self) -> f64 {
        let frame = match self.mode() {
            GraphMode::Timeline => self.frame,
            GraphMode::Live => self.frame % self.duration_frames,
        };
        frame as f64 / self.duration_frames as f64
    }

    /// Patch clock in seconds (timeline: since start or the last wrap; live: since start).
    pub fn seconds(&self) -> f64 {
        self.frame as f64 / self.sample_rate
    }

    /// True once a timeline voice without loop has played to its end.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// How many blocks the non-finite guard has zeroed.
    pub fn guard_trips(&self) -> u32 {
        self.guard_trips
    }

    /// Clock to 0, nodes reset, smoothers at their targets, start fade again. Sona `setPosition(0)` plus a node reset.
    pub fn restart(&mut self) {
        self.frame = 0;
        self.live_frames = 0;
        self.start_frames = 0;
        self.looped = false;
        self.ended = false;
        self.perf_rebase = false;
        self.reset_chain();
        if let Some(plan) = &self.plan {
            for (i, pn) in plan.nodes.iter().enumerate() {
                for (smoother, base) in self.param_smoothers[i].iter_mut().zip(&pn.params) {
                    smoother.jump(*base);
                }
                for (smoother, wire) in self.wire_smoothers[i].iter_mut().zip(&pn.audio_in) {
                    smoother.jump(wire.amount);
                }
                for (smoother, m) in self.mod_smoothers[i].iter_mut().zip(&pn.mods) {
                    smoother.jump(m.amount);
                }
            }
        }
    }

    /// Renders up to `BLOCK` frames into `left` and `right` (frames past `BLOCK` or past the shorter slice are zeroed).
    /// Returns false once the voice has ended, or when it has no playable patch. Never allocates.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        left.fill(0.0);
        right.fill(0.0);
        if self.plan.is_none() {
            return false;
        }
        let frames = left.len().min(right.len()).min(BLOCK);
        let mut tripped = false;
        for sample in 0..frames {
            if self.ended {
                break;
            }
            let (l, r) = self.step();
            if !(l.is_finite() && r.is_finite()) {
                tripped = true;
            }
            left[sample] = l as f32;
            right[sample] = r as f32;
        }
        if tripped {
            left.fill(0.0);
            right.fill(0.0);
            self.trip_guard();
        }
        for (trace, value) in self.trace.iter_mut().zip(&self.values) {
            *trace = *value as f32;
        }
        !self.ended
    }

    /// One output sample: clock, inputs, performance, nodes in plan order, then the output chain. Sona `process` loop body.
    fn step(&mut self) -> (f64, f64) {
        self.advance_inputs();
        self.advance_performance();
        let Some(plan) = &self.plan else {
            return (0.0, 0.0);
        };
        let sr = self.sample_rate;
        let mode = plan.mode;
        let (t, seconds) = match mode {
            GraphMode::Timeline => (
                self.frame as f64 / self.duration_frames as f64,
                self.frame as f64 / sr,
            ),
            GraphMode::Live => (
                (self.frame % self.duration_frames) as f64 / self.duration_frames as f64,
                self.frame as f64 / sr,
            ),
        };
        let ctx = SampleCtx {
            sample_rate: sr,
            dt: 1.0 / sr,
            t,
            seconds,
            duration: plan.duration,
            mode,
            live_frames: self.live_frames,
            looped: self.looped,
            perf_rebase: self.perf_rebase,
            inputs: &self.input_values,
            triggers: &self.triggers,
        };
        let values = &mut self.values;
        let signal_l = &mut self.signal_l;
        let signal_r = &mut self.signal_r;
        for (index, pn) in plan.nodes.iter().enumerate() {
            // Effective params: smoothed base, plus Σ mods × edge-smoothed amount, clamped as Sona's `clamp`.
            let effective = &mut self.effective[index];
            for (k, smoother) in self.param_smoothers[index].iter_mut().enumerate() {
                effective[k] = match pn.def.params[k].kind {
                    ParamKind::Number { .. } => smoother.next(),
                    // sona: enums are strings read from `n.params`; enum, bool and text are never modulated.
                    _ => pn.params[k],
                };
            }
            for (m, smoother) in pn.mods.iter().zip(self.mod_smoothers[index].iter_mut()) {
                effective[m.param] += values[m.from] * smoother.next();
            }
            for (k, param) in pn.def.params.iter().enumerate() {
                if let ParamKind::Number { min, max, .. } = param.kind {
                    effective[k] = clamp(effective[k], min, max);
                }
            }
            let (mut input_l, mut input_r) = (0.0, 0.0);
            for (wire, smoother) in pn
                .audio_in
                .iter()
                .zip(self.wire_smoothers[index].iter_mut())
            {
                let amount = smoother.next();
                input_l += signal_l[wire.from] * amount;
                input_r += signal_r[wire.from] * amount;
            }
            let effect = pn.def.audio_in;
            if pn.muted {
                values[index] = 0.0;
                signal_l[index] = 0.0;
                signal_r[index] = 0.0;
                continue;
            }
            if pn.bypass && effect {
                signal_l[index] = input_l;
                signal_r[index] = input_r;
                values[index] = (input_l + input_r) * 0.5;
                continue;
            }
            let out = self.nodes[index].process_sample(&ctx, (input_l, input_r), effective);
            if effect {
                signal_l[index] = out.l;
                signal_r[index] = out.r;
                values[index] = out.value;
            } else {
                // sona: `values[index] = Number.isFinite(value) ? value : 0`, and the source signal likewise.
                values[index] = finite_or_zero(out.value);
                signal_l[index] = finite_or_zero(out.l);
                signal_r[index] = finite_or_zero(out.r);
            }
        }

        // Output sum, master, DC blocker, limiter, fades.
        let (mut mixed_l, mut mixed_r) = (0.0, 0.0);
        for &o in &self.output_indices {
            mixed_l += signal_l[o];
            mixed_r += signal_r[o];
        }
        mixed_l *= plan.master;
        mixed_r *= plan.master;
        let l = self.dc_l.process(mixed_l);
        let r = self.dc_r.process(mixed_r);
        let peak = l.abs().max(r.abs());
        let target = if peak > LIMIT_PEAK {
            LIMIT_PEAK / peak
        } else {
            1.0
        };
        self.limiter = if target < self.limiter {
            target
        } else {
            self.limiter + (target - self.limiter) * self.limiter_release
        };
        let start = self.start_frames as f64 / (sr * START_FADE_S);
        let perf_live = plan
            .performance
            .as_ref()
            .is_some_and(|perf| perf.mode == PerfMode::Live);
        let fade = if mode == GraphMode::Live || (perf_live && plan.r#loop) {
            clamp(start, 0.0, 1.0)
        } else {
            let from_start = self.frame as f64 / (sr * TIMELINE_START_FADE_S);
            let to_end = (self.duration_frames as f64 - self.frame as f64) / (sr * END_FADE_S);
            clamp(start.min(from_start).min(to_end), 0.0, 1.0)
        };
        let gain = self.limiter * fade;
        let looping = plan.r#loop;

        // Clock advance. A live clock never wraps or ends.
        self.frame += 1;
        self.live_frames += 1;
        self.start_frames += 1;
        self.looped = false;
        if mode == GraphMode::Timeline && self.frame >= self.duration_frames {
            if looping {
                self.frame = 0;
                self.looped = true;
            } else {
                self.frame = self.duration_frames;
                self.ended = true;
            }
        }
        (l * gain, r * gain)
    }

    /// Next task: smooth continuous inputs toward their targets and turn pending triggers into one-sample pulses.
    fn advance_inputs(&mut self) {}

    /// Next task: Sona's performance macro (manual value from the `pedal` input, `perf_rebase` on a mode switch).
    fn advance_performance(&mut self) {}

    /// Next task: with `keep_state`, keep the nodes with the same id and type (`configure`), the clock and the smoothers,
    /// like Sona `setGraph(graph, true)`. Returns true when it reused the previous state; for now every load is fresh.
    fn reuse_state(&mut self, _plan: &Plan, _keep_state: bool) -> bool {
        false
    }

    /// Drops the plan: the voice keeps `issues` and plays silence.
    fn fail(&mut self, issues: Vec<Issue>) -> LoadOutcome {
        self.plan = None;
        self.nodes.clear();
        self.node_ids.clear();
        self.param_smoothers.clear();
        self.wire_smoothers.clear();
        self.mod_smoothers.clear();
        self.effective.clear();
        self.values.clear();
        self.signal_l.clear();
        self.signal_r.clear();
        self.output_indices.clear();
        self.trace.clear();
        self.input_values.clear();
        self.triggers.clear();
        self.ended = false;
        self.issues = issues.clone();
        LoadOutcome {
            playable: false,
            issues,
        }
    }

    /// Builds the node instances and allocates every buffer of `plan`.
    fn install(&mut self, plan: Plan, keep_state: bool) {
        let sr = self.sample_rate;
        // Sona: `durationFrames = Math.max(1, Math.round(clean.duration * this.sampleRate))`.
        self.duration_frames = ((plan.duration * sr).round() as u64).max(1);
        if !self.reuse_state(&plan, keep_state) {
            self.nodes = plan
                .nodes
                .iter()
                .map(|pn| {
                    (pn.def.create)(&NodeInit {
                        id: &pn.id,
                        seed: node_seed(&pn.id, self.voice_seed),
                        sample_rate: sr,
                        params: &pn.params,
                        texts: &pn.texts,
                        curve: pn.curve.as_ref(),
                        inputs: &plan.inputs,
                        duration: plan.duration,
                    })
                })
                .collect();
            self.param_smoothers = plan
                .nodes
                .iter()
                .map(|pn| param_smoothers(pn.def, &pn.params, sr))
                .collect();
            self.wire_smoothers = plan
                .nodes
                .iter()
                .map(|pn| {
                    let amounts = pn.audio_in.iter().map(|wire| wire.amount);
                    amounts
                        .map(|a| Smoother::new(EDGE_SMOOTH_S, sr, a))
                        .collect()
                })
                .collect();
            self.mod_smoothers = plan
                .nodes
                .iter()
                .map(|pn| {
                    let amounts = pn.mods.iter().map(|m| m.amount);
                    amounts
                        .map(|a| Smoother::new(EDGE_SMOOTH_S, sr, a))
                        .collect()
                })
                .collect();
            self.frame = 0;
            self.live_frames = 0;
            self.start_frames = 0;
            self.looped = false;
            self.perf_rebase = false;
            self.reset_chain();
            self.input_values = plan
                .inputs
                .iter()
                .map(|input| input.default)
                .chain(RESERVED_INPUTS.iter().map(|(_, default)| *default))
                .collect();
            self.triggers = vec![false; self.input_values.len()];
        }
        self.ended = false;
        let count = plan.nodes.len();
        self.node_ids = plan.nodes.iter().map(|pn| pn.id.clone()).collect();
        self.effective = plan.nodes.iter().map(|pn| pn.params.clone()).collect();
        self.values = vec![0.0; count];
        self.signal_l = vec![0.0; count];
        self.signal_r = vec![0.0; count];
        self.trace = vec![0.0; count];
        self.output_indices = (0..count)
            .filter(|&i| plan.nodes[i].def.type_name == "output")
            .collect();
        self.plan = Some(plan);
    }

    /// Clears the DC blockers, the limiter and the node signals.
    fn reset_chain(&mut self) {
        self.dc_l.reset();
        self.dc_r.reset();
        self.limiter = 1.0;
        self.values.fill(0.0);
        self.signal_l.fill(0.0);
        self.signal_r.fill(0.0);
        for node in &mut self.nodes {
            node.reset();
        }
    }

    /// The non-finite guard: every node is reset, the chain is cleared and the trip is counted.
    fn trip_guard(&mut self) {
        self.reset_chain();
        self.guard_trips = self.guard_trips.saturating_add(1);
    }

    fn mode(&self) -> GraphMode {
        self.plan
            .as_ref()
            .map_or(GraphMode::Timeline, |plan| plan.mode)
    }
}

/// One base-value smoother per param: 5 ms for smoothed numbers, none (time 0) for the rest.
fn param_smoothers(def: &NodeTypeDef, base: &[f64], sample_rate: f64) -> Vec<Smoother> {
    def.params
        .iter()
        .zip(base)
        .map(|(param, &value)| {
            let smooth = param.smooth && matches!(param.kind, ParamKind::Number { .. });
            let time = if smooth { PARAM_SMOOTH_S } else { 0.0 };
            Smoother::new(time, sample_rate, value)
        })
        .collect()
}

/// Sona's `clamp(x, lo, hi) = Math.min(hi, Math.max(lo, x))`, which lets NaN through and never panics.
fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x > hi {
        hi
    } else if x < lo {
        lo
    } else {
        x
    }
}

fn finite_or_zero(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn patch(duration: f64, looping: bool, with_output: bool) -> String {
        let output = if with_output {
            r#",{"id":"out","type":"output","x":0,"y":0,"label":"Out","params":{"gain":1}}"#
        } else {
            ""
        };
        let wire = if with_output {
            r#",{"id":"c2","from":"amp","to":"out","target":"audio","amount":1}"#
        } else {
            ""
        };
        format!(
            r#"{{"haa":1,"id":"osc-gain","title":"Osc gain","graph":{{"duration":{duration},"loop":{looping},"nodes":[
              {{"id":"osc","type":"oscillator","x":0,"y":0,"label":"Osc","params":{{"frequency":220,"wave":"square","level":1}}}},
              {{"id":"amp","type":"gain","x":0,"y":0,"label":"Gain","params":{{"gain":2}}}}{output}],
              "connections":[{{"id":"c1","from":"osc","to":"amp","target":"audio","amount":1}}{wire}]}}}}"#
        )
    }

    /// Renders `seconds` in BLOCK chunks; returns the peak and whether the last block was still playing.
    fn render(voice: &mut Voice, seconds: f64) -> (f32, bool) {
        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        let mut peak = 0.0f32;
        let mut playing = true;
        let blocks = (seconds * RATE / BLOCK as f64).ceil() as usize;
        for _ in 0..blocks {
            playing = voice.process(&mut left, &mut right);
            for (l, r) in left.iter().zip(&right) {
                assert!(l.is_finite() && r.is_finite());
                peak = peak.max(l.abs()).max(r.abs());
            }
        }
        (peak, playing)
    }

    #[test]
    fn oscillator_gain_output_is_audible_and_limited() {
        let mut voice = Voice::new(RATE, 0);
        let outcome = voice.load(&patch(1.0, true, true), false);
        assert!(outcome.playable, "{:?}", outcome.issues);
        assert_eq!(voice.node_ids(), ["osc", "amp", "out"]);
        let (peak, playing) = render(&mut voice, 1.0);
        assert!(playing);
        assert!(peak > 0.1, "peak {peak}");
        assert!(f64::from(peak) <= LIMIT_PEAK + 1e-6, "peak {peak}");
        assert_eq!(voice.guard_trips(), 0);
    }

    #[test]
    fn timeline_without_loop_ends_after_duration_and_end_fade() {
        let mut voice = Voice::new(RATE, 0);
        assert!(voice.load(&patch(0.5, false, true), false).playable);
        let (peak, playing) = render(&mut voice, 0.4);
        assert!(playing && !voice.ended() && peak > 0.1);
        render(&mut voice, 0.1 + END_FADE_S + BLOCK as f64 / RATE);
        assert!(voice.ended());
        let mut left = [1.0f32; BLOCK];
        let mut right = [1.0f32; BLOCK];
        assert!(!voice.process(&mut left, &mut right));
        assert!(left.iter().chain(&right).all(|&x| x == 0.0));
        assert_eq!(voice.time(), 1.0);
    }

    #[test]
    fn looping_timeline_wraps_and_keeps_playing() {
        let mut voice = Voice::new(RATE, 0);
        assert!(voice.load(&patch(0.5, true, true), false).playable);
        let (_, playing) = render(&mut voice, 1.2);
        assert!(playing && !voice.ended());
        assert!(voice.seconds() < 0.5);
    }

    #[test]
    fn a_patch_without_output_is_silent_and_not_playable() {
        let mut voice = Voice::new(RATE, 0);
        let outcome = voice.load(&patch(1.0, false, false), false);
        assert!(!outcome.playable && !voice.playable());
        assert!(voice.issues().iter().any(|issue| issue.code == "no-output"));
        let mut left = [1.0f32; BLOCK];
        let mut right = [1.0f32; BLOCK];
        assert!(!voice.process(&mut left, &mut right));
        assert!(left.iter().chain(&right).all(|&x| x == 0.0));
        assert!(voice.node_ids().is_empty() && voice.trace().is_empty());
    }
}
