//! `haa` native CLI (ARCHITECTURE §8): hand-written argument parsing over haa-core.

mod inputs_dts;
mod wav;

use std::path::Path;

use haa_core::Issue;
use haa_core::issues::Severity;
use haa_core::render::{InputValue, RenderOptions};

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
        None => fail("usage: haa render <patch.json> --out <file.wav>"),
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
// render: read a patch, render it offline and write a 16-bit stereo WAV.
// ---------------------------------------------------------------------------
struct RenderArgs {
    patch: String,
    out: String,
    options: RenderOptions,
}

fn render(args: &[String]) -> i32 {
    let parsed = match parse_render_args(args) {
        Ok(parsed) => parsed,
        Err(reason) => return fail(reason),
    };
    let source = match std::fs::read_to_string(&parsed.patch) {
        Ok(source) => source,
        Err(e) => return fail(format!("cannot read {}: {e}", parsed.patch)),
    };
    let result = match haa_core::render::render(&source, &parsed.options) {
        Ok(result) => result,
        Err(issues) => return report_issues(&issues),
    };

    for issue in result
        .issues
        .iter()
        .filter(|issue| issue.severity != Severity::Info)
    {
        print_issue(issue);
    }

    if let Err(e) = wav::write_wav(
        Path::new(&parsed.out),
        &result.left,
        &result.right,
        result.sample_rate,
    ) {
        return fail(format!("cannot write {}: {e}", parsed.out));
    }
    println!(
        "[haa] wrote {} ({} frames, {} Hz)",
        parsed.out,
        result.left.len(),
        result.sample_rate
    );
    0
}

fn parse_render_args(args: &[String]) -> Result<RenderArgs, String> {
    let mut patch: Option<&str> = None;
    let mut out: Option<&str> = None;
    let mut options = RenderOptions::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some((flag, inline)) = split_option(arg) {
            let value = match inline {
                Some(value) => value,
                None => {
                    i += 1;
                    args.get(i)
                        .map(String::as_str)
                        .ok_or_else(|| format!("{flag} needs a value"))?
                }
            };
            match flag {
                "--out" => out = Some(value),
                "--seconds" => {
                    options.seconds = Some(parse_f64(flag, value)?);
                }
                "--seed" => {
                    options.seed = value
                        .parse::<u32>()
                        .map_err(|_| format!("bad --seed: {value}"))?;
                }
                "--rate" => {
                    let rate = value
                        .parse::<u32>()
                        .map_err(|_| format!("bad --rate: {value}"))?;
                    if rate == 0 {
                        return Err("bad --rate: 0".to_string());
                    }
                    options.sample_rate = rate;
                }
                "--input" => {
                    let (name, raw) = value
                        .split_once('=')
                        .ok_or_else(|| format!("bad --input (want name=value): {value}"))?;
                    if name.is_empty() {
                        return Err(format!("bad --input (want name=value): {value}"));
                    }
                    let input = match raw.parse::<f64>() {
                        Ok(number) => InputValue::Number(number),
                        Err(_) => InputValue::Name(raw.to_string()),
                    };
                    options.inputs.push((name.to_string(), input));
                }
                "--ramp" => {
                    let (name, range) = value
                        .split_once('=')
                        .ok_or_else(|| format!("bad --ramp (want name=a:b): {value}"))?;
                    let (a, b) = range
                        .split_once(':')
                        .ok_or_else(|| format!("bad --ramp (want name=a:b): {value}"))?;
                    if name.is_empty() {
                        return Err(format!("bad --ramp (want name=a:b): {value}"));
                    }
                    options.ramps.push((
                        name.to_string(),
                        parse_f64("--ramp", a)?,
                        parse_f64("--ramp", b)?,
                    ));
                }
                "--trigger" => {
                    let (name, at) = value
                        .split_once('@')
                        .ok_or_else(|| format!("bad --trigger (want name@seconds): {value}"))?;
                    if name.is_empty() {
                        return Err(format!("bad --trigger (want name@seconds): {value}"));
                    }
                    options
                        .triggers
                        .push((name.to_string(), parse_f64("--trigger", at)?));
                }
                other => return Err(format!("unknown option: {other}")),
            }
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("unknown option: {arg}"));
        } else if patch.is_none() {
            patch = Some(arg);
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
        i += 1;
    }

    let patch = patch.ok_or_else(|| {
        "usage: haa render <patch.json> --out <file.wav> [--seconds S] [--seed N] [--rate HZ]"
            .to_string()
    })?;
    let out = out.ok_or_else(|| "render needs --out <file.wav>".to_string())?;
    Ok(RenderArgs {
        patch: patch.to_string(),
        out: out.to_string(),
        options,
    })
}

/// Splits `--flag=value` into its flag and inline value; `None` for a bare value or short flag.
fn split_option(arg: &str) -> Option<(&str, Option<&str>)> {
    if !arg.starts_with("--") {
        return None;
    }
    match arg.split_once('=') {
        Some((flag, value)) => Some((flag, Some(value))),
        None => Some((arg, None)),
    }
}

fn parse_f64(flag: &str, value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(number) if number.is_finite() => Ok(number),
        _ => Err(format!("bad {flag}: {value}")),
    }
}

/// Prints every issue and returns exit code 1. The reasons are the issue lines.
fn report_issues(issues: &[Issue]) -> i32 {
    let mut printed = 0;
    for issue in issues
        .iter()
        .filter(|issue| issue.severity != Severity::Info)
    {
        print_issue(issue);
        printed += 1;
    }
    if printed == 0 {
        return fail("patch is not playable");
    }
    1
}

fn print_issue(issue: &Issue) {
    let severity = match issue.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    };
    eprintln!(
        "[haa] {severity} {} {}: {}",
        issue.code, issue.path, issue.message
    );
}
