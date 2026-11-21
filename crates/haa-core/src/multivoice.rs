//! HAA voice (the multivoice student): everything its four ONNX graphs (voice, encode, prosody, decode) do not do,
//! shared by the `multivoice` node (crate::nodes::multivoice), the haa.wasm `haa_mv_*` exports and any page that
//! runs the graphs (the HAA voice widget). Owner rule: compute in ONNX (WebGPU) or Rust/wasm, the host is glue.
//!
//! * [`plan`]: a text cut at its tags. Non-verbal `(laugh)` (or haa's inline `{laugh}`) becomes an event token
//!   (ids 76.., repeated round(0.6 s / 0.15 s) = 4 times; an unknown `(word)` stays text). An emotion tag `[angry]`,
//!   `[angry:7]` (haa SCRIPT FORMAT v2 syntax: `key:N`, `key=N`, `key N`, N 0..10 -> intensity N/10; no N = the
//!   node's intensity) sets the emotion of every token after it until the next tag; `[neutral]`, `[]` reset;
//!   `[happy and curious]` mixes two emotions (each at weight 1/2, each at its own intensity) when the student
//!   blends that pair ([`BLEND_PAIRS`]), else says the first. A tag mid-line puts one `<sp>` at the switch, the
//!   tokens from there on carrying the new emotion (how the student was trained). Keys the model has no
//!   input for, `*emphasis*` stars and `|` pause bars are dropped. Text before the first tag has the node's emotion.
//! * [`utterances`]: the parts cut at sentence ends, one rendered part (streamed to the player) per sentence.
//! * [`tokens`] (feature `dizione`): an utterance phonemized and joined as dizione_tok.py `join_segments` does (inner
//!   `<bos>`/`<eos>` dropped, `<sp>` between parts), with each token's emotion.
//! * [`EmotionInput`]: the one place where per-token emotions become the graphs' emotion inputs, for both
//!   interfaces: io 1 = class ids i64 + intensity f32 per token, io 2 = a float mix vector of [`EMOTIONS`] width
//!   per token (intensity x weights, zero = neutral).
//! * [`voice_inputs`], [`Gauss`]: the voice graph inputs (attrs, quality, z = 0.3 x N(0,1) from the voice seed) and
//!   the seeded noise (encode dp_noise, decode z_noise / src_noise).
//! * [`expand`]: the encoder columns repeated by their durations (round, at least 0), the per-frame emotion and the
//!   per-frame prior noise (speech noise, event noise on event tokens), after the duration override.
//! * [`Overrides`]: curves of the graph that replace the predicted durations (as a factor), f0, voiced and energy.
//! * [`finish`]: the 22050 Hz audio at the player's rate, under a 0.99 peak.

use crate::nodes::curve::curve_at;

/// The rate of the decoder's audio.
pub const SAMPLE_RATE: f64 = 22_050.0;
/// Audio samples per frame.
pub const HOP: usize = 256;
/// Encoder / prior channels.
pub const CHANNELS: usize = 192;

const BOS: i64 = 1;
const EOS: i64 = 2;
const SP: i64 = 3;
/// dizione_tok.py EVENT_TAGS (append-only, ids EVENT_BASE..).
pub const EVENT_TAGS: [&str; 24] = [
    "laugh", "chuckle", "giggle", "cough", "clears throat", "sigh", "sob", "cry", "gasp", "yawn", "scream", "shout",
    "groan", "moan", "rattle", "grunt", "scoff", "snort", "sniff", "hum", "whimper", "panting", "wheeze", "sneeze",
];
/// Other spellings of the events.
pub const EVENT_ALIAS: [(&str, &str); 8] = [
    ("laughs heartily", "laugh"), ("laughs", "laugh"), ("screams", "scream"), ("screaming", "scream"),
    ("groans", "groan"), ("death rattle", "rattle"), ("coughing fit", "cough"), ("gasping for air", "gasp"),
];
/// len(SYMBOLS): 5 specials + 12 punctuation + 26 consonants + 33 vowels.
pub const EVENT_BASE: i64 = 76;
const EVENT_TOKEN_S: f64 = 0.15;
const EVENT_DEFAULT_S: f64 = 0.6;
/// The voice seed's spread (hourly_mv.py PRIOR_TAU).
const PRIOR_TAU: f32 = 0.3;

/// The student's emotions in input order (index 0, neutral, is the zero vector of io 2).
pub const EMOTIONS: [&str; 17] = [
    "neutral", "calm", "happy", "sad", "angry", "scared", "surprised", "disgust", "suspicious", "curious", "bored",
    "tired", "sarcastic", "confused", "dismissive", "melancholic", "hesitant",
];
/// Written keys that land on one of [`EMOTIONS`] (haa voice_script/emotions.json aliases and a few more).
const EMOTION_ALIAS: [(&str, &str); 56] = [
    ("relaxed", "calm"), ("serene", "calm"), ("peaceful", "calm"), ("joy", "happy"), ("joyful", "happy"),
    ("glad", "happy"), ("cheerful", "happy"), ("sadness", "sad"), ("unhappy", "sad"), ("sorrow", "sad"),
    ("sorrowful", "sad"), ("anger", "angry"), ("mad", "angry"), ("furious", "angry"), ("rage", "angry"),
    ("fear", "scared"), ("fearful", "scared"), ("afraid", "scared"), ("frightened", "scared"),
    ("terrified", "scared"), ("scary", "scared"), ("disgusted", "disgust"), ("disgusting", "disgust"),
    ("gross", "disgust"), ("surprise", "surprised"), ("astonished", "surprised"), ("amazed", "surprised"),
    ("suspicion", "suspicious"), ("wary", "suspicious"), ("distrustful", "suspicious"), ("doubtful", "suspicious"),
    ("boredom", "bored"), ("uninterested", "bored"), ("indifferent", "bored"), ("curiosity", "curious"),
    ("interested", "curious"), ("inquisitive", "curious"), ("sarcasm", "sarcastic"), ("ironic", "sarcastic"),
    ("irony", "sarcastic"), ("weary", "tired"), ("exhausted", "tired"), ("sleepy", "tired"),
    ("confusion", "confused"), ("puzzled", "confused"), ("perplexed", "confused"), ("dismissal", "dismissive"),
    ("contemptuous", "dismissive"), ("melancholy", "melancholic"), ("wistful", "melancholic"),
    ("hesitation", "hesitant"), ("hesitating", "hesitant"), ("unsure", "hesitant"), ("uncertain", "hesitant"),
    ("default", "neutral"), ("none", "neutral"),
];

/// The emotion pairs the student blends (student_run/gen/emotion_pairs.json, status "blends"); another `and` pair
/// says its first emotion only.
pub const BLEND_PAIRS: [(&str, &str); 27] = [
    ("calm", "happy"), ("calm", "confused"), ("happy", "disgust"), ("happy", "curious"), ("happy", "tired"),
    ("happy", "sarcastic"), ("happy", "confused"), ("sad", "bored"), ("sad", "sarcastic"), ("sad", "confused"),
    ("angry", "suspicious"), ("angry", "tired"), ("angry", "sarcastic"), ("scared", "curious"), ("scared", "tired"),
    ("scared", "confused"), ("scared", "melancholic"), ("surprised", "curious"), ("surprised", "dismissive"),
    ("surprised", "melancholic"), ("disgust", "suspicious"), ("disgust", "confused"), ("suspicious", "sarcastic"),
    ("suspicious", "melancholic"), ("bored", "confused"), ("tired", "dismissive"), ("tired", "melancholic"),
];

/// An emotion written in the text: its classes with their intensity (`None` = the node's intensity); empty = neutral.
/// Several classes mix at equal weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mood {
    pub items: Vec<(usize, Option<f32>)>,
}

/// The emotion of a token: the io 2 mix vector, and the io 1 class and intensity (the first class of a mix).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emotion {
    pub mix: [f32; EMOTIONS.len()],
    pub class: i64,
    pub intensity: f32,
}

impl Emotion {
    pub const NEUTRAL: Emotion = Emotion { mix: [0.0; EMOTIONS.len()], class: 0, intensity: 0.0 };

    /// One class at an intensity (class 0 = neutral).
    pub fn one(class: usize, intensity: f32) -> Emotion {
        let mut e = Emotion::NEUTRAL;
        if class > 0 && class < EMOTIONS.len() {
            let i = intensity.clamp(0.0, 1.0);
            e.mix[class] = i;
            e.class = class as i64;
            e.intensity = i;
        }
        e
    }
}

impl Mood {
    /// The token emotion: each class at weight 1/n times its intensity (`intensity` when the tag gave none).
    pub fn emotion(&self, intensity: f32) -> Emotion {
        let live: Vec<(usize, f32)> = self
            .items
            .iter()
            .map(|&(c, i)| (c, i.unwrap_or(intensity).clamp(0.0, 1.0)))
            .collect();
        if live.is_empty() {
            return Emotion::NEUTRAL;
        }
        let w = 1.0 / live.len() as f32;
        let mut e = Emotion::NEUTRAL;
        for &(c, i) in &live {
            if c > 0 && c < EMOTIONS.len() {
                e.mix[c] += w * i;
            }
        }
        if let Some(&(c, i)) = live.iter().find(|(c, _)| *c > 0) {
            e.class = c as i64;
            e.intensity = i;
        }
        e
    }
}

/// What a part of the text is.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Event { id: i64, count: usize, tag: String },
}

/// A part of the text and the emotion in force there (`None` = the node's emotion).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub piece: Piece,
    pub mood: Option<Mood>,
}

/// An event tag or one of its aliases -> its token id.
pub fn event_id(tag: &str) -> Option<i64> {
    let t = EVENT_ALIAS.iter().find(|(a, _)| *a == tag).map(|(_, b)| *b).unwrap_or(tag);
    EVENT_TAGS.iter().position(|e| *e == t).map(|i| EVENT_BASE + i as i64)
}

/// How many event tokens stand for `seconds` (Python round, half to even; 1..12).
pub fn event_count(seconds: f64) -> usize {
    let x = seconds / EVENT_TOKEN_S;
    let r = x.round();
    let r = if (x - x.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 { r - x.signum() } else { r };
    (r as i64).clamp(1, 12) as usize
}

/// A written emotion key -> class (case, spaces, hyphens and underscores folded like haa's canon_key).
pub fn emotion_class(key: &str) -> Option<usize> {
    let k: String = key.trim().to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').collect();
    let k = EMOTION_ALIAS.iter().find(|(a, _)| *a == k).map(|(_, b)| *b).unwrap_or(&k);
    EMOTIONS.iter().position(|e| *e == k)
}

/// One `key`, `key:N`, `key=N` or `key N` item -> (class, intensity).
fn tag_item(item: &str) -> Option<(usize, Option<f32>)> {
    let item = item.trim();
    let (name, value) = match item.find([':', '=']) {
        Some(i) => (item[..i].trim(), Some(item[i + 1..].trim())),
        None => match item.rsplit_once(char::is_whitespace) {
            Some((a, b)) if b.parse::<f64>().is_ok() => (a.trim(), Some(b)),
            _ => (item, None),
        },
    };
    let class = emotion_class(name)?;
    let level = value.and_then(|v| v.parse::<f32>().ok()).filter(|n| n.is_finite());
    Some((class, level.map(|n| n.clamp(0.0, 10.0) / 10.0)))
}

/// The contents of one `[...]` tag: its emotion (`None` = no emotion key: no change). Items are split by `,`/`;`;
/// the first item naming an emotion counts, and `and` inside it mixes several.
pub fn parse_tag(inside: &str) -> Option<Mood> {
    let body = inside.trim();
    if body.is_empty() {
        return Some(Mood::default());
    }
    for item in body.split([',', ';']) {
        let classes: Vec<(usize, Option<f32>)> = item.split(" and ").filter_map(tag_item).collect();
        if classes.is_empty() {
            continue;
        }
        let mut items: Vec<(usize, Option<f32>)> = classes.into_iter().filter(|(c, _)| *c != 0).collect();
        if items.len() > 2 || (items.len() == 2 && !blends(items[0].0, items[1].0)) {
            items.truncate(1);
        }
        return Some(Mood { items });
    }
    None
}

/// Whether the student blends two emotion classes (either order).
pub fn blends(a: usize, b: usize) -> bool {
    let (Some(x), Some(y)) = (EMOTIONS.get(a), EMOTIONS.get(b)) else { return false };
    BLEND_PAIRS.iter().any(|&(p, q)| (p == *x && q == *y) || (p == *y && q == *x))
}

/// Notes about a text's tags: an `and` mix the student does not blend (only its first emotion is said).
pub fn warnings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('[') {
        let Some(j) = rest[i..].find(']') else { break };
        let inside = &rest[i + 1..i + j];
        for item in inside.split([',', ';']) {
            let classes: Vec<usize> = item.split(" and ").filter_map(tag_item).map(|(c, _)| c).filter(|&c| c != 0).collect();
            if classes.len() > 2 || (classes.len() == 2 && !blends(classes[0], classes[1])) {
                out.push(format!("[{inside}]: the HAA voice does not blend these emotions; it says {} only", EMOTIONS[classes[0]]));
            }
            if !classes.is_empty() {
                break;
            }
        }
        rest = &rest[i + j + 1..];
    }
    out
}

/// The text cut at its non-verbal and emotion tags, each part with the emotion in force.
pub fn plan(text: &str) -> Vec<Part> {
    let flat: String = text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let b = flat.as_bytes();
    let (mut parts, mut buf, mut i) = (Vec::new(), String::new(), 0usize);
    let mut mood: Option<Mood> = None;
    let flush = |buf: &mut String, parts: &mut Vec<Part>, mood: &Option<Mood>| {
        let s = buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if !s.is_empty() {
            parts.push(Part { piece: Piece::Text(s), mood: mood.clone() });
        }
        buf.clear();
    };
    let closing = |open: usize, close: char| flat[open + 1..].find(close).map(|k| open + 1 + k);
    while i < b.len() {
        match b[i] {
            b'(' | b'{' => {
                let close = if b[i] == b'(' { ')' } else { '}' };
                if let Some(j) = closing(i, close) {
                    let inner = &flat[i + 1..j];
                    let word = inner.starts_with(|c: char| c.is_ascii_alphabetic())
                        && inner.chars().all(|c| c.is_ascii_alphabetic() || c == ' ');
                    let tag = inner.trim().to_ascii_lowercase();
                    if let Some(id) = word.then(|| event_id(&tag)).flatten() {
                        flush(&mut buf, &mut parts, &mood);
                        parts.push(Part { piece: Piece::Event { id, count: event_count(EVENT_DEFAULT_S), tag }, mood: mood.clone() });
                        i = j + 1;
                        continue;
                    }
                    if close == '}' {
                        // haa inline phones or an unknown sound: the student has no input for it
                        buf.push(' ');
                        i = j + 1;
                        continue;
                    }
                }
            }
            b'[' => {
                if let Some(j) = closing(i, ']') {
                    if let Some(m) = parse_tag(&flat[i + 1..j]) {
                        flush(&mut buf, &mut parts, &mood);
                        mood = Some(m);
                    } else {
                        buf.push(' ');
                    }
                    i = j + 1;
                    continue;
                }
            }
            b'*' | b'|' => {
                buf.push(' ');
                i += 1;
                continue;
            }
            _ => {}
        }
        let c = flat[i..].chars().next().unwrap_or(' ');
        buf.push(c);
        i += c.len_utf8();
    }
    flush(&mut buf, &mut parts, &mood);
    parts
}

/// Byte offsets just after each sentence end of `s` (`.`, `!`, `?` or `…` runs, closing quotes included, followed by
/// a space or the end of `s`).
fn sentence_ends(s: &str) -> Vec<usize> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < chars.len() {
        if matches!(chars[k].1, '.' | '!' | '?' | '…') {
            let mut e = k;
            while e + 1 < chars.len() && matches!(chars[e + 1].1, '.' | '!' | '?' | '…' | '"' | '\'' | ')' | '»' | '”') {
                e += 1;
            }
            if e + 1 == chars.len() {
                out.push(s.len());
            } else if chars[e + 1].1.is_whitespace() {
                out.push(chars[e + 1].0);
            }
            k = e + 1;
        } else {
            k += 1;
        }
    }
    out
}

/// The parts cut into utterances at sentence ends (an event stays with the sentence it follows or starts).
pub fn utterances(parts: &[Part]) -> Vec<Vec<Part>> {
    let mut out: Vec<Vec<Part>> = Vec::new();
    let mut cur: Vec<Part> = Vec::new();
    for p in parts {
        match &p.piece {
            Piece::Text(s) => {
                let mut from = 0;
                for end in sentence_ends(s) {
                    let piece = s[from..end].trim();
                    if !piece.is_empty() {
                        cur.push(Part { piece: Piece::Text(piece.to_string()), mood: p.mood.clone() });
                    }
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                    from = end;
                }
                let rest = s[from..].trim();
                if !rest.is_empty() {
                    cur.push(Part { piece: Piece::Text(rest.to_string()), mood: p.mood.clone() });
                }
            }
            Piece::Event { .. } => cur.push(p.clone()),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// One utterance as the encoder takes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    pub ids: Vec<i64>,
    pub stress: Vec<i64>,
    pub emotions: Vec<Emotion>,
}

/// dizione_tok.py `join_segments` over phonemized parts (`(ids, stress)` with their `<bos>`/`<eos>`, or an event)
/// with each token's emotion: each part's own, `<bos>` the first part's, a `<sp>` the part after it, `<eos>` the
/// last part's. `node` is the node's emotion (parts without a tag) and `intensity` the node's intensity.
pub fn assemble(parts: &[(Option<(Vec<i64>, Vec<i64>)>, &Part)], node: Emotion, intensity: f32) -> Tokens {
    let emo = |p: &Part| p.mood.as_ref().map_or(node, |m| m.emotion(intensity));
    let first = parts.first().map_or(node, |(_, p)| emo(p));
    let mut out = Tokens { ids: vec![BOS], stress: vec![0], emotions: vec![first] };
    let mut last = first;
    for (k, (phones, p)) in parts.iter().enumerate() {
        let e = emo(p);
        let body: Vec<(i64, i64)> = match (&p.piece, phones) {
            (Piece::Event { id, count, .. }, _) => (0..(*count).clamp(1, 12)).map(|_| (*id, 0)).collect(),
            (Piece::Text(_), Some((ids, stress))) => ids
                .iter()
                .zip(stress.iter().chain(std::iter::repeat(&0)))
                .map(|(&x, &y)| (x, y))
                .filter(|(x, _)| *x != BOS && *x != EOS)
                .collect(),
            (Piece::Text(_), None) => Vec::new(),
        };
        if body.is_empty() {
            continue;
        }
        last = e;
        if k > 0 {
            out.ids.push(SP);
            out.stress.push(0);
            out.emotions.push(e);
        }
        for (x, y) in body {
            out.ids.push(x);
            out.stress.push(y);
            out.emotions.push(e);
        }
    }
    out.ids.push(EOS);
    out.stress.push(0);
    out.emotions.push(last);
    out
}

/// An utterance phonemized with the shared dizione phonemizer and assembled ([`assemble`]).
#[cfg(feature = "dizione")]
pub fn tokens(utterance: &[Part], node: Emotion, intensity: f32) -> Result<Tokens, String> {
    let mut phonemized = Vec::with_capacity(utterance.len());
    for p in utterance {
        let phones = match &p.piece {
            Piece::Text(s) => {
                let g = dizione::with_shared(|ph| ph.g2p(s))
                    .ok_or("no phonemizer loaded (haa_dizione_gguf with the tiny-voca GGUF, or haa_dizione_*)")?;
                match g {
                    Ok(g) => Some((
                        g.tokens.ids.iter().map(|&x| x as i64).collect::<Vec<_>>(),
                        g.tokens.stress.iter().map(|&x| x as i64).collect::<Vec<_>>(),
                    )),
                    Err(_) => None, // nothing to say in this piece (punctuation only)
                }
            }
            Piece::Event { .. } => None,
        };
        phonemized.push((phones, p));
    }
    Ok(assemble(&phonemized, node, intensity))
}

/// Without the `dizione` feature there is no text front end.
#[cfg(not(feature = "dizione"))]
pub fn tokens(_utterance: &[Part], _node: Emotion, _intensity: f32) -> Result<Tokens, String> {
    Err("this build has no text front end (haa-core feature dizione)".into())
}

/// The graphs' emotion inputs, per token or per frame (the only place that knows the two interfaces).
#[derive(Clone, Debug, PartialEq)]
pub enum EmotionInput {
    /// io 1: `emotion` i64 [1,n] class ids and `intensity` f32 [1,n].
    Ids { ids: Vec<i64>, intensity: Vec<f32> },
    /// io 2: `emotion` f32 [1,n,dim] mix vectors (intensity x weights, zero = neutral).
    Mix { dim: usize, data: Vec<f32> },
}

impl EmotionInput {
    /// The inputs of `emotions` for interface `io` (`dim` = the mix width io 2 takes, at most [`EMOTIONS`]).
    pub fn new(io: u32, dim: usize, emotions: &[Emotion]) -> EmotionInput {
        if io >= 2 {
            let dim = dim.clamp(1, EMOTIONS.len());
            let data = emotions.iter().flat_map(|e| e.mix[..dim].iter().copied()).collect();
            EmotionInput::Mix { dim, data }
        } else {
            EmotionInput::Ids {
                ids: emotions.iter().map(|e| e.class.min(15)).collect(),
                intensity: emotions.iter().map(|e| e.intensity).collect(),
            }
        }
    }

    /// Rows `idx` (a token per frame).
    pub fn gather(&self, idx: &[usize]) -> EmotionInput {
        match self {
            EmotionInput::Ids { ids, intensity } => EmotionInput::Ids {
                ids: idx.iter().map(|&i| ids[i]).collect(),
                intensity: idx.iter().map(|&i| intensity[i]).collect(),
            },
            EmotionInput::Mix { dim, data } => EmotionInput::Mix {
                dim: *dim,
                data: idx.iter().flat_map(|&i| data[i * dim..(i + 1) * dim].iter().copied()).collect(),
            },
        }
    }
}

/// The voice graph inputs: attrs [age/63, sex/15, weight/63], quality [rough, breath, tremble]/255 and
/// z [8] = 0.3 x N(0,1) from the voice seed (stream 0).
pub fn voice_inputs(age: f64, sex: f64, weight: f64, rough: f64, breath: f64, tremble: f64, seed: u32) -> ([f32; 3], [f32; 3], [f32; 8]) {
    let lv = |v: f64, max: f64| (v.round().clamp(0.0, max) / max) as f32;
    let mut g = Gauss::new(seed, 0);
    let mut z = [0f32; 8];
    for v in &mut z {
        *v = PRIOR_TAU * g.next();
    }
    ([lv(age, 63.0), lv(sex, 15.0), lv(weight, 63.0)], [lv(rough, 255.0), lv(breath, 255.0), lv(tremble, 255.0)], z)
}

/// SplitMix64 stream + Box-Muller: a fixed (seed, stream) -> fixed N(0,1) numbers (the widget's helper, same values).
pub struct Gauss {
    s: u64,
    spare: Option<f32>,
}

impl Gauss {
    pub fn new(seed: u32, stream: u32) -> Self {
        let mut g = Gauss { s: ((seed as u64) << 32 | stream as u64) ^ 0x9E37_79B9_7F4A_7C15, spare: None };
        g.next_u64();
        g
    }
    fn next_u64(&mut self) -> u64 {
        self.s = self.s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64
    }
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        let (u1, u2) = (self.uniform(), self.uniform());
        let r = (-2.0 * u1.ln()).sqrt();
        let a = std::f64::consts::TAU * u2;
        self.spare = Some((r * a.sin()) as f32);
        (r * a.cos()) as f32
    }
    /// `n` numbers.
    pub fn take(seed: u32, stream: u32, n: usize) -> Vec<f32> {
        let mut g = Gauss::new(seed, stream);
        (0..n).map(|_| g.next()).collect()
    }
}

/// The noise stream `s` (1 dp_noise, 2 z_noise, 3 src_noise) of utterance `k`: the first utterance uses the
/// widget's streams 1, 2, 3.
pub fn stream(k: usize, s: u32) -> u32 {
    4 * k as u32 + s
}

/// A curve of the graph driving an override: the curve node's breakpoints and its gain (its Amplitude times the
/// connection amount).
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub points: Vec<(f64, f64)>,
    pub gain: f64,
}

/// One override: the node param's base, range and the curves wired into it. Like any modulated param, its value at
/// patch time `s` is clamp(base + sum(curve(s / duration) x gain), min, max).
#[derive(Clone, Debug, PartialEq)]
pub struct Override {
    pub base: f64,
    pub min: f64,
    pub max: f64,
    pub drives: Vec<Drive>,
}

impl Override {
    pub fn at(&self, seconds: f64, duration: f64) -> f64 {
        let t = if duration > 0.0 { (seconds / duration).clamp(0.0, 1.0) } else { 0.0 };
        let v = self.base + self.drives.iter().map(|d| curve_at(&d.points, t) * d.gain).sum::<f64>();
        v.clamp(self.min, self.max)
    }
}

/// The overrides of a node: the curves wired into `duration` (a factor on the predicted token durations), `f0`
/// (Hz), `voiced` (0..1) and `energy` (dB), and the patch duration their time runs over. Absent = predicted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    pub duration: Option<Override>,
    pub f0: Option<Override>,
    pub voiced: Option<Override>,
    pub energy: Option<Override>,
    pub patch_seconds: f64,
}

impl Overrides {
    pub fn is_empty(&self) -> bool {
        self.duration.is_none() && self.f0.is_none() && self.voiced.is_none() && self.energy.is_none()
    }

    /// Replaces the predicted curves of frames starting at patch time `offset` (seconds) where an override is wired:
    /// log_f0 = ln(f0 Hz), voiced, energy dB. Returns the names replaced.
    pub fn apply_curves(&self, log_f0: &mut [f32], voiced: &mut [f32], energy: &mut [f32], offset: f64) -> Vec<&'static str> {
        let mut done = Vec::new();
        let time = |j: usize| offset + (j * HOP) as f64 / SAMPLE_RATE;
        if let Some(o) = &self.f0 {
            log_f0.iter_mut().enumerate().for_each(|(j, v)| *v = o.at(time(j), self.patch_seconds).max(1.0).ln() as f32);
            done.push("f0");
        }
        if let Some(o) = &self.voiced {
            voiced.iter_mut().enumerate().for_each(|(j, v)| *v = o.at(time(j), self.patch_seconds) as f32);
            done.push("voiced");
        }
        if let Some(o) = &self.energy {
            energy.iter_mut().enumerate().for_each(|(j, v)| *v = o.at(time(j), self.patch_seconds) as f32);
            done.push("energy");
        }
        done
    }
}

/// The encoder columns per frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Expanded {
    pub frames: usize,
    /// h, m_p, logs_p [CHANNELS, frames].
    pub h: Vec<f32>,
    pub m: Vec<f32>,
    pub logs: Vec<f32>,
    pub emotion: EmotionInput,
    /// The prior noise scale per frame: `noise.1` on frames of event tokens, `noise.0` elsewhere.
    pub noise: Vec<f32>,
    /// The token durations used (frames).
    pub durations: Vec<usize>,
}

/// Expands the encoder outputs (h, m_p, logs_p [CHANNELS, t]) by `durations` [t] (rounded half to even, at least 0;
/// times the duration override at each token's predicted start, patch time `offset` + its frames so far).
pub fn expand(
    durations: &[f32],
    h: &[f32],
    m: &[f32],
    logs: &[f32],
    ids: &[i64],
    emotion: &EmotionInput,
    noise: (f32, f32),
    overrides: &Overrides,
    offset: f64,
) -> Expanded {
    let t = durations.len();
    let round = |x: f32| -> f32 {
        let r = x.round();
        if (x - x.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 { r - x.signum() } else { r }
    };
    let mut d: Vec<usize> = durations.iter().map(|&x| round(x).max(0.0) as usize).collect();
    if let Some(o) = &overrides.duration {
        let mut at = offset;
        for v in &mut d {
            let predicted = *v;
            *v = round((predicted as f64 * o.at(at, overrides.patch_seconds)) as f32).max(0.0) as usize;
            at += (predicted * HOP) as f64 / SAMPLE_RATE;
        }
    }
    let idx: Vec<usize> = d.iter().enumerate().flat_map(|(i, &n)| std::iter::repeat_n(i, n)).collect();
    let frames = idx.len();
    let cols = |x: &[f32]| -> Vec<f32> {
        let mut out = Vec::with_capacity(CHANNELS * frames);
        for c in 0..CHANNELS {
            let row = &x[c * t..(c + 1) * t];
            out.extend(idx.iter().map(|&i| row[i]));
        }
        out
    };
    Expanded {
        frames,
        h: cols(h),
        m: cols(m),
        logs: cols(logs),
        emotion: emotion.gather(&idx),
        noise: idx.iter().map(|&i| if ids.get(i).is_some_and(|&x| x >= EVENT_BASE) { noise.1 } else { noise.0 }).collect(),
        durations: d,
    }
}

/// The decode noise inputs of utterance `k` for interface `io`: (noise_scale, z_noise [CHANNELS, frames],
/// src_noise [HOP x frames]). io 2 takes the per-frame prior noise as noise_scale [1,1,T]; io 1 takes one
/// noise_scale = 1 and the per-frame scale carried by z_noise (z_p = m + z_noise x exp(logs_p) x noise_scale).
pub fn decode_noise(io: u32, seed: u32, k: usize, frame_noise: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let frames = frame_noise.len();
    let mut z = Gauss::take(seed, stream(k, 2), CHANNELS * frames);
    let scale = if io >= 2 {
        frame_noise.to_vec()
    } else {
        for c in 0..CHANNELS {
            for (j, s) in frame_noise.iter().enumerate() {
                z[c * frames + j] *= s;
            }
        }
        vec![1.0]
    };
    (scale, z, Gauss::take(seed, stream(k, 3), HOP * frames))
}

/// The decoder's audio at `rate`, brought under a 0.99 peak (each part on its own, like the `speak` node).
pub fn finish(audio: &[f32], rate: f64) -> Vec<f32> {
    let mut out = tinyvoca::resample(audio, SAMPLE_RATE, rate);
    out.iter_mut().for_each(|v| if !v.is_finite() { *v = 0.0 });
    let peak = out.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    if peak > 0.99 {
        let k = 0.99 / peak;
        out.iter_mut().for_each(|v| *v *= k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_cuts_tags_mixes_and_sentences() {
        let p = plan("Hello. [happy] We won! [sad:7] But (sigh) Tom got hurt. [angry and tired:4] Go. [angry and happy] Hm. [] Ok.");
        let text = |p: &Part| match &p.piece {
            Piece::Text(s) => s.clone(),
            Piece::Event { tag, .. } => format!("({tag})"),
        };
        assert_eq!(p.iter().map(text).collect::<Vec<_>>(), ["Hello.", "We won!", "But", "(sigh)", "Tom got hurt.", "Go.", "Hm.", "Ok."]);
        assert_eq!(p[0].mood, None);
        assert_eq!(p[3].mood, Some(Mood { items: vec![(3, Some(0.7))] }));
        let mix = p[5].mood.as_ref().unwrap().emotion(0.9);
        assert_eq!((mix.mix[4], mix.mix[11], mix.class), (0.45, 0.2, 4));
        assert_eq!(p[6].mood, Some(Mood { items: vec![(4, None)] }));
        assert_eq!(p[7].mood, Some(Mood::default()));
        assert_eq!(warnings("[angry and happy] Hm. [angry and tired] Go."), ["[angry and happy]: the HAA voice does not blend these emotions; it says angry only"]);
        let u = utterances(&p);
        assert_eq!(u.iter().map(|u| u.iter().map(text).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>(),
                   ["Hello.", "We won!", "But (sigh) Tom got hurt.", "Go.", "Hm.", "Ok."]);
        assert_eq!(parse_tag("laughs"), None);
        assert_eq!(parse_tag("hesitant:5"), Some(Mood { items: vec![(16, Some(0.5))] }));
    }

    #[test]
    fn assemble_and_both_emotion_interfaces() {
        let a = Part { piece: Piece::Text("a".into()), mood: None };
        let ev = Part { piece: Piece::Event { id: 79, count: 2, tag: "cough".into() }, mood: Some(Mood { items: vec![(4, Some(0.7))] }) };
        let parts = [(Some((vec![1, 20, 21, 2], vec![0, 1, 0, 0])), &a), (None, &ev)];
        let t = assemble(&parts, Emotion::one(2, 0.9), 0.9);
        assert_eq!(t.ids, [1, 20, 21, 3, 79, 79, 2]);
        assert_eq!(t.stress, [0, 1, 0, 0, 0, 0, 0]);
        let io1 = EmotionInput::new(1, 17, &t.emotions);
        assert_eq!(io1, EmotionInput::Ids { ids: vec![2, 2, 2, 4, 4, 4, 4], intensity: vec![0.9, 0.9, 0.9, 0.7, 0.7, 0.7, 0.7] });
        let EmotionInput::Mix { dim, data } = EmotionInput::new(2, 17, &t.emotions) else { panic!() };
        assert_eq!((dim, data.len(), data[2], data[17 * 3 + 4], data[0]), (17, 7 * 17, 0.9, 0.7, 0.0));
    }

    #[test]
    fn expand_overrides_and_noise() {
        let t = 3;
        let h: Vec<f32> = (0..CHANNELS * t).map(|i| i as f32).collect();
        let emo = EmotionInput::new(1, 17, &[Emotion::NEUTRAL, Emotion::one(4, 1.0), Emotion::NEUTRAL]);
        let none = Overrides::default();
        let x = expand(&[2.4, 0.5, 1.5], &h, &h, &h, &[5, 80, 6], &emo, (0.3, 1.0), &none, 0.0);
        assert_eq!((x.frames, x.durations.clone()), (4, vec![2, 0, 2]));
        assert_eq!(&x.h[..4], &[0.0, 0.0, 2.0, 2.0]);
        assert_eq!(x.noise, [0.3, 0.3, 0.3, 0.3]);
        // a duration override of 2 doubles every token
        let twice = Overrides { duration: Some(Override { base: 2.0, min: 0.25, max: 4.0, drives: vec![] }), patch_seconds: 1.0, ..none.clone() };
        assert_eq!(expand(&[2.0, 1.0, 1.0], &h, &h, &h, &[5, 80, 6], &emo, (0.3, 1.0), &twice, 0.0).durations, [4, 2, 2]);
        // an f0 curve rising 100 -> 300 Hz over the patch replaces the predicted f0
        let f0 = Overrides { f0: Some(Override { base: 100.0, min: 50.0, max: 500.0, drives: vec![Drive { points: vec![(0.0, 0.0), (1.0, 1.0)], gain: 200.0 }] }), patch_seconds: 1.0, ..none };
        let (mut lf0, mut vo, mut en) = (vec![0.0f32; 87], vec![0.5f32; 87], vec![-30.0f32; 87]);
        assert_eq!(f0.apply_curves(&mut lf0, &mut vo, &mut en, 0.0), ["f0"]);
        assert!((lf0[0].exp() - 100.0).abs() < 0.01 && (lf0[86].exp() - (100.0 + 200.0 * 86.0 * 256.0 / 22050.0) as f32).abs() < 0.1);
        assert_eq!((vo[3], en[3]), (0.5, -30.0));
        let (s1, z1, src) = decode_noise(1, 7, 0, &[0.5, 0.0]);
        let raw = Gauss::take(7, 2, 4);
        assert_eq!((s1, &z1[..4], src.len()), (vec![1.0], &[raw[0] * 0.5, 0.0, raw[2] * 0.5, 0.0][..], 512));
        assert_eq!(decode_noise(2, 7, 0, &[0.5, 0.0]).0, [0.5, 0.0]);
    }
}
