//! `crackle` source node: a stochastic cloud of micro-impacts, the texture under crunching snow,
//! gravel, debris, glass shards, rain and frying. HAA-only node (no Sona counterpart).
//!
//! Events arrive as a Poisson process (`regularity` 1 makes them periodic) whose rate is `rate` times
//! a burst envelope: a linear rise over `rise` seconds, then an exponential fall with time constant
//! `fall` (0 keeps the rate constant, for a continuous texture). The envelope starts after `delay`.
//! `clump` makes the events come in avalanches, as fractures do: the rate is multiplied by a
//! log-normal random signal (~25 Hz) of the same mean, so the bursts are uneven, never a steady hiss.
//!
//! Each event draws a size from a truncated power law (Gutenberg-Richter, as fracture and
//! fragmentation events are distributed): its level spans `range` dB below the loudest, and
//! `exponent` sets how much more frequent the small events are. A bigger event is lower in pitch
//! (`sizePitch`: 1 maps the level range onto two octaves); on top of that each event takes a random
//! pitch within `spread` octaves around `frequency`. What one event sounds like is `kind`:
//! - `click`: a contact pulse of width `contact` through a broad resonance (Q ≈ 1.5) at the event
//!   pitch: a dry tick, the micro-fracture of snow, a grit grain under a sole. The pulse is
//!   differentiated first (scaled by 1/ω), so a click carries no DC and thousands of them make no
//!   rumble.
//! - `ring`: the same pulse striking a small free object: `modes` inharmonic modes (shell ratios
//!   1, 2.32, 4.25, 6.63, each jittered ±20 % per event, so no two pieces share an interval
//!   pattern) ringing for `decay` (T60, shorter for the upper modes): a glass shard, a pebble, a coin.
//! - `bubble`: a drop entraining an air bubble (Minnaert resonance), a sine whose pitch rises by a
//!   third over its life and dies in `decay`, plus the drop's own small click: rain on water.
//! - `crack`: a burst of noise decaying in `decay`, band-limited around the event pitch: a fracture,
//!   a splash, a spark of friction.
//!
//! The pulse has unit area, so `contact` changes the brightness (a pulse is flat up to about
//! 1/contact) and not the level. The random stream is NOT replayed by a reset: every restart of a
//! one-shot draws new events, so two triggers of the same preset never sound identical (the
//! sequence of restarts after a load is still deterministic). Output is mono (left = right): the
//! runtime places the voice in 3D.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use crate::rng::Rng;
use core::f64::consts::{PI, TAU};

const KIND: usize = 0;
const RATE: usize = 1;
const RISE: usize = 2;
const FALL: usize = 3;
const FREQUENCY: usize = 4;
const SPREAD: usize = 5;
const SIZE_PITCH: usize = 6;
const DECAY: usize = 7;
const CONTACT: usize = 8;
const RANGE: usize = 9;
const EXPONENT: usize = 10;
const MODES: usize = 11;
const REGULARITY: usize = 12;
const CLUMP: usize = 13;
const DELAY: usize = 14;
const LEVEL: usize = 15;
/// Cutoff (Hz) of the random rate modulation behind `clump`.
const CLUMP_HZ: f64 = 25.0;

const KIND_RING: usize = 1;
const KIND_BUBBLE: usize = 2;
const KIND_CRACK: usize = 3;

/// Most events sounding at once; a new event takes the slot of the quietest one when all are busy.
const EVENTS: usize = 96;
/// Most modes per `ring` event.
const MAX_MODES: usize = 4;
/// Small free object (cup / shell) mode ratios and relative gains.
const RING_RATIOS: [f64; MAX_MODES] = [1.0, 2.32, 4.25, 6.63];
const RING_GAINS: [f64; MAX_MODES] = [1.0, 0.7, 0.45, 0.3];
/// An event is dropped once its envelope bound falls under this.
const SILENT: f64 = 1e-5;

const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    scale: Scale,
    unit: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number { min, max, default, step, scale },
        unit,
        label_en,
        label_fr,
        modulatable: true,
        smooth: true,
    }
}

/// The `crackle` node type.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "crackle",
    category: Category::Source,
    label_en: "Crackle (micro-impacts)",
    label_fr: "Crépitement (micro-impacts)",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    params: &[
        ParamDef {
            key: "kind",
            kind: ParamKind::Enum { values: &["click", "ring", "bubble", "crack"], default: 0 },
            unit: "",
            label_en: "Event",
            label_fr: "Événement",
            modulatable: false,
            smooth: false,
        },
        number("rate", 0.0, 20_000.0, 200.0, 0.1, Scale::Lin, "Hz", "Events per second", "Événements par seconde"),
        number("rise", 0.0, 2.0, 0.01, 0.001, Scale::Lin, "s", "Burst rise", "Montée de la salve"),
        number("fall", 0.0, 10.0, 0.15, 0.001, Scale::Lin, "s", "Burst fall (0 = steady)", "Chute de la salve (0 = continu)"),
        number("frequency", 40.0, 16_000.0, 2_000.0, 1.0, Scale::Log, "Hz", "Pitch", "Hauteur"),
        number("spread", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "oct", "Pitch spread", "Dispersion de hauteur"),
        number("sizePitch", 0.0, 1.0, 0.5, 0.01, Scale::Lin, "", "Bigger is lower", "Plus gros, plus grave"),
        number("decay", 0.0005, 2.0, 0.03, 0.0001, Scale::Log, "s", "Event decay", "Décroissance d'un événement"),
        number("contact", 0.00002, 0.005, 0.0002, 0.00001, Scale::Log, "s", "Contact time", "Durée de contact"),
        number("range", 0.0, 60.0, 30.0, 0.1, Scale::Lin, "dB", "Level range", "Écart de niveaux"),
        number("exponent", 0.3, 3.0, 1.0, 0.01, Scale::Lin, "", "Small events (power law)", "Petits événements (loi de puissance)"),
        ParamDef {
            key: "modes",
            kind: ParamKind::Number { min: 1.0, max: MAX_MODES as f64, default: 2.0, step: 1.0, scale: Scale::Lin },
            unit: "",
            label_en: "Modes per ring",
            label_fr: "Modes par tintement",
            modulatable: false,
            smooth: false,
        },
        number("regularity", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Regularity", "Régularité"),
        number("clump", 0.0, 1.0, 0.0, 0.01, Scale::Lin, "", "Clumping (avalanches)", "Grappes (avalanches)"),
        number("delay", 0.0, 2.0, 0.006, 0.001, Scale::Lin, "s", "Start delay", "Délai initial"),
        number("level", 0.0, 4.0, 1.0, 0.01, Scale::Lin, "", "Level", "Niveau"),
    ],
    create,
};

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Crackle::new(init.sample_rate, init.seed))
}

/// One sounding event: up to four damped complex oscillators (ring, click, crack) or a chirp (bubble).
#[derive(Clone, Copy, Default)]
struct Event {
    active: bool,
    kind: usize,
    /// Oscillator states (re, im) and rotations (r·cos ω, r·sin ω), per mode.
    re: [f64; MAX_MODES],
    im: [f64; MAX_MODES],
    cr: [f64; MAX_MODES],
    ci: [f64; MAX_MODES],
    gain: [f64; MAX_MODES],
    modes: usize,
    /// Excitation: samples of pulse left, pulse length, position in it.
    pulse_left: u32,
    pulse_len: u32,
    /// Envelope bound (for retiring the event) and its per-sample factor.
    bound: f64,
    bound_k: f64,
    /// bubble: phase, phase step, step growth per sample, amplitude, amplitude factor.
    phase: f64,
    step: f64,
    step_k: f64,
    amp: f64,
    amp_k: f64,
    /// crack: noise envelope and its factor.
    noise: f64,
    noise_k: f64,
    /// click: the pulse is differentiated (scaled by 1/ω) so a click carries no DC; the previous pulse sample.
    diff: f64,
    prev: f64,
}

struct Crackle {
    sample_rate: f64,
    seed: u32,
    rng: Rng,
    events: Box<[Event; EVENTS]>,
    /// Samples since the reset.
    frame: u64,
    /// Integrated rate since the last event, and the threshold that fires the next one.
    accumulated: f64,
    threshold: f64,
    /// Low-passed noise driving `clump`, its pole.
    wander: f64,
    wander_a: f64,
    /// Events started since creation.
    spawned: u64,
}

impl Crackle {
    fn new(sample_rate: f64, seed: u32) -> Self {
        let mut rng = Rng::new(seed ^ 0x9e37_79b9);
        let threshold = -(1.0 - rng.next_f64()).max(1e-12).ln();
        let sample_rate = if sample_rate.is_finite() && sample_rate > 1.0 { sample_rate } else { 48_000.0 };
        Crackle {
            wander: 0.0,
            spawned: 0,
            wander_a: (-TAU * CLUMP_HZ / sample_rate).exp(),
            sample_rate,
            seed,
            rng,
            events: Box::new([Event::default(); EVENTS]),
            frame: 0,
            accumulated: 0.0,
            threshold,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.rng.next_f64()
    }

    fn next_threshold(&mut self, regularity: f64) -> f64 {
        let exp = -(1.0 - self.uniform()).max(1e-12).ln();
        regularity + (1.0 - regularity) * exp
    }

    /// Starts one event with the params of this sample.
    fn spawn(&mut self, p: &[f64]) {
        let sr = self.sample_rate;
        let kind = (p[KIND].round().max(0.0) as usize).min(KIND_CRACK);
        let range = p[RANGE].clamp(0.0, 60.0);
        let beta = p[EXPONENT].clamp(0.3, 3.0);
        // Truncated power law on [a_min, 1]: P(A > a) ∝ a^-beta.
        let a_min = 10f64.powf(-range / 20.0);
        let u = self.uniform();
        let amp = if range <= 0.0 {
            1.0
        } else {
            let lo = a_min.powf(-beta);
            let hi = 1.0;
            // Inverse CDF of the density ∝ a^-(beta+1) between a_min and 1.
            (lo - u * (lo - hi)).powf(-1.0 / beta).clamp(a_min, 1.0)
        };
        // Bigger (louder) is lower: the level range maps onto ±1 octave at sizePitch 1.
        let size_oct = if range > 0.0 { p[SIZE_PITCH].clamp(0.0, 1.0) * (2.0 * (20.0 * amp.log10() / -range) - 1.0) } else { 0.0 };
        let spread = p[SPREAD].clamp(0.0, 4.0) * (self.uniform() - 0.5);
        let hz = (p[FREQUENCY].clamp(40.0, 16_000.0) * 2f64.powf(spread + size_oct)).clamp(20.0, 0.45 * sr);
        let decay = p[DECAY].clamp(0.0005, 2.0) * (0.7 + 0.6 * self.uniform());
        let contact = p[CONTACT].clamp(0.00002, 0.005) * (0.75 + 0.5 * self.uniform());
        let modes = (p[MODES].round() as usize).clamp(1, MAX_MODES);
        // Slot: a free one, else the quietest.
        let mut slot = 0;
        let mut quietest = f64::INFINITY;
        for (i, e) in self.events.iter().enumerate() {
            if !e.active {
                slot = i;
                break;
            }
            if e.bound < quietest {
                quietest = e.bound;
                slot = i;
            }
        }
        let mut e = Event { active: true, kind, ..Event::default() };
        let pulse_len = ((contact * sr).round() as u32).max(1);
        e.pulse_len = pulse_len;
        e.pulse_left = pulse_len;
        let set_mode = |e: &mut Event, k: usize, f: f64, t60: f64, gain: f64| {
            let r = (-6.907_755 / (t60.max(0.0002) * sr)).exp();
            let w = TAU * f.min(0.45 * sr) / sr;
            e.cr[k] = r * w.cos();
            e.ci[k] = r * w.sin();
            e.gain[k] = gain;
            e.re[k] = 0.0;
            e.im[k] = 0.0;
        };
        match kind {
            KIND_RING => {
                e.modes = modes;
                let mut norm = 0.0;
                for k in 0..modes {
                    let ratio = RING_RATIOS[k] * (1.0 + 0.4 * (self.uniform() - 0.5) * if k == 0 { 0.0 } else { 1.0 });
                    let g = RING_GAINS[k] * (0.4 + 0.6 * self.uniform());
                    norm += g;
                    let t60 = decay * ratio.powf(-0.6);
                    set_mode(&mut e, k, hz * ratio, t60, g);
                }
                for k in 0..modes {
                    e.gain[k] *= amp / norm.max(1e-9);
                }
                e.bound = amp;
                e.bound_k = (-6.907_755 / (decay * sr)).exp();
            }
            KIND_BUBBLE => {
                e.modes = 1;
                // The drop's click: a broad resonance an octave above the bubble.
                set_mode(&mut e, 0, (2.0 * hz).min(0.45 * sr), 0.002, 0.3 * amp);
                e.phase = 0.0;
                e.step = TAU * hz / sr;
                let life = (decay * sr).max(1.0);
                e.step_k = (1.26f64).powf(1.0 / life); // up a major third over the T60
                e.amp = amp;
                e.amp_k = (-6.907_755 / life).exp();
                e.bound = amp;
                e.bound_k = e.amp_k;
            }
            KIND_CRACK => {
                e.modes = 1;
                // Band around the event pitch, Q ≈ 1.2: bandwidth hz / 1.2, so a pole radius from it.
                let bw = hz / 1.2;
                let r = (-PI * bw / sr).exp();
                let w = TAU * hz / sr;
                e.cr[0] = r * w.cos();
                e.ci[0] = r * w.sin();
                e.gain[0] = amp * (PI * bw / sr).sqrt() * 2.0;
                e.noise = 1.0;
                e.noise_k = (-6.907_755 / (decay * sr)).exp();
                e.pulse_left = 0;
                e.bound = amp;
                e.bound_k = e.noise_k;
            }
            _ => {
                // click (0): a broad resonance (Q ≈ 1.5) rung by the pulse.
                e.modes = 1;
                let t60 = 1.5 * 6.907_755 / (PI * hz);
                set_mode(&mut e, 0, hz, t60, amp * 1.6);
                e.diff = TAU * hz / sr;
                e.bound = amp;
                e.bound_k = (-6.907_755 / (t60 * sr).max(1.0)).exp();
            }
        }
        self.events[slot] = e;
        self.spawned += 1;
    }

    /// The burst envelope at `t` seconds after the delay.
    fn envelope(t: f64, rise: f64, fall: f64) -> f64 {
        if t < 0.0 {
            return 0.0;
        }
        let up = if rise > 0.0 { (t / rise).min(1.0) } else { 1.0 };
        let down = if fall > 0.0 { (-(t - rise).max(0.0) / fall).exp() } else { 1.0 };
        up * down
    }
}

impl Node for Crackle {
    fn process_sample(&mut self, _ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        let sr = self.sample_rate;
        let t = self.frame as f64 / sr - p[DELAY].clamp(0.0, 2.0);
        self.frame += 1;
        let mut rate = p[RATE].clamp(0.0, 20_000.0) * Self::envelope(t, p[RISE].clamp(0.0, 2.0), p[FALL].clamp(0.0, 10.0));
        let clump = p[CLUMP].clamp(0.0, 1.0);
        if clump > 0.0 {
            // Avalanches: the rate follows exp(2.5 · clump · g), g a unit-variance ~25 Hz random signal.
            let a = self.wander_a;
            self.wander = a * self.wander + (1.0 - a) * (self.rng.next_f64() * 2.0 - 1.0);
            let g = self.wander / ((1.0 - a) / (3.0 * (1.0 + a))).sqrt();
            rate *= (2.5 * clump * g - 3.125 * clump * clump).exp();
        }
        if rate > 0.0 {
            self.accumulated += rate / sr;
            // At most a few events per sample, even at extreme rates.
            let mut fired = 0;
            while self.accumulated >= self.threshold && fired < 4 {
                self.accumulated -= self.threshold;
                self.threshold = self.next_threshold(p[REGULARITY].clamp(0.0, 1.0));
                self.spawn(p);
                fired += 1;
            }
            if fired == 4 {
                self.accumulated = 0.0;
            }
        }
        let mut out = 0.0;
        for i in 0..EVENTS {
            if !self.events[i].active {
                continue;
            }
            let noise = if self.events[i].kind == KIND_CRACK { self.rng.next_f64() * 2.0 - 1.0 } else { 0.0 };
            let e = &mut self.events[i];
            // Hann pulse of unit area spread over pulse_len samples.
            let x = if e.pulse_left > 0 {
                let n = (e.pulse_len - e.pulse_left) as f64 + 0.5;
                e.pulse_left -= 1;
                let len = e.pulse_len as f64;
                (1.0 - (TAU * n / len).cos()) / len
            } else {
                0.0
            };
            let x = if e.diff > 0.0 {
                let d = (x - e.prev) / e.diff;
                e.prev = x;
                d
            } else {
                x
            };
            let x = if e.kind == KIND_CRACK {
                let v = noise * e.noise;
                e.noise *= e.noise_k;
                v
            } else {
                x
            };
            let mut s = 0.0;
            for k in 0..e.modes {
                let re = e.cr[k] * e.re[k] - e.ci[k] * e.im[k] + x;
                let im = e.ci[k] * e.re[k] + e.cr[k] * e.im[k];
                e.re[k] = re;
                e.im[k] = im;
                s += im * e.gain[k];
            }
            if e.kind == KIND_BUBBLE {
                s += e.amp * e.phase.sin();
                e.phase += e.step;
                if e.phase > TAU {
                    e.phase -= TAU;
                }
                e.step *= e.step_k;
                e.amp *= e.amp_k;
            }
            out += s;
            e.bound *= e.bound_k;
            if e.pulse_left == 0 && e.prev == 0.0 && e.bound < SILENT {
                e.active = false;
            }
        }
        let out = out * p[LEVEL].clamp(0.0, 4.0);
        let out = if out.is_finite() { out } else { 0.0 };
        NodeOut { l: out, r: out, value: 0.0 }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        if (init.sample_rate - self.sample_rate).abs() > 0.5 || init.seed != self.seed {
            *self = Crackle::new(init.sample_rate, init.seed);
        }
    }

    /// Clears the sounding events and the clock; the random stream carries on (a new draw per trigger).
    fn reset(&mut self) {
        for e in self.events.iter_mut() {
            e.active = false;
        }
        self.frame = 0;
        self.accumulated = 0.0;
        self.threshold = self.next_threshold(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::GraphMode;

    const SR: f64 = 44_100.0;

    fn ctx() -> SampleCtx<'static> {
        SampleCtx {
            sample_rate: SR,
            dt: 1.0 / SR,
            t: 0.0,
            seconds: 0.0,
            duration: 1.0,
            mode: GraphMode::Timeline,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        }
    }

    /// kind, rate, rise, fall, frequency, spread, sizePitch, decay, contact, range, exponent, modes, regularity,
    /// clump, delay, level
    fn params(kind: f64, rate: f64) -> [f64; 16] {
        [kind, rate, 0.0, 0.0, 2_000.0, 1.0, 0.5, 0.01, 0.0002, 30.0, 1.0, 2.0, 0.0, 0.0, 0.0, 1.0]
    }

    fn render(node: &mut dyn Node, p: &[f64], frames: usize) -> Vec<f64> {
        let c = ctx();
        (0..frames).map(|_| node.process_sample(&c, (0.0, 0.0), p).l).collect()
    }

    #[test]
    fn events_follow_the_rate_are_silent_at_zero_and_differ_on_each_restart() {
        let p0 = params(1.0, 0.0);
        let mut node = (DEF.create)(&NodeInit {
            id: "crackle1", seed: 7, sample_rate: SR, params: &p0, texts: &[], curve: None, inputs: &[], duration: 1.0,
        });
        // Rate 0: nothing at all.
        assert!(render(node.as_mut(), &p0, 4410).iter().all(|v| *v == 0.0));
        // Rate 25/s over 8 s: about 200 events (Poisson, so within ±25 %).
        let p = params(0.0, 25.0);
        let mut counted = Crackle::new(SR, 7);
        let out = render(&mut counted, &p, (8.0 * SR) as usize);
        assert!((150..=250).contains(&counted.spawned), "events {}", counted.spawned);
        assert!(out.iter().any(|v| v.abs() > 1e-3));
        node.reset();
        let out = render(node.as_mut(), &p, (8.0 * SR) as usize);
        // A restart is a new draw: the same params give a different texture.
        node.reset();
        let again = render(node.as_mut(), &p, (8.0 * SR) as usize);
        assert!(out.iter().zip(&again).any(|(a, b)| (a - b).abs() > 1e-6));
        assert!(again.iter().all(|v| v.is_finite()));
    }
}
