//! tiny-voca speech export group (crates/tinyvoca): load the GGUF, then run the model whole or stage by stage.
//!
//! `haa_tinyvoca_load(ptr, len) -> i32`: the bytes of an official tiny-voca GGUF (the host fetches it; nothing is
//! bundled). 0 on success, after which the `speak` node and `haa_tinyvoca_run` use it; -1 with
//! `{"error"}` in the result. The one-file `gguf/tiny-voca-js.gguf` also carries the phonemizer (`g2p/*` tensors
//! and the dizione tables): it is installed too, so one download gives text to speech. Result on success:
//! `{"phonemizer": true}`, or `{"phonemizer": false, "phonemizerError"}` for a GGUF without it (an earlier
//! phonemizer, e.g. from `haa_dizione_*`, is kept).
//!
//! `haa_tinyvoca_info() -> u32`: result `{"loaded", "sampleRate", "hop", "speakers", "heads":[98,36], "maxTokens",
//! "phonemizer"}`.
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
//! - `speak {text, voice? | params?, seed?}` -> audio: plain text (script markup allowed) through the phonemizer
//!   and the model, cut per sentence (haa_core::nodes::speak::speak_text, what the `speak` node's `text` says);
//!   `params` are the node params as `paramsToVoice` takes them
//! - `utterances {text}` -> `[{ids, stress, symbols}]`: how `speak` cuts and phonemizes the text
//! - `speechPlan {patch, seed?, rate?}` -> `[{key, parts, complete}]`: queues the speech of every `speak` node of a
//!   patch for voice seed `seed` (what a player without the model needs; `parts` already kept), its parts resampled
//!   to `rate` (the player's, so it only copies them; absent = the model's 24 kHz). Then `speechNext {}`
//!   renders one utterance (the queued source with the least audio first; kept parts are handed out again) and
//!   answers `{key, index, last, rate, frames, error?}`, or `null` when nothing is queued; `clipPart {key, index}`
//!   -> that part's audio. Plan and parts: haa_core::nodes::speak (`plan_patch`, `SpeechJobs`).
//!
//! `haa_speak_part_put(key_ptr, key_len, index, last, pcm_ptr, pcm_len, rate) -> u32`: keeps part `index` of a
//! clip (little-endian f32 bytes) in this instance, model or not; `last` (0/1) completes it. A `speak` node with
//! that key plays the parts it has once it configures again (load the patch again, keep_state = 1). Returns 1, or 0
//! for a bad key or a part out of order. `haa_speak_clip_put(key_ptr, key_len, pcm_ptr, pcm_len, rate)` keeps a
//! whole clip (one complete part).
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
            set_result(install_phonemizer(bytes).to_string());
            0
        }
        Err(e) => fail(e),
    }
}

#[cfg(feature = "dizione")]
fn install_phonemizer(bytes: &[u8]) -> Value {
    match ::dizione::Phonemizer::from_gguf(bytes) {
        Ok(p) => {
            ::dizione::install(p);
            json!({ "phonemizer": true })
        }
        Err(e) => json!({ "phonemizer": false, "phonemizerError": e }),
    }
}

#[cfg(not(feature = "dizione"))]
fn install_phonemizer(_bytes: &[u8]) -> Value {
    json!({ "phonemizer": false, "phonemizerError": "built without the dizione feature" })
}

fn phonemizer_ready() -> bool {
    #[cfg(feature = "dizione")]
    {
        ::dizione::installed()
    }
    #[cfg(not(feature = "dizione"))]
    {
        false
    }
}

/// Describes the loaded model.
#[unsafe(no_mangle)]
pub extern "C" fn haa_tinyvoca_info() -> u32 {
    let v = match tinyvoca::shared() {
        None => json!({ "loaded": false, "phonemizer": phonemizer_ready() }),
        Some(m) => json!({
            "loaded": true,
            "sampleRate": m.config.sample_rate,
            "hop": m.config.hop,
            "speakers": m.config.speakers,
            "heads": [m.head_widths().0, m.head_widths().1],
            "maxTokens": m.config.max_tokens,
            "phonemizer": phonemizer_ready(),
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

/// The `speak` node params (`{m2, warp, tilt, envelope, breath, phonation, shift, range, rate, jitter}`, each
/// optional) as a voice.
fn params_voice(p: &Value) -> Result<(f32, Post), String> {
    let n = |k: &str, d: f64| number(p, k, d) as f32;
    let envelope = haa_core::nodes::speak::envelope_bands(p.get("envelope").and_then(Value::as_str).unwrap_or(""))?;
    let post = Post { warp: n("warp", 1.0), tilt_db_oct: n("tilt", 0.0), env_offset_db: envelope, breath_db: n("breath", 0.0),
        phonation: n("phonation", 0.0), f0_shift_st: n("shift", 0.0), f0_range_x: n("range", 1.0), rate_x: n("rate", 1.0),
        jitter_x: n("jitter", 1.0) };
    post.check()?;
    Ok((n("m2", 1.0), post))
}

#[cfg(feature = "dizione")]
fn utterances(model: &Model, text: &str) -> Result<Value, String> {
    let plain = haa_core::nodes::speak::script(text).plain;
    let limit = model.config.max_tokens.saturating_sub(12).max(1);
    let list = ::dizione::with_shared(|p| p.utterances(&plain, limit)).ok_or("no phonemizer loaded")?.map_err(|e| e.0)?;
    Ok(Value::Array(list.iter().map(|u| json!({ "ids": u.ids, "stress": u.stress, "symbols": u.symbols })).collect()))
}

#[cfg(not(feature = "dizione"))]
fn utterances(_model: &Model, _text: &str) -> Result<Value, String> {
    Err("built without the dizione feature".into())
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
            let (m2, post) = params_voice(v.get("params").ok_or("params: missing")?)?;
            Out::Json(serde_json::from_str(&VoiceFile::to_json(m2, &post)).map_err(|e| e.to_string())?)
        }
        "speak" => {
            let text = v.get("text").and_then(Value::as_str).ok_or("text: missing")?;
            let (m2, post) = match v.get("params") {
                Some(p) => params_voice(p)?,
                None => {
                    let resolved = voice(model, v)?;
                    (resolved.m2, resolved.post)
                }
            };
            Out::Audio(haa_core::nodes::speak::speak_text(text, m2, post, seed)?)
        }
        "speechPlan" => {
            let patch = match v.get("patch") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => return Err("patch: missing".into()),
            };
            let wanted = haa_core::nodes::speak::plan_patch(&patch, seed)?;
            with_jobs(|jobs| jobs.add(&wanted, number(v, "rate", 0.0)));
            Out::Json(Value::Array(
                wanted
                    .iter()
                    .map(|w| {
                        let kept = haa_core::nodes::speak::clip(&w.key);
                        json!({ "key": w.key, "parts": kept.as_ref().map_or(0, |c| c.parts.len()), "complete": kept.is_some_and(|c| c.complete) })
                    })
                    .collect(),
            ))
        }
        "speechNext" => Out::Json(match with_jobs(|jobs| jobs.step()) {
            Some(s) => json!({ "key": s.key, "index": s.index, "last": s.last, "rate": s.rate, "frames": s.frames, "error": s.error }),
            None => Value::Null,
        }),
        "clipPart" => {
            let key = v.get("key").and_then(Value::as_str).ok_or("key: missing")?;
            let index = number(v, "index", 0.0) as usize;
            let kept = haa_core::nodes::speak::clip(key).ok_or("no clip under this key")?;
            Out::Audio(kept.parts.get(index).ok_or("no such part")?.to_vec())
        }
        "utterances" => Out::Json(utterances(model, v.get("text").and_then(Value::as_str).ok_or("text: missing")?)?),
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

static JOBS: std::sync::Mutex<Option<haa_core::nodes::speak::SpeechJobs>> = std::sync::Mutex::new(None);

fn with_jobs<R>(f: impl FnOnce(&mut haa_core::nodes::speak::SpeechJobs) -> R) -> R {
    let mut guard = JOBS.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(Default::default))
}

fn f32s(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// Keeps a whole rendered clip for the `speak` nodes of this instance; see the module comment.
#[unsafe(no_mangle)]
pub extern "C" fn haa_speak_clip_put(key_ptr: MemPtr, key_len: u32, pcm_ptr: MemPtr, pcm_len: u32, rate: f64) -> u32 {
    // SAFETY: the host passes blocks it allocated with `haa_alloc` and filled.
    let (key, pcm) = unsafe { (bytes_at(key_ptr, key_len), bytes_at(pcm_ptr, pcm_len)) };
    let Ok(key) = std::str::from_utf8(key) else { return 0 };
    if !(rate > 0.0) {
        return 0;
    }
    haa_core::nodes::speak::put_clip(key, haa_core::nodes::speak::Clip::whole(rate, f32s(pcm)));
    1
}

/// Keeps one part of a clip for the `speak` nodes of this instance; see the module comment.
#[unsafe(no_mangle)]
pub extern "C" fn haa_speak_part_put(key_ptr: MemPtr, key_len: u32, index: u32, last: u32, pcm_ptr: MemPtr, pcm_len: u32, rate: f64) -> u32 {
    // SAFETY: the host passes blocks it allocated with `haa_alloc` and filled.
    let (key, pcm) = unsafe { (bytes_at(key_ptr, key_len), bytes_at(pcm_ptr, pcm_len)) };
    let Ok(key) = std::str::from_utf8(key) else { return 0 };
    if !(rate > 0.0) {
        return 0;
    }
    u32::from(haa_core::nodes::speak::put_part(key, index as usize, last != 0, rate, f32s(pcm)))
}

/// Runs one op of the model; see the module comment./// Runs one op of the model; see the module comment.
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
