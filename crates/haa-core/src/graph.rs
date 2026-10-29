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
    BLOCK, DEFAULT_INPUT_SMOOTH_S, EDGE_SMOOTH_S, END_FADE_S, LIMIT_PEAK, PARAM_SMOOTH_S,
    RESERVED_INPUTS, START_FADE_S, TIMELINE_START_FADE_S,
};
use crate::dsp::dc_blocker::DcBlocker;
use crate::dsp::smoother::Smoother;
use crate::issues::{Issue, has_errors};
use crate::node::{Node, NodeInit, SampleCtx};
use crate::patch::{GraphMode, InputKind, Patch, PerfMode, load_patch};
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
    input_smoothers: Vec<Smoother>,
    pending_triggers: Vec<bool>,
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
            input_smoothers: Vec::new(),
            pending_triggers: Vec::new(),
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

    /// Host metadata, with published inputs serialized as their complete InputDef JSON.
    pub fn info_json(&self) -> String {
        let plan = self.plan.as_ref();
        serde_json::json!({
            "nodes": self.node_ids,
            "inputs": plan.map(|p| p.inputs.as_slice()).unwrap_or(&[]),
            "mode": self.mode(),
            "duration": plan.map_or(0.0, |p| p.duration),
            "loop": plan.is_some_and(|p| p.r#loop),
        })
        .to_string()
    }

    /// Finds a published input, followed by the three reserved spatial inputs.
    pub fn input_index(&self, name: &str) -> Option<usize> {
        let plan = self.plan.as_ref()?;
        plan.inputs
            .iter()
            .position(|input| input.name == name)
            .or_else(|| {
                RESERVED_INPUTS
                    .iter()
                    .position(|(reserved, _)| *reserved == name)
                    .map(|index| plan.inputs.len() + index)
            })
    }

    /// Sets an input target in game units; enum indices change without smoothing.
    pub fn set_input(&mut self, index: usize, value: f64) {
        if !value.is_finite() {
            return;
        }
        let Some(smoother) = self.input_smoothers.get_mut(index) else {
            return;
        };
        let Some(plan) = &self.plan else { return };
        if let Some(input) = plan.inputs.get(index) {
            match input.kind {
                InputKind::Continuous => smoother.set_target(clamp(value, input.min, input.max)),
                InputKind::Enum => {
                    let value = clamp(
                        value.floor(),
                        0.0,
                        input.values.len().saturating_sub(1) as f64,
                    );
                    smoother.jump(value);
                    self.input_values[index] = value;
                }
                InputKind::Trigger => {}
            }
        } else {
            // Reserved inputs have no header range; their host supplies game-unit values.
            smoother.set_target(value);
        }
    }

    /// Queues a trigger input's pulse for exactly the next rendered sample.
    pub fn trigger(&mut self, index: usize) {
        if self
            .plan
            .as_ref()
            .and_then(|p| p.inputs.get(index))
            .is_some_and(|input| input.kind == InputKind::Trigger)
        {
            self.pending_triggers[index] = true;
        }
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
        self.pending_triggers.fill(false);
        self.triggers.fill(false);
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
        let perf_live = plan
            .performance
            .as_ref()
            .is_some_and(|perf| perf.mode == PerfMode::Live);
        // sona: response and grain receive `controlTime`, which is continuous while
        // performance.mode is live even when the graph clock is a timeline (dsp.js:164-212).
        let control_mode = if mode == GraphMode::Live || perf_live {
            GraphMode::Live
        } else {
            GraphMode::Timeline
        };
        let ctx = SampleCtx {
            sample_rate: sr,
            dt: 1.0 / sr,
            t,
            seconds,
            duration: plan.duration,
            mode: control_mode,
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
        self.perf_rebase = false;
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

    /// Smooth continuous values and consume pending trigger pulses before processing nodes.
    fn advance_inputs(&mut self) {
        let Some(plan) = &self.plan else { return };
        for (index, smoother) in self.input_smoothers.iter_mut().enumerate() {
            self.triggers[index] = core::mem::take(&mut self.pending_triggers[index]);
            self.input_values[index] = if plan
                .inputs
                .get(index)
                .is_some_and(|input| input.kind == InputKind::Trigger)
            {
                f64::from(self.triggers[index])
            } else {
                smoother.next()
            };
        }
    }

    /// sona: app.js setPerformanceMode/Value applies the macro before GraphDSP sees it.
    /// HAA applies those base/edge rules per sample so the published pedal can drive it.
    fn advance_performance(&mut self) {
        let Some(plan) = &self.plan else { return };
        let Some(perf) = &plan.performance else {
            return;
        };
        let live = perf.mode == PerfMode::Live;
        // sona: performanceCommand adds the curve to the node's own base; Sona's
        // setPerformanceMode('live') writes the manual value into that base.
        let base = if live {
            perf.pedal
                .map_or(plan.nodes[perf.node].params[perf.param], |i| {
                    clamp(self.input_values[i], 0.0, 1.0)
                })
        } else {
            plan.nodes[perf.node].params[perf.param]
        };
        let smoother = &mut self.param_smoothers[perf.node][perf.param];
        if self.perf_rebase {
            // Sona setGraph rebases bookkeeping, preserving the control node's response.
            smoother.jump(base);
        }
        smoother.set_target(base);
        for (m, smoother) in plan.nodes[perf.node]
            .mods
            .iter()
            .zip(&mut self.mod_smoothers[perf.node])
        {
            if perf.edge.as_deref() == Some(m.conn_id.as_str()) {
                // Sona control-edge amounts switch immediately (only audio edges glide).
                smoother.jump(if live { 0.0 } else { 1.0 });
            }
        }
    }

    /// Rebind by id/type, preserving node instances, smoother values and input targets.
    fn reuse_state(&mut self, plan: &Plan, keep_state: bool) -> bool {
        let Some(old) = self.plan.as_ref().filter(|_| keep_state) else {
            return false;
        };
        self.perf_rebase = old
            .performance
            .as_ref()
            .is_some_and(|p| p.mode == PerfMode::Live)
            != plan
                .performance
                .as_ref()
                .is_some_and(|p| p.mode == PerfMode::Live);
        let sr = self.sample_rate;
        let mut nodes: Vec<_> = core::mem::take(&mut self.nodes)
            .into_iter()
            .map(Some)
            .collect();
        let mut params = core::mem::take(&mut self.param_smoothers);
        let mut wires = core::mem::take(&mut self.wire_smoothers);
        let mut mods = core::mem::take(&mut self.mod_smoothers);
        for pn in &plan.nodes {
            let previous = old
                .nodes
                .iter()
                .position(|n| n.id == pn.id && n.def.type_name == pn.def.type_name);
            let init = NodeInit {
                id: &pn.id,
                seed: node_seed(&pn.id, self.voice_seed),
                sample_rate: sr,
                params: &pn.params,
                texts: &pn.texts,
                curve: pn.curve.as_ref(),
                inputs: &plan.inputs,
                duration: plan.duration,
            };
            let node = if let Some(i) = previous {
                let mut node = nodes[i].take().expect("validated unique node ids");
                node.configure(&init);
                node
            } else {
                (pn.def.create)(&init)
            };
            self.nodes.push(node);
            let mut smoothers = previous.map_or_else(
                || param_smoothers(pn.def, &pn.params, sr),
                |i| core::mem::take(&mut params[i]),
            );
            for (s, &base) in smoothers.iter_mut().zip(&pn.params) {
                s.set_target(base);
            }
            self.param_smoothers.push(smoothers);
            // Sona matches audio edges globally by connection id. HAA also smooths mods.
            self.wire_smoothers.push(
                pn.audio_in
                    .iter()
                    .map(|wire| {
                        let value = old
                            .nodes
                            .iter()
                            .enumerate()
                            .find_map(|(i, n)| {
                                n.audio_in
                                    .iter()
                                    .position(|w| w.conn_id == wire.conn_id)
                                    .map(|j| wires[i][j].value())
                            })
                            .unwrap_or(wire.amount);
                        let mut s = Smoother::new(EDGE_SMOOTH_S, sr, value);
                        s.set_target(wire.amount);
                        s
                    })
                    .collect(),
            );
            self.mod_smoothers.push(
                pn.mods
                    .iter()
                    .map(|m| {
                        let value = old
                            .nodes
                            .iter()
                            .enumerate()
                            .find_map(|(i, n)| {
                                n.mods
                                    .iter()
                                    .position(|w| w.conn_id == m.conn_id)
                                    .map(|j| mods[i][j].value())
                            })
                            .unwrap_or(m.amount);
                        let mut s = Smoother::new(EDGE_SMOOTH_S, sr, value);
                        s.set_target(m.amount);
                        s
                    })
                    .collect(),
            );
        }
        wires.clear();
        mods.clear();
        let mut inputs: Vec<_> = core::mem::take(&mut self.input_smoothers)
            .into_iter()
            .map(Some)
            .collect();
        let old_values = core::mem::take(&mut self.input_values);
        let old_pending = core::mem::take(&mut self.pending_triggers);
        for i in 0..plan.inputs.len() + RESERVED_INPUTS.len() {
            let def = plan.inputs.get(i);
            let previous = if let Some(input) = def {
                old.inputs
                    .iter()
                    .position(|p| p.name == input.name && p.kind == input.kind)
            } else {
                Some(old.inputs.len() + i - plan.inputs.len())
            };
            let default =
                def.map_or_else(|| RESERVED_INPUTS[i - plan.inputs.len()].1, |d| d.default);
            let time = def.map_or(DEFAULT_INPUT_SMOOTH_S, |d| {
                if d.kind == InputKind::Continuous {
                    d.smooth
                } else {
                    0.0
                }
            });
            let mut smoother = previous
                .and_then(|j| inputs[j].take())
                .unwrap_or_else(|| Smoother::new(time, sr, default));
            smoother.set_time(time, sr);
            self.input_smoothers.push(smoother);
            self.input_values
                .push(previous.map_or(default, |j| old_values[j]));
            self.pending_triggers
                .push(previous.is_some_and(|j| old_pending[j]));
        }
        self.triggers = vec![false; self.input_values.len()];
        // Sona wraps the timeline when duration changes; HAA's live clock stays continuous.
        if plan.mode == GraphMode::Timeline {
            self.frame %= self.duration_frames;
        }
        true
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
        self.input_smoothers.clear();
        self.pending_triggers.clear();
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
            self.pending_triggers = vec![false; self.input_values.len()];
            self.input_smoothers = self
                .input_values
                .iter()
                .enumerate()
                .map(|(index, &value)| {
                    let time = plan
                        .inputs
                        .get(index)
                        .map_or(DEFAULT_INPUT_SMOOTH_S, |input| {
                            if input.kind == InputKind::Continuous {
                                input.smooth
                            } else {
                                0.0
                            }
                        });
                    Smoother::new(time, sr, value)
                })
                .collect();
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

    fn input_patch() -> String {
        let mut json: serde_json::Value = serde_json::from_str(&patch(1.0, false, true)).unwrap();
        json["inputs"] = serde_json::json!([
            {"name":"speed", "kind":"continuous", "min":0, "max":100, "default":0, "smooth":0.05},
            {"name":"gear", "kind":"enum", "values":["low", "high"]},
            {"name":"hit", "kind":"trigger"}
        ]);
        json.to_string()
    }

    #[test]
    fn continuous_inputs_glide_and_enum_inputs_jump() {
        let mut voice = Voice::new(RATE, 0);
        assert!(voice.load(&input_patch(), false).playable);
        assert_eq!(voice.input_index("speed"), Some(0));
        assert_eq!(voice.input_index("_dist"), Some(3));
        assert_eq!(voice.input_index("_pan"), Some(4));
        assert_eq!(voice.input_index("_doppler"), Some(5));
        assert_eq!(voice.input_index("unknown"), None);
        voice.set_input(0, 200.0);
        voice.set_input(1, 20.0);
        voice.set_input(5, 2.0);
        voice.process(&mut [0.0; 48], &mut [0.0; 48]);
        assert!(voice.input_values[0] > 0.0 && voice.input_values[0] < 100.0);
        assert!((voice.input_values[0] - 100.0 * (1.0 - (-0.001_f64 / 0.05).exp())).abs() < 1e-9);
        assert_eq!(voice.input_values[1], 1.0);
        assert!(voice.input_values[5] > 1.0 && voice.input_values[5] < 2.0);
        let info: serde_json::Value = serde_json::from_str(&voice.info_json()).unwrap();
        assert_eq!(info["inputs"].as_array().unwrap().len(), 3);
        assert_eq!(info["inputs"][0]["smooth"], 0.05);
        assert_eq!(info["mode"], "timeline");
        assert_eq!(info["nodes"], serde_json::json!(["osc", "amp", "out"]));
        assert_eq!(info["duration"], 1.0);
        assert_eq!(info["loop"], false);
    }

    #[test]
    fn trigger_is_true_for_exactly_one_sample() {
        let mut voice = Voice::new(RATE, 0);
        assert!(voice.load(&input_patch(), false).playable);
        voice.trigger(2);
        assert!(!voice.triggers[2]);
        voice.process(&mut [0.0], &mut [0.0]);
        assert!(voice.triggers[2]);
        assert_eq!(voice.input_values[2], 1.0);
        voice.process(&mut [0.0], &mut [0.0]);
        assert!(!voice.triggers[2]);
        assert_eq!(voice.input_values[2], 0.0);
    }

    #[test]
    fn live_patch_keeps_playing_for_twelve_seconds() {
        let mut json: serde_json::Value = serde_json::from_str(&patch(1.0, false, true)).unwrap();
        json["graph"]["mode"] = serde_json::json!("live");
        let mut voice = Voice::new(RATE, 0);
        assert!(voice.load(&json.to_string(), false).playable);
        // This task explicitly requires twelve seconds to exercise the live clock.
        for _ in 0..(12 * RATE as usize / BLOCK) {
            assert!(voice.process(&mut [0.0; BLOCK], &mut [0.0; BLOCK]));
            assert!(!voice.ended());
        }
        assert_eq!(voice.seconds(), 12.0);
    }

    #[test]
    fn keep_state_preserves_instance_phase_clock_and_input_glide() {
        let mut json: serde_json::Value = serde_json::from_str(&input_patch()).unwrap();
        json["graph"]["nodes"][0]["params"]["wave"] = serde_json::json!("sine");
        let mut voice = Voice::new(RATE, 0);
        assert!(voice.load(&json.to_string(), false).playable);
        voice.set_input(0, 100.0);
        voice.process(&mut [0.0; 48], &mut [0.0; 48]);
        let instance = &*voice.nodes[0] as *const dyn Node as *const ();
        let input = voice.input_values[0];
        json["graph"]["nodes"][0]["params"]["frequency"] = serde_json::json!(440);
        assert!(voice.load(&json.to_string(), true).playable);
        assert_eq!(&*voice.nodes[0] as *const dyn Node as *const (), instance);
        assert_eq!(voice.seconds(), 0.001);
        assert_eq!(voice.input_values[0], input);
        assert_eq!(voice.param_smoothers[0][0].value(), 220.0);
        voice.process(&mut [0.0], &mut [0.0]);
        assert!(voice.input_values[0] > input);
        // The sine is still near its quarter-cycle peak, not its initial phase.
        assert!(voice.signal_l[0] > 0.6);
        assert!(voice.param_smoothers[0][0].value() > 220.0);
        assert!(voice.param_smoothers[0][0].value() < 440.0);
    }

    #[test]
    fn performance_switches_curve_edge_and_manual_base_and_rebases_once() {
        let mut json: serde_json::Value = serde_json::from_str(&patch(1.0, true, true)).unwrap();
        json["graph"]["nodes"].as_array_mut().unwrap().extend([
            serde_json::json!({"id":"gesture","type":"curve","curve":[[0,0.75],[1,0.75]]}),
            serde_json::json!({"id":"pedal-control","type":"curve","params":{"amount":0},"curve":[[0,1],[1,1]]}),
        ]);
        json["graph"]["connections"].as_array_mut().unwrap().push(
            serde_json::json!({"id":"pedal-edge","from":"gesture","to":"pedal-control","target":"amount","amount":1}));
        json["graph"]["performance"] = serde_json::json!({
            "node":"pedal-control","parameter":"amount","curveNode":"gesture",
            "edge":"pedal-edge","mode":"curve","manualValue":0.25
        });
        let mut voice = Voice::new(RATE, 0);
        let outcome = voice.load(&json.to_string(), false);
        assert!(outcome.playable, "{:?}", outcome.issues);
        let target = voice
            .node_ids
            .iter()
            .position(|id| id == "pedal-control")
            .unwrap();
        voice.process(&mut [0.0], &mut [0.0]);
        assert_eq!(voice.effective[target][0], 0.75);
        json["graph"]["performance"]["mode"] = serde_json::json!("live");
        assert!(voice.load(&json.to_string(), true).playable);
        assert!(voice.perf_rebase);
        voice.process(&mut [0.0], &mut [0.0]);
        assert!(!voice.perf_rebase);
        assert_eq!(voice.effective[target][0], 0.25);
        let pedal = voice.input_index("pedal").unwrap();
        voice.set_input(pedal, 1.0);
        voice.process(&mut [0.0; 48], &mut [0.0; 48]);
        assert!(voice.effective[target][0] > 0.25 && voice.effective[target][0] < 1.0);
        json["graph"]["performance"]["mode"] = serde_json::json!("curve");
        assert!(voice.load(&json.to_string(), true).playable);
        voice.process(&mut [0.0], &mut [0.0]);
        assert_eq!(voice.effective[target][0], 0.75);
    }

    #[test]
    fn performance_live_keeps_the_control_clock_continuous_at_the_loop() {
        let patch_for = |mode: &str| {
            serde_json::json!({
                "haa": 1, "id": "perf-clock", "title": "Perf clock",
                "graph": {
                    "duration": 0.01, "loop": true,
                    "nodes": [
                        {"id":"hold","type":"curve","x":0,"y":0,"label":"Hold",
                         "params":{"amount":1},"curve":[[0,1],[1,1]]},
                        {"id":"resp","type":"response","x":0,"y":0,"label":"Resp",
                         "params":{"input":0,"mode":"follow","attack":0.001,"release":0.01}},
                        {"id":"macro","type":"curve","x":0,"y":0,"label":"Macro",
                         "params":{"amount":0},"curve":[[0,0],[1,0]]},
                        {"id":"osc","type":"oscillator","x":0,"y":0,"label":"Osc",
                         "params":{"frequency":220,"wave":"sine","level":1}},
                        {"id":"amp","type":"gain","x":0,"y":0,"label":"Amp","params":{"gain":0}},
                        {"id":"out","type":"output","x":0,"y":0,"label":"Out","params":{"gain":1}}
                    ],
                    "connections": [
                        {"id":"c-hold","from":"hold","to":"resp","target":"input","amount":1},
                        {"id":"c-resp","from":"resp","to":"amp","target":"gain","amount":1},
                        {"id":"c-audio","from":"osc","to":"amp","target":"audio","amount":1},
                        {"id":"c-out","from":"amp","to":"out","target":"audio","amount":1}
                    ],
                    "performance": {"node":"macro","parameter":"amount","mode":mode,"manualValue":0}
                }
            })
            .to_string()
        };
        let trace_after_loop = |mode: &str| {
            let mut voice = Voice::new(RATE, 0);
            assert!(voice.load(&patch_for(mode), false).playable);
            let index = voice.node_ids().iter().position(|id| id == "resp").unwrap();
            for _ in 0..481 {
                voice.process(&mut [0.0], &mut [0.0]);
            }
            f64::from(voice.trace()[index])
        };
        // sona: dsp.js gives response the continuous totalFrames/sr clock while the
        // performance mode is live, so the loop wrap must not reset its envelope.
        assert!(
            trace_after_loop("live") > 0.9,
            "the live control clock reset at the loop"
        );
        assert!(
            trace_after_loop("curve") < 0.5,
            "the timeline control clock should reset at the loop"
        );
    }

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
