//! Structure mutations: add or remove one catalog node on a copy of a patch.
//!
//! Adds use only catalog node types with their catalog defaults (no new types, no new semantics):
//! - insert an effect (`filter` lowpass/highpass/bandpass, `equalizer`, `drive`, `space`) in front
//!   of the first `output`, taking over all its audio inputs;
//! - add a layer, a `noise` or `oscillator` source into a `gain` whose level a 4-point decaying
//!   `curve` drives (points in 0..1, far under the 128-point limit), wired to the first `output`.
//!
//! A remove drops one source, effect or control node; an effect's audio inputs are bridged to its
//! audio outputs. Every mutant is validated by haa-core and dropped when it does not play.

use haa_core::catalog::{self, ParamKind};
use haa_core::issues::has_errors;
use haa_core::patch::{Connection, NodeSpec, Patch};
use haa_core::validate::validate;
use serde_json::{Map, Value, json};

/// One structure change.
#[derive(Clone, Debug, PartialEq)]
pub enum MutationKind {
    /// Insert an effect in front of the output: (type, param overrides, label).
    Insert(&'static str, &'static [(&'static str, &'static str)], &'static str),
    /// Add a source layer with a curve-driven gain: (source type, param overrides, label).
    Layer(&'static str, &'static [(&'static str, &'static str)], &'static str),
    /// Remove the node with this id.
    Remove(String),
}

/// A mutated copy, its label and the ids of the nodes it added.
#[derive(Clone, Debug)]
pub struct Mutant {
    pub patch: Patch,
    pub label: String,
    pub added: Vec<String>,
}

const INSERTS: [MutationKind; 6] = [
    MutationKind::Insert("filter", &[("mode", "lowpass")], "insert filter lowpass"),
    MutationKind::Insert("filter", &[("mode", "highpass")], "insert filter highpass"),
    MutationKind::Insert("filter", &[("mode", "bandpass")], "insert filter bandpass"),
    MutationKind::Insert("equalizer", &[], "insert equalizer"),
    MutationKind::Insert("drive", &[], "insert drive"),
    MutationKind::Insert("space", &[], "insert space"),
];

const LAYERS: [MutationKind; 3] = [
    MutationKind::Layer("noise", &[("color", "white")], "add noise layer"),
    MutationKind::Layer("noise", &[("color", "pink")], "add pink noise layer"),
    MutationKind::Layer("oscillator", &[("wave", "sine")], "add sine layer"),
];

/// Every applicable mutation of `patch`, adds first, in a fixed order.
pub fn candidates(patch: &Patch) -> Vec<MutationKind> {
    let mut out: Vec<MutationKind> = Vec::new();
    for m in INSERTS.iter().chain(LAYERS.iter()) {
        let t = match m {
            MutationKind::Insert(t, ..) | MutationKind::Layer(t, ..) => *t,
            MutationKind::Remove(_) => continue,
        };
        if catalog::find(t).is_some() {
            out.push(m.clone());
        }
    }
    for node in &patch.graph.nodes {
        if matches!(node.kind.as_str(), "output" | "input" | "map" | "math") {
            continue;
        }
        out.push(MutationKind::Remove(node.id.clone()));
    }
    out
}

/// Applies one mutation to a copy of `patch`; None when it does not apply or does not validate.
pub fn apply(patch: &Patch, kind: &MutationKind) -> Option<Mutant> {
    let mut p = patch.clone();
    let mut added = Vec::new();
    let label = match kind {
        MutationKind::Insert(t, overrides, label) => {
            let out = first_output(&p)?;
            let (ox, oy) = (p.graph.nodes[out].x, p.graph.nodes[out].y);
            let out_id = p.graph.nodes[out].id.clone();
            let id = unique_id(&p, t);
            let node = new_node(t, &id, ox - 160.0, oy + 90.0, overrides)?;
            let mut moved = 0;
            for c in &mut p.graph.connections {
                if c.to == out_id && c.target == "audio" {
                    c.to = id.clone();
                    moved += 1;
                }
            }
            if moved == 0 {
                return None;
            }
            p.graph.nodes.push(node);
            let cid = unique_conn(&p);
            p.graph.connections.push(conn(&cid, &id, &out_id, "audio", 1.0));
            added.push(id);
            label.to_string()
        }
        MutationKind::Layer(t, overrides, label) => {
            let out = first_output(&p)?;
            let (ox, oy) = (p.graph.nodes[out].x, p.graph.nodes[out].y);
            let out_id = p.graph.nodes[out].id.clone();
            let src_id = unique_id(&p, t);
            let src = new_node(t, &src_id, ox - 480.0, oy + 200.0, overrides)?;
            p.graph.nodes.push(src);
            let gain_id = unique_id(&p, "gain");
            let gain = new_node("gain", &gain_id, ox - 320.0, oy + 200.0, &[("gain", "0")])?;
            p.graph.nodes.push(gain);
            let curve_id = unique_id(&p, "curve");
            let mut curve = new_node("curve", &curve_id, ox - 320.0, oy + 320.0, &[])?;
            curve.curve = Some(json!([[0, 0], [0.01, 1], [0.25, 0.25], [1, 0]]));
            p.graph.nodes.push(curve);
            for (from, to, target) in [(&src_id, &gain_id, "audio"), (&curve_id, &gain_id, "gain"), (&gain_id, &out_id, "audio")] {
                let cid = unique_conn(&p);
                p.graph.connections.push(conn(&cid, from, to, target, 1.0));
            }
            added.extend([src_id, gain_id, curve_id]);
            label.to_string()
        }
        MutationKind::Remove(id) => {
            let index = p.graph.nodes.iter().position(|n| &n.id == id)?;
            let kind = p.graph.nodes[index].kind.clone();
            let def = catalog::find(&kind);
            let bridge = def.is_some_and(|d| d.audio_in && d.audio_out);
            if bridge {
                let ins: Vec<(String, f64)> = p
                    .graph
                    .connections
                    .iter()
                    .filter(|c| &c.to == id && c.target == "audio")
                    .map(|c| (c.from.clone(), c.amount))
                    .collect();
                let outs: Vec<(String, String, f64)> = p
                    .graph
                    .connections
                    .iter()
                    .filter(|c| &c.from == id && c.from_port.is_none())
                    .map(|c| (c.to.clone(), c.target.clone(), c.amount))
                    .collect();
                for (from, a) in &ins {
                    for (to, target, b) in &outs {
                        if target != "audio" {
                            continue;
                        }
                        let cid = unique_conn(&p);
                        p.graph.connections.push(conn(&cid, from, to, target, a * b));
                    }
                }
            }
            p.graph.connections.retain(|c| &c.from != id && &c.to != id);
            p.graph.nodes.remove(index);
            if let Some(perf) = &p.graph.performance {
                if &perf.node == id || perf.curve_node.as_deref() == Some(id.as_str()) {
                    return None;
                }
            }
            format!("remove {kind} {id}")
        }
    };
    match validate(&p) {
        Ok((_, issues)) if !has_errors(&issues) => Some(Mutant { patch: p, label, added }),
        _ => None,
    }
}

/// Start-point variants of a mutant: an added layer at three levels (its curve `amount` 0.5,
/// 0.15, 0.05) times three decays (a 5-point curve falling to 0.2 at 2, 8 or 30 % of the sample
/// and to 0 at three times that), an inserted filter at three cutoffs (catalog default, ×0.25, ×4). Empty for others.
pub fn level_scan(m: &Mutant) -> Vec<Patch> {
    let mut out = Vec::new();
    let find = |p: &Patch, prefix: &str| m.added.iter().find(|id| id.starts_with(prefix)).and_then(|id| p.graph.nodes.iter().position(|n| &n.id == id));
    if let Some(curve) = find(&m.patch, "fit-curve-") {
        for decay in [0.02, 0.08, 0.3] {
            for amount in [0.5, 0.15, 0.05] {
                let mut p = m.patch.clone();
                let node = &mut p.graph.nodes[curve];
                node.params.insert("amount".to_string(), Value::from(amount));
                node.curve = Some(json!([[0, 0], [0.004, 1], [decay, 0.2], [(decay * 3.0f64).min(1.0), 0], [1, 0]]));
                out.push(p);
            }
        }
    } else if let Some(filter) = find(&m.patch, "fit-filter-") {
        let base = m.patch.graph.nodes[filter].params.get("cutoff").and_then(Value::as_f64).unwrap_or(1000.0);
        for k in [1.0, 0.25, 4.0] {
            let mut p = m.patch.clone();
            p.graph.nodes[filter].params.insert("cutoff".to_string(), Value::from((base * k).clamp(10.0, 18_000.0)));
            out.push(p);
        }
    }
    out
}

fn first_output(p: &Patch) -> Option<usize> {
    p.graph.nodes.iter().position(|n| n.kind == "output" && n.muted != Some(true))
}

fn unique_id(p: &Patch, t: &str) -> String {
    (1..)
        .map(|k| format!("fit-{t}-{k}"))
        .find(|id| !p.graph.nodes.iter().any(|n| &n.id == id))
        .unwrap_or_default()
}

fn unique_conn(p: &Patch) -> String {
    (1..)
        .map(|k| format!("fit-c{k}"))
        .find(|id| !p.graph.connections.iter().any(|c| &c.id == id))
        .unwrap_or_default()
}

fn conn(id: &str, from: &str, to: &str, target: &str, amount: f64) -> Connection {
    Connection {
        from_port: None,
        id: id.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        target: target.to_string(),
        amount,
        extra: Map::new(),
    }
}

/// A node of catalog type `t` with every param at its catalog default, then `overrides`
/// (numbers parse as numbers, anything else is an enum value or text).
pub fn new_node(t: &str, id: &str, x: f64, y: f64, overrides: &[(&str, &str)]) -> Option<NodeSpec> {
    let def = catalog::find(t)?;
    let mut params = Map::new();
    for p in def.params {
        let v = match p.kind {
            ParamKind::Number { default, .. } => Value::from(default),
            ParamKind::Enum { values, default } => Value::from(values.get(default).copied().unwrap_or_default()),
            ParamKind::Bool { default } => Value::from(default),
            ParamKind::Text { default } => Value::from(default),
        };
        params.insert(p.key.to_string(), v);
    }
    for (k, v) in overrides {
        let value = v.parse::<f64>().map(Value::from).unwrap_or_else(|_| Value::from(*v));
        params.insert((*k).to_string(), value);
    }
    Some(NodeSpec {
        id: id.to_string(),
        kind: t.to_string(),
        x,
        y,
        label: def.label_en.to_string(),
        params,
        curve: None,
        compact: None,
        muted: None,
        bypass: None,
        extra: Map::new(),
    })
}
