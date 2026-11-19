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
        Some("speak") => speak(&args[1..]),
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
    /// A tiny-voca GGUF for `speak` nodes (`--tinyvoca`).
    tinyvoca: Option<String>,
    options: RenderOptions,
}

fn render(args: &[String]) -> i32 {
    let parsed = match parse_render_args(args) {
        Ok(parsed) => parsed,
        Err(reason) => return fail(reason),
    };
    if let Some(gguf) = &parsed.tinyvoca
        && let Err(reason) = load_tinyvoca(gguf)
    {
        return fail(reason);
    }
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
    let mut tinyvoca: Option<&str> = None;
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
                "--tinyvoca" => tinyvoca = Some(value),
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
        tinyvoca: tinyvoca.map(str::to_string),
        options,
    })
}

// ---------------------------------------------------------------------------
// speak: English text through tiny-voca natively (the wasm `speak` op, the same Rust).
// ---------------------------------------------------------------------------
const SPEAK_USAGE: &str = "usage: haa speak <text> --tinyvoca <tiny-voca-js.gguf> [--voice <voice.tvv.json>] [--seed N] --out <file.wav | file.f32>";

/// Loads a tiny-voca GGUF as the shared model, and its phonemizer when the file carries one.
fn load_tinyvoca(path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    tinyvoca::install(tinyvoca::Model::from_gguf(&bytes)?);
    if let Ok(p) = dizione::Phonemizer::from_gguf(&bytes) {
        dizione::install(p);
    }
    Ok(())
}

fn speak(args: &[String]) -> i32 {
    let (mut text, mut gguf, mut voice, mut out, mut seed) = (None, None, None, None, 0u32);
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some((flag, inline)) = split_option(arg) {
            let value = match inline.or_else(|| {
                i += 1;
                args.get(i).map(String::as_str)
            }) {
                Some(value) => value,
                None => return fail(format!("{flag} needs a value")),
            };
            match flag {
                "--tinyvoca" => gguf = Some(value),
                "--voice" => voice = Some(value),
                "--out" => out = Some(value),
                "--seed" => match value.parse() {
                    Ok(n) => seed = n,
                    Err(_) => return fail(format!("bad --seed: {value}")),
                },
                other => return fail(format!("unknown option: {other}")),
            }
        } else if text.is_none() {
            text = Some(arg);
        } else {
            return fail(format!("unexpected argument: {arg}"));
        }
        i += 1;
    }
    let (Some(text), Some(gguf), Some(out)) = (text, gguf, out) else {
        return fail(SPEAK_USAGE);
    };
    if let Err(reason) = load_tinyvoca(gguf) {
        return fail(reason);
    }
    let (m2, post) = match voice {
        None => (1.0, tinyvoca::Post::default()),
        Some(path) => match std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}")).and_then(|t| tinyvoca::VoiceFile::parse(&t)) {
            Ok(file) => (file.m2_weight(), file.post),
            Err(reason) => return fail(reason),
        },
    };
    let audio = match haa_core::nodes::speak::speak_script(text, m2, post, seed) {
        Ok(spoken) => {
            for warning in &spoken.warnings {
                eprintln!("[haa] script warning: {warning}");
            }
            spoken.audio
        }
        Err(reason) => return fail(reason),
    };
    let rate = tinyvoca::shared().map_or(24_000, |m| m.config.sample_rate as u32);
    let written = if out.ends_with(".f32") {
        std::fs::write(out, audio.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
    } else {
        wav::write_wav(Path::new(out), &audio, &audio, rate)
    };
    if let Err(e) = written {
        return fail(format!("cannot write {out}: {e}"));
    }
    println!("[haa] wrote {out} ({} samples, {rate} Hz)", audio.len());
    0
}

// ---------------------------------------------------------------------------
// fit: haa-fit (the Decompose fitter) natively, one thread, on a target WAV.
// ---------------------------------------------------------------------------
const FIT_USAGE: &str =
    "usage: haa fit <target.wav> --template <patch.json> [--template ...] [--impact [--keep-room]] [--stroke [--modes-below <Hz>]] [--options <json|file>] --out <result.json> [--patch <best.json>] [--render <best.wav>] [--sfx <id> --title <text> [--loudness <dB>]]";

fn fit(args: &[String]) -> i32 {
    let mut target: Option<String> = None;
    let mut templates: Vec<String> = Vec::new();
    let mut options = String::from("{}");
    let (mut impact, mut stroke, mut keep_room) = (false, false, false);
    let (mut sfx_id, mut title, mut loudness): (Option<String>, Option<String>, Option<f64>) = (None, None, None);
    let mut modes_below = f64::INFINITY;
    let (mut out, mut patch_out, mut render_out): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--stroke" {
            stroke = true;
        } else if arg == "--impact" {
            impact = true;
        } else if arg == "--keep-room" {
            keep_room = true;
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
                "--modes-below" => match parse_f64(flag, &value) {
                    Ok(v) if v > 0.0 => modes_below = v,
                    Ok(_) => return fail(format!("{flag} must be > 0")),
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
    if templates.is_empty() && !impact && !stroke {
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
    // --impact fits the source alone: a recording's room tail is the world reverb's to play
    // (haa_fit::room). --keep-room fits the recording as it is.
    let pcm = if impact && !keep_room {
        let (dry, room) = haa_fit::room::separate(&pcm, rate as f64);
        match room {
            Ok(r) => println!("[haa] room: {} (target cut to {:.3} s)", r.json(), dry.len() as f64 / rate as f64),
            Err(why) => println!("[haa] room: none ({why}), the target is fitted whole"),
        }
        dry
    } else {
        pcm
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
    if impact || stroke {
        // The template is measured on the target as the fitter will see it (peakDb applied).
        let peak = pcm.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let scale = opts.peak_db.map_or(1.0, |db| 10f64.powf(db / 20.0) as f32 / peak.max(1e-9));
        let scaled: Vec<f32> = pcm.iter().map(|x| x * scale).collect();
        if impact {
            let (text, report) = haa_fit::impact::template(&scaled, rate as f64, "struck-object");
            eprintln!("[haa] impact template {report}");
            texts.insert(0, text);
        }
        if stroke {
            let (text, report) = haa_fit::impact::stroke_template_below(&scaled, rate as f64, "stroke", modes_below);
            eprintln!("[haa] stroke template {report}");
            texts.insert(0, text);
        }
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
        // The level of a take depends on its noise (each take of a preset has its own seed): a short
        // one-shot's loudest 50 ms varied 5 dB over 8 seeds (round 16, the crate drop), and seed 0 alone
        // set every take's level off by up to that. The level is set on the mean power of 8 takes.
        let level = match loudness {
            Some(want) => {
                let mut power = 0.0;
                for seed in 0..TAKES {
                    match haa_core::render::render(text, &RenderOptions { seed, ..render_opts.clone() }) {
                        Ok(r) => power += 10f64.powf(loudest_db(&r.left, &r.right, r.sample_rate, 0.05) / 10.0),
                        Err(issues) => return report_issues(&issues),
                    }
                }
                let mean_db = 10.0 * (power / TAKES as f64).log10();
                10f64.powf((want - mean_db) / 20.0)
            }
            None => 1.0,
        };
        let wrap = if stroke { haa_fit::impact::slide_preset } else { haa_fit::impact::sfx_preset };
        // The fit was judged at the target's level (peakDb); at --loudness the voice limiter can engage
        // (a sub-millisecond strike spike ducks the attack for its 60 ms release), and a limited preset
        // is no longer linear in energy. Lower the preset 1 dB at a time until energy 1 is unlimited;
        // the scene's slot gain (after the voice) gives the cut back.
        let mut renderer = haa_fit::eval::Renderer::new(rate as f64, pcm.len());
        let mut cut_db = 0.0;
        let preset = loop {
            let p = match wrap(text, id, title.as_deref().unwrap_or(id), level * 10f64.powf(-cut_db / 20.0)) {
                Ok(p) => p,
                Err(e) => return fail(e),
            };
            let limited = match haa_core::load_patch(&p) {
                Ok((patch, _)) => match renderer.render(&patch) {
                    Some(_) => renderer.limited_db(),
                    None => 0.0,
                },
                Err(e) => return fail(format!("{e:?}")),
            };
            if limited <= 0.1 || cut_db >= 40.0 {
                break p;
            }
            cut_db += 1.0;
        };
        if cut_db > 0.0 {
            println!("[haa] sfx: preset {cut_db} dB under --loudness (the voice limiter engaged above): give its scene slot +{cut_db} dB");
        }
        best = Some(preset);
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

/// Takes (seeds) whose mean loudness sets a fitted preset's level (`haa fit --sfx --loudness`).
const TAKES: u32 = 8;

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
