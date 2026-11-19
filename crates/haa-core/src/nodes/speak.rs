//! `speak` source node: tiny-voca English speech (crates/tinyvoca, a port of n4ze3m/tiny-voca, Apache-2.0).
//!
//! The model is the official GGUF the host loaded once with `haa_tinyvoca_load` (wasm) or `tinyvoca::install`
//! (native): nothing is bundled. Load it before the patch; without it the node is silent.
//! `text` is plain English (built with the `dizione` feature; the phonemizer comes from the same one-file GGUF or
//! the tiny-voca `phonemizer/` folder, see dizione::install): it is cut into utterances as tiny-voca's page cuts
//! it (dizione::utterances: sentences, then clauses, at most `max_tokens - 12` tokens each), each utterance is
//! synthesized with seed `seed + its index`, and the clip is limited to a 0.99 peak, as upstream `say.mjs` does.
//! Script markup in `text` goes through [`script`] first (today `[...]` tags are taken out). When `text` is
//! empty, `input` is used: JSON, either a `perf.json` v1 performance (it has `durations`: its durations and curves are used) or
//! `{"tokens": [...], "stress": [...]}` (dizione ids: the model predicts the performance).
//! A voice is only these parameters (voice.tvv.json v3 is their import/export form): `m2` blends the GGUF's M2 and
//! F2 speakers (1 = M2, 0 = F2, a little outside extrapolates); `warp`, `tilt`, `envelope` (32 band offsets in dB,
//! 1/6 octave from 150 Hz, as text), `breath` and `phonation` edit the spectral envelope before the head; `shift`,
//! `range`, `rate` and `jitter` edit the curves (see tinyvoca::edit).
//! `configure` renders the whole utterance (24 kHz, resampled to the patch rate) and may allocate: changing a
//! voice parameter renders again; only `gain` follows modulation per sample. `process_sample` reads the audio at
//! the patch clock, so a loop or a restart replays it, and never allocates.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

pub const TEXT: usize = 0;
pub const INPUT: usize = 1;
pub const M2: usize = 2;
pub const WARP: usize = 3;
pub const TILT: usize = 4;
pub const ENVELOPE: usize = 5;
pub const BREATH: usize = 6;
pub const PHONATION: usize = 7;
pub const SHIFT: usize = 8;
pub const RANGE: usize = 9;
pub const RATE: usize = 10;
pub const JITTER: usize = 11;
pub const GAIN: usize = 12;

const fn number(key: &'static str, min: f64, max: f64, default: f64, step: f64, scale: Scale, unit: &'static str, en: &'static str, fr: &'static str) -> ParamDef {
    ParamDef { key, kind: ParamKind::Number { min, max, default, step, scale }, unit, label_en: en, label_fr: fr, modulatable: false, smooth: false }
}

/// The `speak` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "speak",
    category: Category::Source,
    label_en: "Speak (tiny-voca)",
    label_fr: "Parole (tiny-voca)",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "text",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Text",
            label_fr: "Texte",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "input",
            kind: ParamKind::Text { default: "" },
            unit: "",
            label_en: "Tokens or performance (JSON)",
            label_fr: "Tokens ou performance (JSON)",
            modulatable: false,
            smooth: false,
        },
        number("m2", -0.5, 1.5, 1.0, 0.01, Scale::Lin, "", "Voice blend (F2 0 - M2 1)", "Mélange de voix (F2 0 - M2 1)"),
        number("warp", 0.7, 1.4, 1.0, 0.005, Scale::Log, "×", "Tract warp", "Échelle du conduit"),
        number("tilt", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "dB/oct", "Spectral tilt", "Pente spectrale"),
        ParamDef {
            key: "envelope",
            kind: ParamKind::Text { default: "" },
            unit: "dB",
            label_en: "Envelope offsets (32 bands)",
            label_fr: "Décalages d'enveloppe (32 bandes)",
            modulatable: false,
            smooth: false,
        },
        number("breath", -24.0, 24.0, 0.0, 0.1, Scale::Lin, "dB", "Breath", "Souffle"),
        number("phonation", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "dB", "Phonation (H1-H2)", "Phonation (H1-H2)"),
        number("shift", -12.0, 12.0, 0.0, 0.1, Scale::Lin, "st", "Pitch shift", "Transposition"),
        number("range", 0.0, 3.0, 1.0, 0.01, Scale::Lin, "×", "Pitch range", "Ambitus"),
        number("rate", 0.25, 4.0, 1.0, 0.01, Scale::Log, "×", "Rate", "Débit"),
        number("jitter", 1.0, 5.0, 1.0, 0.01, Scale::Lin, "×", "Jitter", "Instabilité"),
        ParamDef {
            key: "gain",
            kind: ParamKind::Number { min: 0.0, max: 2.0, default: 1.0, step: 0.01, scale: Scale::Lin },
            unit: "×",
            label_en: "Gain",
            label_fr: "Gain",
            modulatable: true,
            smooth: true,
        },
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    let mut node = Speak { audio: Vec::new(), key: String::new(), sample_rate: init.sample_rate };
    node.configure(init);
    Box::new(node)
}

struct Speak {
    /// The utterance at the patch rate.
    audio: Vec<f32>,
    /// What `audio` was rendered from, so a live swap of `gain` alone does not render again.
    key: String,
    sample_rate: f64,
}

/// The 32 band offsets of the `envelope` text (numbers separated by commas or spaces); empty = none.
pub fn envelope_bands(text: &str) -> Result<Vec<f32>, String> {
    let values: Result<Vec<f32>, _> = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(str::parse).collect();
    let values = values.map_err(|_| "envelope: expected numbers".to_string())?;
    if !values.is_empty() && values.len() != tinyvoca::voice::ENV_BANDS {
        return Err(format!("envelope: expected {} values", tinyvoca::voice::ENV_BANDS));
    }
    Ok(values)
}

/// The voice the node's parameters describe: (M2 weight, post edits).
pub fn voice_params(params: &[f64], envelope: &str) -> Result<(f32, tinyvoca::Post), String> {
    let p = |i: usize| params.get(i).copied().unwrap_or(0.0) as f32;
    Ok((
        p(M2),
        tinyvoca::Post {
            warp: p(WARP),
            tilt_db_oct: p(TILT),
            env_offset_db: envelope_bands(envelope)?,
            breath_db: p(BREATH),
            phonation: p(PHONATION),
            f0_shift_st: p(SHIFT),
            f0_range_x: p(RANGE),
            rate_x: p(RATE),
            jitter_x: p(JITTER),
        },
    ))
}

/// Script markup taken out of a `speak` text: what is said, and the tags met on the way.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    /// The text to say, tags removed.
    pub plain: String,
    /// Each `[...]` tag: its byte offset in `plain` and what was inside the brackets.
    pub tags: Vec<(usize, String)>,
}

/// Reads the script markup of a `speak` text. Today it only takes `[...]` tags out (a space is left where a tag
/// separated two words); an unclosed `[` stays as text.
// TODO(agent C, emotion tags): give `tags` (e.g. `[angry,5]`) a meaning per span (voice or curve edits from the
// tag to the next one) in `speak_text`; the studio already passes the raw text here.
pub fn script(text: &str) -> Script {
    let mut out = Script::default();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(len) = rest[open..].find(']') else { break };
        out.plain.push_str(&rest[..open]);
        let after = &rest[open + len + 1..];
        out.tags.push((out.plain.len(), rest[open + 1..open + len].trim().to_string()));
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        if word(out.plain.chars().next_back()) && word(after.chars().next()) {
            out.plain.push(' ');
        }
        rest = after;
    }
    out.plain.push_str(rest);
    out
}

/// Says `text` (plain English, script markup allowed) with the shared model and phonemizer for a voice
/// `(m2, post)`: 24 kHz audio. The wasm `speak` op, the `haa speak` command and the node all run this.
#[cfg(feature = "dizione")]
pub fn speak_text(text: &str, m2: f32, post: tinyvoca::Post, seed: u32) -> Result<Vec<f32>, String> {
    let model = tinyvoca::shared().ok_or("no tiny-voca model loaded (haa_tinyvoca_load)")?;
    let voice = model.blend_voice(m2, post)?;
    let said = script(text);
    let limit = model.config.max_tokens.saturating_sub(12).max(1);
    let utterances = dizione::with_shared(|p| p.utterances(&said.plain, limit))
        .ok_or("no phonemizer loaded (a one-file tiny-voca GGUF, or haa_dizione_*)")?
        .map_err(|e| e.0)?;
    let mut audio = Vec::new();
    for (i, u) in utterances.iter().enumerate() {
        let stress: Vec<u32> = u.stress.iter().map(|&s| s as u32).collect();
        let options = tinyvoca::Options { seed: seed.wrapping_add(i as u32), ..Default::default() };
        audio.extend(model.synthesize(&u.ids, &stress, &voice, &options, None)?.audio);
    }
    // say.mjs `limited`: the whole clip brought under a 0.99 peak
    let peak = audio.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    if peak > 0.99 {
        let k = 0.99 / peak;
        for v in &mut audio {
            *v *= k;
        }
    }
    Ok(audio)
}

/// Without the `dizione` feature there is no text front end.
#[cfg(not(feature = "dizione"))]
pub fn speak_text(_text: &str, _m2: f32, _post: tinyvoca::Post, _seed: u32) -> Result<Vec<f32>, String> {
    Err("this build has no text front end (haa-core feature dizione)".into())
}

/// Renders `text` (or, when it is empty, `input`) at `sample_rate`; see the module comment.
pub fn render_node(text: &str, input: &str, m2: f32, post: tinyvoca::Post, seed: u32, sample_rate: f64) -> Result<Vec<f32>, String> {
    if text.trim().is_empty() {
        return render(input, m2, post, seed, sample_rate);
    }
    let audio = speak_text(text, m2, post, seed)?;
    let from = tinyvoca::shared().map_or(24_000.0, |m| m.config.sample_rate as f64);
    Ok(tinyvoca::resample(&audio, from, sample_rate))
}

/// Renders `input` with the shared model for a voice `(m2, post)`, at `sample_rate`.
pub fn render(input: &str, m2: f32, post: tinyvoca::Post, seed: u32, sample_rate: f64) -> Result<Vec<f32>, String> {
    let model = tinyvoca::shared().ok_or("no tiny-voca model loaded (haa_tinyvoca_load)")?;
    let voice = model.blend_voice(m2, post)?;
    let value: serde_json::Value = serde_json::from_str(input).map_err(|e| format!("speak input: {e}"))?;
    let audio = if value.get("durations").is_some() {
        let perf: tinyvoca::Perf = serde_json::from_value(value).map_err(|e| format!("perf.json: {e}"))?;
        model.perform(&perf, &voice, None, seed)?
    } else {
        let list = |k: &str| -> Result<Vec<u32>, String> {
            value.get(k).and_then(|v| v.as_array()).ok_or(format!("speak input: no {k}"))?.iter()
                .map(|v| v.as_u64().map(|n| n as u32).ok_or(format!("speak input: {k} must be integers")))
                .collect()
        };
        let options = tinyvoca::Options { seed, ..Default::default() };
        model.synthesize(&list("tokens")?, &list("stress")?, &voice, &options, None)?.audio
    };
    Ok(tinyvoca::resample(&audio, model.config.sample_rate as f64, sample_rate))
}

impl Node for Speak {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), params: &[f64]) -> NodeOut {
        let at = (ctx.seconds * self.sample_rate).floor();
        let v = if at >= 0.0 { self.audio.get(at as usize).copied().unwrap_or(0.0) as f64 * params[GAIN] } else { 0.0 };
        let v = if v.is_finite() { v } else { 0.0 };
        NodeOut { l: v, r: v, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        let text = |i: usize| init.texts.get(i).and_then(|t| t.as_deref()).unwrap_or("").to_string();
        let (said, input, envelope) = (text(TEXT), text(INPUT), text(ENVELOPE));
        let loaded = tinyvoca::shared().is_some();
        let voice: Vec<f64> = init.params.iter().take(GAIN).copied().collect();
        let key = format!("{loaded}\u{1}{said}\u{1}{input}\u{1}{envelope}\u{1}{voice:?}\u{1}{}\u{1}{}", init.seed, init.sample_rate);
        if key == self.key {
            return;
        }
        self.key = key;
        self.sample_rate = init.sample_rate;
        self.audio.clear();
        if said.trim().is_empty() && input.trim().is_empty() {
            return;
        }
        // an input or voice the model refuses leaves the node silent; the wasm `synthesize`/`perform` ops report
        // the same error as text
        if let Ok((m2, post)) = voice_params(init.params, &envelope)
            && let Ok(a) = render_node(&said, &input, m2, post, init.seed, init.sample_rate)
        {
            self.audio = a;
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_without_input_and_params_map_to_a_voice() {
        let params = [0.0, 0.0, 0.25, 1.1, -2.0, 0.0, 3.0, 1.0, 2.0, 1.2, 0.9, 1.5, 1.0];
        let texts = [Some(String::new()), Some(String::new()), None, None, None, Some("1 ".repeat(32)), None, None, None, None, None, None, None];
        let (m2, post) = voice_params(&params, texts[ENVELOPE].as_deref().unwrap()).unwrap();
        assert_eq!((m2, post.warp, post.rate_x, post.env_offset_db.len()), (0.25, 1.1, 0.9, 32));
        let mut node = (DEF.create)(&NodeInit {
            id: "speak1",
            seed: 0,
            sample_rate: 48_000.0,
            params: &params,
            texts: &texts,
            curve: None,
            inputs: &[],
            duration: 1.0,
        });
        let ctx = SampleCtx {
            sample_rate: 48_000.0,
            dt: 1.0 / 48_000.0,
            t: 0.0,
            seconds: 0.01,
            duration: 1.0,
            mode: crate::patch::GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        };
        let out = node.process_sample(&ctx, (0.0, 0.0), &params);
        assert_eq!((out.l, out.r), (0.0, 0.0));
        let said = script("[calm] Hello[laughs]there, [angry,5]you. [open");
        assert_eq!(said.plain, " Hello there, you. [open");
        assert_eq!(said.tags, [(0, "calm".to_string()), (6, "laughs".to_string()), (14, "angry,5".to_string())]);
    }
}
