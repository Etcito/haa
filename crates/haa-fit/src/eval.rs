//! Candidate render: a patch to a mono buffer of exactly the target's length, with haa-core's voice
//! (the same per-block loop as `haa_core::render::render`, voice seed 0, no input script).

use haa_core::consts::BLOCK;
use haa_core::patch::{GraphMode, Patch};
use haa_core::Voice;

/// Renders candidates at one rate and length, reusing its buffers.
pub struct Renderer {
    sample_rate: f64,
    frames: usize,
    left: Vec<f32>,
    right: Vec<f32>,
    mono: Vec<f32>,
}

impl Renderer {
    pub fn new(sample_rate: f64, frames: usize) -> Renderer {
        Renderer { sample_rate, frames, left: vec![0.0; frames], right: vec![0.0; frames], mono: vec![0.0; frames] }
    }

    /// Renders `patch` to mono ((L+R)/2); None when the patch does not validate.
    pub fn render(&mut self, patch: &Patch) -> Option<&[f32]> {
        let mut voice = Voice::new(self.sample_rate, 0);
        if !voice.load_patch(patch, false).playable {
            return None;
        }
        self.left.fill(0.0);
        self.right.fill(0.0);
        let mut frame = 0;
        while frame < self.frames {
            let len = (self.frames - frame).min(BLOCK);
            let playing = voice.process(&mut self.left[frame..frame + len], &mut self.right[frame..frame + len]);
            frame += len;
            if !playing {
                break;
            }
        }
        for i in 0..self.frames {
            self.mono[i] = 0.5 * (self.left[i] + self.right[i]);
        }
        Some(&self.mono)
    }

    /// The last rendered mono buffer.
    pub fn last(&self) -> &[f32] {
        &self.mono
    }
}

/// Fits a template to a sample of `seconds`: a one-shot timeline (no loop) is stretched to the
/// sample, so its curves span it; looping and live patches keep their own clock and are windowed.
pub fn prepare(patch: &mut Patch, seconds: f64) {
    if patch.graph.mode == GraphMode::Timeline && !patch.graph.r#loop && seconds > 0.0 {
        patch.graph.duration = seconds;
    }
}
