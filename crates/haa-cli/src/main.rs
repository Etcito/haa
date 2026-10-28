//! `haa` native CLI (ARCHITECTURE §8): hand-written argument parsing over haa-core.

mod inputs_dts;
#[allow(dead_code)]
mod wav;

use std::path::Path;

/// Print one `[haa] <reason>` line on stderr and return exit code 1.
fn fail(reason: impl std::fmt::Display) -> i32 {
    eprintln!("[haa] {reason}");
    1
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    if code != 0 {
        std::process::exit(code);
    }
}

fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("inputs") => inputs(&args[1..]),
        Some("render") => render(&args[1..]),
        Some(other) => fail(format!("unknown command: {other}")),
        None => fail("usage: haa inputs <patch.json> [--out <file.d.ts>]"),
    }
}

fn inputs(args: &[String]) -> i32 {
    let mut patch_path: Option<&str> = None;
    let mut out: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--out" {
            i += 1;
            match args.get(i) {
                Some(value) => out = Some(value),
                None => return fail("--out needs a file"),
            }
        } else if let Some(value) = arg.strip_prefix("--out=") {
            out = Some(value);
        } else if arg.starts_with('-') && arg != "-" {
            return fail(format!("unknown option: {arg}"));
        } else if patch_path.is_none() {
            patch_path = Some(arg);
        } else {
            return fail(format!("unexpected argument: {arg}"));
        }
        i += 1;
    }

    let Some(path) = patch_path else {
        return fail("usage: haa inputs <patch.json> [--out <file.d.ts>]");
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => return fail(format!("cannot read {path}: {e}")),
    };
    let patch = match haa_core::load_patch(&source) {
        Ok((patch, _issues)) => patch,
        Err(issue) => return fail(format!("{}: {}", issue.code, issue.message)),
    };
    let file_name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let text = inputs_dts::inputs_dts(&patch, &file_name);

    match out {
        Some(out) => {
            if let Some(parent) = Path::new(out).parent() {
                if !parent.as_os_str().is_empty() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return fail(format!("cannot create {}: {e}", parent.display()));
                    }
                }
            }
            if let Err(e) = std::fs::write(out, text) {
                return fail(format!("cannot write {out}: {e}"));
            }
        }
        None => print!("{text}"),
    }
    0
}

// ---------------------------------------------------------------------------
// render: the next task replaces this branch with the haa_core::render call.
// ---------------------------------------------------------------------------
fn render(_args: &[String]) -> i32 {
    fail("render is not available yet")
}
