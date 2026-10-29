//! Generates the node and DSP module lists from `src/nodes/*.rs` and `src/dsp/*.rs`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    let nodes = rust_files(&manifest.join("src/nodes"));
    let dsp = rust_files(&manifest.join("src/dsp"));

    fs::write(out.join("nodes_gen.rs"), module_source(&nodes, true)).unwrap();
    fs::write(out.join("dsp_gen.rs"), module_source(&dsp, false)).unwrap();

    println!("cargo:rerun-if-changed=src/nodes");
    println!("cargo:rerun-if-changed=src/dsp");
}

fn rust_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs"))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_string();
            Some((stem, path))
        })
        .collect();
    files.sort();
    files
}

fn module_source(files: &[(String, PathBuf)], with_all: bool) -> String {
    let mut source = String::new();
    for (stem, path) in files {
        source.push_str(&format!(
            "#[path = {:?}]\npub mod {stem};\n",
            path.display().to_string()
        ));
    }
    if with_all {
        source.push_str("pub static ALL: &[&crate::catalog::NodeTypeDef] = &[");
        for (stem, _) in files {
            source.push_str(&format!("&{stem}::DEF, "));
        }
        source.push_str("];\n");
    }
    source
}
