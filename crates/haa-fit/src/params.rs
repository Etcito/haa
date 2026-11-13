//! Param space: the numbers of a template patch the optimiser may move, read from the catalog.
//!
//! Fit-able values are the numeric node params with a catalog range (enum, bool and text stay as
//! the template has them), the breakpoints of `curve` nodes (y in 0..1, interior x in 0..1, the
//! 128-point Sona limit is never exceeded because points are only moved), and the defaults of
//! continuous published inputs. Plumbing is left alone: `input`, `map`, `math` and `motion` nodes,
//! `retrigger`, and unbounded-looking ranges.
//!
//! The optimiser works in the unit cube: `x = 0..1` maps to `min..max`, logarithmically for
//! `log`-scale params and for times that span decades (attack, decay, ...).
//! Selection keeps at most `max_dims` values, round-robin over the nodes so that every node gets
//! its most telling param (pitch, cutoff, then times and levels) before any node gets a second one.

use haa_core::catalog::{self, ParamKind, Scale};
use haa_core::patch::{InputKind, InputCurve, Patch};
use serde_json::Value;

/// Hard cap on the dimensions of one optimisation.
pub const MAX_DIMS: usize = 40;

const SKIP_TYPES: [&str; 4] = ["input", "map", "math", "motion"];
const SKIP_KEYS: [&str; 7] = ["retrigger", "in", "input", "inMin", "inMax", "outMin", "outMax"];

/// Where a fit-able value lives in the patch.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    /// `graph.nodes[node].params[key]`.
    Param { node: usize, key: String },
    /// `graph.nodes[node].curve[point][0]` (interior points only).
    CurveX { node: usize, point: usize },
    /// `graph.nodes[node].curve[point][1]`.
    CurveY { node: usize, point: usize },
    /// `inputs[index].default`.
    Input { index: usize },
}

/// One fit-able value with its range and the template's value.
#[derive(Clone, Debug, PartialEq)]
pub struct FitParam {
    pub slot: Slot,
    /// Node id, or the input name for an input.
    pub node: String,
    pub node_type: String,
    /// Param key, `curve[i].x` / `curve[i].y`, or `default` for an input.
    pub key: String,
    pub min: f64,
    pub max: f64,
    pub log: bool,
    pub value: f64,
    /// A source's `frequency`/`pitch`: the sound's pitch itself.
    pub pitch: bool,
    /// For a breakpoint time, the distance to its nearer neighbour (else 1).
    gap: f64,
    /// Lower is more telling; sorts selection within a round.
    tier: u8,
    order: usize,
}

impl FitParam {
    /// The unit-cube coordinate of `v`.
    pub fn to_unit(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        let u = if self.log { (v / self.min).ln() / (self.max / self.min).ln() } else { (v - self.min) / (self.max - self.min) };
        if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The value at unit-cube coordinate `u`, rounded to 6 significant digits.
    pub fn from_unit(&self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        let v = if self.log { self.min * (self.max / self.min).powf(u) } else { self.min + u * (self.max - self.min) };
        round_sig(v).clamp(self.min, self.max)
    }

    /// Initial search step in unit-cube units: half an octave on log axes, 15 % of the range on
    /// linear ones, half the gap to the nearer neighbour (at most 10 %) for breakpoint times. On a pitched target a source's pitch starts at a
    /// quarter of a semitone: screening already matched the median f0, and the fine STFT
    /// resolutions punish any detune, so wide pitch steps would swamp every other dimension.
    pub fn step(&self, pitched: bool) -> f64 {
        if self.pitch && pitched {
            (0.25 / 12.0 * core::f64::consts::LN_2 / (self.max / self.min).ln()).max(1e-4)
        } else if self.log {
            (0.5 * core::f64::consts::LN_2 / (self.max / self.min).ln()).clamp(0.01, 0.3)
        } else if matches!(self.slot, Slot::CurveX { .. }) {
            (0.5 * self.gap).clamp(0.002, 0.1)
        } else {
            0.15
        }
    }

    pub fn to_json(&self, value: f64) -> Value {
        let kind = match self.slot {
            Slot::Param { .. } => "param",
            Slot::CurveX { .. } => "curve-x",
            Slot::CurveY { .. } => "curve-y",
            Slot::Input { .. } => "input",
        };
        serde_json::json!({
            "node": self.node, "type": self.node_type, "key": self.key, "kind": kind,
            "min": self.min, "max": self.max, "scale": if self.log { "log" } else { "lin" },
            "value": value,
        })
    }
}

/// Every fit-able value of `patch`, in selection order (round-robin over nodes, telling params first).
/// Values of nodes listed in `prefer` come before all others.
pub fn enumerate(patch: &Patch, prefer: &[String]) -> Vec<FitParam> {
    let mut per_owner: Vec<Vec<FitParam>> = Vec::new();
    let mut preferred: Vec<bool> = Vec::new();
    let mut order = 0usize;
    let mut inputs = Vec::new();
    for (index, input) in patch.inputs.iter().enumerate() {
        if input.kind != InputKind::Continuous || !(input.max > input.min) {
            continue;
        }
        let log = input.curve == InputCurve::Log && input.min > 0.0;
        order += 1;
        inputs.push(FitParam {
            slot: Slot::Input { index },
            node: input.name.clone(),
            node_type: "input".to_string(),
            key: "default".to_string(),
            min: input.min,
            max: input.max,
            log,
            value: input.default.clamp(input.min, input.max),
            pitch: false,
            gap: 1.0,
            tier: 0,
            order,
        });
    }
    if !inputs.is_empty() {
        per_owner.push(inputs);
        preferred.push(false);
    }
    for (n, node) in patch.graph.nodes.iter().enumerate() {
        if node.muted == Some(true) || SKIP_TYPES.contains(&node.kind.as_str()) {
            continue;
        }
        let Some(def) = catalog::find(&node.kind) else { continue };
        let mut list = Vec::new();
        for p in def.params {
            let ParamKind::Number { min, max, default, scale, .. } = p.kind else { continue };
            if SKIP_KEYS.contains(&p.key) || !(max > min) || (scale == Scale::Lin && max - min > 1.0e4) {
                continue;
            }
            let value = node.params.get(p.key).and_then(Value::as_f64).unwrap_or(default);
            order += 1;
            list.push(FitParam {
                slot: Slot::Param { node: n, key: p.key.to_string() },
                node: node.id.clone(),
                node_type: node.kind.clone(),
                key: p.key.to_string(),
                min,
                max,
                log: min > 0.0 && (scale == Scale::Log || (time_like(p.key, p.unit) && max / min >= 50.0)),
                value: if value.is_finite() { value.clamp(min, max) } else { default },
                pitch: def.category == haa_core::catalog::Category::Source && (p.key == "frequency" || p.key == "pitch"),
                gap: 1.0,
                tier: key_tier(p.key),
                order,
            });
        }
        if node.kind == "curve" {
            if let Some(points) = node.curve.as_ref().and_then(Value::as_array) {
                let count = points.len();
                if (2..=128).contains(&count) {
                    let xs: Vec<f64> = points.iter().map(|p| pair(p, 0).unwrap_or(0.0)).collect();
                    for (i, point) in points.iter().enumerate() {
                        let (Some(x), Some(y)) = (pair(point, 0), pair(point, 1)) else { continue };
                        order += 1;
                        list.push(curve_param(Slot::CurveY { node: n, point: i }, node, format!("curve[{i}].y"), y, 2, order));
                        if i > 0 && i + 1 < count {
                            order += 1;
                            let mut p = curve_param(Slot::CurveX { node: n, point: i }, node, format!("curve[{i}].x"), x, 3, order);
                            p.gap = (x - xs[i - 1]).abs().min((xs[i + 1] - x).abs());
                            list.push(p);
                        }
                    }
                }
            }
        }
        if !list.is_empty() {
            list.sort_by_key(|p| (p.tier, p.order));
            preferred.push(prefer.contains(&node.id));
            per_owner.push(list);
        }
    }
    // Round-robin: round r takes the r-th value of every owner; preferred owners go first.
    let mut keyed: Vec<(bool, usize, u8, usize, FitParam)> = Vec::new();
    for (owner, list) in per_owner.into_iter().enumerate() {
        let pref = preferred[owner];
        for (round, p) in list.into_iter().enumerate() {
            keyed.push((!pref, round, p.tier, owner, p));
        }
    }
    keyed.sort_by(|a, b| (a.0, a.1, a.2, a.3).cmp(&(b.0, b.1, b.2, b.3)));
    keyed.into_iter().map(|k| k.4).collect()
}

fn curve_param(slot: Slot, node: &haa_core::patch::NodeSpec, key: String, v: f64, tier: u8, order: usize) -> FitParam {
    FitParam {
        slot,
        node: node.id.clone(),
        node_type: node.kind.clone(),
        key,
        min: 0.0,
        max: 1.0,
        log: false,
        value: v.clamp(0.0, 1.0),
        pitch: false,
        gap: 1.0,
        tier,
        order,
    }
}

fn pair(point: &Value, i: usize) -> Option<f64> {
    point.as_array().and_then(|p| p.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// Times spanning decades are searched on a log axis (only the search axis; the value is the same).
fn time_like(key: &str, unit: &str) -> bool {
    unit == "s" || matches!(key, "attack" | "decay" | "release" | "time" | "hold" | "impulse")
}

/// How telling a param key usually is for timbre: 0 pitch/cutoff, 1 times and levels, 2 the rest.
fn key_tier(key: &str) -> u8 {
    match key {
        "frequency" | "cutoff" | "pitch" | "tension" => 0,
        "decay" | "release" | "attack" | "time" | "gain" | "level" | "amount" | "mix" | "sustain" | "hold" => 1,
        "resonance" | "damping" | "rate" | "size" | "density" | "radius" | "depth" => 2,
        _ => 3,
    }
}

/// The selected dimensions of one optimisation over one patch.
#[derive(Clone, Debug)]
pub struct ParamSpace {
    pub params: Vec<FitParam>,
    /// Total fit-able values before the `max_dims` cut.
    pub available: usize,
}

impl ParamSpace {
    /// The first `max_dims` values of `enumerate` (capped at `MAX_DIMS`).
    pub fn select(patch: &Patch, max_dims: usize, prefer: &[String]) -> ParamSpace {
        let all = enumerate(patch, prefer);
        let available = all.len();
        let params = all.into_iter().take(max_dims.clamp(1, MAX_DIMS)).collect();
        ParamSpace { params, available }
    }

    pub fn dims(&self) -> usize {
        self.params.len()
    }

    /// Initial search step of every dimension.
    pub fn steps(&self, pitched: bool) -> Vec<f64> {
        self.params.iter().map(|p| p.step(pitched)).collect()
    }

    /// The template's own point in the unit cube.
    pub fn x0(&self) -> Vec<f64> {
        self.params.iter().map(|p| p.to_unit(p.value)).collect()
    }

    /// Writes the values of unit point `x` into `patch` (which must be the patch the space was built on).
    pub fn apply(&self, patch: &mut Patch, x: &[f64]) {
        for (p, u) in self.params.iter().zip(x) {
            let v = p.from_unit(*u);
            match &p.slot {
                Slot::Param { node, key } => {
                    if let Some(spec) = patch.graph.nodes.get_mut(*node) {
                        spec.params.insert(key.clone(), Value::from(v));
                    }
                }
                Slot::CurveX { node, point } => set_curve(patch, *node, *point, 0, v),
                Slot::CurveY { node, point } => set_curve(patch, *node, *point, 1, v),
                Slot::Input { index } => {
                    if let Some(input) = patch.inputs.get_mut(*index) {
                        input.default = v;
                    }
                }
            }
        }
        // Keep each curve sorted by time, as Sona reads it.
        let mut touched: Vec<usize> = self
            .params
            .iter()
            .filter_map(|p| match p.slot {
                Slot::CurveX { node, .. } => Some(node),
                _ => None,
            })
            .collect();
        touched.dedup();
        for node in touched {
            if let Some(points) = patch.graph.nodes.get_mut(node).and_then(|n| n.curve.as_mut()).and_then(Value::as_array_mut) {
                let mut xs: Vec<f64> = points.iter().filter_map(|p| pair(p, 0)).collect();
                if xs.len() != points.len() {
                    continue;
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                for (point, x) in points.iter_mut().zip(xs) {
                    if let Some(arr) = point.as_array_mut() {
                        arr[0] = Value::from(x);
                    }
                }
            }
        }
    }

    /// The values of unit point `x` as JSON (one object per dimension).
    pub fn values_json(&self, x: &[f64]) -> Value {
        Value::Array(self.params.iter().zip(x).map(|(p, u)| p.to_json(p.from_unit(*u))).collect())
    }
}

fn set_curve(patch: &mut Patch, node: usize, point: usize, axis: usize, v: f64) {
    if let Some(arr) = patch
        .graph
        .nodes
        .get_mut(node)
        .and_then(|n| n.curve.as_mut())
        .and_then(Value::as_array_mut)
        .and_then(|points| points.get_mut(point))
        .and_then(Value::as_array_mut)
    {
        if arr.len() > axis {
            arr[axis] = Value::from(v);
        }
    }
}

/// `describe_params`: every fit-able value of a patch JSON, with the first `max_dims` marked selected.
pub fn describe_params(patch_json: &str, max_dims: usize) -> Result<String, String> {
    let (patch, issues) = haa_core::load_patch(patch_json).map_err(|i| format!("{}: {}", i.code, i.message))?;
    if haa_core::issues::has_errors(&issues) {
        let first = issues.iter().find(|i| i.severity == haa_core::issues::Severity::Error);
        return Err(first.map_or("patch has errors".to_string(), |i| format!("{}: {}", i.code, i.message)));
    }
    let all = enumerate(&patch, &[]);
    let cap = max_dims.clamp(1, MAX_DIMS);
    let list: Vec<Value> = all
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut v = p.to_json(p.value);
            v["selected"] = Value::from(i < cap);
            v
        })
        .collect();
    Ok(serde_json::json!({ "id": patch.id, "total": all.len(), "selected": all.len().min(cap), "params": list }).to_string())
}

/// Rounds to 6 significant digits so patch JSON stays readable.
pub fn round_sig(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let mag = v.abs().log10().floor() as i32;
    let scale = 10f64.powi(5 - mag);
    (v * scale).round() / scale
}
