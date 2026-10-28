//! Offline render (ARCHITECTURE §8): a patch JSON to a stereo buffer with the shared render rules.
//! The rules, shared by `haa render`, `renderPatch` and `dist/cli.js`:
//! - `seconds` defaults to the graph duration in timeline mode and to `LIVE_DEFAULT_SECONDS` in live mode;
//! - the sample rate defaults to `OFFLINE_RATE` and the seed to 0;
//! - inputs are set before the first block;
//! - a ramp sets its input to the linear value between `a` and `b` at every block start;
//! - a trigger fires at the first block whose start is at or after `at` seconds;
//! - the result holds exactly `round(seconds × rate)` frames, silent after `ended`.

use crate::consts::{BLOCK, LIVE_DEFAULT_SECONDS, OFFLINE_RATE};
use crate::graph::Voice;
use crate::issues::{Issue, Severity};
use crate::patch::{GraphMode, InputKind, Patch, load_patch};

/// One input value given to a render: a plain number, or an enum value name.
#[derive(Clone, Debug, PartialEq)]
pub enum InputValue {
    Number(f64),
    Name(String),
}

/// The offline render settings. `Default` renders at `OFFLINE_RATE` with seed 0.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    /// Render length in seconds; `None` uses the duration or `LIVE_DEFAULT_SECONDS`.
    pub seconds: Option<f64>,
    pub sample_rate: u32,
    pub seed: u32,
    pub inputs: Vec<(String, InputValue)>,
    /// `(name, a, b)`: linear from `a` to `b` over the render, set at every block start.
    pub ramps: Vec<(String, f64, f64)>,
    /// `(name, at)`: pulse at the first block whose start is at or after `at` seconds.
    pub triggers: Vec<(String, f64)>,
}

impl Default for RenderOptions {
    fn default() -> RenderOptions {
        RenderOptions {
            seconds: None,
            sample_rate: OFFLINE_RATE,
            seed: 0,
            inputs: Vec::new(),
            ramps: Vec::new(),
            triggers: Vec::new(),
        }
    }
}

/// The rendered stereo buffer and the loader/validate issues of the patch it came from.
#[derive(Clone, Debug)]
pub struct StereoRender {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub sample_rate: u32,
    pub issues: Vec<Issue>,
}

/// Renders patch JSON with the §8 render rules. Err holds the issues of a patch that cannot play,
/// or the `unknown-input` issues of the options.
pub fn render(json: &str, o: &RenderOptions) -> Result<StereoRender, Vec<Issue>> {
    // Voice::new clamps the rate to 8000..192000 and falls back to 48000 for 0.
    let sample_rate = if o.sample_rate == 0 {
        OFFLINE_RATE
    } else {
        o.sample_rate.clamp(8_000, 192_000)
    };
    let rate = f64::from(sample_rate);
    let (patch, mut issues) = match load_patch(json) {
        Ok(loaded) => loaded,
        Err(issue) => return Err(vec![issue]),
    };
    let mut voice = Voice::new(rate, o.seed);
    let outcome = voice.load_patch(&patch, false);
    issues.extend(outcome.issues);
    if !outcome.playable {
        return Err(issues);
    }

    let mut inputs = Vec::with_capacity(o.inputs.len());
    for (name, value) in &o.inputs {
        let index = match resolve(&voice, &mut issues, name) {
            Ok(index) => index,
            Err(issues) => return Err(issues),
        };
        let value = match value {
            InputValue::Number(value) => *value,
            InputValue::Name(name_value) => match enum_index(&patch, index, name_value) {
                Some(value) => value,
                None => {
                    issues.push(unknown_input(&format!(
                        "input `{name}` has no enum value `{name_value}`"
                    )));
                    return Err(issues);
                }
            },
        };
        inputs.push((index, value));
    }
    let mut ramps = Vec::with_capacity(o.ramps.len());
    for (name, a, b) in &o.ramps {
        match resolve(&voice, &mut issues, name) {
            Ok(index) => ramps.push((index, *a, *b)),
            Err(issues) => return Err(issues),
        }
    }
    let mut triggers = Vec::with_capacity(o.triggers.len());
    for (name, at) in &o.triggers {
        match resolve(&voice, &mut issues, name) {
            Ok(index) => triggers.push((index, *at, false)),
            Err(issues) => return Err(issues),
        }
    }

    let seconds = o.seconds.unwrap_or(match patch.graph.mode {
        GraphMode::Timeline => patch.graph.duration,
        GraphMode::Live => LIVE_DEFAULT_SECONDS,
    });
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let total = (seconds * rate).round() as usize;
    let mut left = vec![0.0f32; total];
    let mut right = vec![0.0f32; total];

    // Inputs are in place before the first block.
    for &(index, value) in &inputs {
        voice.set_input(index, value);
    }
    let mut frame = 0usize;
    while frame < total {
        let start = frame as f64 / rate;
        for &(index, a, b) in &ramps {
            voice.set_input(index, a + (b - a) * (frame as f64 / total as f64));
        }
        for trigger in &mut triggers {
            let (index, at, fired) = *trigger;
            if !fired && start >= at {
                voice.trigger(index);
                *trigger = (index, at, true);
            }
        }
        let len = (total - frame).min(BLOCK);
        let playing = voice.process(
            &mut left[frame..frame + len],
            &mut right[frame..frame + len],
        );
        frame += len;
        if !playing {
            break;
        }
    }

    Ok(StereoRender {
        left,
        right,
        sample_rate,
        issues,
    })
}

/// Resolves a published or reserved input name, or returns every issue so far plus `unknown-input`.
fn resolve(voice: &Voice, issues: &mut Vec<Issue>, name: &str) -> Result<usize, Vec<Issue>> {
    match voice.input_index(name) {
        Some(index) => Ok(index),
        None => {
            issues.push(unknown_input(&format!("input `{name}` is not declared")));
            Err(core::mem::take(issues))
        }
    }
}

/// The index of an enum input value name.
fn enum_index(patch: &Patch, index: usize, value: &str) -> Option<f64> {
    let def = patch.inputs.get(index)?;
    if def.kind != InputKind::Enum {
        return None;
    }
    def.values
        .iter()
        .position(|item| item == value)
        .map(|value| value as f64)
}

fn unknown_input(message: &str) -> Issue {
    Issue {
        severity: Severity::Error,
        code: "unknown-input".to_string(),
        path: "inputs".to_string(),
        node: None,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(duration: f64) -> String {
        format!(
            r#"{{"haa":1,"id":"osc-gain","title":"Osc gain","graph":{{"duration":{duration},"loop":false,"mode":"timeline","nodes":[
              {{"id":"osc","type":"oscillator","x":0,"y":0,"label":"Osc","params":{{"frequency":220,"wave":"square","level":1}}}},
              {{"id":"amp","type":"gain","x":0,"y":0,"label":"Gain","params":{{"gain":2}}}},
              {{"id":"out","type":"output","x":0,"y":0,"label":"Out","params":{{"gain":1}}}}],
              "connections":[
                {{"id":"c1","from":"osc","to":"amp","target":"audio","amount":1}},
                {{"id":"c2","from":"amp","to":"out","target":"audio","amount":1}}]}}}}"#
        )
    }

    #[test]
    fn one_second_timeline_renders_exactly_48000_frames() {
        let render = render(&patch(1.0), &RenderOptions::default()).unwrap();
        assert_eq!(render.sample_rate, OFFLINE_RATE);
        assert_eq!(render.left.len(), 48_000);
        assert_eq!(render.right.len(), 48_000);
        assert!(render.left.iter().chain(&render.right).any(|&x| x != 0.0));
    }

    #[test]
    fn seconds_option_overrides_the_patch_duration() {
        let options = RenderOptions {
            seconds: Some(0.5),
            ..RenderOptions::default()
        };
        let render = render(&patch(1.0), &options).unwrap();
        assert_eq!(render.left.len(), 24_000);
        assert_eq!(render.right.len(), 24_000);
        assert!(render.left.iter().chain(&render.right).any(|&x| x != 0.0));
    }

    #[test]
    fn unknown_input_name_gives_an_unknown_input_issue() {
        let options = RenderOptions {
            inputs: vec![("nope".to_string(), InputValue::Number(1.0))],
            ..RenderOptions::default()
        };
        let issues = render(&patch(1.0), &options).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "unknown-input" && issue.path == "inputs"),
            "{issues:?}"
        );
    }
}
