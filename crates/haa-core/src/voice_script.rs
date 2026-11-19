//! Script markup of the `speak` node (SCRIPT FORMAT v2, NPC voice PLAN.md) and the emotion edits it drives.
//!
//! ```text
//! [calm] Hello. [scared:4, question:10] What do *you* want? || [angry:10] Leave me alone!
//! ```
//! - A tag `[key:N, key:N, ...]` holds keys separated by commas, each with a level N from 0 to 10 (5 when `:N` is
//!   left out; `key=N` and `key N` are read too). It applies until the next tag; `[neutral]` or `[]` resets.
//!   Keys are emotions and delivery modes of `emotions.json` (case-insensitive, aliases folded: fear = scared,
//!   ask = question...) or author overrides whose value is a value (`rate:1.1`, `pitch:+2`, `range`, `breath`,
//!   `energy`/`loud`, `pause`, `tilt`, or a parameter name). An unknown key gives a warning and is ignored; a tag
//!   with only unknown keys (`[laughs]`) changes nothing.
//! - `*word*` emphasises (one step, `emph:5`), `**word**` twice (`emph:10`).
//! - `|` is a pause of 150 ms, `||` of 400 ms, placed after the word before it.
//! - `{...}` is an inline sound ([`sounds`]): phones that bypass G2P (`{m: h}`, `{mmm hh}`, `{h ə l ˈoʊ}`,
//!   `{HH AH0 L OW1}`, `{m:0.8s}`) or a named vocal event (`{hmm}`, `{sigh}`, `{laugh}`...). It takes the keys of the
//!   tag in force like any word. dizione gets a one-vowel inline word `{ə}` in its place (dizione's own inline
//!   syntax), whose token is replaced by the sound's phones after G2P.
//!
//! [`parse`] gives the plain text for dizione and the spans, emphasis, pauses and sounds in its bytes, with
//! warnings. [`plan_utterances`] maps them onto the tokens of each utterance dizione cut from the plain text, and
//! [`synthesize_edited`] runs tiny-voca's stages with the edits of [`emotion`] between them: durations after
//! `encode`, curves after `prosody`, the cepstral delta before `vocode`.

pub mod emotion;
pub mod sounds;

pub use emotion::{Curves, Edit, Emotions, TokenSpan, install, with_emotions};
pub use sounds::{Events, Sound, with_events};

/// What dizione is given in place of an inline sound: one inline word of one vowel (replaced after G2P).
pub const SOUND_PLACEHOLDER: &str = "{ə}";

/// A stretch of the plain text and the keys of the tag in force there (canonical names; empty = neutral).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub keys: Vec<(String, f64)>,
}

/// An emphasised stretch of the plain text: level 5 for `*...*`, 10 for `**...**`.
#[derive(Clone, Debug, PartialEq)]
pub struct Emphasis {
    pub start: usize,
    pub end: usize,
    pub level: f64,
}

/// A pause at a byte offset of the plain text.
#[derive(Clone, Debug, PartialEq)]
pub struct Pause {
    pub at: usize,
    pub seconds: f64,
}

/// A parsed script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    /// What is said: tags, emphasis marks and pause bars removed (a space is left where one separated two words).
    pub plain: String,
    /// Each `[...]` tag as written: its byte offset in `plain` and what was inside the brackets.
    pub tags: Vec<(usize, String)>,
    /// The tag states over `plain`, in order, covering it from the first tag (text before it is neutral).
    pub spans: Vec<Span>,
    pub emphasis: Vec<Emphasis>,
    pub pauses: Vec<Pause>,
    /// Inline sounds, in order: the byte offset of their placeholder in `plain` and the sound.
    pub sounds: Vec<(usize, Sound)>,
    pub warnings: Vec<String>,
}

impl Script {
    /// True when the markup changes the performance (a non-neutral tag, emphasis, a pause or a sound).
    pub fn edits(&self) -> bool {
        self.spans.iter().any(|s| !s.keys.is_empty()) || !self.emphasis.is_empty() || !self.pauses.is_empty() || !self.sounds.is_empty()
    }

    /// The text cut at each tag, with the keys in force: `(text, keys)`. Text before the first tag is a neutral
    /// segment when it holds more than spaces.
    pub fn segments(&self) -> Vec<(String, Vec<(String, f64)>)> {
        let mut out = Vec::new();
        let first = self.spans.first().map_or(self.plain.len(), |s| s.start);
        if !self.plain[..first].trim().is_empty() {
            out.push((self.plain[..first].trim().to_string(), Vec::new()));
        }
        for s in &self.spans {
            out.push((self.plain[s.start..s.end].trim().to_string(), s.keys.clone()));
        }
        out
    }

    /// The keys in force at byte `at` of `plain`.
    pub fn keys_at(&self, at: usize) -> &[(String, f64)] {
        self.spans.iter().rev().find(|s| s.start <= at).map_or(&[], |s| s.keys.as_slice())
    }
}

fn is_word_char(c: Option<char>) -> bool {
    c.is_some_and(char::is_alphanumeric)
}

/// Pushes a space when the text pushed so far and `after` both touch a word (so two words never merge).
fn separate(plain: &mut String, after: &str) {
    if is_word_char(plain.chars().next_back()) && is_word_char(after.chars().next()) {
        plain.push(' ');
    }
}

/// The canonical preset name of a written key, tolerant of case, spaces, hyphens and underscores.
fn canon_key(emotions: &Emotions, key: &str) -> Option<String> {
    let k = key.trim().to_lowercase();
    let spaced: String = k.split(|c: char| c.is_whitespace() || c == '-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("_");
    let bare: String = spaced.chars().filter(|&c| c != '_').collect();
    emotions.canon(&k).or_else(|| emotions.canon(&spaced)).or_else(|| emotions.canon(&bare))
}

/// The keys of one tag's contents, or `None` when it names nothing known (the state then stays as it is).
fn parse_tag(inside: &str, emotions: &Emotions, warnings: &mut Vec<String>) -> Option<Vec<(String, f64)>> {
    let body = inside.trim();
    if body.is_empty() {
        return Some(Vec::new());
    }
    let mut keys: Vec<(String, f64)> = Vec::new();
    let mut known = false;
    let put = |keys: &mut Vec<(String, f64)>, k: String, n: f64| match keys.iter_mut().find(|(x, _)| *x == k) {
        Some(slot) => slot.1 = n,
        None => keys.push((k, n)),
    };
    for item in body.split([',', ';']) {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        // key:N, key=N, key N, or a bare key / a bare number
        let (name, value) = match item.find([':', '=']) {
            Some(i) => (item[..i].trim(), Some(item[i + 1..].trim())),
            None => match item.rsplit_once(char::is_whitespace) {
                Some((a, b)) if b.parse::<f64>().is_ok() => (a.trim(), Some(b)),
                _ => (item, None),
            },
        };
        if value.is_none() && name.parse::<f64>().is_ok() {
            match keys.last_mut() {
                Some(last) if !emotion::is_override(&last.0) => {
                    let n = name.parse::<f64>().unwrap_or(5.0);
                    warnings.push(format!("[{body}]: the bare number {name} is read as the level of {}", last.0));
                    last.1 = n.clamp(0.0, 10.0);
                }
                _ => warnings.push(format!("[{body}]: the number {name} names no key and is ignored")),
            }
            continue;
        }
        let number = match value {
            None => None,
            Some(v) => match v.parse::<f64>() {
                Ok(n) if n.is_finite() => Some(n),
                _ => {
                    warnings.push(format!("[{body}]: {item} has no number after the key and is ignored"));
                    continue;
                }
            },
        };
        let lower = name.to_lowercase();
        if let Some(c) = canon_key(emotions, name) {
            known = true;
            if c == "neutral" {
                continue;
            }
            let n = number.unwrap_or(5.0);
            if !(0.0..=10.0).contains(&n) {
                warnings.push(format!("[{body}]: the level of {name} is {n}, outside 0..10, and is clipped"));
            }
            put(&mut keys, c, n.clamp(0.0, 10.0));
        } else if emotion::is_emphasis(&lower) {
            known = true;
            put(&mut keys, "emph".into(), number.unwrap_or(5.0).clamp(0.0, 10.0));
        } else if emotion::is_override(&lower) {
            match number {
                Some(n) => {
                    known = true;
                    put(&mut keys, lower, n);
                }
                None => warnings.push(format!("[{body}]: the override {name} needs a value ({name}:x) and is ignored")),
            }
        } else {
            warnings.push(format!("[{body}]: unknown key \"{name}\" is ignored"));
        }
    }
    known.then_some(keys)
}

/// Reads the script markup of a `speak` text with the presets `emotions` (they say which keys exist) and the
/// vocal events `events`.
pub fn parse(text: &str, emotions: &Emotions, events: &Events) -> Script {
    let mut out = Script::default();
    let mut open: [Option<usize>; 2] = [None, None]; // `*` and `**`: where the emphasis began in `plain`
    let mut rest = text;
    let mut changes: Vec<(usize, Vec<(String, f64)>)> = Vec::new();
    while let Some(i) = rest.find(['[', '*', '|', '{']) {
        let c = rest.as_bytes()[i];
        match c {
            b'{' => {
                let Some(len) = rest[i..].find('}') else {
                    out.plain.push_str(&rest[..i]);
                    out.warnings.push("an unclosed { is ignored".into());
                    rest = &rest[i + 1..];
                    continue;
                };
                out.plain.push_str(&rest[..i]);
                let inside = &rest[i + 1..i + len];
                let after = &rest[i + len + 1..];
                if let Some(sound) = sounds::parse_sound(inside, events, &mut out.warnings) {
                    if out.plain.chars().next_back().is_some_and(|c| !c.is_whitespace()) {
                        out.plain.push(' ');
                    }
                    out.sounds.push((out.plain.len(), sound));
                    out.plain.push_str(SOUND_PLACEHOLDER);
                    if after.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '{' || c == '[') {
                        out.plain.push(' ');
                    }
                } else {
                    separate(&mut out.plain, after);
                }
                rest = after;
            }
            b'[' => {
                let Some(len) = rest[i..].find(']') else {
                    out.plain.push_str(rest);
                    rest = "";
                    break;
                };
                out.plain.push_str(&rest[..i]);
                let inside = &rest[i + 1..i + len];
                let after = &rest[i + len + 1..];
                out.tags.push((out.plain.len(), inside.trim().to_string()));
                if let Some(keys) = parse_tag(inside, emotions, &mut out.warnings) {
                    changes.push((out.plain.len(), keys));
                }
                separate(&mut out.plain, after);
                rest = after;
            }
            b'*' => {
                out.plain.push_str(&rest[..i]);
                let run = rest[i..].bytes().take_while(|&b| b == b'*').count();
                let (which, used) = if run >= 2 { (1, 2) } else { (0, 1) };
                let marker = if which == 1 { "**" } else { "*" };
                let after = &rest[i + used..];
                match open[which].take() {
                    Some(start) => {
                        let end = out.plain.trim_end().len().max(start);
                        if end > start {
                            out.emphasis.push(Emphasis { start, end, level: if which == 1 { 10.0 } else { 5.0 } });
                        }
                    }
                    None => {
                        // an opening mark counts only if it is closed later (before the next tag)
                        let closes = after.split('[').next().unwrap_or("").contains(marker)
                            && (which == 1 || after.split('[').next().unwrap_or("").replace("**", "").contains('*'));
                        if closes {
                            open[which] = Some(out.plain.len() + (after.len() - after.trim_start().len()));
                        } else {
                            out.warnings.push(format!("an unclosed {marker} is ignored"));
                        }
                    }
                }
                rest = after;
            }
            _ => {
                out.plain.push_str(&rest[..i]);
                let run = rest[i..].bytes().take_while(|&b| b == b'|').count();
                let seconds = (run / 2) as f64 * 0.4 + (run % 2) as f64 * 0.15;
                out.pauses.push(Pause { at: out.plain.len(), seconds });
                let after = &rest[i + run..];
                let after = if out.plain.ends_with(char::is_whitespace) { after.trim_start_matches(' ') } else { after };
                separate(&mut out.plain, after);
                rest = after;
            }
        }
    }
    out.plain.push_str(rest);
    for (k, (at, keys)) in changes.iter().enumerate() {
        let end = changes.get(k + 1).map_or(out.plain.len(), |c| c.0);
        if end > *at || k + 1 == changes.len() {
            match out.spans.last_mut() {
                Some(last) if last.keys == *keys => last.end = end,
                _ => out.spans.push(Span { start: *at, end, keys: keys.clone() }),
            }
        }
    }
    out
}

/// One spoken word of an utterance: its token range, its byte range in the utterance's text (when dizione could
/// place it) and whether it is an inline pronunciation (`{...}`, the placeholder of a sound).
#[derive(Clone, Debug, PartialEq)]
pub struct WordPos {
    pub tokens: (usize, usize),
    pub span: Option<(usize, usize)>,
    pub inline: bool,
}

/// One utterance as dizione cut it: the text it phonemized, its tokens and stress, and its spoken words.
pub struct UtteranceWords<'a> {
    pub text: &'a str,
    pub tokens: &'a [u32],
    pub stress: &'a [u32],
    pub words: Vec<WordPos>,
}

/// The edits of one utterance: its tokens and stress (inline sounds spliced in), token spans for [`Edit`], pauses
/// `(token, seconds)` and sounds `(first token, sound)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UtterancePlan {
    pub tokens: Vec<u32>,
    pub stress: Vec<u32>,
    pub spans: Vec<TokenSpan>,
    pub pauses: Vec<(usize, f64)>,
    pub sounds: Vec<(usize, Sound)>,
}

impl UtterancePlan {
    /// True when nothing changes: the utterance is said as dizione gave it.
    pub fn is_empty(&self) -> bool {
        self.pauses.is_empty() && self.sounds.is_empty() && self.spans.iter().all(|s| s.keys.is_empty())
    }
}

/// Maps each byte of an utterance's text onto the plain text: dizione collapses spaces and may add a final full
/// stop, so the non-space characters are matched in order.
fn piece_map(plain: &str, piece: &str, cursor: &mut usize) -> Vec<usize> {
    let mut map = vec![plain.len(); piece.len() + 1];
    let chars: Vec<(usize, char)> = plain.char_indices().collect();
    let mut k = chars.partition_point(|&(b, _)| b < *cursor);
    for (b, c) in piece.char_indices() {
        if c.is_whitespace() {
            map[b] = chars.get(k).map_or(plain.len(), |&(pb, _)| pb);
            continue;
        }
        let mut j = k;
        while j < chars.len() && chars[j].1 != c {
            j += 1;
        }
        if j < chars.len() {
            map[b] = chars[j].0;
            k = j + 1;
            *cursor = chars[j].0 + c.len_utf8();
        } else {
            map[b] = plain.len();
        }
    }
    map[piece.len()] = *cursor;
    map
}

/// The gap token after word `word` (or before the first word when `None`) where a pause goes: the first
/// punctuation mark of the gap, else its `<sp>`, else `<bos>` / `<eos>`.
fn pause_token(tokens: &[u32], words: &[WordPos], word: Option<usize>) -> usize {
    let n = tokens.len();
    let Some(w) = word else { return 0 };
    let from = words[w].tokens.1;
    let to = words.get(w + 1).map_or(n.saturating_sub(1), |x| x.tokens.0);
    let gap = from..to.max(from);
    gap.clone().find(|&j| (5..17).contains(&tokens[j])).or_else(|| gap.clone().find(|&j| tokens[j] == 3)).unwrap_or(to.min(n - 1))
}

/// Maps the script's spans, emphasis, pauses and sounds onto the tokens of each utterance (in order, as dizione
/// cut `script.plain`). Each inline word takes the next sound's phones in place of its token. A word takes the keys
/// in force where it starts, plus `emph` when it overlaps an emphasis; a span starts at the first token of a word
/// whose keys differ from the word before. Problems go to `warnings`.
pub fn plan_utterances(script: &Script, utterances: &[UtteranceWords<'_>], warnings: &mut Vec<String>) -> Vec<UtterancePlan> {
    let mut cursor = 0usize;
    let mut next_sound = 0usize;
    let mut plans: Vec<UtterancePlan> = Vec::new();
    // per utterance, per word: its byte range in `plain` (after splicing: words with their new token ranges)
    let mut placed: Vec<Vec<Option<(usize, usize)>>> = Vec::new();
    let mut spliced_words: Vec<Vec<WordPos>> = Vec::new();
    for u in utterances {
        let map = piece_map(&script.plain, u.text, &mut cursor);
        let mut plan = UtterancePlan::default();
        let mut words = Vec::new();
        let mut here = Vec::new();
        let mut copied = 0usize;
        for w in &u.words {
            let mut place = w.span.filter(|&(a, b)| b > a && b <= u.text.len()).map(|(a, b)| {
                let last = u.text[..b].char_indices().next_back().map_or(a, |(i, _)| i);
                let c = u.text[last..].chars().next().map_or(1, char::len_utf8);
                (map[a], map[last].saturating_add(c).min(script.plain.len()))
            });
            let (a, b) = w.tokens;
            plan.tokens.extend_from_slice(&u.tokens[copied..a]);
            plan.stress.extend_from_slice(&u.stress[copied..a]);
            let from = plan.tokens.len();
            match (w.inline, script.sounds.get(next_sound)) {
                (true, Some((at, sound))) => {
                    next_sound += 1;
                    place = Some((*at, *at + SOUND_PLACEHOLDER.len()));
                    plan.sounds.push((from, sound.clone()));
                    plan.tokens.extend(sound.phones.iter().map(|p| p.id));
                    plan.stress.extend(sound.phones.iter().map(|p| p.stress));
                }
                _ => {
                    plan.tokens.extend_from_slice(&u.tokens[a..b]);
                    plan.stress.extend_from_slice(&u.stress[a..b]);
                }
            }
            copied = b;
            words.push(WordPos { tokens: (from, plan.tokens.len()), span: w.span, inline: w.inline });
            here.push(place);
        }
        plan.tokens.extend_from_slice(&u.tokens[copied..]);
        plan.stress.extend_from_slice(&u.stress[copied..]);
        plans.push(plan);
        placed.push(here);
        spliced_words.push(words);
    }
    if next_sound < script.sounds.len() {
        warnings.push(format!("{} inline sound(s) found no place in the utterances and are not said", script.sounds.len() - next_sound));
    }
    let mut last_start: Option<usize> = None;
    for (ui, plan) in plans.iter_mut().enumerate() {
        let n = plan.tokens.len();
        let mut spans: Vec<TokenSpan> = Vec::new();
        for (wi, w) in spliced_words[ui].iter().enumerate() {
            let place = placed[ui][wi];
            if let Some((a, _)) = place {
                last_start = Some(a);
            }
            let at = place.map(|p| p.0).or(last_start).unwrap_or(0);
            let mut keys: Vec<(String, f64)> = script.keys_at(at).to_vec();
            if let Some((a, b)) = place
                && let Some(level) = script.emphasis.iter().filter(|e| a < e.end && b > e.start).map(|e| e.level).reduce(f64::max)
            {
                keys.push(("emph".into(), level));
            }
            match spans.last_mut() {
                None => spans.push(TokenSpan { from: 0, to: n, keys }),
                Some(last) if last.keys == keys => {}
                Some(last) => {
                    last.to = w.tokens.0;
                    spans.push(TokenSpan { from: w.tokens.0, to: n, keys });
                }
            }
        }
        if spans.is_empty() && n > 0 {
            spans.push(TokenSpan { from: 0, to: n, keys: script.keys_at(last_start.unwrap_or(0)).to_vec() });
        }
        plan.spans = spans;
    }
    // pauses: after the last word that starts before the bar
    for pause in &script.pauses {
        let mut best: Option<(usize, Option<usize>)> = None;
        for (ui, words) in placed.iter().enumerate() {
            for (wi, p) in words.iter().enumerate() {
                if p.is_some_and(|(a, _)| a < pause.at) {
                    best = Some((ui, Some(wi)));
                }
            }
        }
        if let Some((ui, w)) = best.or_else(|| (!plans.is_empty()).then_some((0, None))) {
            let j = pause_token(&plans[ui].tokens, &spliced_words[ui], w);
            plans[ui].pauses.push((j, pause.seconds));
        }
    }
    plans
}

/// What [`synthesize_edited`] rendered: the audio and the edited performance.
pub struct Rendered {
    pub audio: Vec<f32>,
    pub durations: Vec<u32>,
    pub curves: Curves,
    /// `cep_delta [T x 112]`, row-major (empty when nothing touches the head).
    pub cep_delta: Vec<f64>,
}

/// Frame starts of `durations` (one more than tokens).
fn frame_starts(durations: &[u32]) -> Vec<usize> {
    let mut start = vec![0usize; durations.len() + 1];
    for (j, &d) in durations.iter().enumerate() {
        start[j + 1] = start[j] + d as usize;
    }
    start
}

/// One utterance (`plan.tokens`) through tiny-voca with the edits of `plan` between the stages. The voice's own
/// `post` edits apply as in `Model::synthesize`: `rate_x` on the predicted durations, `f0_range_x` on the pitch, the
/// envelope edits on the cepstra, the oscillator's shift and jitter. Inline sounds: their phones' lengths
/// (stretch or seconds) after the durations stage; an event's voicing, pitch contour (relative to the voiced pitch
/// within 40 frames around it), energy and breath after the curves stage and on the head.
pub fn synthesize_edited(model: &tinyvoca::Model, voice: &tinyvoca::Voice, plan: &UtterancePlan, emotions: &Emotions, seed: u32) -> Result<Rendered, String> {
    let (tokens, stress) = (&plan.tokens, &plan.stress);
    let post = &voice.post;
    let speed = post.rate_x;
    if !(0.25..=4.0).contains(&speed) || !(0.0..=3.0).contains(&post.f0_range_x) {
        return Err("rate_x must be 0.25 to 4 and f0_range_x 0 to 3".into());
    }
    let (rate, hop) = (model.config.sample_rate as f64, model.config.hop as f64);
    let enc = model.encode(tokens, stress, voice)?;
    let mut durations = if speed == 1.0 { enc.durations.clone() } else { model.durations(tokens, &enc.duration_raw, speed) };
    let edit = Edit::new(tokens.len(), &plan.spans, emotions, rate, hop)?;
    if let Some(e) = &edit {
        durations = e.durations(tokens, &durations);
    }
    for (from, sound) in &plan.sounds {
        for (i, p) in sound.phones.iter().enumerate() {
            if let Some(d) = durations.get_mut(from + i) {
                *d = match p.seconds {
                    Some(s) => (s * rate / hop).round().max(1.0) as u32,
                    None => ((*d as f64) * p.stretch).round().max(1.0) as u32,
                };
            }
        }
    }
    for &(j, seconds) in &plan.pauses {
        if let Some(d) = durations.get_mut(j) {
            *d += (seconds * rate / hop).round().max(0.0) as u32;
        }
    }
    let frames = model.expand(&enc.hidden, &durations, voice)?;
    let prosody = model.prosody(&frames, tokens);
    let mut log_f0 = prosody.log_f0.clone();
    tinyvoca::widen_pitch(&mut log_f0, &prosody.voiced, post.f0_range_x);
    let mut curves = Curves {
        log_f0: log_f0.iter().map(|&x| x as f64).collect(),
        voiced: prosody.voiced.iter().map(|&v| if v > 0.0 { 1.0 } else { 0.0 }).collect(),
        energy: prosody.energy.iter().map(|&x| x as f64).collect(),
    };
    if let Some(e) = &edit {
        e.curves(tokens, stress, &durations, &mut curves)?;
    }
    let start = frame_starts(&durations);
    let frames_total = start[durations.len()];
    for &(j, _) in &plan.pauses {
        if j < durations.len() {
            for i in start[j]..start[j + 1] {
                curves.voiced[i] = 0.0;
                curves.energy[i] = curves.energy[i].min(emotion::PAUSE_SILENCE);
            }
        }
    }
    let mut cep_delta = edit.as_ref().map(|e| e.cep(&durations, &curves.log_f0)).unwrap_or_default();
    let db2ln = std::f64::consts::LN_10 / 20.0;
    for (from, sound) in &plan.sounds {
        let to = (from + sound.phones.len()).min(durations.len());
        let (a, b) = (start[*from], start[to]);
        for (i, p) in sound.phones.iter().enumerate() {
            if let Some(v) = p.voiced
                && from + i < durations.len()
            {
                for f in start[from + i]..start[from + i + 1] {
                    curves.voiced[f] = if v { 1.0 } else { 0.0 };
                }
            }
        }
        let Some(shape) = &sound.shape else { continue };
        if b <= a {
            continue;
        }
        let around: Vec<f64> = (a.saturating_sub(40)..(b + 40).min(frames_total))
            .filter(|&f| (f < a || f >= b) && curves.voiced[f] > 0.5)
            .map(|f| curves.log_f0[f])
            .collect();
        let everywhere: Vec<f64> = (0..frames_total).filter(|&f| (f < a || f >= b) && curves.voiced[f] > 0.5).map(|f| curves.log_f0[f]).collect();
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let base = if !around.is_empty() {
            mean(&around)
        } else if !everywhere.is_empty() {
            mean(&everywhere)
        } else {
            mean(&curves.log_f0[a..b])
        };
        let points = &shape.f0_st;
        for f in a..b {
            let u = (f - a) as f64 + 0.5;
            let st = match points.len() {
                0 => 0.0,
                1 => points[0],
                n => {
                    let x = u / (b - a) as f64 * (n - 1) as f64;
                    let k = (x.floor() as usize).min(n - 2);
                    points[k] + (points[k + 1] - points[k]) * (x - k as f64).clamp(0.0, 1.0)
                }
            };
            if !points.is_empty() {
                curves.log_f0[f] = base + st * emotion::semitone();
            }
            curves.energy[f] += emotion::energy_per_db() * shape.energy_db;
        }
        if shape.energy_db != 0.0 || shape.breath_db != 0.0 {
            if cep_delta.is_empty() {
                cep_delta = vec![0.0; frames_total * emotion::CEP_WIDTH];
            }
            for f in a..b {
                let row = &mut cep_delta[f * emotion::CEP_WIDTH..(f + 1) * emotion::CEP_WIDTH];
                row[0] += shape.energy_db * db2ln;
                row[emotion::CEP_CAUSAL] += (shape.energy_db + shape.breath_db) * db2ln;
            }
        }
    }
    let lf: Vec<f32> = curves.log_f0.iter().map(|&x| x as f32).collect();
    let vo: Vec<f32> = curves.voiced.iter().map(|&x| x as f32).collect();
    let en: Vec<f32> = curves.energy.iter().map(|&x| x as f32).collect();
    let mut cepstra = model.decode(&prosody.hidden, &lf, &vo, &en)?;
    let pitch = tinyvoca::edit::oscillator_pitch(&lf, post, 0.0, seed);
    model.edit_envelope(&mut cepstra, &pitch, post);
    let (hw, nw) = model.head_widths();
    let render = |delta: &[f64]| -> Result<Vec<f32>, String> {
        if delta.is_empty() {
            model.vocode(&cepstra, &pitch, None, None, seed)
        } else {
            model.vocode(&cepstra, &pitch, Some(&emotion::head_delta(delta, hw, nw)), None, seed)
        }
    };
    let mut audio = render(&cep_delta)?;
    let lifts = event_lifts(plan, &start, &curves, &audio, model.config.hop);
    if !lifts.is_empty() {
        if cep_delta.is_empty() {
            cep_delta = vec![0.0; frames_total * emotion::CEP_WIDTH];
        }
        for (a, b, db) in lifts {
            for f in a..b {
                cep_delta[f * emotion::CEP_WIDTH] += db * db2ln;
            }
        }
        audio = render(&cep_delta)?;
    }
    Ok(Rendered { audio, durations, curves, cep_delta })
}

/// The decoder renders a held event phone (the 0.45 s hum of `{mmh}`) far quieter than speech (measured: -51 dB
/// against -20 dB for an /n/ inside a word): a phone that long is outside what it learned. So each forced-voiced
/// phone of a named event is measured in the first render and lifted, in dB on the head's harmonic c0, to the
/// level of the utterance's other voiced frames plus the event's `energy_db`: `(first frame, end, dB)`. Never
/// lowered, at most +30 dB. (A level read from the cepstra instead, c0 or the envelope's peak, missed the rendered
/// level by up to 30 dB on nasals, through the head's clamp.)
fn event_lifts(plan: &UtterancePlan, start: &[usize], curves: &Curves, audio: &[f32], hop: usize) -> Vec<(usize, usize, f64)> {
    let frames = start[start.len() - 1].min(audio.len() / hop.max(1));
    let power = |a: usize, b: usize| -> f64 {
        let s = &audio[a * hop..b * hop];
        s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / s.len().max(1) as f64
    };
    let mut inside = vec![false; frames];
    let mut wanted: Vec<(usize, usize, f64)> = Vec::new();
    for (from, sound) in &plan.sounds {
        let Some(shape) = &sound.shape else { continue };
        for (i, p) in sound.phones.iter().enumerate() {
            let j = from + i;
            if j + 1 < start.len() && start[j + 1] <= frames {
                inside[start[j]..start[j + 1]].fill(true);
                if p.voiced == Some(true) && start[j + 1] > start[j] {
                    wanted.push((start[j], start[j + 1], shape.energy_db));
                }
            }
        }
    }
    let others: Vec<usize> = (0..frames).filter(|&f| !inside[f] && curves.voiced[f] > 0.5).collect();
    if wanted.is_empty() || others.is_empty() {
        return Vec::new();
    }
    let reference = 10.0 * libm::log10(others.iter().map(|&f| power(f, f + 1)).sum::<f64>() / others.len() as f64 + 1e-20);
    wanted
        .into_iter()
        .filter_map(|(a, b, energy_db)| {
            let measured = 10.0 * libm::log10(power(a, b) + 1e-20);
            let lift = (reference + energy_db - measured).min(30.0);
            (lift > 0.0).then_some((a, b, lift))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's line parses into 4 segments with canonical keys; aliases, case, bare numbers, overrides,
    /// unknown keys, emphasis, pauses and inline sounds; mapping and splicing onto made-up utterance tokens.
    #[test]
    fn parses_owner_line_tags_emphasis_pauses_and_sounds() {
        let e = Emotions::parse(emotion::BUILTIN_JSON).unwrap();
        let ev = Events::parse(sounds::BUILTIN_JSON).unwrap();
        let s = parse("[calm] Hello. [scared] What do you want ? [suspicious:3] You need something ? [angry:10] Leave me Alone!", &e, &ev);
        let seg = s.segments();
        assert_eq!(seg.len(), 4);
        assert_eq!(seg[0], ("Hello.".to_string(), vec![("calm".to_string(), 5.0)]));
        assert_eq!(seg[2].1, vec![("suspicious".to_string(), 3.0)]);
        assert_eq!(seg[3], ("Leave me Alone!".to_string(), vec![("angry".to_string(), 10.0)]));
        assert!(s.warnings.is_empty());

        let text = "[Fear:4, ASK:10, rate:1.1, pitch:+2, foo:3] What *do* you **want**? {mmm h:0.2s} || [laughs]Go | now. [] Bye [suspicious,2]x {Sighs}{qq}";
        let s = parse(text, &e, &ev);
        assert_eq!(s.spans[0].keys, vec![("scared".into(), 4.0), ("question".into(), 10.0), ("rate".into(), 1.1), ("pitch".into(), 2.0)]);
        assert!(s.plain.starts_with(" What do you want? {ə} ") && s.plain.contains("Go now.") && s.plain.trim_end().ends_with("Bye x {ə}"), "{:?}", s.plain);
        assert_eq!(&s.plain[s.emphasis[0].start..s.emphasis[0].end], "do");
        assert_eq!((&s.plain[s.emphasis[1].start..s.emphasis[1].end], s.emphasis[1].level), ("want", 10.0));
        assert_eq!(s.pauses.iter().map(|p| p.seconds).collect::<Vec<_>>(), [0.4, 0.15]);
        assert_eq!(s.spans.len(), 3); // [laughs] changes nothing, [] resets, [suspicious,2]
        assert!(s.spans[1].keys.is_empty() && s.spans[2].keys == [("suspicious".to_string(), 2.0)]);
        let m = &s.sounds[0].1.phones;
        assert_eq!((m.len(), m[0].id, m[0].stretch, m[1].id, m[1].seconds), (2, 32, 3.0, 31, Some(0.2)));
        assert_eq!(s.sounds[1].1.event.as_deref(), Some("sigh"));
        assert_eq!(s.warnings.len(), 4, "{:?}", s.warnings); // foo, laughs, the bare 2, {qq}

        // "What do you want? {ə}": <bos> w ʌ t <sp> d u <sp> j u <sp> w ɑ n t ? ə <eos>
        let toks = [1, 40, 52, 19, 3, 20, 57, 3, 37, 57, 3, 40, 53, 33, 19, 7, 43, 2];
        let stress = [0u32; 18];
        let w = |t: (usize, usize), span: Option<(usize, usize)>, inline: bool| WordPos { tokens: t, span, inline };
        let words = vec![
            w((1, 4), Some((0, 4)), false),
            w((5, 7), Some((5, 7)), false),
            w((8, 10), Some((8, 11)), false),
            w((11, 15), Some((12, 16)), false),
            w((16, 17), None, true),
        ];
        let u = UtteranceWords { text: "What do you want? {ə}", tokens: &toks, stress: &stress, words };
        let mut warnings = Vec::new();
        let plans = plan_utterances(&s, &[u], &mut warnings);
        let p = &plans[0];
        assert_eq!(&p.tokens[15..], [7, 32, 31, 2]);
        assert_eq!(p.sounds[0].0, 16);
        assert_eq!(p.spans.iter().map(|x| x.from).collect::<Vec<_>>(), [0, 5, 8, 11, 16]);
        assert_eq!(p.spans[1].keys.last(), Some(&("emph".to_string(), 5.0)));
        assert_eq!(p.spans[3].keys.last(), Some(&("emph".to_string(), 10.0)));
        assert_eq!(p.pauses, vec![(18, 0.4), (18, 0.15)]); // "Go now." is not in this utterance: after the sound
        assert_eq!(warnings.len(), 1); // {Sighs} has no utterance here
    }

    /// Parity with the reference apply.py on its golden pairs (HAA_EMO_GOLDEN = the golden/ folder of
    /// /root/work/haa/temp/npc-voice/emo): durations exact, curves and cep_delta within 1e-4.
    #[test]
    #[ignore = "needs HAA_EMO_GOLDEN (agent B's golden pairs, not committed)"]
    fn golden_parity_with_apply_py() {
        use serde_json::Value;
        let dir = std::env::var("HAA_EMO_GOLDEN").expect("HAA_EMO_GOLDEN");
        let read = |name: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(format!("{dir}/{name}")).unwrap()).unwrap() };
        let manifest = read("manifest.json");
        let base = read("emotions.json");
        let floats = |v: &Value, k: &str| -> Option<Vec<f64>> { v.get(k).map(|a| a.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()) };
        let ints = |v: &Value, k: &str| -> Vec<u32> { v[k].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect() };
        let mut worst = 0.0f64;
        for case in manifest["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mut doc = base.clone();
            if let Some(caps) = case["emotions_extra"].get("caps") {
                doc["caps"] = caps.clone();
            }
            let emotions = Emotions::from_value(&doc).unwrap();
            let pin = read(&format!("{name}.in.json"));
            let pout = read(&format!("{name}.out.json"));
            let stages: Vec<&str> = case["stages"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
            let tokens = ints(&pin, "tokens");
            let stress = ints(&pin, "stress");
            let spans: Vec<TokenSpan> = pin["spans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| TokenSpan {
                    from: s["from"].as_u64().unwrap() as usize,
                    to: s["to"].as_u64().unwrap() as usize,
                    keys: s["keys"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_f64().unwrap())).collect(),
                })
                .collect();
            let mut durations = ints(&pin, "durations");
            let mut curves = floats(&pin, "log_f0").map(|lf| Curves { log_f0: lf, voiced: floats(&pin, "voiced").unwrap(), energy: floats(&pin, "energy").unwrap() });
            let mut cep = None;
            if let Some(edit) = Edit::new(tokens.len(), &spans, &emotions, 24_000.0, 256.0).unwrap() {
                if stages.contains(&"durations") {
                    let new = edit.durations(&tokens, &durations);
                    curves = curves.map(|c| emotion::resample(&c, &durations, &new));
                    durations = new;
                }
                if let Some(c) = curves.as_mut() {
                    if stages.contains(&"curves") {
                        edit.curves(&tokens, &stress, &durations, c).unwrap();
                    }
                    if stages.contains(&"cep") {
                        cep = Some(edit.cep(&durations, &c.log_f0));
                    }
                }
            }
            assert_eq!(durations, ints(&pout, "durations"), "{name}: durations");
            let err = |a: &[f64], b: &[f64]| -> f64 {
                assert_eq!(a.len(), b.len());
                a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
            };
            let mut report = format!("{name}:");
            if let Some(c) = &curves {
                for (k, mine) in [("log_f0", &c.log_f0), ("voiced", &c.voiced), ("energy", &c.energy)] {
                    let e = err(mine, &floats(&pout, k).unwrap());
                    report += &format!(" {k} {e:.2e}");
                    worst = worst.max(e);
                }
            }
            match (&cep, pout.get("cep_delta")) {
                (Some(mine), Some(theirs)) => {
                    let flat: Vec<f64> = theirs.as_array().unwrap().iter().flat_map(|r| r.as_array().unwrap().iter().map(|x| x.as_f64().unwrap())).collect();
                    let e = err(mine, &flat);
                    report += &format!(" cep_delta {e:.2e}");
                    worst = worst.max(e);
                }
                (None, None) => {}
                _ => panic!("{name}: cep_delta present on one side only"),
            }
            println!("{report}");
        }
        assert!(worst < 1e-4, "worst {worst}");
    }
}
