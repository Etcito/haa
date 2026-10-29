//! Integration coverage for exported Sona presets and projects.

use std::{fs, path::PathBuf};

use haa_core::{
    Issue, Voice,
    issues::{Severity, has_errors},
    load_patch, validate,
};
use serde_json::{Value, json};

fn presets() -> Vec<PathBuf> {
    let mut files: Vec<_> =
        fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../patches/sona"))
            .expect("read Sona presets")
            .map(|entry| entry.expect("read preset entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
    files.sort();
    assert!(
        files.len() >= 14,
        "expected at least 14 Sona presets, found {}",
        files.len()
    );
    files
}

fn first_error(issues: &[Issue]) -> String {
    issues
        .iter()
        .find(|issue| issue.severity == Severity::Error)
        .map(|issue| format!("{} {}: {}", issue.code, issue.path, issue.message))
        .unwrap_or_else(|| "no error issue supplied".to_owned())
}

fn read(path: &PathBuf) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn every_sona_preset_validates_with_zero_errors() {
    for path in presets() {
        let json = read(&path);
        let (patch, issues) = load_patch(&json).unwrap_or_else(|issue| {
            panic!(
                "{}: {} {}: {}",
                path.display(),
                issue.code,
                issue.path,
                issue.message
            )
        });
        assert!(
            !has_errors(&issues),
            "{}: {}",
            path.display(),
            first_error(&issues)
        );
        let issues = match validate(&patch) {
            Ok((_, issues)) => issues,
            Err(issues) => panic!("{}: {}", path.display(), first_error(&issues)),
        };
        assert!(
            !has_errors(&issues),
            "{}: {}",
            path.display(),
            first_error(&issues)
        );
        assert_eq!(
            patch.id,
            path.file_stem().unwrap().to_str().unwrap(),
            "{}: header id",
            path.display()
        );
    }
}

#[test]
fn v12_patches_publish_the_pedal_input() {
    let files = presets();
    for name in ["v12.json", "v12-live.json"] {
        let path = files
            .iter()
            .find(|path| path.file_name().is_some_and(|file| file == name))
            .unwrap_or_else(|| panic!("missing {name}"));
        let (patch, issues) = load_patch(&read(&path)).unwrap_or_else(|issue| {
            panic!(
                "{}: {} {}: {}",
                path.display(),
                issue.code,
                issue.path,
                issue.message
            )
        });
        assert!(
            patch.inputs.iter().any(|input| input.name == "pedal"),
            "{}: missing pedal",
            path.display()
        );
        assert!(
            !issues
                .iter()
                .any(|issue| issue.code == "performance-converted"),
            "{}: pedal was converted",
            path.display()
        );
    }
}

#[test]
fn every_sona_preset_renders_half_a_second_finite() {
    for path in presets() {
        let mut voice = Voice::new(48_000.0, 0);
        let outcome = voice.load(&read(&path), false);
        assert!(
            outcome.playable,
            "{}: {}",
            path.display(),
            first_error(&outcome.issues)
        );
        let mut left = [0.0_f32; 128];
        let mut right = [0.0_f32; 128];
        let mut remaining = 24_000;
        while remaining > 0 {
            let frames = remaining.min(128);
            voice.process(&mut left[..frames], &mut right[..frames]);
            for sample in left[..frames].iter().chain(&right[..frames]) {
                assert!(
                    sample.is_finite() && sample.abs() <= 0.92_f32,
                    "{}: invalid sample {sample}",
                    path.display()
                );
            }
            remaining -= frames;
        }
        assert_eq!(voice.guard_trips(), 0, "{}: guard tripped", path.display());
    }
}

#[test]
fn every_sona_preset_loads_as_a_sona_v2_project() {
    for path in presets() {
        let patch: Value = serde_json::from_str(&read(&path))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let project = json!({
            "version": 2, "recipeRevision": 8, "title": patch["title"],
            "titleCustom": false, "presetId": patch["id"],
            "graph": patch["graph"], "master": patch["master"]
        });
        let (patch, issues) = load_patch(&project.to_string()).unwrap_or_else(|issue| {
            panic!(
                "{}: {} {}: {}",
                path.display(),
                issue.code,
                issue.path,
                issue.message
            )
        });
        assert!(
            !has_errors(&issues),
            "{}: {}",
            path.display(),
            first_error(&issues)
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.severity == Severity::Info
                    && issue.code == "wrapped-sona-project"),
            "{}: missing wrapped-sona-project info",
            path.display()
        );
        let issues = match validate(&patch) {
            Ok((_, issues)) => issues,
            Err(issues) => panic!("{}: {}", path.display(), first_error(&issues)),
        };
        assert!(
            !has_errors(&issues),
            "{}: {}",
            path.display(),
            first_error(&issues)
        );
    }
}
