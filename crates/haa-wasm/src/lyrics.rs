//! Timed lyrics of the `chimera` node (`lyrics` mode): when each word is said.
//!
//! `haa_lyrics_timing(ptr, len)`: request JSON `{"text", "language": "english"|"ipa"|"french", "female", "child",
//! "rate": [[seconds, rate], ...], "hop", "seconds"}` (the node's rate input over the patch, linear between points;
//! none = 1). The result JSON is `{"lines": [{"start", "end", "words": [{"word", "start", "end"}], "phones": [{"ipa",
//! "start", "end"}]}]}` in patch seconds, the node's own plan (G2P, durations, vowel-stretching rate) integrated in
//! Rust; with `hop` and `seconds`, also `"tau"` (per frame, the playing line's score time, -1 where none plays) and
//! `"vowel"` (1 in a held sound: a vowel, a nasal, a lateral).
//!
//! `haa_lyrics_fit(ptr, len)`: the start and rate input that make each line say its sounds when the audio does.
//! Request `{"text", "language", "female", "child", "lines": [{"phones": [[ipa, start s, lead s?], ...], "end"}],
//! "maxPoints"}` (one entry per text line, its phones as written in the text, each played `lead` s early); result `{"lines": [{"start", "rate": [[seconds,
//! rate], ...]}], "error": mean |planned - wanted| start of the sounds (s) with that rate, "worst"}` (haa-core
//! `lyric_rate_fit`, checked with the node's own clock).

use haa_core::nodes::chimera::{lyric_held, lyric_rate_fit, lyric_speed, lyrics_phone_timing, lyrics_timing, plan_lyrics, LyricLanguage, LyricLine, LYRIC_LEAD};
use serde_json::Value;

use crate::{set_result, MemPtr};

/// The request fields, read leniently (a missing number is 0, a missing rate 1).
struct Request {
    text: String,
    language: LyricLanguage,
    female: f64,
    child: f64,
    rate: Vec<[f64; 2]>,
    hop: f64,
    seconds: f64,
}

fn request(v: &Value) -> Option<Request> {
    let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let rate = v
        .get("rate")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect())
        .unwrap_or_default();
    Some(Request {
        text: v.get("text")?.as_str()?.to_string(),
        language: LyricLanguage::from_name(v.get("language").and_then(Value::as_str).unwrap_or("english")),
        female: num("female"),
        child: num("child"),
        rate,
        hop: num("hop"),
        seconds: num("seconds"),
    })
}

/// The rate polyline at `t`: held before the first point and after the last.
fn rate_at(points: &[[f64; 2]], t: f64) -> f64 {
    match points.iter().position(|p| p[0] > t) {
        None => points.last().map_or(1.0, |p| p[1]),
        Some(0) => points[0][1],
        Some(i) => {
            let (a, b) = (points[i - 1], points[i]);
            let u = if b[0] > a[0] { (t - a[0]) / (b[0] - a[0]) } else { 1.0 };
            a[1] + (b[1] - a[1]) * u
        }
    }
}

/// Per `hop` frame over `seconds`: the playing line's score time (-1 where none plays) and whether it is in a held sound,
/// the node's clock integrated every millisecond (a line plays from its scheduling until its plan ends or the next
/// line is scheduled).
fn track(lines: &[LyricLine], rate: &[[f64; 2]], hop: f64, seconds: f64) -> (Vec<f64>, Vec<u8>) {
    let frames = (seconds / hop).ceil().max(0.0) as usize;
    let (mut tau, mut vowel) = (vec![-1.0; frames], vec![0u8; frames]);
    let dt = 0.001;
    for (k, line) in lines.iter().enumerate() {
        let stop = lines.get(k + 1).map_or(f64::INFINITY, |n| n.start - LYRIC_LEAD);
        let end = line.spans.last().map_or(0.0, |s| s.1) + 0.1;
        let (mut t, mut x) = (line.start - LYRIC_LEAD, 0.0);
        let mut f = (t / hop).ceil().max(0.0) as usize;
        while x < end && t < stop && f < frames {
            if t >= f as f64 * hop {
                tau[f] = (x * 1e5).round() / 1e5;
                vowel[f] = lyric_held(line, x) as u8;
                f += 1;
                continue;
            }
            x += lyric_speed(line, x, rate_at(rate, t)) * dt;
            t += dt;
        }
    }
    (tau, vowel)
}

fn timing_json(req: &Request) -> String {
    let lines = plan_lyrics(&req.text, req.language, req.female, req.child, 120.0);
    let timed = lyrics_timing(&lines, |t| rate_at(&req.rate, t));
    let phones = lyrics_phone_timing(&lines, |t| rate_at(&req.rate, t));
    let r = |v: f64| if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { -1.0 };
    let lines_json = serde_json::Value::Array(
        timed
            .into_iter()
            .zip(phones)
            .map(|((start, end, words), phones)| {
                serde_json::json!({
                    "start": r(start),
                    "end": r(end),
                    "words": words.into_iter().map(|(w, a, b)| serde_json::json!({ "word": w, "start": r(a), "end": r(b) })).collect::<Vec<_>>(),
                    "phones": phones.into_iter().map(|(p, a, b)| serde_json::json!({ "ipa": p, "start": r(a), "end": r(b) })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    );
    if req.hop > 0.0 && req.seconds > 0.0 && req.seconds / req.hop < 2e6 {
        let (tau, vowel) = track(&lines, &req.rate, req.hop, req.seconds);
        serde_json::json!({ "lines": lines_json, "tau": tau, "vowel": vowel }).to_string()
    } else {
        serde_json::json!({ "lines": lines_json }).to_string()
    }
}

/// Writes the timing JSON (or `{"error"}`) into the result and returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn haa_lyrics_timing(ptr: MemPtr, len: u32) -> u32 {
    let bytes: &[u8] = if ptr == 0 || len == 0 {
        &[]
    } else {
        // SAFETY: the host passes a buffer from `haa_alloc` that it filled, `len` bytes long.
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
    };
    let json = match serde_json::from_slice::<Value>(bytes).ok().as_ref().and_then(request) {
        Some(req) => timing_json(&req),
        None => serde_json::json!({ "error": "expected {text, language?, female?, child?, rate?}" }).to_string(),
    };
    let n = json.len() as u32;
    set_result(json);
    n
}

fn fit_json(v: &Value) -> Option<String> {
    let req = request(v)?;
    let mut lines = plan_lyrics(&req.text, req.language, req.female, req.child, 120.0);
    let wanted = v.get("lines")?.as_array()?;
    let max_points = v.get("maxPoints").and_then(Value::as_u64).unwrap_or(120) as usize;
    let mut fits = Vec::new();
    for (line, w) in lines.iter().zip(wanted) {
        let phones: Vec<(String, f64)> = w
            .get("phones")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_f64()? - p.get(2).and_then(Value::as_f64).unwrap_or(0.0))))
                    .collect()
            })
            .unwrap_or_default();
        let end = w.get("end").and_then(Value::as_f64).unwrap_or_else(|| phones.last().map_or(line.start, |p| p.1 + 0.2));
        fits.push(lyric_rate_fit(line, &phones, end, max_points));
    }
    // the check: the lines at their fitted starts, played at the fitted rate by the node's clock
    for (line, f) in lines.iter_mut().zip(&fits) {
        line.start = f.start;
    }
    let mut all: Vec<[f64; 2]> = fits.iter().flat_map(|f| f.rate.iter().copied()).collect();
    all.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let timed = lyrics_phone_timing(&lines, |t| rate_at(&all, t));
    let (mut err, mut n, mut worst) = (0.0, 0usize, 0.0f64);
    for (phones, f) in timed.iter().zip(&fits) {
        for ((_, a, _), want) in phones.iter().zip(&f.sounds) {
            if a.is_finite() && want.is_finite() {
                let e = (a - want).abs();
                err += e;
                worst = worst.max(e);
                n += 1;
            }
        }
    }
    let r = |x: f64| (x * 1e4).round() / 1e4;
    let out: Vec<Value> = fits
        .iter()
        .map(|f| serde_json::json!({ "start": r(f.start), "rate": f.rate.iter().map(|p| [r(p[0]), r(p[1])]).collect::<Vec<_>>() }))
        .collect();
    Some(serde_json::json!({ "lines": out, "error": r(err / n.max(1) as f64), "worst": r(worst) }).to_string())
}

/// Writes the fitted starts and rate JSON (or `{"error"}`) into the result and returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn haa_lyrics_fit(ptr: MemPtr, len: u32) -> u32 {
    let bytes: &[u8] = if ptr == 0 || len == 0 {
        &[]
    } else {
        // SAFETY: the host passes a buffer from `haa_alloc` that it filled, `len` bytes long.
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
    };
    let json = serde_json::from_slice::<Value>(bytes).ok().as_ref().and_then(fit_json).unwrap_or_else(|| {
        serde_json::json!({ "error": "expected {text, language?, female?, child?, lines: [{phones: [[ipa, start]], end}]}" }).to_string()
    });
    let n = json.len() as u32;
    set_result(json);
    n
}
