//! Circular membrane with optional lumped cavity, port and cylindrical shell breathing.
//! Compact low-frequency radiation; no 3D air field, second head or full shell modes.
//! Mechanical changes take effect at the next strike and clear old modes.
#[path = "membrane/coupled.rs"]
mod coupled;
use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind, Scale};
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};
use coupled::{Body, CoupledBank};
mod asset {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/membrane_asset.rs"
    ));
}
use asset::{BRANCH, FREQUENCIES, N, NORMALIZATION, ORDER, VALID, ZERO};
const fn number(
    key: &'static str,
    min: f64,
    max: f64,
    default: f64,
    step: f64,
    unit: &'static str,
    en: &'static str,
    fr: &'static str,
    modulatable: bool,
) -> ParamDef {
    ParamDef {
        key,
        kind: ParamKind::Number {
            min,
            max,
            default,
            step,
            scale: Scale::Lin,
        },
        unit,
        label_en: en,
        label_fr: fr,
        modulatable,
        smooth: false,
    }
}
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "membrane",
    category: Category::Source,
    label_en: "Drum (membrane / body)",
    label_fr: "Tambour (peau / caisse)",
    audio_in: false,
    audio_out: true,
    control_out: false,
    control_outputs: &[],
    uses_curve: false,
    hidden: false,
    create,
    params: &[
        number(
            "radius",
            0.04,
            0.5,
            0.15,
            0.005,
            "m",
            "Radius — next strike",
            "Rayon — prochaine frappe",
            false,
        ),
        number(
            "tension",
            50.,
            5000.,
            800.,
            10.,
            "N/m",
            "Tension — next strike",
            "Tension — prochaine frappe",
            true,
        ),
        number(
            "density",
            500.,
            3000.,
            1400.,
            10.,
            "kg/m³",
            "Density — next strike",
            "Masse volumique — prochaine frappe",
            false,
        ),
        number(
            "thickness",
            0.05,
            2.,
            0.25,
            0.01,
            "mm",
            "Thickness — next strike",
            "Épaisseur — prochaine frappe",
            false,
        ),
        number(
            "damping",
            0.1,
            80.,
            12.,
            0.1,
            "1/s",
            "Damping — next strike",
            "Amortissement — prochaine frappe",
            true,
        ),
        number(
            "highDamping",
            0.,
            20.,
            1.,
            0.1,
            "µs",
            "High damping — next strike",
            "Amortissement aigu — prochaine frappe",
            false,
        ),
        number(
            "strikeRadius",
            0.,
            0.95,
            0.65,
            0.01,
            "R",
            "Strike radius",
            "Rayon de frappe",
            true,
        ),
        number(
            "strikeAngle",
            0.,
            360.,
            0.,
            1.,
            "°",
            "Strike angle",
            "Angle de frappe",
            true,
        ),
        number(
            "contact",
            0.2,
            30.,
            2.,
            0.1,
            "ms",
            "Contact duration",
            "Durée de contact",
            true,
        ),
        number(
            "impulse",
            0.,
            0.01,
            0.002,
            0.0001,
            "N·s",
            "Impulse",
            "Impulsion",
            true,
        ),
        number(
            "strike",
            0.,
            1.,
            0.,
            0.01,
            "",
            "Strike (rising edge)",
            "Frappe (front montant)",
            true,
        ),
        number("gain", 0., 2., 0.8, 0.01, "×", "Level", "Niveau", true),
        ParamDef {
            key: "autoStrike",
            kind: ParamKind::Bool { default: true },
            unit: "",
            label_en: "Strike on Play / loop",
            label_fr: "Frappe au Play / boucle",
            modulatable: false,
            smooth: false,
        },
        ParamDef {
            key: "bodyEnabled",
            kind: ParamKind::Bool { default: false },
            unit: "",
            label_en: "Shell + cavity — next strike",
            label_fr: "Caisse + air — prochaine frappe",
            modulatable: false,
            smooth: false,
        },
        number(
            "depth",
            0.05,
            0.8,
            0.25,
            0.01,
            "m",
            "Cavity depth",
            "Profondeur de caisse",
            false,
        ),
        number(
            "ventRadius",
            0.,
            0.08,
            0.015,
            0.001,
            "m",
            "Vent radius (0 = sealed)",
            "Rayon évent (0 = fermé)",
            false,
        ),
        number(
            "shellThickness",
            1.,
            20.,
            6.,
            0.1,
            "mm",
            "Shell thickness",
            "Épaisseur caisse",
            false,
        ),
        number(
            "shellDensity",
            400.,
            9000.,
            650.,
            10.,
            "kg/m³",
            "Shell density",
            "Masse volumique caisse",
            false,
        ),
        number(
            "shellYoung",
            0.1,
            200.,
            1.,
            0.1,
            "GPa",
            "Shell hoop modulus",
            "Module circonférentiel caisse",
            false,
        ),
        number(
            "shellDamping",
            1.,
            150.,
            30.,
            1.,
            "1/s",
            "Shell damping",
            "Amortissement caisse",
            false,
        ),
        number(
            "airLoss",
            1.,
            150.,
            35.,
            1.,
            "1/s",
            "Vent losses",
            "Pertes évent",
            false,
        ),
        number(
            "radiation",
            0.,
            1.,
            1.,
            0.01,
            "",
            "Local pickup → compact radiation",
            "Capteur local → rayonnement compact",
            true,
        ),
    ],
};
fn material(p: &[f64]) -> Material {
    Material {
        radius: p[0],
        tension: p[1],
        density: p[2],
        thickness: p[3] * 0.001,
        alpha: p[4],
        beta: p[5] * 1e-6,
    }
}
struct Membrane {
    bank: ModalBank,
    coupled: CoupledBank,
    body_settings: [f64; 8],
    mechanics: [f64; 6],
    started: bool,
    gate: bool,
}
fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(Membrane::new(init.sample_rate, init.params))
}
impl Membrane {
    fn new(rate: f64, p: &[f64]) -> Self {
        let mut mechanics = [0.; 6];
        mechanics.copy_from_slice(&p[..6]);
        let mut body_settings = [0.; 8];
        body_settings.copy_from_slice(&p[13..21]);
        Self {
            bank: ModalBank::new(rate, material(p), (0.66, 0.54)),
            coupled: CoupledBank::new(rate, material(p), Body::from_params(p)),
            body_settings,
            mechanics,
            started: false,
            gate: false,
        }
    }
}
impl Node for Membrane {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), p: &[f64]) -> NodeOut {
        let gate = p[10] > 0.5;
        if (gate && !self.gate) || ((!self.started || ctx.looped) && p[12] > 0.5) {
            if self.mechanics != p[..6] || self.body_settings != p[13..21] {
                self.bank = ModalBank::new(ctx.sample_rate, material(p), (0.66, 0.54));
                self.mechanics.copy_from_slice(&p[..6]);
                self.coupled = CoupledBank::new(ctx.sample_rate, material(p), Body::from_params(p));
                self.body_settings.copy_from_slice(&p[13..21]);
            }
            let a = p[7].to_radians();
            if self.body_settings[0] > 0.5 {
                self.coupled
                    .strike(p[6] * a.cos(), p[6] * a.sin(), p[9], p[8] * 0.001);
            } else {
                self.bank
                    .strike(p[6] * a.cos(), p[6] * a.sin(), p[9], p[8] * 0.001);
            }
        }
        self.started = true;
        self.gate = gate;
        let v = if self.body_settings[0] > 0.5 {
            let (local, pressure) = self.coupled.sample();
            ((1. - p[21]) * local + p[21] * pressure) * p[11]
        } else {
            self.bank.sample() * p[11]
        };
        NodeOut {
            l: v,
            r: v,
            value: 0.,
        }
    }
    fn reset(&mut self) {
        self.coupled.reset();
        self.bank.re.fill(0.);
        self.bank.im.fill(0.);
        self.bank.contact_len = 0;
        self.bank.contact_age = 0;
        self.started = false;
        self.gate = false;
    }
}
// Bessel J_m by a power series. Event-rate only, never inside the audio loop.
fn bessel(m: usize, x: f64) -> f64 {
    if x == 0. {
        return if m == 0 { 1. } else { 0. };
    }
    let mut term = 1.;
    for k in 1..=m {
        term *= x / (2. * k as f64);
    }
    let mut sum = term;
    for k in 1..120 {
        term *= -x * x / (4. * k as f64 * (k + m) as f64);
        sum += term;
        if term.abs() < 1e-17 * sum.abs().max(1.) {
            break;
        }
    }
    sum
}
fn shape_at(x: f64, y: f64) -> [f64; N] {
    let r = (x * x + y * y).sqrt();
    let mut out = [0.; N];
    if r >= 1. {
        return out;
    }
    let theta = y.atan2(x);
    for i in 0..VALID {
        let a = ORDER[i] as f64 * theta;
        let angular = if BRANCH[i] == 0 { a.cos() } else { a.sin() };
        out[i] = NORMALIZATION[i] * bessel(ORDER[i], ZERO[i] * r) * angular;
    }
    out
}
#[cfg(test)]
fn complete_modes(requested: usize) -> usize {
    let n = requested.clamp(1, VALID);
    if ORDER[n - 1] > 0 && BRANCH[n - 1] == 0 {
        n - 1
    } else {
        n
    }
}
#[derive(Clone, Copy)]
struct Material {
    radius: f64,
    tension: f64,
    density: f64,
    thickness: f64,
    alpha: f64,
    beta: f64,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            radius: 0.15,
            tension: 800.,
            density: 1400.,
            thickness: 0.00025,
            alpha: 12.,
            beta: 1e-6,
        }
    }
}
#[derive(Clone)]
struct ModalBank {
    re: [f64; N],
    im: [f64; N],
    c: [f64; N],
    s: [f64; N],
    sigma_ratio: [f64; N],
    pickup: [f64; N],
    inject_re: [f64; N],
    inject_im: [f64; N],
    force_shape: [f64; N],
    contact_age: usize,
    contact_len: usize,
    contact_impulse: f64,
    rate: f64,
    mass_scale: f64,
    active_modes: usize,
}
impl ModalBank {
    fn new(rate: f64, material: Material, pickup: (f64, f64)) -> Self {
        assert!(rate >= 8000. && rate.is_finite());
        assert!(
            material.radius > 0.
                && material.tension > 0.
                && material.density > 0.
                && material.thickness > 0.
        );
        let mu = material.density * material.thickness;
        let factor = 0.15 / material.radius * (material.tension / 800. * 0.35 / mu).sqrt();
        let mass_scale = (0.35 * 0.15 * 0.15 / (mu * material.radius * material.radius)).sqrt();
        let mut bank = Self {
            re: [0.; N],
            im: [0.; N],
            c: [0.; N],
            s: [0.; N],
            sigma_ratio: [0.; N],
            pickup: shape_at(pickup.0, pickup.1),
            inject_re: [0.; N],
            inject_im: [0.; N],
            force_shape: [0.; N],
            contact_age: 0,
            contact_len: 0,
            contact_impulse: 0.,
            rate,
            mass_scale,
            active_modes: VALID,
        };
        for m in 0..N {
            let f = FREQUENCIES[m] * factor;
            let w = std::f64::consts::TAU * f;
            let sigma = (material.alpha + material.beta * w * w) * 0.5;
            // This prototype handles underdamped modes only. Never fold inaudible modes into audio.
            if f >= rate * 0.45 || sigma >= w {
                bank.pickup[m] = 0.;
                continue;
            }
            let wd = (w * w - sigma * sigma).sqrt();
            let r = (-sigma / rate).exp();
            bank.c[m] = r * (wd / rate).cos();
            bank.s[m] = r * (wd / rate).sin();
            bank.sigma_ratio[m] = sigma / wd;
            bank.pickup[m] *= mass_scale;
            // Exact integration of a held force over one sample: (exp(lambda dt)-1)/lambda.
            let a = bank.c[m] - 1.;
            let b = bank.s[m];
            let den = sigma * sigma + wd * wd;
            bank.inject_re[m] = (-sigma * a + wd * b) / den;
            bank.inject_im[m] = (-wd * a - sigma * b) / den;
        }
        bank
    }
    fn strike(&mut self, x: f64, y: f64, impulse: f64, contact_seconds: f64) {
        // One active finite-duration contact per body; vibrations from previous impacts persist.
        self.force_shape = shape_at(x, y);
        for f in &mut self.force_shape {
            *f *= self.mass_scale;
        }
        self.contact_age = 0;
        self.contact_len = (contact_seconds * self.rate).round().max(2.) as usize;
        self.contact_impulse = impulse;
    }
    #[cfg(test)]
    fn impulse(&mut self, x: f64, y: f64, impulse: f64) {
        let shape = shape_at(x, y);
        for m in 0..N {
            self.re[m] += shape[m] * self.mass_scale * impulse;
        }
    }
    fn sample(&mut self) -> f64 {
        let force = if self.contact_age < self.contact_len {
            let phase = (self.contact_age as f64 + 0.5) / self.contact_len as f64;
            self.contact_age += 1;
            self.contact_impulse * self.rate / self.contact_len as f64
                * (1. - (std::f64::consts::TAU * phase).cos())
        } else {
            0.
        };
        let mut out = 0.;
        for m in 0..self.active_modes {
            let re = self.c[m] * self.re[m] - self.s[m] * self.im[m]
                + force * self.force_shape[m] * self.inject_re[m];
            let im = self.s[m] * self.re[m]
                + self.c[m] * self.im[m]
                + force * self.force_shape[m] * self.inject_im[m];
            self.re[m] = re;
            self.im[m] = im;
            out += (re - self.sigma_ratio[m] * im) * self.pickup[m];
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rim_fixed_and_center_symmetric() {
        assert!(shape_at(1., 0.).iter().all(|x| *x == 0.));
        let c = shape_at(0., 0.);
        for i in 0..VALID {
            if ORDER[i] > 0 {
                assert_eq!(c[i], 0.);
            }
        }
    }
    #[test]
    fn bessel_known_values_and_roots() {
        assert!((bessel(0, 1.) - 0.7651976865579666).abs() < 1e-14);
        for i in 0..VALID {
            assert!(bessel(ORDER[i], ZERO[i]).abs() < 1e-8);
        }
    }
    #[test]
    fn rotational_invariance() {
        let a = shape_at(0.6, 0.);
        let b = shape_at(0.2, 0.3);
        let theta = 0.731_f64;
        let rot = |x: f64, y: f64| {
            (
                x * theta.cos() - y * theta.sin(),
                x * theta.sin() + y * theta.cos(),
            )
        };
        let ar = rot(0.6, 0.);
        let br = rot(0.2, 0.3);
        let aa = shape_at(ar.0, ar.1);
        let bb = shape_at(br.0, br.1);
        for count in [16, 32, 64] {
            let n = complete_modes(count);
            let x: f64 = (0..n).map(|i| a[i] * b[i]).sum();
            let y: f64 = (0..n).map(|i| aa[i] * bb[i]).sum();
            assert!((x - y).abs() < 1e-7);
        }
    }
    #[test]
    fn reciprocity() {
        let mut x = ModalBank::new(48000., Material::default(), (0.2, 0.3));
        let mut y = ModalBank::new(48000., Material::default(), (0.6, 0.));
        x.impulse(0.6, 0., 0.001);
        y.impulse(0.2, 0.3, 0.001);
        for _ in 0..48000 {
            assert!((x.sample() - y.sample()).abs() < 1e-10);
        }
    }
    #[test]
    fn scaling_of_tension_and_mass() {
        let base = Material::default();
        let a = ModalBank::new(48000., base, (0.2, 0.3));
        let b = ModalBank::new(
            48000.,
            Material {
                tension: 1600.,
                ..base
            },
            (0.2, 0.3),
        );
        let c = ModalBank::new(
            48000.,
            Material {
                thickness: 0.0005,
                ..base
            },
            (0.2, 0.3),
        );
        let angle = |bank: &ModalBank| bank.s[0].atan2(bank.c[0]);
        assert!((angle(&b) / angle(&a) - 2_f64.sqrt()).abs() < 0.0001);
        assert!((angle(&c) / angle(&a) - 0.5_f64.sqrt()).abs() < 0.0001);
    }
    #[test]
    fn finite_and_decaying() {
        let mut b = ModalBank::new(48000., Material::default(), (0.2, 0.3));
        b.strike(0.6, 0., 0.002, 0.002);
        let mut early = 0.;
        let mut late = 0.;
        for i in 0..240000 {
            let x = b.sample();
            assert!(x.is_finite());
            if i < 48000 {
                early += x * x;
            }
            if i >= 192000 {
                late += x * x;
            }
        }
        assert!(late < early * 1e-12);
    }

    #[test]
    fn preset_plays_and_trigger_restarts_sound() {
        use crate::render::{RenderOptions, render};
        let patch = include_str!("../../../../patches/examples/drum-membrane.json");
        let plain = render(
            patch,
            &RenderOptions {
                seconds: Some(3.),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(plain.left.iter().all(|v| v.is_finite()));
        let first: f32 = plain.left[..4800].iter().map(|x| x * x).sum();
        assert!(first > 0.01);
        let hit = render(
            patch,
            &RenderOptions {
                seconds: Some(3.),
                triggers: vec![("strike".into(), 2.)],
                ..Default::default()
            },
        )
        .unwrap();
        let quiet: f32 = plain.left[96000..100800].iter().map(|x| x * x).sum();
        let struck: f32 = hit.left[96000..100800].iter().map(|x| x * x).sum();
        assert!(struck > quiet * 100. && struck > 0.01);
    }
    #[test]
    fn held_gate_strikes_once_and_reset_repeats() {
        use crate::patch::GraphMode;
        let mut p: Vec<f64> = DEF
            .params
            .iter()
            .map(|p| match p.kind {
                ParamKind::Number { default, .. } => default,
                ParamKind::Bool { default } => f64::from(default),
                _ => 0.,
            })
            .collect();
        p[10] = 1.;
        p[12] = 0.;
        let ctx = SampleCtx {
            sample_rate: 48000.,
            dt: 1. / 48000.,
            t: 0.,
            seconds: 0.,
            duration: 3.,
            mode: GraphMode::Live,
            live_frames: 0,
            looped: false,
            perf_rebase: false,
            inputs: &[],
            triggers: &[],
        };
        let mut node = Membrane::new(48000., &p);
        let first = node.process_sample(&ctx, (0., 0.), &p).l;
        for _ in 0..96000 {
            node.process_sample(&ctx, (0., 0.), &p);
        }
        assert_eq!(node.bank.contact_age, node.bank.contact_len);
        node.reset();
        assert_eq!(first, node.process_sample(&ctx, (0., 0.), &p).l);
    }
}
