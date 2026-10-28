//! Strict validation (ARCHITECTURE §8, §12): a loaded patch becomes a `Plan` only when it has no error.
//! The rules and their order follow Sona v11 `dsp.js` `validateGraph`, with HAA's issue codes.

use crate::catalog::{self, NodeTypeDef, ParamDef, ParamKind, param_index};
use crate::consts::{LEGACY_TYPES, MAX_CONNECTIONS, MAX_NODES, RESERVED_INPUTS};
use crate::issues::{Issue, Severity, has_errors};
use crate::patch::{
    GraphMode, InputCurve, InputDef, InputKind, NodeSpec, Patch, PerfMode, param_value_f64,
};
use serde_json::Value;
use std::collections::BTreeSet;

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
        nodes.push(plan);
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
    let plan = Plan {
        nodes,
        inputs,
        duration: graph.duration,
        r#loop: graph.r#loop,
        mode: graph.mode,
        performance: None,
        master,
    };
    Ok((plan, issues))
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

    fn lookup(name: &str) -> Option<&'static NodeTypeDef> {
        match name {
            "test-osc" => Some(&OSC_DEF),
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
}
