//! HAA voice export group (the multivoice student, haa_core::nodes::multivoice and haa_core::multivoice): the host
//! runs the four ONNX graphs (voice, encode, prosody, decode; WebGPU in a browser), this module does everything
//! between them. The text front end is the shared dizione phonemizer: load it first (`haa_dizione_gguf` with the
//! tiny-voca one-file GGUF, which carries it).
//!
//! Tensors cross in named slots of little-endian bytes: `haa_mv_put(name_ptr, name_len, data_ptr, data_len) -> u32`
//! (1) stores one, `haa_mv_get(name_ptr, name_len) -> i32` copies one into the result buffer (its length, -1 if
//! absent). Slot names are the graph input/output names.
//!
//! `haa_mv_run(ptr, len) -> i32`: request JSON `{"op", ...}`; the JSON reply's length, or -1 with `{"error"}`.
//! - `interface {io, emotionDim}`: the graphs' interface (io 1 = emotion ids + intensity, one decode noise_scale;
//!   io 2 = emotion mix vectors [.., emotionDim] and a per-frame noise_scale [1,1,T]). Default io 1.
//! - `plan {patch, seed?, rate}` -> `[{key, parts, complete}]`: queues the speech of every `multivoice` node of a
//!   patch, its parts to be kept at `rate` (the player's).
//! - `next {}` -> `null` when nothing is queued; `{key, index, last, kept: true, rate, frames}` for a part already
//!   kept (`clipPart` gives it); `{key, index, last, failed: true, error}`; or `{key, index, last, kept: false, t}`
//!   with the slots `attrs` [1,3], `z` [1,8] (voice.onnx), `quality` [1,3], `input` i64 [1,t], `stress` i64 [1,t],
//!   `emotion` (io 1: i64 [1,t], io 2: f32 [1,t,dim]), `intensity` f32 [1,t] (io 1), `scales` [2], `dp_noise`
//!   [1,2,t] filled for encode.
//! - `expand {}`: reads `durations`, `h`, `m_p`, `logs_p` (encode's outputs) and fills `h_f`, `m_f`, `l_f`
//!   [1,192,T], `e_f` (io 1: i64 [1,T], io 2: f32 [1,T,dim]), `i_f` (io 1) and `n_f` [1,1,T] -> `{frames}`.
//! - `curves {}`: reads `log_f0`, `voiced`, `energy` (prosody's outputs), writes them back with the wired curves in
//!   place of the predicted values -> `{replaced: [names]}`.
//! - `noise {}`: fills `noise_scale`, `z_noise`, `src_noise` for decode -> `{io}`.
//! - `finish {}`: reads `audio` (decode's, 22050 Hz), keeps the part at the source's rate in this instance's clip
//!   store and fills `pcm` with it -> `{key, index, last, rate, frames}`.
//! - `clipPart {key, index}`: fills `pcm` with a kept part -> `{rate, frames}`.
//! - `clear {}`: drops the queue. `tags {}` -> `{events, eventAliases, emotions}` (what the text understands).

use std::collections::HashMap;
use std::sync::Mutex;

use haa_core::multivoice::{self as mv, EmotionInput};
use haa_core::nodes::multivoice::{Interface, MvJobs, Step};
use serde_json::{Value, json};

use crate::{MemPtr, set_result, with_result};

static JOBS: Mutex<Option<MvJobs>> = Mutex::new(None);
static SLOTS: Mutex<Option<HashMap<String, Vec<u8>>>> = Mutex::new(None);

fn with_jobs<R>(f: impl FnOnce(&mut MvJobs) -> R) -> R {
    let mut guard = JOBS.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(Default::default))
}

fn with_slots<R>(f: impl FnOnce(&mut HashMap<String, Vec<u8>>) -> R) -> R {
    let mut guard = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
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

fn put_f32(name: &str, v: &[f32]) {
    with_slots(|s| s.insert(name.into(), v.iter().flat_map(|x| x.to_le_bytes()).collect()));
}

fn put_i64(name: &str, v: &[i64]) {
    with_slots(|s| s.insert(name.into(), v.iter().flat_map(|x| x.to_le_bytes()).collect()));
}

fn get_f32(name: &str) -> Result<Vec<f32>, String> {
    with_slots(|s| {
        let b = s.get(name).ok_or(format!("slot {name} is empty (haa_mv_put)"))?;
        Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    })
}

fn put_emotion(emotion: &EmotionInput, e: &str, i: &str) {
    match emotion {
        EmotionInput::Ids { ids, intensity } => {
            put_i64(e, ids);
            put_f32(i, intensity);
        }
        EmotionInput::Mix { data, .. } => {
            put_f32(e, data);
            with_slots(|s| s.remove(i));
        }
    }
}

fn step_json(step: &Step) -> Value {
    match step {
        Step::Kept { key, index, last, rate, frames } => json!({ "key": key, "index": index, "last": last, "kept": true, "rate": rate, "frames": frames }),
        Step::Failed { key, index, error } => json!({ "key": key, "index": index, "last": true, "failed": true, "error": error }),
        Step::Render { key, index, last, inputs } => {
            let x = inputs;
            put_f32("attrs", &x.attrs);
            put_f32("z", &x.z);
            put_f32("quality", &x.quality);
            put_i64("input", &x.tokens.ids);
            put_i64("stress", &x.tokens.stress);
            put_emotion(&x.emotion, "emotion", "intensity");
            put_f32("scales", &x.scales);
            put_f32("dp_noise", &x.dp_noise);
            json!({ "key": key, "index": index, "last": last, "kept": false, "t": x.tokens.ids.len() })
        }
    }
}

fn run(request: &Value) -> Result<Value, String> {
    let op = request.get("op").and_then(Value::as_str).ok_or("request needs an op")?;
    match op {
        "interface" => {
            let io = request.get("io").and_then(Value::as_u64).unwrap_or(1) as u32;
            let dim = request.get("emotionDim").and_then(Value::as_u64).unwrap_or(if io >= 2 { 17 } else { 16 }) as usize;
            if !(1..=2).contains(&io) || !(1..=mv::EMOTIONS.len()).contains(&dim) {
                return Err(format!("unsupported interface io {io}, emotionDim {dim}"));
            }
            with_jobs(|j| j.interface = Interface { io, emotion_dim: dim });
            Ok(json!({ "io": io, "emotionDim": dim }))
        }
        "plan" => {
            let patch = request.get("patch").and_then(Value::as_str).ok_or("plan needs {patch}")?;
            let seed = request.get("seed").and_then(Value::as_u64).unwrap_or(0) as u32;
            let rate = request.get("rate").and_then(Value::as_f64).filter(|r| *r > 0.0).ok_or("plan needs {rate}")?;
            let wanted = haa_core::nodes::multivoice::plan_patch(patch, seed)?;
            with_jobs(|j| j.add(&wanted, rate));
            Ok(Value::Array(
                wanted
                    .iter()
                    .map(|w| {
                        let kept = haa_core::nodes::speak::clip(&w.key).filter(|c| c.rate == rate);
                        json!({ "key": w.key, "parts": kept.as_ref().map_or(0, |c| c.parts.len()), "complete": kept.is_some_and(|c| c.complete) })
                    })
                    .collect(),
            ))
        }
        "next" => Ok(with_jobs(|j| j.step()).map_or(Value::Null, |s| step_json(&s))),
        "expand" => {
            let (d, h, m, l) = (get_f32("durations")?, get_f32("h")?, get_f32("m_p")?, get_f32("logs_p")?);
            with_jobs(|j| -> Result<Value, String> {
                let x = j.expand(&d, &h, &m, &l)?;
                put_f32("h_f", &x.h);
                put_f32("m_f", &x.m);
                put_f32("l_f", &x.logs);
                put_emotion(&x.emotion, "e_f", "i_f");
                put_f32("n_f", &x.noise);
                Ok(json!({ "frames": x.frames }))
            })
        }
        "curves" => {
            let (mut f0, mut vo, mut en) = (get_f32("log_f0")?, get_f32("voiced")?, get_f32("energy")?);
            let replaced = with_jobs(|j| j.curves(&mut f0, &mut vo, &mut en))?;
            put_f32("log_f0", &f0);
            put_f32("voiced", &vo);
            put_f32("energy", &en);
            Ok(json!({ "replaced": replaced }))
        }
        "noise" => {
            let (scale, z, src) = with_jobs(|j| j.decode_noise())?;
            put_f32("noise_scale", &scale);
            put_f32("z_noise", &z);
            put_f32("src_noise", &src);
            Ok(json!({ "io": with_jobs(|j| j.interface.io) }))
        }
        "finish" => {
            let audio = get_f32("audio")?;
            let (step, pcm) = with_jobs(|j| j.finish(&audio))?;
            put_f32("pcm", &pcm);
            Ok(step_json(&step))
        }
        "clipPart" => {
            let key = request.get("key").and_then(Value::as_str).ok_or("clipPart needs {key}")?;
            let index = request.get("index").and_then(Value::as_u64).ok_or("clipPart needs {index}")? as usize;
            let c = haa_core::nodes::speak::clip(key).ok_or("no clip under this key")?;
            let part = c.parts.get(index).ok_or("no such part")?;
            put_f32("pcm", part);
            Ok(json!({ "rate": c.rate, "frames": part.len() }))
        }
        "clear" => {
            with_jobs(MvJobs::clear);
            Ok(json!({}))
        }
        "tags" => Ok(json!({
            "events": mv::EVENT_TAGS,
            "eventAliases": mv::EVENT_ALIAS.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "emotions": mv::EMOTIONS,
        })),
        other => Err(format!("unknown op {other}")),
    }
}

/// Runs one op; see the module comment.
#[unsafe(no_mangle)]
pub extern "C" fn haa_mv_run(ptr: MemPtr, len: u32) -> i32 {
    // SAFETY: the host passes a block it allocated with `haa_alloc` and filled.
    let bytes = unsafe { bytes_at(ptr, len) };
    let out = serde_json::from_slice::<Value>(bytes).map_err(|e| format!("request: {e}")).and_then(|r| run(&r));
    match out {
        Ok(v) => {
            let s = v.to_string();
            let n = s.len() as i32;
            set_result(s);
            n
        }
        Err(e) => {
            set_result(json!({ "error": e }).to_string());
            -1
        }
    }
}

/// Stores a named tensor (little-endian bytes); see the module comment.
#[unsafe(no_mangle)]
pub extern "C" fn haa_mv_put(name_ptr: MemPtr, name_len: u32, data_ptr: MemPtr, data_len: u32) -> u32 {
    // SAFETY: the host passes blocks it allocated with `haa_alloc` and filled.
    let (name, data) = unsafe { (bytes_at(name_ptr, name_len), bytes_at(data_ptr, data_len)) };
    let Ok(name) = std::str::from_utf8(name) else { return 0 };
    with_slots(|s| s.insert(name.to_string(), data.to_vec()));
    1
}

/// Copies a named tensor into the result buffer; see the module comment.
#[unsafe(no_mangle)]
pub extern "C" fn haa_mv_get(name_ptr: MemPtr, name_len: u32) -> i32 {
    // SAFETY: the host passes a block it allocated with `haa_alloc` and filled.
    let name = unsafe { bytes_at(name_ptr, name_len) };
    let Ok(name) = std::str::from_utf8(name) else { return -1 };
    with_slots(|s| match s.get(name) {
        Some(b) => with_result(|buffer| {
            buffer.clear();
            buffer.extend_from_slice(b);
            buffer.len() as i32
        }),
        None => -1,
    })
}
