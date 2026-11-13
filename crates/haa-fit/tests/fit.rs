//! Fitter: the param space a template exposes, the structure mutations, and a stepped fit that
//! recovers a detuned tone inside its budget with a patch haa-core validates.

use haa_core::issues::has_errors;
use haa_fit::eval::Renderer;
use haa_fit::{FitOptions, Fitter, describe_params, mutate};
use serde_json::Value;

const RATE: f64 = 16_000.0;

fn tone_patch(freq: f64, curve: &str) -> String {
    format!(
        r#"{{"haa":1,"id":"tone","title":"Tone","graph":{{"duration":0.6,"loop":false,"mode":"timeline","nodes":[
          {{"id":"osc","type":"oscillator","x":0,"y":0,"label":"Osc","params":{{"frequency":{freq},"wave":"sine","level":0.35}}}},
          {{"id":"env","type":"curve","x":0,"y":0,"label":"Env","params":{{"amount":1}},"curve":{curve}}},
          {{"id":"amp","type":"gain","x":0,"y":0,"label":"Gain","params":{{"gain":0.6}}}},
          {{"id":"out","type":"output","x":0,"y":0,"label":"Out","params":{{"gain":0.75}}}}],
          "connections":[
            {{"id":"c1","from":"osc","to":"amp","target":"audio","amount":1}},
            {{"id":"c2","from":"amp","to":"out","target":"audio","amount":1}},
            {{"id":"c3","from":"env","to":"amp","target":"gain","amount":1}}]}}}}"#
    )
}

#[test]
fn describe_lists_numeric_params_and_curve_points_only() {
    let json: Value = serde_json::from_str(&describe_params(&tone_patch(220.0, "[[0,0],[0.25,1],[0.75,1],[1,0]]"), 6).unwrap()).unwrap();
    let params = json["params"].as_array().unwrap();
    let freq = params.iter().find(|p| p["node"] == "osc" && p["key"] == "frequency").unwrap();
    assert_eq!(freq["scale"], "log");
    assert_eq!(freq["value"], 220.0);
    assert!(params.iter().any(|p| p["key"] == "curve[1].x" && p["kind"] == "curve-x"));
    assert!(!params.iter().any(|p| p["key"] == "wave" || p["key"] == "clock" || p["key"] == "retrigger"));
    assert_eq!(params.iter().filter(|p| p["selected"] == true).count(), 6);
}

#[test]
fn every_mutation_uses_catalog_nodes_and_validates() {
    let (patch, _) = haa_core::load_patch(&tone_patch(220.0, "[[0,0],[0.25,1],[0.75,1],[1,0]]")).unwrap();
    let kinds = mutate::candidates(&patch);
    assert!(kinds.iter().any(|k| matches!(k, mutate::MutationKind::Layer(..))));
    for kind in &kinds {
        let m = mutate::apply(&patch, kind).unwrap_or_else(|| panic!("{kind:?} does not apply"));
        let (_, issues) = haa_core::validate(&m.patch).unwrap();
        assert!(!has_errors(&issues), "{kind:?}: {issues:?}");
        for node in &m.patch.graph.nodes {
            assert!(haa_core::catalog::find(&node.kind).is_some(), "{}", node.kind);
            if let Some(points) = node.curve.as_ref().and_then(Value::as_array) {
                assert!(points.len() <= 128);
            }
        }
        if let mutate::MutationKind::Insert(t, ..) = kind {
            let added = m.patch.graph.nodes.iter().find(|n| m.added.contains(&n.id)).unwrap();
            assert_eq!(added.kind, *t);
            assert!(m.patch.graph.connections.iter().any(|c| c.from == added.id && c.to == "out"));
        }
    }
}

#[test]
fn stepped_fit_recovers_a_detuned_tone_within_budget() {
    let target_patch = tone_patch(330.0, "[[0,0],[0.08,1],[0.45,0.6],[1,0]]");
    let (patch, _) = haa_core::load_patch(&target_patch).unwrap();
    let target = Renderer::new(RATE, (0.6 * RATE) as usize).render(&patch).unwrap().to_vec();
    let templates = vec![tone_patch(220.0, "[[0,0],[0.25,1],[0.75,1],[1,0]]")];
    let opts = FitOptions { budget: 120, mutations: 0, ..FitOptions::default() };
    let mut fitter = Fitter::new(&target, RATE, &templates, opts).unwrap();
    let mut calls = 0;
    while fitter.step(7) {
        calls += 1;
        assert!(fitter.evals() <= 120);
    }
    assert!(calls > 3, "the fit must be stepped, not run in one call");
    let result: Value = serde_json::from_str(&fitter.result_json()).unwrap();
    assert_eq!(result["evals"], 120);
    let score0 = result["templates"][0]["score0"].as_f64().unwrap();
    let best = result["best"]["score"].as_f64().unwrap();
    assert!(best < score0, "best {best} vs screening {score0}");
    let fitted = &result["best"]["patch"];
    let freq = fitted["graph"]["nodes"][0]["params"]["frequency"].as_f64().unwrap();
    assert!((freq / 330.0 - 1.0).abs() < 0.02, "fitted frequency {freq}");
    let text = fitter.best_patch_json().unwrap();
    let (fitted, issues) = haa_core::load_patch(&text).unwrap();
    assert!(!has_errors(&issues));
    assert!(haa_core::validate(&fitted).is_ok_and(|(_, issues)| !has_errors(&issues)));
}
