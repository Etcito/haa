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
        Some("fit") => fit(&args[1..]),
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

// ---------------------------------------------------------------------------
// fit: haa-fit (the Decompose fitter) natively, one thread, on a target WAV.
// ---------------------------------------------------------------------------
const FIT_USAGE: &str =
    "usage: haa fit <target.wav> --template <patch.json> [--template ...] [--impact] [--options <json|file>] --out <result.json> [--patch <best.json>] [--render <best.wav>] [--sfx <id> --title <text> [--loudness <dB>]]";

fn fit(args: &[String]) -> i32 {
    let mut target: Option<String> = None;
    let mut templates: Vec<String> = Vec::new();
    let mut options = String::from("{}");
    let mut impact = false;
    let (mut sfx_id, mut title, mut loudness): (Option<String>, Option<String>, Option<f64>) = (None, None, None);
    let (mut out, mut patch_out, mut render_out): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--impact" {
            impact = true;
        } else if let Some((flag, inline)) = split_option(arg) {
            let value = match inline {
                Some(v) => v.to_string(),
                None => {
                    i += 1;
                    match args.get(i) {
                        Some(v) => v.clone(),
                        None => return fail(format!("{flag} needs a value")),
                    }
                }
            };
            match flag {
                "--template" => templates.push(value),
                "--options" => options = value,
                "--out" => out = Some(value),
                "--patch" => patch_out = Some(value),
                "--render" => render_out = Some(value),
                "--sfx" => sfx_id = Some(value),
                "--title" => title = Some(value),
                "--loudness" => match parse_f64(flag, &value) {
                    Ok(v) => loudness = Some(v),
                    Err(e) => return fail(e),
                },
                _ => return fail(format!("unknown option: {flag}")),
            }
        } else if target.is_none() {
            target = Some(arg.to_string());
        } else {
            return fail(format!("unexpected argument: {arg}"));
        }
        i += 1;
    }
    let (Some(target), Some(out)) = (target, out) else { return fail(FIT_USAGE) };
    if templates.is_empty() && !impact {
        return fail(FIT_USAGE);
    }
    let bytes = match std::fs::read(&target) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot read {target}: {e}")),
    };
    let (pcm, rate) = match wav::decode_wav_mono(&bytes) {
        Ok(x) => x,
        Err(e) => return fail(format!("{target}: {e}")),
    };
    let mut texts = Vec::new();
    for t in &templates {
        match std::fs::read_to_string(t) {
            Ok(s) => texts.push(s),
            Err(e) => return fail(format!("cannot read {t}: {e}")),
        }
    }
    let options_text = if options.trim_start().starts_with('{') {
        options
    } else {
        match std::fs::read_to_string(&options) {
            Ok(s) => s,
            Err(e) => return fail(format!("cannot read {options}: {e}")),
        }
    };
    let options_json: serde_json::Value = match serde_json::from_str(&options_text) {
        Ok(v) => v,
        Err(e) => return fail(format!("bad --options: {e}")),
    };
    let opts = haa_fit::FitOptions::from_json(&options_json);
    if impact {
        // The template is measured on the target as the fitter will see it (peakDb applied).
        let peak = pcm.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let scale = opts.peak_db.map_or(1.0, |db| 10f64.powf(db / 20.0) as f32 / peak.max(1e-9));
        let scaled: Vec<f32> = pcm.iter().map(|x| x * scale).collect();
        let (text, report) = haa_fit::impact::template(&scaled, rate as f64, "struck-object");
        eprintln!("[haa] impact template {report}");
        texts.insert(0, text);
    }
    let mut fitter = match haa_fit::Fitter::new(&pcm, rate as f64, &texts, opts) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    while fitter.step(64) {
        eprintln!("[haa] fit {}", fitter.progress_json());
    }
    let result = fitter.result_json();
    if let Err(e) = std::fs::write(&out, &result) {
        return fail(format!("cannot write {out}: {e}"));
    }
    let seconds = pcm.len() as f64 / rate as f64;
    let render_opts = RenderOptions { seconds: Some(seconds), sample_rate: rate, ..RenderOptions::default() };
    let mut best = fitter.best_patch_json();
    // --sfx: the fitted patch as a scene preset (an energy input scales its level), at --loudness.
    if let (Some(id), Some(text)) = (&sfx_id, &best) {
        let level = match (loudness, haa_core::render::render(text, &render_opts)) {
            (Some(want), Ok(r)) => 10f64.powf((want - loudest_db(&r.left, &r.right, r.sample_rate, 0.05)) / 20.0),
            (Some(_), Err(issues)) => return report_issues(&issues),
            (None, _) => 1.0,
        };
        match haa_fit::impact::sfx_preset(text, id, title.as_deref().unwrap_or(id), level) {
            Ok(p) => best = Some(p),
            Err(e) => return fail(e),
        }
    }
    if let (Some(path), Some(text)) = (&patch_out, &best) {
        if let Err(e) = std::fs::write(path, text) {
            return fail(format!("cannot write {path}: {e}"));
        }
    }
    if let (Some(path), Some(text)) = (&render_out, &best) {
        match haa_core::render::render(text, &render_opts) {
            Ok(r) => {
                if let Err(e) = wav::write_wav(Path::new(path), &r.left, &r.right, r.sample_rate) {
                    return fail(format!("cannot write {path}: {e}"));
                }
            }
            Err(issues) => return report_issues(&issues),
        }
    }
    println!("[haa] fit done: best {} -> {out}", fitter.best_loss());
    0
}

/// The loudest moving-window mean power of (L+R)/2 over `window` s, dB.
fn loudest_db(left: &[f32], right: &[f32], sample_rate: u32, window: f64) -> f64 {
    let n = ((window * sample_rate as f64) as usize).max(1);
    let sq: Vec<f64> = left.iter().zip(right).map(|(l, r)| (0.5 * (*l as f64 + *r as f64)).powi(2)).collect();
    let (mut sum, mut best) = (0.0f64, 0.0f64);
    for i in 0..sq.len() {
        sum += sq[i];
        if i >= n {
            sum -= sq[i - n];
        }
        best = best.max(sum / n as f64);
    }
    10.0 * (best + 1e-20).log10()
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
