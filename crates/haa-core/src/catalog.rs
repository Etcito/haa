//! Node type catalog: the static definition of every node type and the §11 JSON the studio reads.
//! Each node type lives in `nodes/<type>.rs` as `pub static DEF: NodeTypeDef`; `nodes::ALL` lists them.

use crate::consts::{PATCH_FORMAT, SONA_SOUND_REVISION};
use crate::node::{Node, NodeInit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Source,
    Control,
    Effect,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Lin,
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Number { min: f64, max: f64, default: f64, step: f64, scale: Scale },
    Enum { values: &'static [&'static str], default: usize },
    Bool { default: bool },
    Text { default: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamDef {
    pub key: &'static str,
    pub kind: ParamKind,
    pub unit: &'static str,
    pub label_en: &'static str,
    pub label_fr: &'static str,
    pub modulatable: bool,
    pub smooth: bool,
}

pub struct NodeTypeDef {
    pub type_name: &'static str,
    pub category: Category,
    pub label_en: &'static str,
    pub label_fr: &'static str,
    pub audio_in: bool,
    pub audio_out: bool,
    pub control_out: bool,
    pub uses_curve: bool,
    pub hidden: bool,
    pub params: &'static [ParamDef],
    pub create: fn(&NodeInit<'_>) -> Box<dyn Node>,
}

pub fn all() -> &'static [&'static NodeTypeDef] {
    crate::nodes::ALL
}

pub fn find(type_name: &str) -> Option<&'static NodeTypeDef> {
    all().iter().copied().find(|def| def.type_name == type_name)
}

pub fn param_index(def: &NodeTypeDef, key: &str) -> Option<usize> {
    def.params.iter().position(|param| param.key == key)
}

/// The §11 catalog, compact, with types sorted by `type_name`.
pub fn catalog_json() -> String {
    let mut defs: Vec<&NodeTypeDef> = all().to_vec();
    defs.sort_by(|a, b| a.type_name.cmp(b.type_name));

    let mut out = format!("{{\"format\":{PATCH_FORMAT},\"soundRevision\":{SONA_SOUND_REVISION},\"types\":[");
    for (i, def) in defs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_type(&mut out, def);
    }
    out.push_str("]}");
    out
}

fn write_type(out: &mut String, def: &NodeTypeDef) {
    out.push_str("{\"type\":");
    push_str_lit(out, def.type_name);
    out.push_str(",\"category\":");
    push_str_lit(out, category_name(def.category));
    out.push_str(",\"label\":");
    push_label(out, def.label_en, def.label_fr);
    out.push_str(&format!(
        ",\"ports\":{{\"audioIn\":{},\"audioOut\":{},\"controlOut\":{}}}",
        bool_lit(def.audio_in),
        bool_lit(def.audio_out),
        bool_lit(def.control_out)
    ));
    out.push_str(&format!(
        ",\"usesCurve\":{},\"hidden\":{},\"params\":[",
        bool_lit(def.uses_curve),
        bool_lit(def.hidden)
    ));
    for (i, param) in def.params.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_param(out, param);
    }
    out.push_str("]}");
}

fn write_param(out: &mut String, param: &ParamDef) {
    out.push_str("{\"key\":");
    push_str_lit(out, param.key);
    match param.kind {
        ParamKind::Number { min, max, default, step, scale } => {
            out.push_str(&format!(
                ",\"kind\":\"number\",\"min\":{min},\"max\":{max},\"default\":{default},\"step\":{step},\"scale\":"
            ));
            push_str_lit(out, scale_name(scale));
            out.push_str(",\"unit\":");
            push_str_lit(out, param.unit);
        }
        ParamKind::Enum { values, default } => {
            out.push_str(",\"kind\":\"enum\",\"values\":[");
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_str_lit(out, value);
            }
            out.push_str("],\"default\":");
            push_str_lit(out, values.get(default).copied().unwrap_or_default());
        }
        ParamKind::Bool { default } => {
            out.push_str(&format!(",\"kind\":\"bool\",\"default\":{}", bool_lit(default)));
        }
        ParamKind::Text { default } => {
            out.push_str(",\"kind\":\"text\",\"default\":");
            push_str_lit(out, default);
        }
    }
    out.push_str(",\"label\":");
    push_label(out, param.label_en, param.label_fr);
    out.push_str(&format!(
        ",\"modulatable\":{},\"smooth\":{}}}",
        bool_lit(param.modulatable),
        bool_lit(param.smooth)
    ));
}

fn push_label(out: &mut String, en: &str, fr: &str) {
    out.push_str("{\"en\":");
    push_str_lit(out, en);
    out.push_str(",\"fr\":");
    push_str_lit(out, fr);
    out.push('}');
}

fn push_str_lit(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn bool_lit(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::Source => "source",
        Category::Control => "control",
        Category::Effect => "effect",
        Category::Output => "output",
    }
}

fn scale_name(scale: Scale) -> &'static str {
    match scale {
        Scale::Lin => "lin",
        Scale::Log => "log",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{NodeOut, SampleCtx};
    use std::collections::BTreeSet;

    struct Silent;

    impl Node for Silent {
        fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), _params: &[f64]) -> NodeOut {
            NodeOut { l: 0.0, r: 0.0, value: 0.0 }
        }

        fn reset(&mut self) {}
    }

    fn silent(_init: &NodeInit<'_>) -> Box<dyn Node> {
        Box::new(Silent)
    }

    static TEST_PARAMS: [ParamDef; 2] = [
        ParamDef {
            key: "cutoff",
            kind: ParamKind::Number { min: 20.0, max: 20000.0, default: 1000.0, step: 1.0, scale: Scale::Log },
            unit: "Hz",
            label_en: "Cutoff",
            label_fr: "Coupure",
            modulatable: true,
            smooth: true,
        },
        ParamDef {
            key: "shape",
            kind: ParamKind::Enum { values: &["lp", "hp"], default: 1 },
            unit: "",
            label_en: "Shape",
            label_fr: "Forme",
            modulatable: false,
            smooth: false,
        },
    ];

    static TEST_DEF: NodeTypeDef = NodeTypeDef {
        type_name: "test-filter",
        category: Category::Effect,
        label_en: "Test filter",
        label_fr: "Filtre test",
        audio_in: true,
        audio_out: true,
        control_out: false,
        uses_curve: false,
        hidden: false,
        params: &TEST_PARAMS,
        create: silent,
    };

    fn file_stems() -> BTreeSet<String> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/nodes");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return BTreeSet::new();
        };
        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("rs"))
            .filter_map(|path| path.file_stem().and_then(|s| s.to_str()).map(str::to_string))
            .collect()
    }

    #[test]
    fn type_names_are_unique_and_match_file_stems() {
        let names: Vec<&str> = all().iter().map(|def| def.type_name).collect();
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len());
        let names: BTreeSet<String> = names.iter().map(|name| name.to_string()).collect();
        assert_eq!(names, file_stems());
    }

    #[test]
    fn catalog_json_parses_and_lists_every_type() {
        let json: serde_json::Value = serde_json::from_str(&catalog_json()).unwrap();
        assert_eq!(json["format"], PATCH_FORMAT);
        assert_eq!(json["soundRevision"], SONA_SOUND_REVISION);
        assert_eq!(json["types"].as_array().unwrap().len(), all().len());
    }

    #[test]
    fn type_json_uses_spec_keys_and_string_enum_default() {
        let mut out = String::new();
        write_type(&mut out, &TEST_DEF);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["type"], "test-filter");
        assert_eq!(json["category"], "effect");
        assert_eq!(json["ports"]["audioIn"], true);
        assert_eq!(json["params"][0]["scale"], "log");
        assert_eq!(json["params"][0]["unit"], "Hz");
        assert_eq!(json["params"][1]["kind"], "enum");
        assert_eq!(json["params"][1]["default"], "hp");
    }

    #[test]
    fn param_index_finds_key_by_name() {
        assert_eq!(param_index(&TEST_DEF, "cutoff"), Some(0));
        assert_eq!(param_index(&TEST_DEF, "shape"), Some(1));
        assert_eq!(param_index(&TEST_DEF, "missing"), None);
    }
}
