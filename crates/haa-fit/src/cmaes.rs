//! CMA-ES (Hansen's (μ/μ_w, λ) with rank-one and rank-μ updates, full covariance) in the unit cube,
//! driven one evaluation at a time.
//!
//! Full covariance rather than separable: a patch's levels multiply each other (source level ×
//! gain × curve amount × output gain), so the good region is a correlated valley that a diagonal
//! covariance crosses badly. With at most 40 dimensions the O(n²) sampling and the occasional
//! Jacobi eigendecomposition cost nothing next to one render.
//!
//! `ask` hands out the next candidate of the current generation and `tell` records its loss; the
//! distribution updates when a generation is complete, so a caller can stop between any two
//! evaluations. Box constraints: a sample is clamped into 0..1 for evaluation and its loss gets a
//! quadratic penalty on the clamped distance; the update uses the raw sample. Clamping (not
//! mirroring) lets a value sit exactly on a bound, where many patch values belong (a gain base of 0
//! under a curve, a curve's end at 0). Randomness: Sona's xorshift (`haa_core::rng::Rng`) with
//! Box–Muller normals.

use haa_core::rng::Rng;

/// Normal deviates from the engine's xorshift generator.
pub struct Gauss {
    rng: Rng,
    spare: Option<f64>,
}

impl Gauss {
    pub fn new(seed: u32) -> Gauss {
        Gauss { rng: Rng::new(seed), spare: None }
    }

    pub fn uniform(&mut self) -> f64 {
        self.rng.next_f64()
    }

    /// A uniform index below `n` (0 for n = 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.rng.next_u32() as usize) % n }
    }

    pub fn normal(&mut self) -> f64 {
        if let Some(z) = self.spare.take() {
            return z;
        }
        let u1 = 1.0 - self.rng.next_f64();
        let u2 = self.rng.next_f64();
        let r = (-2.0 * u1.ln()).sqrt();
        let a = 2.0 * core::f64::consts::PI * u2;
        self.spare = Some(r * a.sin());
        r * a.cos()
    }
}

/// One CMA-ES run.
pub struct Cma {
    n: usize,
    lambda: usize,
    mu: usize,
    weights: Vec<f64>,
    mueff: f64,
    cs: f64,
    ds: f64,
    cc: f64,
    c1: f64,
    cmu: f64,
    chi_n: f64,
    mean: Vec<f64>,
    sigma: f64,
    /// Covariance (row-major n×n), its eigenvectors (columns of `b`) and the square roots of its eigenvalues.
    c: Vec<f64>,
    b: Vec<f64>,
    d: Vec<f64>,
    eigen_every: usize,
    ps: Vec<f64>,
    pc: Vec<f64>,
    generation: usize,
    /// Raw samples (may leave the box) and their clamped copies (what gets evaluated).
    raw: Vec<Vec<f64>>,
    pop: Vec<Vec<f64>>,
    losses: Vec<f64>,
    next: usize,
    pub best_x: Vec<f64>,
    pub best_f: f64,
    stall: usize,
    gen_best: f64,
}

impl Cma {
    /// Default population for `n` dimensions.
    pub fn default_lambda(n: usize) -> usize {
        4 + (3.0 * (n.max(1) as f64).ln()).floor() as usize
    }

    /// A run centred on `x0` with global step `sigma`, per-dimension steps `steps` (the initial
    /// covariance is diagonal, their squares) and population `lambda` (0 = default).
    pub fn new(x0: &[f64], sigma: f64, steps: &[f64], lambda: usize, g: &mut Gauss) -> Cma {
        let n = x0.len().max(1);
        let lambda = if lambda == 0 { Self::default_lambda(n) } else { lambda.max(4) };
        let mu = lambda / 2;
        let raw: Vec<f64> = (0..mu).map(|i| ((lambda as f64 + 1.0) / 2.0).ln() - ((i + 1) as f64).ln()).collect();
        let sum: f64 = raw.iter().sum();
        let weights: Vec<f64> = raw.iter().map(|w| w / sum).collect();
        let mueff = 1.0 / weights.iter().map(|w| w * w).sum::<f64>();
        let nf = n as f64;
        let cs = (mueff + 2.0) / (nf + mueff + 5.0);
        let ds = 1.0 + 2.0 * (((mueff - 1.0) / (nf + 1.0)).sqrt() - 1.0).max(0.0) + cs;
        let cc = (4.0 + mueff / nf) / (nf + 4.0 + 2.0 * mueff / nf);
        let c1 = 2.0 / ((nf + 1.3).powi(2) + mueff);
        let cmu = (2.0 * (mueff - 2.0 + 1.0 / mueff) / ((nf + 2.0).powi(2) + mueff)).min(1.0 - c1);
        let chi_n = nf.sqrt() * (1.0 - 1.0 / (4.0 * nf) + 1.0 / (21.0 * nf * nf));
        let mut c = vec![0.0; n * n];
        let mut b = vec![0.0; n * n];
        let mut d = vec![1.0; n];
        for i in 0..n {
            let s = steps.get(i).copied().unwrap_or(1.0).clamp(1e-4, 1.0);
            c[i * n + i] = s * s;
            b[i * n + i] = 1.0;
            d[i] = s;
        }
        let eigen_every = ((1.0 / (10.0 * nf * (c1 + cmu))).floor() as usize).max(1);
        let mut mean: Vec<f64> = x0.iter().map(|x| x.clamp(0.0, 1.0)).collect();
        mean.resize(n, 0.5);
        let mut s = Cma {
            n,
            lambda,
            mu,
            weights,
            mueff,
            cs,
            ds,
            cc,
            c1,
            cmu,
            chi_n,
            best_x: mean.clone(),
            mean,
            sigma,
            c,
            b,
            d,
            eigen_every,
            ps: vec![0.0; n],
            pc: vec![0.0; n],
            generation: 0,
            raw: Vec::new(),
            pop: Vec::new(),
            losses: Vec::new(),
            next: 0,
            best_f: f64::INFINITY,
            stall: 0,
            gen_best: f64::INFINITY,
        };
        s.sample(g);
        s
    }

    pub fn lambda(&self) -> usize {
        self.lambda
    }

    pub fn sigma(&self) -> f64 {
        self.sigma
    }

    fn sample(&mut self, g: &mut Gauss) {
        let n = self.n;
        self.pop.clear();
        self.raw.clear();
        self.losses.clear();
        self.next = 0;
        let mut z = vec![0.0; n];
        for _ in 0..self.lambda {
            for v in z.iter_mut() {
                *v = g.normal();
            }
            let x: Vec<f64> = (0..n)
                .map(|i| {
                    let mut y = 0.0;
                    for j in 0..n {
                        y += self.b[i * n + j] * self.d[j] * z[j];
                    }
                    self.mean[i] + self.sigma * y
                })
                .collect();
            self.pop.push(x.iter().map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 }).collect());
            self.raw.push(x);
        }
    }

    /// The next candidate to evaluate.
    pub fn ask(&self) -> &[f64] {
        &self.pop[self.next.min(self.lambda - 1)]
    }

    /// Records the loss of the candidate `ask` returned; updates after a full generation.
    pub fn tell(&mut self, loss: f64, g: &mut Gauss) {
        let loss = if loss.is_finite() { loss } else { f64::MAX };
        if loss < self.best_f {
            self.best_f = loss;
            self.best_x = self.pop[self.next].clone();
        }
        let i = self.next;
        let outside: f64 = self.raw[i].iter().zip(&self.pop[i]).map(|(r, c)| (r - c) * (r - c)).sum();
        self.losses.push(loss + PENALTY * outside);
        self.next += 1;
        if self.next >= self.lambda {
            self.update();
            self.sample(g);
        }
    }

    fn update(&mut self) {
        let n = self.n;
        let mut idx: Vec<usize> = (0..self.lambda).collect();
        idx.sort_by(|a, b| self.losses[*a].partial_cmp(&self.losses[*b]).unwrap_or(core::cmp::Ordering::Equal).then(a.cmp(b)));
        let best = self.losses[idx[0]];
        if best < self.gen_best - 1e-9 * self.gen_best.abs().max(1e-12) {
            self.gen_best = best;
            self.stall = 0;
        } else {
            self.stall += 1;
        }
        let old = self.mean.clone();
        let ys: Vec<Vec<f64>> = idx.iter().take(self.mu).map(|&i| (0..n).map(|d| (self.raw[i][d] - old[d]) / self.sigma).collect()).collect();
        let mut yw = vec![0.0; n];
        for (k, y) in ys.iter().enumerate() {
            for d in 0..n {
                yw[d] += self.weights[k] * y[d];
            }
        }
        for d in 0..n {
            self.mean[d] = (old[d] + self.sigma * yw[d]).clamp(-0.5, 1.5);
        }
        // C^{-1/2} yw = B D^{-1} Bᵀ yw.
        let mut t = vec![0.0; n];
        for j in 0..n {
            let mut s = 0.0;
            for i in 0..n {
                s += self.b[i * n + j] * yw[i];
            }
            t[j] = s / self.d[j];
        }
        let cs_norm = (self.cs * (2.0 - self.cs) * self.mueff).sqrt();
        for i in 0..n {
            let mut s = 0.0;
            for j in 0..n {
                s += self.b[i * n + j] * t[j];
            }
            self.ps[i] = (1.0 - self.cs) * self.ps[i] + cs_norm * s;
        }
        let ps_norm = self.ps.iter().map(|v| v * v).sum::<f64>().sqrt();
        let generation = (self.generation + 1) as f64;
        let hsig = ps_norm / (1.0 - (1.0 - self.cs).powf(2.0 * generation)).sqrt() / self.chi_n < 1.4 + 2.0 / (n as f64 + 1.0);
        let cc_norm = (self.cc * (2.0 - self.cc) * self.mueff).sqrt();
        for d in 0..n {
            self.pc[d] = (1.0 - self.cc) * self.pc[d] + if hsig { cc_norm * yw[d] } else { 0.0 };
        }
        let delta = if hsig { 0.0 } else { self.cc * (2.0 - self.cc) };
        let keep = 1.0 - self.c1 - self.cmu;
        for i in 0..n {
            for j in 0..=i {
                let mut rank_mu = 0.0;
                for (k, y) in ys.iter().enumerate() {
                    rank_mu += self.weights[k] * y[i] * y[j];
                }
                let v = keep * self.c[i * n + j] + self.c1 * (self.pc[i] * self.pc[j] + delta * self.c[i * n + j]) + self.cmu * rank_mu;
                self.c[i * n + j] = v;
                self.c[j * n + i] = v;
            }
        }
        self.sigma *= ((self.cs / self.ds) * (ps_norm / self.chi_n - 1.0)).min(1.0).exp();
        self.sigma = self.sigma.clamp(1e-10, 2.0);
        self.generation += 1;
        if self.generation % self.eigen_every == 0 {
            self.decompose();
        }
    }

    /// Refreshes `b` and `d` from `c` (cyclic Jacobi; eigenvalues floored to keep sampling sane).
    fn decompose(&mut self) {
        let n = self.n;
        let mut a = self.c.clone();
        let mut v = vec![0.0; n * n];
        for i in 0..n {
            v[i * n + i] = 1.0;
        }
        for _sweep in 0..60 {
            let mut off = 0.0;
            let mut diag = 0.0;
            for i in 0..n {
                diag += a[i * n + i] * a[i * n + i];
                for j in 0..i {
                    off += a[i * n + j] * a[i * n + j];
                }
            }
            if off <= 1e-24 * diag {
                break;
            }
            for p in 0..n {
                for q in p + 1..n {
                    let apq = a[p * n + q];
                    if apq == 0.0 {
                        continue;
                    }
                    let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                    let t = if theta == 0.0 { 1.0 } else { theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt()) };
                    let c = 1.0 / (t * t + 1.0).sqrt();
                    let s = t * c;
                    for k in 0..n {
                        let akp = a[k * n + p];
                        let akq = a[k * n + q];
                        a[k * n + p] = c * akp - s * akq;
                        a[k * n + q] = s * akp + c * akq;
                    }
                    for k in 0..n {
                        let apk = a[p * n + k];
                        let aqk = a[q * n + k];
                        a[p * n + k] = c * apk - s * aqk;
                        a[q * n + k] = s * apk + c * aqk;
                    }
                    for k in 0..n {
                        let vkp = v[k * n + p];
                        let vkq = v[k * n + q];
                        v[k * n + p] = c * vkp - s * vkq;
                        v[k * n + q] = s * vkp + c * vkq;
                    }
                }
            }
        }
        let max = (0..n).map(|i| a[i * n + i]).fold(0.0f64, f64::max).max(1e-20);
        for i in 0..n {
            self.d[i] = a[i * n + i].max(max * 1e-14).sqrt();
        }
        self.b = v;
    }

    /// True when the run has collapsed or stopped improving.
    pub fn converged(&self) -> bool {
        let spread = self.sigma * self.d.iter().cloned().fold(0.0, f64::max);
        let patience = 10 + (30 * self.n).div_ceil(self.lambda);
        spread < 1e-5 || self.stall > patience
    }

    /// True between generations (no candidate of the current generation evaluated yet).
    pub fn at_generation_start(&self) -> bool {
        self.next == 0
    }
}

/// Loss added per squared unit of distance between a raw sample and its clamped copy.
const PENALTY: f64 = 1.0;
