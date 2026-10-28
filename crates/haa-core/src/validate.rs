//! Strict validation (ARCHITECTURE §8, §12): a loaded patch becomes a `Plan` only when it has no error.
//! The rules and their order follow Sona v11 `dsp.js` `validateGraph`, with HAA's issue codes.

use crate::catalog::{self, NodeTypeDef, ParamDef, ParamKind, param_index};
use crate::consts::{LEGACY_TYPES, MAX_CONNECTIONS, MAX_NODES, RESERVED_INPUTS};
use crate::issues::{Issue, Severity, has_errors};
use crate::patch::{
    Connection, GraphMode, InputCurve, InputDef, InputKind, NodeSpec, Patch, PerfMode, Performance,
    param_value_f64,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A validated graph: the nodes the interpreter runs, in plan order.
#[derive(Clone)]
pub struct Plan {
    pub nodes: Vec<PlanNode>,
    pub inputs: Vec<InputDef>,
    pub duration: f64,
    pub r#loop: bool,
    pub mode: GraphMode,
    pub performance: Option<PlanPerformance>,
    pub master: f64,
}

/// One node of a plan. `params` and `texts` follow the order of `def.params`.
#[derive(Clone)]
pub struct PlanNode {
    pub id: String,
    pub def: &'static NodeTypeDef,
    pub params: Vec<f64>,
    pub texts: Vec<Option<String>>,
    pub curve: Option<Value>,
    pub muted: bool,
    pub bypass: bool,
    pub audio_in: Vec<Wire>,
    pub mods: Vec<Mod>,
}

/// An audio wire into a node. `from` is a plan index.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub from: usize,
    pub amount: f64,
    pub conn_id: String,
}

/// A control wire into a node param. `from` and `param` are plan indices.
#[derive(Clone, Debug, PartialEq)]
pub struct Mod {
    pub from: usize,
    pub param: usize,
    pub amount: f64,
    pub conn_id: String,
}

/// The resolved performance macro: plan indices of its node, param and curve node, and the pedal input.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanPerformance {
    pub node: usize,
    pub param: usize,
    pub curve_node: Option<usize>,
    pub edge: Option<String>,
    pub mode: PerfMode,
    pub pedal: Option<usize>,
}

/// Validate a loaded patch against the engine's rules. Warnings come back with `Ok`.
pub fn validate(p: &Patch) -> Result<(Plan, Vec<Issue>), Vec<Issue>> {
    validate_with(p, catalog::find)
}

fn validate_with(
    p: &Patch,
    lookup: impl Fn(&str) -> Option<&'static NodeTypeDef>,
) -> Result<(Plan, Vec<Issue>), Vec<Issue>> {
    let mut issues = Vec::new();
    let inputs = check_inputs(&p.inputs, &mut issues);
    let graph = &p.graph;
    if !(graph.duration.is_finite() && graph.duration > 0.0) {
        push(
            &mut issues,
            Severity::Error,
            "bad-duration",
            "graph.duration".into(),
            None,
            "Duration must be a positive number of seconds",
        );
    }
    if graph.nodes.len() > MAX_NODES {
        push(
            &mut issues,
            Severity::Error,
            "too-many-nodes",
            "graph.nodes".into(),
            None,
            &format!("A graph holds at most {MAX_NODES} nodes"),
        );
    }
    let mut ids = BTreeSet::new();
    let mut nodes = Vec::with_capacity(graph.nodes.len());
    for (i, spec) in graph.nodes.iter().enumerate() {
        let path = format!("graph.nodes[{i}]");
        let id = spec.id.as_str();
        if !ids.insert(id) {
            push(
                &mut issues,
                Severity::Error,
                "duplicate-node-id",
                format!("{path}.id"),
                Some(id),
                "Another node already has this id",
            );
        }
        let Some(def) = node_type(spec, &path, &lookup, &mut issues) else {
            nodes.push(None);
            continue;
        };
        let plan = plan_node(spec, def, &path, &mut issues);
        if spec.kind == "input" {
            let name = param_index(def, "name")
                .and_then(|i| plan.texts[i].as_deref())
                .unwrap_or("");
            let known = p.inputs.iter().any(|input| input.name == name)
                || RESERVED_INPUTS
                    .iter()
                    .any(|(reserved, _)| *reserved == name);
            if !known {
                push(
                    &mut issues,
                    Severity::Error,
                    "unknown-input",
                    format!("{path}.params.name"),
                    Some(id),
                    "No published input or reserved input has this name",
                );
            }
        }
        nodes.push(Some(plan));
    }
    if !graph.nodes.iter().any(|node| node.kind == "output") {
        push(
            &mut issues,
            Severity::Error,
            "no-output",
            "graph.nodes".into(),
            None,
            "The graph needs an output node",
        );
    }
    if graph.connections.len() > MAX_CONNECTIONS {
        push(
            &mut issues,
            Severity::Error,
            "too-many-connections",
            "graph.connections".into(),
            None,
            &format!("A graph holds at most {MAX_CONNECTIONS} connections"),
        );
    }
    let edges = check_connections(&p.graph.connections, &graph.nodes, &nodes, &mut issues);
    let master = p.master.as_f64().unwrap_or_else(|| {
        push(
            &mut issues,
            Severity::Error,
            "bad-field",
            "master".into(),
            None,
            "Master must be a number; using 1",
        );
        1.0
    });
    if has_errors(&issues) {
        return Err(issues);
    }
    let Some(order) = kahn_order(&graph.nodes, &edges, &mut issues) else {
        return Err(issues);
    };
    let nodes = plan_in_order(nodes, &order, &edges);
    let performance = match &graph.performance {
        Some(perf) => match resolve_performance(perf, &nodes, &p.graph.connections, &inputs) {
            Some(resolved) => Some(resolved),
            None => {
                push(
                    &mut issues,
                    Severity::Error,
                    "bad-port",
                    "graph.performance".into(),
                    None,
                    "The performance names an unknown node or parameter",
                );
                return Err(issues);
            }
        },
        None => None,
    };
    let plan = Plan {
        nodes,
        inputs,
        duration: graph.duration,
        r#loop: graph.r#loop,
        mode: graph.mode,
        performance,
        master,
    };
    Ok((plan, issues))
}

/// Where a checked connection lands: the audio input or a param index of its `to` node.
#[derive(Clone, Copy)]
enum Port {
    Audio,
    Param(usize),
}

/// A connection that passed its checks, with document indices of its endpoints.
struct Edge {
    from: usize,
    to: usize,
    port: Port,
    amount: f64,
    conn_id: String,
}

/// Checks every connection (§7, §12) in document order, like Sona `validateGraph`:
/// endpoints, then ports, then the id. Returns the connections that passed.
fn check_connections(
    connections: &[Connection],
    specs: &[NodeSpec],
    nodes: &[Option<PlanNode>],
    issues: &mut Vec<Issue>,
) -> Vec<Edge> {
    let mut by_id = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        by_id.entry(spec.id.as_str()).or_insert(i);
    }
    let mut edge_ids = BTreeSet::new();
    let mut edges = Vec::with_capacity(connections.len());
    for (i, conn) in connections.iter().enumerate() {
        let path = format!("graph.connections[{i}]");
        let from = by_id.get(conn.from.as_str()).copied();
        let to = by_id.get(conn.to.as_str()).copied();
        for (end, id, found) in [("from", &conn.from, from), ("to", &conn.to, to)] {
            if found.is_none() {
                push(
                    issues,
                    Severity::Error,
                    "unknown-node",
                    format!("{path}.{end}"),
                    None,
                    &format!("No node has the id \"{id}\""),
                );
            }
        }
        // sona: an empty target is an audio wire (`edge.target || 'audio'`).
        let target = if conn.target.is_empty() {
            "audio"
        } else {
            conn.target.as_str()
        };
        let port = match (from, to) {
            (Some(from), Some(to)) => {
                let source = nodes[from].as_ref().map(|node| node.def);
                let dest = nodes[to].as_ref().map(|node| node.def);
                match (source, dest) {
                    (Some(source), Some(dest)) => {
                        let ends = (conn.from.as_str(), conn.to.as_str());
                        check_port(source, dest, target, ends, &path, issues)
                    }
                    // An unknown or legacy type was already reported.
                    _ => None,
                }
            }
            _ => None,
        };
        // sona: a missing id becomes `edge-<index>`.
        let conn_id = if conn.id.is_empty() {
            format!("edge-{i}")
        } else {
            conn.id.clone()
        };
        if !edge_ids.insert(conn_id.clone()) {
            push(
                issues,
                Severity::Error,
                "duplicate-connection-id",
                format!("{path}.id"),
                None,
                "Another connection already has this id",
            );
        }
        if let (Some(from), Some(to), Some(port)) = (from, to, port) {
            edges.push(Edge {
                from,
                to,
                port,
                // sona: validateGraph clamps the amount to -20000..20000.
                amount: conn.amount.clamp(-20000.0, 20000.0),
                conn_id,
            });
        }
    }
    edges
}

/// Port compatibility of one connection: `bad-port` unless the wire is audio out → audio in
/// or control out → a modulatable number param.
fn check_port(
    source: &NodeTypeDef,
    dest: &NodeTypeDef,
    target: &str,
    (from_id, to_id): (&str, &str),
    path: &str,
    issues: &mut Vec<Issue>,
) -> Option<Port> {
    let mut bad = |at: &str, node: &str, message: &str| {
        push(
            issues,
            Severity::Error,
            "bad-port",
            format!("{path}.{at}"),
            Some(node),
            message,
        );
    };
    if target == "audio" {
        let mut ok = true;
        if !source.audio_out {
            bad("from", from_id, "This node has no audio output");
            ok = false;
        }
        if !dest.audio_in {
            bad("target", to_id, "This node has no audio input");
            ok = false;
        }
        return ok.then_some(Port::Audio);
    }
    let mut ok = true;
    if !source.control_out {
        bad("from", from_id, "This node has no control output");
        ok = false;
    }
    let param = param_index(dest, target);
    let modulatable = param.is_some_and(|i| {
        let def = &dest.params[i];
        def.modulatable && !matches!(def.kind, ParamKind::Enum { .. } | ParamKind::Text { .. })
    });
    if !modulatable {
        let message = match param {
            None => format!("Unknown parameter \"{target}\""),
            Some(_) => format!("Parameter \"{target}\" cannot be modulated"),
        };
        bad("target", to_id, &message);
        ok = false;
    }
    if ok { param.map(Port::Param) } else { None }
}

/// Kahn's algorithm over audio and control edges, seeded with the zero in-degree nodes in
/// document order, exactly like Sona `validateGraph`. Returns document indices in plan order,
/// or `None` after one `cycle` error per node left over.
fn kahn_order(specs: &[NodeSpec], edges: &[Edge], issues: &mut Vec<Issue>) -> Option<Vec<usize>> {
    let mut incoming = vec![0usize; specs.len()];
    let mut outgoing = vec![Vec::new(); specs.len()];
    for edge in edges {
        incoming[edge.to] += 1;
        outgoing[edge.from].push(edge.to);
    }
    let mut queue: Vec<usize> = (0..specs.len()).filter(|&i| incoming[i] == 0).collect();
    let mut i = 0;
    while i < queue.len() {
        for &next in &outgoing[queue[i]] {
            incoming[next] -= 1;
            if incoming[next] == 0 {
                queue.push(next);
            }
        }
        i += 1;
    }
    if queue.len() == specs.len() {
        return Some(queue);
    }
    for (i, spec) in specs.iter().enumerate() {
        if incoming[i] > 0 {
            push(
                issues,
                Severity::Error,
                "cycle",
                format!("graph.nodes[{i}]"),
                Some(&spec.id),
                "This node is part of a dependency loop",
            );
        }
    }
    None
}

/// Moves the checked nodes into plan order and attaches their wires and mods (plan indices).
fn plan_in_order(nodes: Vec<Option<PlanNode>>, order: &[usize], edges: &[Edge]) -> Vec<PlanNode> {
    let mut slots = nodes;
    let mut position = vec![0usize; slots.len()];
    let mut planned = Vec::with_capacity(order.len());
    for (plan_index, &doc) in order.iter().enumerate() {
        position[doc] = plan_index;
        if let Some(node) = slots[doc].take() {
            planned.push(node);
        }
    }
    for edge in edges {
        let node = &mut planned[position[edge.to]];
        let from = position[edge.from];
        match edge.port {
            Port::Audio => node.audio_in.push(Wire {
                from,
                amount: edge.amount,
                conn_id: edge.conn_id.clone(),
            }),
            Port::Param(param) => node.mods.push(Mod {
                from,
                param,
                amount: edge.amount,
                conn_id: edge.conn_id.clone(),
            }),
        }
    }
    planned
}

/// Resolves the performance macro against the plan. `None` when its node or param is unknown.
fn resolve_performance(
    perf: &Performance,
    nodes: &[PlanNode],
    connections: &[Connection],
    inputs: &[InputDef],
) -> Option<PlanPerformance> {
    let node = nodes.iter().position(|node| node.id == perf.node)?;
    let param = param_index(nodes[node].def, &perf.parameter)?;
    // sona: app.js performanceBinding keeps the curve node only when it is a `curve`, and the
    // edge only when it wires that curve into the performance param.
    let curve_node = perf.curve_node.as_deref().and_then(|id| {
        nodes
            .iter()
            .position(|candidate| candidate.id == id && candidate.def.type_name == "curve")
    });
    let edge = curve_node.and_then(|curve| {
        let curve_id = nodes[curve].id.as_str();
        let wanted = perf.edge.as_deref()?;
        connections
            .iter()
            .find(|conn| {
                conn.id == wanted
                    && conn.from == curve_id
                    && conn.to == perf.node
                    && conn.target == perf.parameter
            })
            .map(|conn| conn.id.clone())
    });
    Some(PlanPerformance {
        node,
        param,
        curve_node,
        edge,
        mode: perf.mode,
        pedal: inputs.iter().position(|input| input.name == "pedal"),
    })
}

fn push(
    issues: &mut Vec<Issue>,
    severity: Severity,
    code: &str,
    path: String,
    node: Option<&str>,
    message: &str,
) {
    issues.push(Issue {
        severity,
        code: code.into(),
        path,
        node: node.map(str::to_string),
        message: message.into(),
    });
}

fn node_type(
    spec: &NodeSpec,
    path: &str,
    lookup: &impl Fn(&str) -> Option<&'static NodeTypeDef>,
    issues: &mut Vec<Issue>,
) -> Option<&'static NodeTypeDef> {
    let id = Some(spec.id.as_str());
    if LEGACY_TYPES.contains(&spec.kind.as_str()) {
        push(
            issues,
            Severity::Error,
            "legacy-type",
            format!("{path}.type"),
            id,
            &format!("Node type \"{}\" belongs to the v0.1 engine", spec.kind),
        );
        return None;
    }
    let def = lookup(&spec.kind);
    if def.is_none() {
        push(
            issues,
            Severity::Error,
            "unknown-type",
            format!("{path}.type"),
            id,
            &format!("Unknown node type \"{}\"", spec.kind),
        );
    }
    def
}

fn plan_node(
    spec: &NodeSpec,
    def: &'static NodeTypeDef,
    path: &str,
    issues: &mut Vec<Issue>,
) -> PlanNode {
    let id = Some(spec.id.as_str());
    let mut params = Vec::with_capacity(def.params.len());
    let mut texts = Vec::with_capacity(def.params.len());
    for param in def.params {
        let at = format!("{path}.params.{}", param.key);
        let raw = spec.params.get(param.key).filter(|value| !value.is_null());
        let (value, text) = param_value_f64(param, raw);
        params.push(check_param(param, raw, value, &at, id, issues));
        texts.push(text);
    }
    for key in spec.params.keys() {
        if param_index(def, key).is_none() {
            push(
                issues,
                Severity::Warning,
                "unknown-param",
                format!("{path}.params.{key}"),
                id,
                "Unknown parameter is kept in the patch and ignored",
            );
        }
    }
    PlanNode {
        id: spec.id.clone(),
        def,
        params,
        texts,
        curve: spec.curve.clone(),
        muted: spec.muted.unwrap_or(false),
        bypass: spec.bypass.unwrap_or(false),
        audio_in: Vec::new(),
        mods: Vec::new(),
    }
}

/// Checks one param value against its def and returns the value the interpreter uses.
fn check_param(
    param: &ParamDef,
    raw: Option<&Value>,
    value: f64,
    at: &str,
    node: Option<&str>,
    issues: &mut Vec<Issue>,
) -> f64 {
    match (param.kind, raw) {
        (ParamKind::Number { min, max, .. }, Some(raw)) => match raw.as_f64() {
            Some(number) if number < min || number > max => {
                push(
                    issues,
                    Severity::Warning,
                    "param-clamped",
                    at.into(),
                    node,
                    &format!("Value is outside {min}..{max} and is clamped"),
                );
                number.clamp(min, max)
            }
            Some(_) => value,
            None => {
                push(
                    issues,
                    Severity::Error,
                    "bad-field",
                    at.into(),
                    node,
                    "Expected a number",
                );
                value
            }
        },
        (ParamKind::Enum { values, .. }, Some(raw)) => {
            if !raw.as_str().is_some_and(|s| values.contains(&s)) {
                push(
                    issues,
                    Severity::Error,
                    "bad-enum",
                    at.into(),
                    node,
                    "Value is not one of the enum values",
                );
            }
            value
        }
        (ParamKind::Bool { .. }, Some(raw)) if !raw.is_boolean() => {
            push(
                issues,
                Severity::Error,
                "bad-field",
                at.into(),
                node,
                "Expected a boolean",
            );
            value
        }
        (ParamKind::Text { .. }, Some(raw)) if !raw.is_string() => {
            push(
                issues,
                Severity::Error,
                "bad-field",
                at.into(),
                node,
                "Expected text",
            );
            value
        }
        _ => value,
    }
}

/// Checks the published input defs (§8). Returns them with each default clamped to its range.
fn check_inputs(inputs: &[InputDef], issues: &mut Vec<Issue>) -> Vec<InputDef> {
    let mut names = BTreeSet::new();
    let mut checked = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let path = format!("inputs[{i}]");
        if !names.insert(input.name.as_str()) {
            push(
                issues,
                Severity::Error,
                "duplicate-input",
                format!("{path}.name"),
                None,
                "Another input already has this name",
            );
        }
        if input.name.starts_with('_') {
            push(
                issues,
                Severity::Error,
                "bad-input",
                format!("{path}.name"),
                None,
                "Names starting with _ are reserved",
            );
        }
        if input.min >= input.max {
            push(
                issues,
                Severity::Error,
                "bad-input",
                format!("{path}.max"),
                None,
                "Max must be above min",
            );
        }
        if input.curve == InputCurve::Log && input.min <= 0.0 {
            push(
                issues,
                Severity::Error,
                "bad-input",
                format!("{path}.min"),
                None,
                "A log input needs min above 0",
            );
        }
        let mut fixed = input.clone();
        if input.kind == InputKind::Enum {
            if input.values.is_empty() {
                push(
                    issues,
                    Severity::Error,
                    "bad-input",
                    format!("{path}.values"),
                    None,
                    "An enum input needs values",
                );
            } else if !(input.default.fract() == 0.0
                && input.default >= 0.0
                && input.default < input.values.len() as f64)
            {
                push(
                    issues,
                    Severity::Error,
                    "bad-input",
                    format!("{path}.default"),
                    None,
                    "Default must be an index of values",
                );
            }
        } else if input.min < input.max {
            fixed.default = input.default.clamp(input.min, input.max);
            if fixed.default != input.default {
                push(
                    issues,
                    Severity::Warning,
                    "param-clamped",
                    format!("{path}.default"),
                    None,
                    "Default is outside min..max and is clamped",
                );
            }
        }
        checked.push(fixed);
    }
    checked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Category, Scale};
    use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
    use crate::patch::load_patch;

    struct Silent;

    impl Node for Silent {
        fn process_sample(
            &mut self,
            _ctx: &SampleCtx<'_>,
            _input: (f64, f64),
            _params: &[f64],
        ) -> NodeOut {
            NodeOut {
                l: 0.0,
                r: 0.0,
                value: 0.0,
            }
        }

        fn reset(&mut self) {}
    }

    fn silent(_init: &NodeInit<'_>) -> Box<dyn Node> {
        Box::new(Silent)
    }

    static OSC_PARAMS: [ParamDef; 2] = [
        ParamDef {
            key: "cutoff",
            kind: ParamKind::Number {
                min: 20.0,
                max: 20000.0,
                default: 1000.0,
                step: 1.0,
                scale: Scale::Log,
            },
            unit: "Hz",
            label_en: "Cutoff",
            label_fr: "Coupure",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "shape",
            kind: ParamKind::Enum {
                values: &["lp", "hp"],
                default: 0,
            },
            unit: "",
            label_en: "Shape",
            label_fr: "Forme",
            modulatable: false,
            smooth: false,
        },
    ];

    static INPUT_PARAMS: [ParamDef; 1] = [ParamDef {
        key: "name",
        kind: ParamKind::Text { default: "" },
        unit: "",
        label_en: "Name",
        label_fr: "Nom",
        modulatable: false,
        smooth: false,
    }];

    static OSC_DEF: NodeTypeDef = NodeTypeDef {
        type_name: "test-osc",
        category: Category::Source,
        label_en: "Test oscillator",
        label_fr: "Oscillateur test",
        audio_in: false,
        audio_out: true,
        control_out: false,
        uses_curve: false,
        hidden: false,
        params: &OSC_PARAMS,
        create: silent,
    };

    static INPUT_DEF: NodeTypeDef = NodeTypeDef {
        type_name: "input",
        category: Category::Control,
        label_en: "Input",
        label_fr: "Entrée",
        audio_in: false,
        audio_out: false,
        control_out: true,
        uses_curve: false,
        hidden: false,
        params: &INPUT_PARAMS,
        create: silent,
    };

    static OUTPUT_DEF: NodeTypeDef = NodeTypeDef {
        type_name: "output",
        category: Category::Output,
        label_en: "Output",
        label_fr: "Sortie",
        audio_in: true,
        audio_out: false,
        control_out: false,
        uses_curve: false,
        hidden: false,
        params: &[],
        create: silent,
    };

    static AMOUNT_PARAMS: [ParamDef; 1] = [ParamDef {
        key: "amount",
        kind: ParamKind::Number {
            min: 0.0,
            max: 1.0,
            default: 1.0,
            step: 0.01,
            scale: Scale::Lin,
        },
        unit: "",
        label_en: "Amount",
        label_fr: "Quantité",
        modulatable: true,
        smooth: true,
    }];

    static CURVE_DEF: NodeTypeDef = NodeTypeDef {
        type_name: "curve",
        category: Category::Control,
        label_en: "Curve",
        label_fr: "Courbe",
        audio_in: false,
        audio_out: false,
        control_out: true,
        uses_curve: true,
        hidden: false,
        params: &AMOUNT_PARAMS,
        create: silent,
    };

    static FX_DEF: NodeTypeDef = NodeTypeDef {
        type_name: "test-fx",
        category: Category::Effect,
        label_en: "Test effect",
        label_fr: "Effet test",
        audio_in: true,
        audio_out: true,
        control_out: false,
        uses_curve: false,
        hidden: false,
        params: &AMOUNT_PARAMS,
        create: silent,
    };

    fn lookup(name: &str) -> Option<&'static NodeTypeDef> {
        match name {
            "test-osc" => Some(&OSC_DEF),
            "test-fx" => Some(&FX_DEF),
            "curve" => Some(&CURVE_DEF),
            "input" => Some(&INPUT_DEF),
            "output" => Some(&OUTPUT_DEF),
            _ => None,
        }
    }

    fn run(json: &str) -> Result<(Plan, Vec<Issue>), Vec<Issue>> {
        validate_with(&load_patch(json).expect("valid JSON").0, lookup)
    }

    fn has(issues: &[Issue], code: &str, path: &str) -> bool {
        issues
            .iter()
            .any(|issue| issue.code == code && issue.path == path)
    }

    const CLEAN: &str = r#"{"haa":1,"id":"clean","title":"Clean",
      "inputs":[{"name":"pitch","kind":"continuous","min":200,"max":1200,"default":440}],
      "graph":{"duration":2,"nodes":[
        {"id":"in1","type":"input","x":0,"y":0,"label":"Pitch","params":{"name":"pitch"}},
        {"id":"osc","type":"test-osc","x":0,"y":0,"label":"Osc","params":{"cutoff":500,"shape":"hp"}},
        {"id":"out","type":"output","x":0,"y":0,"label":"Out","params":{}}]}}"#;

    #[test]
    fn a_valid_graph_plans_nodes_in_document_order() {
        let (plan, issues) = run(CLEAN).ok().expect("valid patch");
        assert!(issues.is_empty());
        let ids: Vec<&str> = plan.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["in1", "osc", "out"]);
        assert_eq!(plan.nodes[1].params, [500.0, 1.0]);
        assert!(plan.nodes[1].audio_in.is_empty() && plan.nodes[1].mods.is_empty());
        assert_eq!(plan.master, 1.0);
        assert!(plan.performance.is_none());
    }

    #[test]
    fn out_of_range_param_is_clamped_with_a_warning() {
        let json = CLEAN.replace("\"cutoff\":500", "\"cutoff\":50000");
        let (plan, issues) = run(&json).ok().expect("a warning keeps the patch playable");
        assert_eq!(plan.nodes[1].params[0], 20000.0);
        let warning = issues
            .iter()
            .find(|issue| issue.code == "param-clamped")
            .expect("warning");
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.path, "graph.nodes[1].params.cutoff");
        assert_eq!(warning.node.as_deref(), Some("osc"));
    }

    #[test]
    fn duplicate_node_id_is_an_error() {
        let json = CLEAN.replace("\"id\":\"out\"", "\"id\":\"osc\"");
        let issues = run(&json).err().expect("duplicate id");
        assert!(has(&issues, "duplicate-node-id", "graph.nodes[2].id"));
    }

    #[test]
    fn reserved_input_name_is_bad_input() {
        let json = r#"{"haa":1,"id":"r","title":"R",
          "inputs":[{"name":"_dist","kind":"continuous"}],
          "graph":{"duration":2,"nodes":[
            {"id":"out","type":"output","x":0,"y":0,"label":"Out","params":{}}]}}"#;
        let issues = run(json).err().expect("reserved name");
        assert!(has(&issues, "bad-input", "inputs[0].name"));
    }

    #[test]
    fn input_node_must_name_a_header_or_reserved_input() {
        let json = CLEAN.replace(
            "\"params\":{\"name\":\"pitch\"}",
            "\"params\":{\"name\":\"gain\"}",
        );
        let issues = run(&json).err().expect("unknown input name");
        assert!(has(&issues, "unknown-input", "graph.nodes[0].params.name"));
    }

    fn graph(nodes: &[(&str, &str)], connections: &str, extra: &str) -> String {
        let nodes: Vec<String> = nodes
            .iter()
            .map(|(id, kind)| {
                format!(r#"{{"id":"{id}","type":"{kind}","x":0,"y":0,"label":"","params":{{}}}}"#)
            })
            .collect();
        format!(
            r#"{{"haa":1,"id":"g","title":"G","graph":{{"duration":2,"nodes":[{}],"connections":[{connections}]{extra}}}}}"#,
            nodes.join(",")
        )
    }

    #[test]
    fn a_cycle_is_an_error() {
        let json = graph(
            &[
                ("osc", "test-osc"),
                ("a", "test-fx"),
                ("b", "test-fx"),
                ("out", "output"),
            ],
            r#"{"id":"c1","from":"osc","to":"a","target":"audio"},
               {"id":"c2","from":"a","to":"b","target":"audio"},
               {"id":"c3","from":"b","to":"a","target":"audio"},
               {"id":"c4","from":"b","to":"out","target":"audio"}"#,
            "",
        );
        let issues = run(&json).err().expect("cycle");
        assert!(has(&issues, "cycle", "graph.nodes[1]"));
        assert!(has(&issues, "cycle", "graph.nodes[2]"));
        assert!(!has(&issues, "cycle", "graph.nodes[0]"));
    }

    #[test]
    fn audio_wire_into_a_control_node_is_bad_port() {
        let json = graph(
            &[("osc", "test-osc"), ("env", "curve"), ("out", "output")],
            r#"{"id":"c1","from":"osc","to":"env","target":"audio"},
               {"id":"c2","from":"osc","to":"out","target":"audio"}"#,
            "",
        );
        let issues = run(&json).err().expect("bad port");
        assert!(has(&issues, "bad-port", "graph.connections[0].target"));
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn control_wires_need_a_control_source_and_a_modulatable_number() {
        let json = graph(
            &[("osc", "test-osc"), ("env", "curve"), ("out", "output")],
            r#"{"id":"c1","from":"osc","to":"osc","target":"cutoff"},
               {"id":"c2","from":"env","to":"osc","target":"shape"},
               {"id":"c3","from":"env","to":"osc","target":"nope"},
               {"id":"c3","from":"env","to":"ghost","target":"cutoff"}"#,
            "",
        );
        let issues = run(&json).err().expect("bad wires");
        assert!(has(&issues, "bad-port", "graph.connections[0].from"));
        assert!(has(&issues, "bad-port", "graph.connections[1].target"));
        assert!(has(&issues, "bad-port", "graph.connections[2].target"));
        assert!(has(&issues, "unknown-node", "graph.connections[3].to"));
        assert!(has(
            &issues,
            "duplicate-connection-id",
            "graph.connections[3].id"
        ));
    }

    #[test]
    fn a_chain_in_reverse_document_order_plans_in_dependency_order() {
        let json = graph(
            &[("out", "output"), ("fx", "test-fx"), ("osc", "test-osc")],
            r#"{"id":"c1","from":"fx","to":"out","target":"audio","amount":0.5},
               {"id":"c2","from":"osc","to":"fx","target":"audio"}"#,
            "",
        );
        let (plan, _) = run(&json).ok().expect("valid chain");
        let ids: Vec<&str> = plan.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["osc", "fx", "out"]);
        let wire = |from, amount: f64, id: &str| Wire {
            from,
            amount,
            conn_id: id.into(),
        };
        assert_eq!(plan.nodes[1].audio_in, [wire(0, 1.0, "c2")]);
        assert_eq!(plan.nodes[2].audio_in, [wire(1, 0.5, "c1")]);
    }

    #[test]
    fn mods_and_performance_resolve_to_plan_indices() {
        let json = graph(
            &[
                ("out", "output"),
                ("osc", "test-osc"),
                ("env", "curve"),
                ("lfo", "curve"),
            ],
            r#"{"id":"c1","from":"osc","to":"out","target":"audio"},
               {"id":"c2","from":"env","to":"lfo","target":"amount"},
               {"id":"c3","from":"lfo","to":"osc","target":"cutoff","amount":300}"#,
            r#","performance":{"node":"lfo","parameter":"amount","curveNode":"env","edge":"c2","mode":"live","manualValue":0.5}"#,
        );
        let (plan, _) = run(&json).ok().expect("valid patch");
        let ids: Vec<&str> = plan.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["env", "lfo", "osc", "out"]);
        let lfo_mod = Mod {
            from: 0,
            param: 0,
            amount: 1.0,
            conn_id: "c2".into(),
        };
        assert_eq!(plan.nodes[1].mods, [lfo_mod]);
        let osc_mod = Mod {
            from: 1,
            param: 0,
            amount: 300.0,
            conn_id: "c3".into(),
        };
        assert_eq!(plan.nodes[2].mods, [osc_mod]);
        let perf = plan.performance.expect("performance");
        assert_eq!((perf.node, perf.param, perf.curve_node), (1, 0, Some(0)));
        assert_eq!(
            (perf.edge.as_deref(), perf.mode, perf.pedal),
            (Some("c2"), PerfMode::Live, Some(0))
        );

        let unknown = json.replace(r#""parameter":"amount""#, r#""parameter":"rate""#);
        let issues = run(&unknown).err().expect("unknown performance param");
        assert!(has(&issues, "bad-port", "graph.performance"));
    }
}
