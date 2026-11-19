//! tiny-voca speech export group (crates/tinyvoca): load the GGUF, then run the model whole or stage by stage.
//!
//! `haa_tinyvoca_load(ptr, len) -> i32`: the bytes of an official tiny-voca GGUF (the host fetches it; nothing is
//! bundled). 0 on success, after which the `speak` node and `haa_tinyvoca_run` use it; -1 with
//! `{"error"}` in the result.
//!
//! `haa_tinyvoca_info() -> u32`: result `{"loaded", "sampleRate", "hop", "speakers", "heads":[98,36]}`.
//!
//! `haa_tinyvoca_run(ptr, len) -> i32`: request JSON `{"op", ...}`, `voice` being `"M2"`/`"F2"` or a voice.tvv.json
//! v3 object (absent = M2). Returns the result length in bytes, or -1 with `{"error"}` in the result.
//! - `encode {tokens, stress, voice}` -> `{hidden [N*C], durationRaw [N*2], durations [N]}`
//! - `durations {tokens, durationRaw, speed}` -> `{durations}`
//! - `expand {tokens, hidden, durations, voice}` -> `{logF0, voiced, energy, prosodyHidden [T*C], owner}`
//!   (expansion + frame transformer A)
//! - `decode {prosodyHidden, logF0, voiced, energy}` -> `{harmonic [T*98], noise [T*36]}`
//! - `envelope {harmonic, noise, logF0, voice}` -> `{harmonic, noise}`: the voice's post envelope edits
//!   (warp, tilt, env_offset_db, breath, phonation; `logF0` is the oscillator's)
//! - `vocode {harmonic, noise, logF0, cepDelta?: {harmonic, noise}, noiseSamples?, seed?}` -> audio
//! - `synthesize {tokens, stress, voice, speed?, pitchShift?, pitchRange?, seed?, noiseSamples?}` -> audio
//! - `perform {perf, voice, seed?, noiseSamples?}` -> audio (perf.json v1)
//! - `voiceToParams {voice}` -> the `speak` node params `{m2, warp, tilt, envelope, breath, phonation, shift, range,
//!   rate, jitter}`; `paramsToVoice {params}` -> the voice.tvv.json v3 (a voice is only these graph params)
//!
//! Audio results are raw little-endian f32 samples at 24 kHz in the result buffer (length / 4 samples).

use serde_json::{Value, json};
use tinyvoca::{Cepstra, Model, Options, Perf, Post, Voice, VoiceFile};

use crate::{MemPtr, set_result, with_result};

fn fail(message: String) -> i32 {
    set_result(json!({ "error": message }).to_string());
    -1
}

/// # Safety
/// `ptr`/`len` must describe a live host allocation, or be 0/0.
unsafe fn bytes_at<'a>(ptr: MemPtr, len: u32) -> &'a [u8] {
    if ptr == 0 || len == 0 {
        return &[];
    }
    // SAFETY: the caller guarantees the range is a live host allocation.
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
}

/// Loads a tiny-voca GGUF as the shared model.
#[unsafe(no_mangle)]
pub extern "C" fn haa_tinyvoca_load(ptr: MemPtr, len: u32) -> i32 {
    // SAFETY: the host passes a block it allocated with `haa_alloc` and filled.
    let bytes = unsafe { bytes_at(ptr, len) };
    match Model::from_gguf(bytes) {
        Ok(model) => {
            tinyvoca::install(model);
            set_result(String::new());
            0
        }
        Err(e) => fail(e),
    }
}

/// Describes the loaded model.
#[unsafe(no_mangle)]
pub extern "C" fn haa_tinyvoca_info() -> u32 {
    let v = match tinyvoca::shared() {
        None => json!({ "loaded": false }),
        Some(m) => json!({
            "loaded": true,
            "sampleRate": m.config.sample_rate,
            "hop": m.config.hop,
            "speakers": m.config.speakers,
            "heads": [m.head_widths().0, m.head_widths().1],
        }),
    };
    let s = v.to_string();
    let n = s.len() as u32;
    set_result(s);
    n
}

fn floats(v: &Value, key: &str) -> Result<Vec<f32>, String> {
    v.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key}: expected an array of numbers"))?
        .iter()
        .map(|x| x.as_f64().map(|f| f as f32).ok_or_else(|| format!("{key}: expected numbers")))
        .collect()
}

fn ints(v: &Value, key: &str) -> Result<Vec<u32>, String> {
    v.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key}: expected an array of integers"))?
        .iter()
        .map(|x| x.as_u64().filter(|&n| n <= u32::MAX as u64).map(|n| n as u32).ok_or_else(|| format!("{key}: expected integers")))
        .collect()
}

fn number(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn voice(model: &Model, v: &Value) -> Result<Voice, String> {
    match v.get("voice") {
        None => model.base_voice("M2"),
        Some(Value::String(name)) => model.base_voice(name),
        Some(obj) => {
            let file = VoiceFile::parse(&obj.to_string())?;
            model.blend_voice(file.m2_weight(), file.post)
        }
    }
}

fn noise_samples(v: &Value) -> Result<Option<Vec<f32>>, String> {
    if v.get("noiseSamples").is_some() { floats(v, "noiseSamples").map(Some) } else { Ok(None) }
}

enum Out {
    Json(Value),
    Audio(Vec<f32>),
}

fn run(model: &Model, v: &Value) -> Result<Out, String> {
    let op = v.get("op").and_then(Value::as_str).ok_or("op: missing")?;
    let seed = number(v, "seed", 0.0) as u32;
    Ok(match op {
        "encode" => {
            let e = model.encode(&ints(v, "tokens")?, &ints(v, "stress")?, &voice(model, v)?)?;
            Out::Json(json!({ "hidden": e.hidden, "durationRaw": e.duration_raw, "durations": e.durations }))
        }
        "durations" => {
            let tokens = ints(v, "tokens")?;
            let raw = floats(v, "durationRaw")?;
            if raw.len() != 2 * tokens.len() {
                return Err("durationRaw must have 2 values per token".into());
            }
            Out::Json(json!({ "durations": model.durations(&tokens, &raw, number(v, "speed", 1.0) as f32) }))
        }
        "expand" => {
            let tokens = ints(v, "tokens")?;
            let frames = model.expand(&floats(v, "hidden")?, &ints(v, "durations")?, &voice(model, v)?)?;
            let p = model.prosody(&frames, &tokens);
            Out::Json(json!({ "logF0": p.log_f0, "voiced": p.voiced, "energy": p.energy, "prosodyHidden": p.hidden, "owner": frames.owner }))
        }
        "decode" => {
            let c = model.decode(&floats(v, "prosodyHidden")?, &floats(v, "logF0")?, &floats(v, "voiced")?, &floats(v, "energy")?)?;
            Out::Json(json!({ "harmonic": c.harmonic, "noise": c.noise }))
        }
        "envelope" => {
            let mut c = Cepstra { harmonic: floats(v, "harmonic")?, noise: floats(v, "noise")? };
            let log_f0 = floats(v, "logF0")?;
            let (hw, nw) = model.head_widths();
            if c.harmonic.len() != log_f0.len() * hw || c.noise.len() != log_f0.len() * nw {
                return Err(format!("envelope: cepstra must be [T x {hw}] and [T x {nw}]"));
            }
            model.edit_envelope(&mut c, &log_f0, &voice(model, v)?.post);
            Out::Json(json!({ "harmonic": c.harmonic, "noise": c.noise }))
        }
        "voiceToParams" => {
            let file = VoiceFile::parse(&v.get("voice").ok_or("voice: missing")?.to_string())?;
            let p = &file.post;
            let envelope: Vec<String> = p.env_offset_db.iter().map(|x| x.to_string()).collect();
            Out::Json(json!({ "m2": file.m2_weight(), "warp": p.warp, "tilt": p.tilt_db_oct, "envelope": envelope.join(" "),
                "breath": p.breath_db, "phonation": p.phonation, "shift": p.f0_shift_st, "range": p.f0_range_x,
                "rate": p.rate_x, "jitter": p.jitter_x }))
        }
        "paramsToVoice" => {
            let p = v.get("params").ok_or("params: missing")?;
            let n = |k: &str, d: f64| number(p, k, d) as f32;
            let envelope = haa_core::nodes::speak::envelope_bands(p.get("envelope").and_then(Value::as_str).unwrap_or(""))?;
            let post = Post { warp: n("warp", 1.0), tilt_db_oct: n("tilt", 0.0), env_offset_db: envelope, breath_db: n("breath", 0.0),
                phonation: n("phonation", 0.0), f0_shift_st: n("shift", 0.0), f0_range_x: n("range", 1.0), rate_x: n("rate", 1.0),
                jitter_x: n("jitter", 1.0) };
            post.check()?;
            Out::Json(serde_json::from_str(&VoiceFile::to_json(n("m2", 1.0), &post)).map_err(|e| e.to_string())?)
        }
        "vocode" => {
            let cep = Cepstra { harmonic: floats(v, "harmonic")?, noise: floats(v, "noise")? };
            let delta = match v.get("cepDelta") {
                Some(d) => Some(Cepstra { harmonic: floats(d, "harmonic")?, noise: floats(d, "noise")? }),
                None => None,
            };
            let noise = noise_samples(v)?;
            Out::Audio(model.vocode(&cep, &floats(v, "logF0")?, delta.as_ref(), noise.as_deref(), seed)?)
        }
        "synthesize" => {
            let o = Options {
                speed: number(v, "speed", 1.0) as f32,
                pitch_shift: number(v, "pitchShift", 0.0) as f32,
                pitch_range: number(v, "pitchRange", 1.0) as f32,
                seed,
            };
            let noise = noise_samples(v)?;
            Out::Audio(model.synthesize(&ints(v, "tokens")?, &ints(v, "stress")?, &voice(model, v)?, &o, noise.as_deref())?.audio)
        }
        "perform" => {
            let perf: Perf = serde_json::from_value(v.get("perf").cloned().ok_or("perf: missing")?).map_err(|e| format!("perf.json: {e}"))?;
            let noise = noise_samples(v)?;
            Out::Audio(model.perform(&perf, &voice(model, v)?, noise.as_deref(), seed)?)
        }
        other => return Err(format!("op: unknown {other}")),
    })
}

/// Runs one op of the model; see the module comment.
#[unsafe(no_mangle)]
pub extern "C" fn haa_tinyvoca_run(ptr: MemPtr, len: u32) -> i32 {
    let Some(model) = tinyvoca::shared() else {
        return fail("no tiny-voca model loaded (haa_tinyvoca_load)".into());
    };
    // SAFETY: the host passes a block it allocated with `haa_alloc` and filled.
    let bytes = unsafe { bytes_at(ptr, len) };
    let request: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return fail(format!("request: {e}")),
    };
    match run(&model, &request) {
        Ok(Out::Json(v)) => {
            let s = v.to_string();
            let n = s.len() as i32;
            set_result(s);
            n
        }
        Ok(Out::Audio(a)) => with_result(|buffer| {
            buffer.clear();
            buffer.extend(a.iter().flat_map(|x| x.to_le_bytes()));
            buffer.len() as i32
        }),
        Err(e) => fail(e),
    }
}
