//! Patch format (ARCHITECTURE §7–§8): the serde types of a HAA patch, its published inputs and its graph.
//! Every type keeps unknown keys in `extra`, so a saved patch keeps the keys this engine does not read.

use crate::consts::DEFAULT_INPUT_SMOOTH_S;
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
}
