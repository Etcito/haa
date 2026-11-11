//! Low-frequency lumped cavity + circular cylindrical shell breathing mode.
//! q are mass-normalized coordinates, K=diag(omega²)+kappa*b*bᵀ.
//! Implicit midpoint conserves undamped quadratic energy; diagonal damping is passive.
//! Compact-source pressure at 1m, no propagation/directivity/radiation loading solver.
use super::{FREQUENCIES, Material, NORMALIZATION, ORDER, VALID, ZERO, bessel, shape_at};
const COUNT: usize = VALID + 2;
const SHELL: usize = VALID;
const PORT: usize = VALID + 1;
const RHO: f64 = 1.204;
const C: f64 = 343.;
const PI: f64 = std::f64::consts::PI;

#[derive(Clone, Copy)]
pub(super) struct Body {
    pub depth: f64,
    pub vent: f64,
    pub thickness: f64,
    pub density: f64,
    pub young: f64,
    pub damping: f64,
    pub air_loss: f64,
}
impl Body {
    pub fn from_params(p: &[f64]) -> Self {
        Self {
            depth: p[14],
            vent: p[15],
            thickness: p[16] * 0.001,
            density: p[17],
            young: p[18] * 1e9,
            damping: p[19],
            air_loss: p[20],
        }
    }
}

pub(super) struct CoupledBank {
    q: [f64; COUNT],
    v: [f64; COUNT],
    k: [f64; COUNT],
    damping: [f64; COUNT],
    b: [f64; COUNT],
    inv: [f64; COUNT],
    diag_v: [f64; COUNT],
    rhs: [f64; COUNT],
    force_shape: [f64; COUNT],
    pickup: [f64; COUNT],
    radiation: [f64; COUNT],
    coupling_gain: f64,
    kappa: f64,
    dt: f64,
    mass_scale: f64,
    contact_age: usize,
    contact_len: usize,
    impulse: f64,
}
impl CoupledBank {
    pub fn new(rate: f64, mat: Material, body: Body) -> Self {
        let dt = 1. / rate;
        let r = mat.radius;
        let mu = mat.density * mat.thickness;
        let mass_scale = (0.35 * 0.15 * 0.15 / (mu * r * r)).sqrt();
        let factor = 0.15 / r * (mat.tension / 800. * 0.35 / mu).sqrt();
        let mut s = Self {
            q: [0.; COUNT],
            v: [0.; COUNT],
            k: [0.; COUNT],
            damping: [0.; COUNT],
            b: [0.; COUNT],
            inv: [0.; COUNT],
            diag_v: [0.; COUNT],
            rhs: [0.; COUNT],
            force_shape: [0.; COUNT],
            pickup: [0.; COUNT],
            radiation: [0.; COUNT],
            coupling_gain: 0.,
            kappa: RHO * C * C / (PI * r * r * body.depth),
            dt,
            mass_scale,
            contact_age: 0,
            contact_len: 0,
            impulse: 0.,
        };
        let pickup = shape_at(0.66, 0.54);
        for i in 0..VALID {
            let f = FREQUENCIES[i] * factor;
            if f > rate * 0.35 {
                continue;
            }
            let w = 2. * PI * f;
            s.k[i] = w * w;
            s.damping[i] = mat.alpha + mat.beta * w * w;
            s.pickup[i] = pickup[i] * mass_scale;
            // Exact surface integral: all non-axisymmetric modes integrate to zero.
            if ORDER[i] == 0 {
                s.b[i] =
                    2. * PI * r * r * NORMALIZATION[i] * mass_scale * bessel(1, ZERO[i]) / ZERO[i];
            }
            // Head displacement is positive into the cavity, opposite outward radiation.
            s.radiation[i] = -RHO * s.b[i] / (2. * PI);
        }
        let area = 2. * PI * r * body.depth;
        let shell_mass = area * body.thickness * body.density;
        // Thin isotropic cylindrical wall, uniform radial breathing, plane-stress hoop strain.
        let shell_w = (body.young / (body.density * r * r)).sqrt();
        if shell_w / (2. * PI) < rate * 0.35 {
            s.k[SHELL] = shell_w * shell_w;
            s.damping[SHELL] = body.damping;
            s.b[SHELL] = -area / shell_mass.sqrt();
            s.radiation[SHELL] = -RHO * s.b[SHELL] / (4. * PI);
        }
        if body.vent > 0. {
            let port_area = PI * body.vent * body.vent;
            // Short baffled opening: 0.85r end correction at each end, low-frequency approximation.
            let mass = RHO * port_area * (body.thickness + 1.7 * body.vent);
            s.b[PORT] = -port_area / mass.sqrt();
            s.damping[PORT] = body.air_loss;
            s.radiation[PORT] = -RHO * s.b[PORT] / (4. * PI);
        }
        s.prepare();
        s
    }
    fn prepare(&mut self) {
        let h = self.dt;
        let rank = h * h * self.kappa * 0.25;
        let mut denom = 1.;
        for i in 0..COUNT {
            self.inv[i] = 1. / (1. + self.damping[i] * h * 0.5 + self.k[i] * h * h * 0.25);
            self.diag_v[i] = 1. - self.damping[i] * h * 0.5 - self.k[i] * h * h * 0.25;
            denom += rank * self.b[i] * self.b[i] * self.inv[i];
        }
        self.coupling_gain = rank / denom;
    }
    pub fn reset(&mut self) {
        self.q.fill(0.);
        self.v.fill(0.);
        self.contact_len = 0;
        self.contact_age = 0;
    }
    pub fn strike(&mut self, x: f64, y: f64, impulse: f64, seconds: f64) {
        let f = shape_at(x, y);
        for i in 0..VALID {
            self.force_shape[i] = if self.k[i] > 0. {
                f[i] * self.mass_scale
            } else {
                0.
            };
        }
        self.impulse = impulse;
        self.contact_age = 0;
        self.contact_len = (seconds / self.dt).round().max(2.) as usize;
    }
    /// Returns (local head velocity, compact-source pressure in Pa at one metre).
    pub fn sample(&mut self) -> (f64, f64) {
        let force = if self.contact_age < self.contact_len {
            let p = (self.contact_age as f64 + 0.5) / self.contact_len as f64;
            self.contact_age += 1;
            self.impulse / (self.dt * self.contact_len as f64) * (1. - (2. * PI * p).cos())
        } else {
            0.
        };
        let h = self.dt;
        let mut volume = 0.;
        for i in 0..COUNT {
            volume += self.b[i] * (self.q[i] + 0.25 * h * self.v[i]);
        }
        let pressure = self.kappa * volume;
        let mut projection = 0.;
        for i in 0..COUNT {
            self.rhs[i] = self.inv[i]
                * (self.diag_v[i] * self.v[i]
                    - h * self.k[i] * self.q[i]
                    - h * self.b[i] * pressure
                    + h * self.force_shape[i] * force);
            projection += self.b[i] * self.rhs[i];
        }
        let correction = self.coupling_gain * projection;
        let mut local = 0.;
        let mut radiated = 0.;
        for i in 0..COUNT {
            let v = self.rhs[i] - self.inv[i] * self.b[i] * correction;
            self.q[i] += h * 0.5 * (v + self.v[i]);
            radiated += self.radiation[i] * (v - self.v[i]) / h;
            self.v[i] = v;
            local += self.pickup[i] * v;
        }
        (local, radiated)
    }
    #[cfg(test)]
    fn energy(&self) -> f64 {
        let mut e = 0.;
        let mut volume = 0.;
        for i in 0..COUNT {
            e += self.v[i] * self.v[i] + self.k[i] * self.q[i] * self.q[i];
            volume += self.b[i] * self.q[i];
        }
        0.5 * (e + self.kappa * volume * volume)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn body() -> Body {
        Body {
            depth: 0.25,
            vent: 0.015,
            thickness: 0.006,
            density: 650.,
            young: 1e9,
            damping: 30.,
            air_loss: 35.,
        }
    }
    #[test]
    fn conservative_coupling_preserves_energy() {
        let mut b = CoupledBank::new(48000., Material::default(), body());
        b.damping.fill(0.);
        b.prepare();
        b.v[0] = 0.1;
        let e = b.energy();
        for _ in 0..96000 {
            b.sample();
        }
        assert!((b.energy() / e - 1.).abs() < 1e-9);
        assert!(b.q[PORT].abs() > 1e-10);
        assert!(b.q[SHELL].abs() > 1e-12);
    }
    #[test]
    fn damping_is_passive() {
        let mut b = CoupledBank::new(48000., Material::default(), body());
        b.v[0] = 0.1;
        let mut energy = b.energy();
        for _ in 0..48000 {
            b.sample();
            let e = b.energy();
            assert!(e <= energy + 1e-13);
            energy = e;
        }
        assert!(energy < 1e-7);
    }
    #[test]
    fn helmholtz_limit_matches_analytic_frequency() {
        let x = body();
        let mut b = CoupledBank::new(48000., Material::default(), x);
        for i in 0..PORT {
            b.b[i] = 0.;
        }
        b.damping.fill(0.);
        b.prepare();
        b.v[PORT] = 0.1;
        let omega = (b.kappa * b.b[PORT].powi(2)).sqrt();
        let analytic = C / (2. * PI)
            * (PI * x.vent * x.vent
                / (PI * 0.15_f64.powi(2) * x.depth * (x.thickness + 1.7 * x.vent)))
                .sqrt();
        assert!((omega / (2. * PI) - analytic).abs() < 1e-10);
        let expected = 2. / b.dt * (omega * b.dt / 2.).atan() / (2. * PI);
        let mut crossings = Vec::new();
        let mut prev = b.v[PORT];
        for n in 0..96000 {
            b.sample();
            if prev > 0. && b.v[PORT] <= 0. {
                crossings.push(n as f64 + prev / (prev - b.v[PORT]));
            }
            prev = b.v[PORT];
        }
        let measured =
            48000. * (crossings.len() - 1) as f64 / (crossings.last().unwrap() - crossings[0]);
        assert!((measured / expected - 1.).abs() < 1e-6);
    }
    #[test]
    fn extremes_and_sealed_cavity_are_finite() {
        for rate in [8000., 48000., 192000.] {
            for vent in [0., 0.001, 0.08] {
                let mut x = body();
                x.vent = vent;
                x.depth = 0.05;
                x.young = 200e9;
                let mut b = CoupledBank::new(
                    rate,
                    Material {
                        radius: 0.04,
                        tension: 5000.,
                        thickness: 0.00005,
                        ..Material::default()
                    },
                    x,
                );
                b.strike(0.1, 0.1, 0.01, 0.0002);
                for _ in 0..4800 {
                    let (l, p) = b.sample();
                    assert!(l.is_finite() && p.is_finite());
                }
            }
        }
    }
    #[test]
    fn non_axisymmetric_modes_do_not_pressurize_uniform_air() {
        let mut b = CoupledBank::new(48000., Material::default(), body());
        b.v[1] = 0.1;
        for _ in 0..4800 {
            b.sample();
        }
        assert_eq!(b.v[PORT], 0.);
        assert_eq!(b.v[SHELL], 0.);
    }
}
