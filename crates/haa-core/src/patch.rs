//! Patch format (ARCHITECTURE §7–§8): the serde types of a HAA patch, its published inputs and its graph.
//! Every type keeps unknown keys in `extra`, so a saved patch keeps the keys this engine does not read.

use crate::consts::{DEFAULT_INPUT_SMOOTH_S, PATCH_FORMAT};
use crate::issues::{Issue, Severity};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A HAA patch file: header, published inputs and the Sona graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub haa: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub inputs: Vec<InputDef>,
    pub graph: Graph,
    /// Sona's project master value, copied as-is.
    #[serde(default = "unity")]
    pub master: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A published input the game sets by name (ARCHITECTURE §8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputDef {
    pub name: String,
    pub kind: InputKind,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub min: f64,
    #[serde(default = "one")]
    pub max: f64,
    #[serde(default)]
    pub default: f64,
    /// One-pole time constant in seconds; 0 means unsmoothed.
    #[serde(default = "default_smooth")]
    pub smooth: f64,
    #[serde(default)]
    pub curve: InputCurve,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// How a published input behaves: a slider, a one-sample pulse or an index into `values`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    Continuous,
    Trigger,
    Enum,
}

/// Slider and `map` axis of a published input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputCurve {
    #[default]
    Lin,
    Log,
}

/// Sona's graph, unchanged, plus HAA's `mode`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub duration: f64,
    #[serde(rename = "loop", default)]
    pub r#loop: bool,
    #[serde(default)]
    pub mode: GraphMode,
    pub nodes: Vec<NodeSpec>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<Performance>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `timeline` plays Sona's normalized time once; `live` runs a continuous clock driven by inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphMode {
    #[default]
    Timeline,
    Live,
}

/// One node of the graph: a Sona node with its type, position and params.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub label: String,
    pub params: Map<String, Value>,
    /// Sona's breakpoint data, kept as JSON and read by the nodes that use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A wire between two nodes. `target` is `"audio"` or the key of a param of `to`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub from: String,
    pub to: String,
    pub target: String,
    #[serde(default = "one")]
    pub amount: f64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Sona's one-pedal macro, kept as-is and played with Sona's semantics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Performance {
    pub node: String,
    pub parameter: String,
    #[serde(rename = "curveNode", default, skip_serializing_if = "Option::is_none")]
    pub curve_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    pub mode: PerfMode,
    #[serde(rename = "manualValue")]
    pub manual_value: f64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Whether a performance plays its curve node or the live pedal input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PerfMode {
    Curve,
    Live,
}

fn unity() -> Value {
    Value::from(1)
}

fn one() -> f64 {
    1.0
}

fn default_smooth() -> f64 {
    DEFAULT_INPUT_SMOOTH_S
}

fn issue(severity: Severity, code: &str, path: &str, message: &str) -> Issue {
    Issue {
        severity,
        code: code.into(),
        path: path.into(),
        node: None,
        message: message.into(),
    }
}

// Consume known fields individually; everything left is the object's flattened extra map.
struct Fields<'a> {
    extra: Map<String, Value>,
    path: String,
    issues: &'a mut Vec<Issue>,
}

impl<'a> Fields<'a> {
    fn new(value: Value, path: String, issues: &'a mut Vec<Issue>) -> Self {
        let extra = match value {
            Value::Object(object) => object,
            _ => {
                issues.push(issue(
                    Severity::Error,
                    "bad-field",
                    &path,
                    "Expected an object",
                ));
                Map::new()
            }
        };
        Self {
            extra,
            path,
            issues,
        }
    }

    fn path(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.into()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn get<T: DeserializeOwned>(&mut self, key: &str, default: T) -> T {
        match self.extra.remove(key) {
            None => default,
            Some(value) => match serde_json::from_value(value) {
                Ok(value) => value,
                Err(_) => {
                    self.issues.push(issue(
                        Severity::Error,
                        "bad-field",
                        &self.path(key),
                        "Invalid field; using default",
                    ));
                    default
                }
            },
        }
    }

    fn list<T>(&mut self, key: &str, parse: fn(Value, String, &mut Vec<Issue>) -> T) -> Vec<T> {
        let values: Vec<Value> = self.get(key, Vec::new());
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| parse(value, format!("{}[{i}]", self.path(key)), self.issues))
            .collect()
    }
}

fn read_input(value: Value, path: String, issues: &mut Vec<Issue>) -> InputDef {
    let mut f = Fields::new(value, path, issues);
    InputDef {
        name: f.get("name", String::new()),
        kind: f.get("kind", InputKind::Continuous),
        unit: f.get("unit", String::new()),
        min: f.get("min", 0.0),
        max: f.get("max", 1.0),
        default: f.get("default", 0.0),
        smooth: f.get("smooth", default_smooth()),
        curve: f.get("curve", InputCurve::Lin),
        values: f.list("values", |value, path, issues| {
            serde_json::from_value(value).unwrap_or_else(|_| {
                issues.push(issue(
                    Severity::Error,
                    "bad-field",
                    &path,
                    "Expected a string",
                ));
                String::new()
            })
        }),
        extra: f.extra,
    }
}

fn read_node(value: Value, path: String, issues: &mut Vec<Issue>) -> NodeSpec {
    let mut f = Fields::new(value, path, issues);
    NodeSpec {
        id: f.get("id", String::new()),
        kind: f.get("type", String::new()),
        x: f.get("x", 0.0),
        y: f.get("y", 0.0),
        label: f.get("label", String::new()),
        params: f.get("params", Map::new()),
        curve: f.get("curve", None),
        compact: f.get("compact", None),
        muted: f.get("muted", None),
        bypass: f.get("bypass", None),
        extra: f.extra,
    }
}

fn read_connection(value: Value, path: String, issues: &mut Vec<Issue>) -> Connection {
    let mut f = Fields::new(value, path, issues);
    Connection {
        id: f.get("id", String::new()),
        from: f.get("from", String::new()),
        to: f.get("to", String::new()),
        target: f.get("target", String::new()),
        amount: f.get("amount", 1.0),
        extra: f.extra,
    }
}

fn read_performance(value: Value, path: String, issues: &mut Vec<Issue>) -> Performance {
    let mut f = Fields::new(value, path, issues);
    Performance {
        node: f.get("node", String::new()),
        parameter: f.get("parameter", String::new()),
        curve_node: f.get("curveNode", None),
        edge: f.get("edge", None),
        mode: f.get("mode", PerfMode::Curve),
        manual_value: f.get("manualValue", 0.0),
        extra: f.extra,
    }
}

fn read_graph(value: Value, path: String, issues: &mut Vec<Issue>) -> Graph {
    let mut f = Fields::new(value, path, issues);
    Graph {
        duration: f.get("duration", 0.0),
        r#loop: f.get("loop", false),
        mode: f.get("mode", GraphMode::Timeline),
        nodes: f.list("nodes", read_node),
        connections: f.list("connections", read_connection),
        performance: {
            let value: Option<Map<String, Value>> = f.get("performance", None);
            value.map(|object| {
                read_performance(Value::Object(object), f.path("performance"), f.issues)
            })
        },
        extra: f.extra,
    }
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    for part in value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
    {
        if !result.is_empty() {
            result.push('-');
        }
        result.push_str(&part.to_ascii_lowercase());
    }
    if result.is_empty() {
        "patch".into()
    } else {
        result
    }
}

/// Load HAA patches, Sona v2 projects and bare graphs, retaining unknown fields.
/// Only invalid JSON and non-object roots fail; field errors accompany a defaulted patch.
pub fn load_patch(json: &str) -> Result<(Patch, Vec<Issue>), Issue> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| issue(Severity::Error, "not-json", "", &e.to_string()))?;
    let Value::Object(mut object) = value else {
        return Err(issue(
            Severity::Error,
            "not-object",
            "",
            "Expected a patch object",
        ));
    };
    let mut issues = Vec::new();
    if !object.contains_key("haa") {
        if object.contains_key("nodes") && object.contains_key("duration") {
            object = Map::from_iter([
                ("graph".into(), Value::Object(object)),
                ("id".into(), Value::from("patch")),
                ("title".into(), Value::from("Untitled")),
            ]);
            issues.push(issue(
                Severity::Info,
                "wrapped-sona-graph",
                "",
                "Wrapped a Sona graph",
            ));
        } else {
            let id = slug(
                object
                    .get("presetId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| object.get("title").and_then(Value::as_str))
                    .unwrap_or("patch"),
            );
            object.insert("id".into(), Value::from(id));
            object
                .entry("title")
                .or_insert_with(|| Value::from("Untitled"));
            let mut sona = match object.remove("sona") {
                Some(Value::Object(sona)) => sona,
                Some(value) => {
                    object.insert("sona".into(), value);
                    Map::new()
                }
                None => Map::new(),
            };
            for key in ["version", "recipeRevision", "presetId", "titleCustom"] {
                if let Some(value) = object.remove(key) {
                    sona.insert(key.into(), value);
                }
            }
            if !sona.is_empty() {
                object.insert("sona".into(), Value::Object(sona));
            }
            issues.push(issue(
                Severity::Info,
                "wrapped-sona-project",
                "",
                "Wrapped a Sona project",
            ));
        }
    }
    if object
        .get("haa")
        .and_then(Value::as_f64)
        .is_some_and(|v| v > f64::from(PATCH_FORMAT))
    {
        issues.push(issue(
            Severity::Error,
            "format-too-new",
            "haa",
            "Patch format is newer than supported",
        ));
    }
    let mut f = Fields::new(Value::Object(object), String::new(), &mut issues);
    let mut patch = Patch {
        haa: f.get("haa", PATCH_FORMAT),
        id: f.get("id", "patch".into()),
        title: f.get("title", "Untitled".into()),
        inputs: f.list("inputs", read_input),
        graph: read_graph(
            f.extra.remove("graph").unwrap_or(Value::Object(Map::new())),
            "graph".into(),
            f.issues,
        ),
        master: f.get("master", unity()),
        extra: f.extra,
    };
    if let Some(performance) = &patch.graph.performance {
        if !patch.inputs.iter().any(|input| input.name == "pedal") {
            patch.inputs.push(InputDef {
                name: "pedal".into(),
                kind: InputKind::Continuous,
                unit: String::new(),
                // Sona v11 dist/app.js: setPerformanceValue clamps manualValue to 0..1;
                // the performanceValue input handler divides the UI's 0..100 by 100.
                min: 0.0,
                max: 1.0,
                default: performance.manual_value,
                smooth: default_smooth(),
                curve: InputCurve::Lin,
                values: Vec::new(),
                extra: Map::new(),
            });
            issues.push(issue(
                Severity::Info,
                "performance-converted",
                "graph.performance",
                "Published the performance pedal input",
            ));
        }
    }
    Ok((patch, issues))
}

/// The patch as a file: 2-space pretty JSON with a trailing newline.
pub fn to_json(p: &Patch) -> String {
    let mut out = serde_json::to_string_pretty(p).unwrap_or_default();
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SIREN: &str = r#"{
      "haa": 1,
      "id": "siren",
      "title": "Siren",
      "master": 0.8,
      "future_top": {"a": 1},
      "inputs": [
        {"name": "pitch", "kind": "continuous", "unit": "Hz", "min": 200, "max": 1200,
         "default": 440, "smooth": 0.05, "curve": "log", "future_input": true}
      ],
      "graph": {
        "duration": 4, "loop": true, "mode": "live", "future_graph": [1, 2],
        "nodes": [
          {"id": "in1", "type": "input", "x": 40, "y": 80, "label": "Pitch",
           "params": {"name": "pitch"}, "future_node": "x"},
          {"id": "map1", "type": "map", "x": 1, "y": 2, "label": "Map",
           "params": {"in": 0}, "curve": {"points": [[0, 0], [1, 1]]}, "compact": true}
        ],
        "connections": [
          {"id": "c1", "from": "in1", "to": "map1", "target": "in", "amount": 0.5,
           "future_conn": "keep"}
        ],
        "performance": {"node": "map1", "parameter": "in", "curveNode": "map1",
          "edge": "c1", "mode": "live", "manualValue": 0.25, "future_perf": 3}
      }
    }"#;

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let patch: Patch = serde_json::from_str(SIREN).unwrap();
        let text = to_json(&patch);
        let out: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(out["future_top"], json!({"a": 1}));
        assert_eq!(out["inputs"][0]["future_input"], json!(true));
        assert_eq!(out["graph"]["future_graph"], json!([1, 2]));
        assert_eq!(out["graph"]["nodes"][0]["future_node"], json!("x"));
        assert_eq!(out["graph"]["connections"][0]["future_conn"], json!("keep"));
        assert_eq!(out["graph"]["performance"]["future_perf"], json!(3));
        assert_eq!(
            out["graph"]["nodes"][1]["curve"],
            json!({"points": [[0, 0], [1, 1]]})
        );
        assert_eq!(out["graph"]["performance"]["curveNode"], json!("map1"));
        assert_eq!(out["graph"]["performance"]["manualValue"], json!(0.25));
        assert_eq!(out["graph"]["loop"], json!(true));
        assert_eq!(out["graph"]["nodes"][1]["type"], json!("map"));
    }

    #[test]
    fn to_json_is_two_space_pretty_with_trailing_newline() {
        let patch: Patch = serde_json::from_str(SIREN).unwrap();
        let text = to_json(&patch);
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\n  \"haa\": 1,"));
    }

    #[test]
    fn defaults_apply_when_keys_are_missing() {
        let patch: Patch = serde_json::from_str(
            r#"{"haa": 1, "id": "p", "title": "P",
                "inputs": [{"name": "pitch", "kind": "trigger"}],
                "graph": {"duration": 2, "nodes": []}}"#,
        )
        .unwrap();

        assert_eq!(patch.master.as_f64(), Some(1.0));
        assert!(!patch.graph.r#loop);
        assert_eq!(patch.graph.mode, GraphMode::Timeline);
        assert!(patch.graph.connections.is_empty());
        assert!(patch.graph.performance.is_none());

        let input = &patch.inputs[0];
        assert_eq!(input.unit, "");
        assert_eq!(input.min, 0.0);
        assert_eq!(input.max, 1.0);
        assert_eq!(input.default, 0.0);
        assert_eq!(input.smooth, DEFAULT_INPUT_SMOOTH_S);
        assert_eq!(input.curve, InputCurve::Lin);
        assert!(input.values.is_empty());
    }

    #[test]
    fn connection_amount_defaults_to_one() {
        let c: Connection =
            serde_json::from_str(r#"{"id": "c1", "from": "a", "to": "b", "target": "audio"}"#)
                .unwrap();
        assert_eq!(c.amount, 1.0);
    }

    #[test]
    fn optional_fields_stay_out_when_absent() {
        let node: NodeSpec = serde_json::from_str(
            r#"{"id": "o", "type": "output", "x": 0, "y": 0, "label": "Out", "params": {}}"#,
        )
        .unwrap();
        let value = serde_json::to_value(&node).unwrap();
        for key in ["curve", "compact", "muted", "bypass"] {
            assert!(value.get(key).is_none(), "{key} must not be written");
        }
        assert_eq!(value["type"], json!("output"));
    }

    #[test]
    fn loader_wraps_projects_and_bare_graphs() {
        let (patch, issues) = load_patch(
            r#"{"version":2,"recipeRevision":8,
            "presetId":"V12 One Pedal","title":"Engine","titleCustom":true,
            "master":{"gain":0.8},"future":42,"graph":{"duration":4,"nodes":[]}}"#,
        )
        .unwrap();
        assert_eq!(patch.id, "v12-one-pedal");
        assert_eq!(patch.title, "Engine");
        assert_eq!(patch.extra["sona"]["presetId"], "V12 One Pedal");
        assert_eq!(patch.extra["sona"]["version"], 2);
        assert_eq!(patch.extra["sona"]["recipeRevision"], 8);
        assert_eq!(patch.extra["sona"]["titleCustom"], true);
        assert_eq!(patch.extra["future"], 42);
        assert_eq!(patch.master, json!({"gain":0.8}));
        assert_eq!(issues[0].code, "wrapped-sona-project");
        let (patch, issues) = load_patch(r#"{"nodes":[],"duration":2,"future":true}"#).unwrap();
        assert_eq!(patch.id, "patch");
        assert_eq!(patch.title, "Untitled");
        assert_eq!(patch.graph.extra["future"], true);
        assert_eq!(issues[0].code, "wrapped-sona-graph");
    }

    #[test]
    fn loader_preserves_unknown_fields_and_converts_pedal_once() {
        let original: Patch = serde_json::from_str(SIREN).unwrap();
        let (mut expected, _) = load_patch(SIREN).unwrap();
        let pedal = expected.inputs.pop().unwrap();
        assert_eq!(expected, original);
        assert_eq!(pedal.name, "pedal");
        assert_eq!((pedal.min, pedal.max, pedal.default), (0.0, 1.0, 0.25));
        assert_eq!(pedal.smooth, DEFAULT_INPUT_SMOOTH_S);
        let (patch, issues) = load_patch(SIREN).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "performance-converted");
        assert_eq!(issues[0].severity, Severity::Info);
        let (again, issues) = load_patch(&to_json(&patch)).unwrap();
        assert_eq!(again, patch);
        assert!(issues.is_empty());
    }

    #[test]
    fn loader_defaults_bad_fields_with_precise_paths() {
        let (patch, issues) = load_patch(
            r#"{"haa":1,"id":false,"inputs":[
            {"name":"gear","kind":"invalid","values":["first",2]}],
            "graph":{"duration":"four","loop":2,"mode":"invalid",
            "nodes":[{},null,{"x":"left","params":[],"muted":"yes"}],
            "connections":[{"amount":false}],"performance":{"manualValue":"half"}}}"#,
        )
        .unwrap();
        assert_eq!(patch.id, "patch");
        assert_eq!(patch.graph.duration, 0.0);
        assert_eq!(patch.graph.nodes[2].x, 0.0);
        assert_eq!(patch.graph.connections[0].amount, 1.0);
        for path in [
            "id",
            "inputs[0].kind",
            "inputs[0].values[1]",
            "graph.duration",
            "graph.loop",
            "graph.mode",
            "graph.nodes[1]",
            "graph.nodes[2].x",
            "graph.nodes[2].params",
            "graph.nodes[2].muted",
            "graph.connections[0].amount",
            "graph.performance.manualValue",
        ] {
            assert!(
                issues.iter().any(|i| i.code == "bad-field"
                    && i.path == path
                    && i.severity == Severity::Error),
                "{path}"
            );
        }
        for json in [
            r#"{"haa":1,"graph":false}"#,
            r#"{"haa":1,"inputs":false,"graph":{"nodes":false,"performance":false}}"#,
        ] {
            let (_, issues) = load_patch(json).unwrap();
            assert!(issues.iter().all(|i| i.code == "bad-field"));
            assert!(!issues.is_empty());
        }
    }

    #[test]
    fn loader_rejects_only_invalid_json_and_non_object_roots() {
        assert_eq!(load_patch("{").unwrap_err().code, "not-json");
        for json in ["null", "[]", "42", "true", "\"patch\""] {
            assert_eq!(load_patch(json).unwrap_err().code, "not-object");
        }
        for version in ["2", "1.5", "4294967296"] {
            let (_, issues) = load_patch(&format!(r#"{{"haa":{version},"graph":{{}}}}"#)).unwrap();
            assert!(
                issues
                    .iter()
                    .any(|i| i.code == "format-too-new" && i.severity == Severity::Error)
            );
        }
    }
}
