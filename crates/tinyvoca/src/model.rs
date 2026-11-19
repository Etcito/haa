//! The tiny-voca network, staged. Port of `TinyVoca` in tiny-voca `src/tiny_voca/model.py` (`encode` line 268,
//! `expand` 277, `decode` 310, `synthesize`) and of its plain-JS twin `js/tiny-voca.js`.
//!
//! ```text
//! encode(tokens, stress, voice)                 -> hidden [N x C], durations [N]
//! expand(hidden, durations, voice)              -> frames [T x C]
//! prosody(frames, tokens)                       -> log_f0 [T], voiced [T], energy [T], prosody_hidden [T x C]
//! decode(prosody_hidden, log_f0, voiced, energy) -> cepstra (harmonic [T x 98], noise [T x 36])
//! edit_envelope(cepstra, osc_log_f0, post)      -> cepstra (the voice's warp, tilt, env offset, breath, phonation)
//! vocode(cepstra, log_f0, cep_delta?, noise)    -> audio [T * 256] at 24 kHz
//! ```
//! Every stage takes and returns plain arrays, so durations, curves and cepstra can be edited between stages.
//! The voice (a blend of the M2/F2 rows) enters at `encode` and `expand`; its `post` curve edits are in
//! [`crate::edit`]. [`Model::synthesize`] and [`Model::perform`] chain everything.

use crate::gguf::{self, Gguf};
use crate::head::{self, HeadConfig};
use crate::kernels::{self, Conv, Linear, Norm};
use crate::edit::{self, Envelope};
use crate::voice::{Post, Voice, VoiceFile};
use serde::Deserialize;

const VOWELS: [&str; 33] = [
    "ə", "ɐ", "ɚ", "ᵻ", "ɪ", "i", "iː", "ɛ", "æ", "ʌ", "ɑː", "ɔ", "ɔː", "ʊ", "uː", "ɜː", "eɪ", "aɪ", "ɔɪ", "oʊ", "aʊ",
    "iə", "oː", "ɑːɹ", "ɔːɹ", "oːɹ", "ɪɹ", "ɛɹ", "ʊɹ", "aɪɚ", "aɪə", "əl", "n̩",
];
const PAUSES: [&str; 16] =
    ["<pad>", "<bos>", "<eos>", "<sp>", ",", ".", "?", "!", ";", ":", "\"", "'", "(", ")", "—", "…"];

/// `config.json` `model` (the fields inference reads).
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub sample_rate: usize,
    pub hop: usize,
    pub f0_min: f32,
    pub f0_max: f32,
    pub speakers: Vec<String>,
    pub vocab_size: usize,
    pub channels: usize,
    pub heads: usize,
    pub encoder_layers: usize,
    pub prosody_layers: usize,
    pub acoustic_layers: usize,
    pub synth_fft: usize,
    pub synth_hop: usize,
    pub noise_fft: usize,
    pub noise_hop: usize,
    pub cep_causal: usize,
    pub cep_anti: usize,
    pub noise_cep: usize,
    pub harmonic_limit: f64,
    pub max_tokens: usize,
    pub max_frames: usize,
    pub pitch_reference_hz: f32,
}

#[derive(Deserialize)]
struct Settings {
    model: Config,
    vocab: std::collections::HashMap<String, usize>,
}

struct Block {
    norm1: Norm,
    qkv: Linear,
    out: Linear,
    norm2: Norm,
    depthwise_w: Vec<f32>,
    depthwise_b: Vec<f32>,
    taps: usize,
    expand: Linear,
    project: Linear,
}

struct Stack {
    blocks: Vec<Block>,
    norm: Norm,
}

struct Predictor {
    conv1: Conv,
    norm1: Norm,
    conv2: Conv,
    norm2: Norm,
    out: Linear,
}

/// The loaded model. Build it once from the GGUF bytes; every stage borrows it immutably.
pub struct Model {
    pub config: Config,
    /// Symbol of each token id (config.json `vocab`).
    pub symbols: Vec<String>,
    is_vowel: Vec<bool>,
    is_phone: Vec<bool>,
    tokens: Vec<f32>,
    stress: Vec<f32>,
    voices: Vec<f32>,
    frame_voices: Vec<f32>,
    encoder: Stack,
    duration: Predictor,
    frame_position: Linear,
    prosody_stack: Stack,
    prosody_head: Linear,
    prosody_projection: Linear,
    acoustic_stack: Stack,
    harmonic: Linear,
    noise: Linear,
    envelope: Envelope,
}

/// Output of [`Model::encode`].
#[derive(Clone, Debug)]
pub struct Encoded {
    /// Encoder states `[N x C]`.
    pub hidden: Vec<f32>,
    /// The duration predictor's raw outputs `[N x 2]`: log(1 + frames) and the pause-presence logit.
    pub duration_raw: Vec<f32>,
    /// Frames per token at speed 1 (0 for an absent pause).
    pub durations: Vec<u32>,
}

/// Output of [`Model::expand`].
#[derive(Clone, Debug)]
pub struct Frames {
    /// Frame states `[T x C]`.
    pub x: Vec<f32>,
    /// The token each frame belongs to.
    pub owner: Vec<u32>,
}

/// Output of [`Model::prosody`] (frame transformer A).
#[derive(Clone, Debug)]
pub struct Prosody {
    /// ln(f0 / 200) per frame.
    pub log_f0: Vec<f32>,
    /// Voicing logit per frame, vowels forced to at least 2 (> 0 means voiced; 0/1 works as well).
    pub voiced: Vec<f32>,
    /// Normalized log RMS energy per frame.
    pub energy: Vec<f32>,
    /// Frame transformer A's output `[T x C]`, the decoder's input.
    pub hidden: Vec<f32>,
}

/// Per-frame head outputs, raw (before the lifter). `harmonic [T x (cep_causal + cep_anti + hop/synth_hop)]`
/// = 80 causal, 16 anticausal, 2 sub-frame gains; `noise [T x (noise_cep + hop/noise_hop)]` = 32 cepstra, 4 gains.
#[derive(Clone, Debug, PartialEq)]
pub struct Cepstra {
    pub harmonic: Vec<f32>,
    pub noise: Vec<f32>,
}

/// Options of [`Model::synthesize`], with model.py's meaning.
#[derive(Clone, Debug)]
pub struct Options {
    pub speed: f32,
    /// Added to ln(f0/200) for the oscillator only.
    pub pitch_shift: f32,
    /// Pitch movement around the voiced mean, before the decoder.
    pub pitch_range: f32,
    pub seed: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options { speed: 1.0, pitch_shift: 0.0, pitch_range: 1.0, seed: 0 }
    }
}

/// Everything [`Model::synthesize`] produced.
#[derive(Clone, Debug)]
pub struct Synthesis {
    pub audio: Vec<f32>,
    pub durations: Vec<u32>,
    pub prosody: Prosody,
    pub cepstra: Cepstra,
}

fn round_even(x: f32) -> f32 {
    let down = x.floor();
    let rest = x - down;
    if rest > 0.5 || (rest == 0.5 && down % 2.0 != 0.0) { down + 1.0 } else { down }
}

impl Model {
    /// Reads the model from GGUF bytes (any of the official files: orig f32, f16, q8_0, q4_0 or the JS pack).
    pub fn from_gguf(bytes: &[u8]) -> Result<Model, String> {
        let file = gguf::read(bytes)?;
        Self::from_parsed(&file)
    }

    pub fn from_parsed(file: &Gguf) -> Result<Model, String> {
        let json = file.files.get("config.json").ok_or("GGUF: no embedded config.json")?;
        let settings: Settings = serde_json::from_slice(json).map_err(|e| format!("config.json: {e}"))?;
        let c = settings.model;
        let mut symbols = vec![String::new(); c.vocab_size];
        for (symbol, &id) in &settings.vocab {
            if id < symbols.len() {
                symbols[id] = symbol.clone();
            }
        }
        let is_vowel = symbols.iter().map(|s| VOWELS.contains(&s.as_str())).collect();
        let is_phone = symbols.iter().map(|s| !PAUSES.contains(&s.as_str())).collect();

        let tensor = |name: &str| -> Result<&gguf::Tensor, String> {
            file.tensors.get(&format!("model/{name}")).ok_or_else(|| format!("GGUF: tensor model/{name} is missing"))
        };
        let pair = |name: &str| -> Result<(&gguf::Tensor, &gguf::Tensor), String> {
            Ok((tensor(&format!("{name}.weight"))?, tensor(&format!("{name}.bias"))?))
        };
        let linear = |name: &str| -> Result<Linear, String> {
            let (w, b) = pair(name)?;
            if w.shape.len() != 2 {
                return Err(format!("{name}: expected a matrix"));
            }
            Ok(Linear::new(&w.data, &b.data, w.shape[0], w.shape[1]))
        };
        let norm = |name: &str| -> Result<Norm, String> {
            let (w, b) = pair(name)?;
            Ok(Norm { weight: w.data.clone(), bias: b.data.clone() })
        };
        let conv = |name: &str| -> Result<Conv, String> {
            let (w, b) = pair(name)?;
            Ok(Conv::new(&w.data, &b.data, w.shape[0], w.shape[1], w.shape[2]))
        };
        let stack = |name: &str, layers: usize| -> Result<Stack, String> {
            let mut blocks = Vec::with_capacity(layers);
            for i in 0..layers {
                let at = format!("{name}.blocks.{i}.");
                let (dw, db) = pair(&format!("{at}depthwise"))?;
                blocks.push(Block {
                    norm1: norm(&format!("{at}norm1"))?,
                    qkv: linear(&format!("{at}attention.qkv"))?,
                    out: linear(&format!("{at}attention.out"))?,
                    norm2: norm(&format!("{at}norm2"))?,
                    depthwise_w: dw.data.clone(),
                    depthwise_b: db.data.clone(),
                    taps: *dw.shape.last().unwrap_or(&1),
                    expand: linear(&format!("{at}expand"))?,
                    project: linear(&format!("{at}project"))?,
                });
            }
            Ok(Stack { blocks, norm: norm(&format!("{name}.norm"))? })
        };
        let model = Model {
            tokens: tensor("tokens.weight")?.data.clone(),
            stress: tensor("stress.weight")?.data.clone(),
            voices: tensor("voices.weight")?.data.clone(),
            frame_voices: tensor("frame_voices.weight")?.data.clone(),
            envelope: Envelope::new(&c),
            encoder: stack("encoder", c.encoder_layers)?,
            duration: Predictor {
                conv1: conv("duration.conv1")?,
                norm1: norm("duration.norm1")?,
                conv2: conv("duration.conv2")?,
                norm2: norm("duration.norm2")?,
                out: linear("duration.out")?,
            },
            frame_position: linear("frame_position")?,
            prosody_stack: stack("prosody_stack", c.prosody_layers)?,
            prosody_head: linear("prosody_head")?,
            prosody_projection: linear("prosody_projection")?,
            acoustic_stack: stack("acoustic_stack", c.acoustic_layers)?,
            harmonic: linear("vocoder.harmonic")?,
            noise: linear("vocoder.noise")?,
            config: c,
            symbols,
            is_vowel,
            is_phone,
        };
        let c = &model.config;
        if model.harmonic.outputs != c.cep_causal + c.cep_anti + c.hop / c.synth_hop
            || model.noise.outputs != c.noise_cep + c.hop / c.noise_hop
            || model.tokens.len() != c.vocab_size * c.channels
            || model.voices.len() < c.speakers.len() * c.channels
            || model.frame_voices.len() < c.speakers.len() * c.channels
        {
            return Err("GGUF: tensor sizes do not match config.json".into());
        }
        Ok(model)
    }

    /// Harmonic head width (98) and noise head width (36).
    pub fn head_widths(&self) -> (usize, usize) {
        (self.harmonic.outputs, self.noise.outputs)
    }

    fn speaker_row(&self, name: &str) -> Result<usize, String> {
        self.config.speakers.iter().position(|s| s == name).ok_or_else(|| format!("this GGUF has no speaker {name}"))
    }

    /// The blend `w M2 + (1 - w) F2` of both speaker tables (`w` may go a little outside [0, 1]), with `post`.
    pub fn blend_voice(&self, m2: f32, post: Post) -> Result<Voice, String> {
        post.check()?;
        if !m2.is_finite() {
            return Err("voice: the M2 weight is not finite".into());
        }
        let c = self.config.channels;
        let (a, b) = (self.speaker_row("M2")?, self.speaker_row("F2")?);
        let mix = |table: &[f32]| -> Vec<f32> { (0..c).map(|i| m2 * table[a * c + i] + (1.0 - m2) * table[b * c + i]).collect() };
        Ok(Voice { m2, voice: mix(&self.voices), frame_voice: mix(&self.frame_voices), post })
    }

    /// A stock speaker (`"M2"`, `"F2"`) with neutral post edits.
    pub fn base_voice(&self, name: &str) -> Result<Voice, String> {
        let c = self.config.channels;
        let i = self.speaker_row(name)?;
        Ok(Voice {
            m2: if name == "M2" { 1.0 } else if name == "F2" { 0.0 } else { f32::NAN },
            voice: self.voices[i * c..(i + 1) * c].to_vec(),
            frame_voice: self.frame_voices[i * c..(i + 1) * c].to_vec(),
            post: Post::default(),
        })
    }

    /// Resolves a voice: a stock speaker name (`M2`, `"F2"`) or `voice.tvv.json` v3.
    pub fn voice(&self, text: &str) -> Result<Voice, String> {
        let t = text.trim();
        if !t.starts_with('{') {
            return self.base_voice(t.trim_matches('"'));
        }
        let f = VoiceFile::parse(t)?;
        self.blend_voice(f.m2_weight(), f.post)
    }

    fn transformer(&self, mut x: Vec<f32>, count: usize, stack: &Stack) -> Vec<f32> {
        let c = self.config.channels;
        let (cos, sin) = kernels::rotary(count, c / self.config.heads);
        for b in &stack.blocks {
            let qkv = kernels::linear(&kernels::normalize(&x, count, &b.norm1), count, &b.qkv);
            let mixed = kernels::attention(&qkv, count, c, self.config.heads, &cos, &sin);
            kernels::add(&mut x, &kernels::linear(&mixed, count, &b.out));
            let near = kernels::depthwise(&kernels::normalize(&x, count, &b.norm2), count, &b.depthwise_w, &b.depthwise_b, b.taps);
            let mut wide = kernels::linear(&near, count, &b.expand);
            kernels::gelu(&mut wide);
            kernels::add(&mut x, &kernels::linear(&wide, count, &b.project));
        }
        kernels::normalize(&x, count, &stack.norm)
    }

    fn check_tokens(&self, tokens: &[u32], stress: &[u32]) -> Result<(), String> {
        let n = tokens.len();
        if n < 1 || n > self.config.max_tokens || stress.len() != n {
            return Err(format!("expected 1 to {} tokens and as many stress values", self.config.max_tokens));
        }
        if tokens.iter().any(|&t| t as usize >= self.config.vocab_size) || stress.iter().any(|&s| s > 2) {
            return Err("a token id or stress value is out of range".into());
        }
        Ok(())
    }

    /// Text encoder and duration predictor.
    pub fn encode(&self, tokens: &[u32], stress: &[u32], voice: &Voice) -> Result<Encoded, String> {
        self.check_tokens(tokens, stress)?;
        let c = self.config.channels;
        let n = tokens.len();
        let mut x = vec![0.0; n * c];
        for (t, (&id, &s)) in tokens.iter().zip(stress).enumerate() {
            for i in 0..c {
                x[t * c + i] = self.tokens[id as usize * c + i] + self.stress[s as usize * c + i] + voice.voice[i];
            }
        }
        let hidden = self.transformer(x, n, &self.encoder);
        let d = &self.duration;
        let mut y = kernels::convolve(&hidden, n, &d.conv1);
        kernels::gelu(&mut y);
        let y = kernels::normalize(&y, n, &d.norm1);
        let mut y = kernels::convolve(&y, n, &d.conv2);
        kernels::gelu(&mut y);
        let duration_raw = kernels::linear(&kernels::normalize(&y, n, &d.norm2), n, &d.out);
        let durations = self.durations(tokens, &duration_raw, 1.0);
        Ok(Encoded { hidden, duration_raw, durations })
    }

    /// Frames per token from the predictor's raw outputs at `speed` (model.py `predict_durations`).
    pub fn durations(&self, tokens: &[u32], raw: &[f32], speed: f32) -> Vec<u32> {
        tokens
            .iter()
            .enumerate()
            .map(|(n, &id)| {
                let length = ((raw[2 * n].clamp(0.0, 401f64.ln() as f32) as f64).exp_m1()) as f32;
                let present = self.is_phone[id as usize] || raw[2 * n + 1] > 0.0;
                if present { round_even(length / speed).max(1.0) as u32 } else { 0 }
            })
            .collect()
    }

    /// Repeats token states by their durations and adds each frame's position and the frame voice.
    pub fn expand(&self, hidden: &[f32], durations: &[u32], voice: &Voice) -> Result<Frames, String> {
        let c = self.config.channels;
        if hidden.len() != durations.len() * c {
            return Err("expand: hidden must be [tokens x channels]".into());
        }
        let frames: usize = durations.iter().map(|&d| d as usize).sum();
        if frames < 1 || frames > self.config.max_frames {
            return Err(format!("expand: {frames} frames (1 to {}): split the text", self.config.max_frames));
        }
        let mut owner = Vec::with_capacity(frames);
        let mut place = Vec::with_capacity(frames * 3);
        for (n, &d) in durations.iter().enumerate() {
            let owned = d.max(1) as f32;
            for k in 0..d {
                owner.push(n as u32);
                let relative = (k as f32 + 0.5) / owned;
                place.extend_from_slice(&[relative, 1.0 - relative, owned.ln_1p() / 4.0]);
            }
        }
        let mut x = kernels::linear(&place, frames, &self.frame_position);
        for (t, &n) in owner.iter().enumerate() {
            let row = &mut x[t * c..(t + 1) * c];
            let h = &hidden[n as usize * c..(n as usize + 1) * c];
            for i in 0..c {
                row[i] += h[i] + voice.frame_voice[i];
            }
        }
        Ok(Frames { x, owner })
    }

    /// Frame transformer A and the prosody head. `tokens` gives each frame's token for the voiced-vowel rule.
    pub fn prosody(&self, frames: &Frames, tokens: &[u32]) -> Prosody {
        let count = frames.owner.len();
        let hidden = self.transformer(frames.x.clone(), count, &self.prosody_stack);
        let p = kernels::linear(&hidden, count, &self.prosody_head);
        let mut out = Prosody { log_f0: vec![0.0; count], voiced: vec![0.0; count], energy: vec![0.0; count], hidden };
        for t in 0..count {
            out.log_f0[t] = p[3 * t];
            let vowel = tokens.get(frames.owner[t] as usize).is_some_and(|&id| self.is_vowel[id as usize]);
            out.voiced[t] = if vowel { p[3 * t + 1].max(2.0) } else { p[3 * t + 1] };
            out.energy[t] = p[3 * t + 2];
        }
        out
    }

    /// Frame transformer B and the two heads.
    pub fn decode(&self, prosody_hidden: &[f32], log_f0: &[f32], voiced: &[f32], energy: &[f32]) -> Result<Cepstra, String> {
        let c = self.config.channels;
        let count = log_f0.len();
        if count < 1 || voiced.len() != count || energy.len() != count || prosody_hidden.len() != count * c {
            return Err("decode: prosody_hidden [T x C], log_f0, voiced and energy [T] must agree".into());
        }
        let mut condition = Vec::with_capacity(count * 3);
        for t in 0..count {
            condition.extend_from_slice(&[
                log_f0[t].clamp(-2.0, 1.5),
                if voiced[t] > 0.0 { 1.0 } else { 0.0 },
                energy[t].clamp(-4.5, 3.0),
            ]);
        }
        let mut x = kernels::linear(&condition, count, &self.prosody_projection);
        kernels::add(&mut x, prosody_hidden);
        let acoustic = self.transformer(x, count, &self.acoustic_stack);
        let harmonic = kernels::linear(&acoustic, count, &self.harmonic);
        let noise = kernels::linear(&acoustic, count, &self.noise);
        Ok(Cepstra { harmonic, noise })
    }

    /// f0 in Hz for ln(f0/200), clamped to the model's range (model.py `frequency`).
    pub fn frequency(&self, log_f0: f32) -> f32 {
        (self.config.pitch_reference_hz * log_f0.exp()).clamp(self.config.f0_min, self.config.f0_max)
    }

    fn head_config(&self) -> HeadConfig {
        let c = &self.config;
        HeadConfig {
            sample_rate: c.sample_rate,
            hop: c.hop,
            synth_fft: c.synth_fft,
            synth_hop: c.synth_hop,
            noise_fft: c.noise_fft,
            noise_hop: c.noise_hop,
            cep_causal: c.cep_causal,
            cep_anti: c.cep_anti,
            noise_cep: c.noise_cep,
            harmonic_limit: c.harmonic_limit,
        }
    }

    /// The source-filter head. `log_f0` drives the oscillator (it may differ from the decoder's, e.g. with a pitch
    /// shift or a wobble). `cep_delta`, in the same raw units and layout as `cepstra`, is added first; it holds
    /// either `T` rows or one row applied to every frame. `noise` is `T * hop` unit Gaussian samples; `None` uses
    /// [`head::gaussian_noise`] with `seed`.
    pub fn vocode(&self, cepstra: &Cepstra, log_f0: &[f32], cep_delta: Option<&Cepstra>, noise: Option<&[f32]>, seed: u32) -> Result<Vec<f32>, String> {
        let count = log_f0.len();
        let (hw, nw) = self.head_widths();
        if count < 1 || cepstra.harmonic.len() != count * hw || cepstra.noise.len() != count * nw {
            return Err(format!("vocode: cepstra must be [T x {hw}] and [T x {nw}] for T = log_f0 length"));
        }
        let samples = count * self.config.hop;
        let own_noise;
        let noise = match noise {
            Some(n) if n.len() == samples => n,
            Some(_) => return Err(format!("vocode: noise must have {samples} samples")),
            None => {
                own_noise = head::gaussian_noise(samples, seed);
                &own_noise
            }
        };
        let f0: Vec<f32> = log_f0.iter().map(|&p| self.frequency(p)).collect();
        let edited;
        let cep = match cep_delta {
            None => cepstra,
            Some(d) => {
                let apply = |base: &[f32], delta: &[f32], width: usize, name: &str| -> Result<Vec<f32>, String> {
                    if delta.len() != base.len() && delta.len() != width {
                        return Err(format!("vocode: cep_delta {name} must be [T x {width}] or [{width}]"));
                    }
                    Ok(base.iter().enumerate().map(|(i, &v)| v + delta[i % delta.len()]).collect())
                };
                edited = Cepstra { harmonic: apply(&cepstra.harmonic, &d.harmonic, hw, "harmonic")?, noise: apply(&cepstra.noise, &d.noise, nw, "noise")? };
                &edited
            }
        };
        Ok(head::render(&self.head_config(), &cep.harmonic, &cep.noise, &f0, noise))
    }

    /// The voice's envelope edits (`post`: warp, tilt, env_offset_db, breath, phonation) on `cepstra`, in place;
    /// `osc_log_f0` is the oscillator's ln(f0/200) (it places the phonation edit). See [`crate::edit`].
    pub fn edit_envelope(&self, cepstra: &mut Cepstra, osc_log_f0: &[f32], post: &Post) {
        let f0: Vec<f32> = osc_log_f0.iter().map(|&p| self.frequency(p)).collect();
        self.envelope.apply(cepstra, &f0, post);
    }

    /// Free-running synthesis, model.py `synthesize` for one utterance, plus the voice's `post` edits
    /// (`rate_x` multiplies `speed`, `f0_range_x` multiplies `pitch_range`, `f0_shift_st` adds to `pitch_shift`).
    /// With a neutral `post` it is model.py's `synthesize` exactly.
    pub fn synthesize(&self, tokens: &[u32], stress: &[u32], voice: &Voice, o: &Options, noise: Option<&[f32]>) -> Result<Synthesis, String> {
        let post = &voice.post;
        let speed = o.speed * post.rate_x;
        let range = o.pitch_range * post.f0_range_x;
        if !(0.25..=4.0).contains(&speed) || !o.pitch_shift.is_finite() || !(0.0..=3.0).contains(&range) {
            return Err("speed (x rate_x) must be 0.25 to 4, pitch_shift finite and pitch_range (x f0_range_x) 0 to 3".into());
        }
        let enc = self.encode(tokens, stress, voice)?;
        let durations = if speed == 1.0 { enc.durations.clone() } else { self.durations(tokens, &enc.duration_raw, speed) };
        let frames = self.expand(&enc.hidden, &durations, voice)?;
        let mut prosody = self.prosody(&frames, tokens);
        widen_pitch(&mut prosody.log_f0, &prosody.voiced, range);
        let mut cepstra = self.decode(&prosody.hidden, &prosody.log_f0, &prosody.voiced, &prosody.energy)?;
        let pitch = edit::oscillator_pitch(&prosody.log_f0, post, o.pitch_shift, o.seed);
        self.edit_envelope(&mut cepstra, &pitch, post);
        let audio = self.vocode(&cepstra, &pitch, None, noise, o.seed)?;
        Ok(Synthesis { audio, durations, prosody, cepstra })
    }

    /// Renders a `perf.json` v1 performance: the text encoder runs on its tokens, then its durations and curves
    /// replace the predicted ones (frame transformer A still gives the decoder its hidden states). The voice's
    /// `post` edits apply on top (`rate_x` retimes the performance, `f0_range_x` widens its pitch).
    /// `noise`, if given, must match the retimed length.
    pub fn perform(&self, perf: &crate::Perf, voice: &Voice, noise: Option<&[f32]>, seed: u32) -> Result<Vec<f32>, String> {
        perf.check(self)?;
        let post = &voice.post;
        let (durations, mut curves) = if post.rate_x == 1.0 {
            (perf.durations.clone(), vec![perf.log_f0.clone(), perf.voiced.clone(), perf.energy.clone()])
        } else {
            edit::retime(&perf.durations, &[&perf.log_f0, &perf.voiced, &perf.energy], post.rate_x)
        };
        let enc = self.encode(&perf.tokens, &perf.stress, voice)?;
        let frames = self.expand(&enc.hidden, &durations, voice)?;
        let prosody = self.prosody(&frames, &perf.tokens);
        let energy = curves.pop().unwrap();
        let voiced = curves.pop().unwrap();
        let mut log_f0 = curves.pop().unwrap();
        widen_pitch(&mut log_f0, &voiced, post.f0_range_x);
        let mut cepstra = self.decode(&prosody.hidden, &log_f0, &voiced, &energy)?;
        let pitch = edit::oscillator_pitch(&log_f0, post, 0.0, seed);
        self.edit_envelope(&mut cepstra, &pitch, post);
        self.vocode(&cepstra, &pitch, None, noise, seed)
    }
}

/// Scales the pitch movement around the voiced mean (model.py `widen_pitch`); 1 leaves it untouched.
pub fn widen_pitch(log_f0: &mut [f32], voiced: &[f32], range: f32) {
    if range == 1.0 {
        return;
    }
    let (mut sum, mut n) = (0.0f32, 0usize);
    for (&p, &v) in log_f0.iter().zip(voiced) {
        if v > 0.0 {
            sum += p;
            n += 1;
        }
    }
    let mean = sum / n.max(1) as f32;
    for p in log_f0.iter_mut() {
        *p = mean + (*p - mean) * range;
    }
}

/// The oscillator's pitch wobble (model.py `pitch_wobble`, line 339) in ln-f0 units, from external Gaussian
/// draws so it is deterministic: `fast [T]` and `slow [T + 16]` (the same numbers torch draws give the same
/// wobble). Two thirds slow drift (a 17-frame mean, scaled to unit spread), one third frame-level.
pub fn pitch_wobble(fast: &[f32], slow: &[f32], semitones: f32) -> Vec<f32> {
    let t = fast.len();
    assert!(slow.len() == t + 16, "pitch_wobble: slow needs T + 16 values");
    let pooled: Vec<f64> = (0..t).map(|i| slow[i..i + 17].iter().map(|&v| v as f64).sum::<f64>() / 17.0).collect();
    let mean = pooled.iter().sum::<f64>() / t as f64;
    let var = pooled.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (t.max(2) - 1) as f64;
    let std = var.sqrt().max(1e-6);
    let k = semitones as f64 * std::f64::consts::LN_2 / 12.0 / 5f64.sqrt();
    (0..t).map(|i| ((2.0 * pooled[i] / std + fast[i] as f64) * k) as f32).collect()
}
