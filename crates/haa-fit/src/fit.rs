//! The fitter: a bounded, deterministic search for the patch whose render best matches a sample.
//!
//! Stages, all inside one evaluation budget (one evaluation = one offline render + one loss):
//! 1. `screen`: every template renders once as-is; then a measured alignment is tried (one more
//!    render): source `frequency`/`pitch` params transposed by the target/template median-f0 ratio
//!    when both are pitched, and the output gain scaled by the target/template RMS ratio. The
//!    better of the two is that template's start point;
//! 2. `optimize`: the `top_k` best templates race by successive halving over 60 % of the rest
//!    (90 % without mutations): every round splits its share equally, the better half goes on
//!    (without the runs 1.5x worse than the leader), the last one left gets the rest. Each runs CMA-ES from its start point with IPOP restarts
//!    (population doubled, restarted from its best point) when a run converges;
//! 3. `mutate`: up to `mutations` one-node structure mutations of the best patch so far race the
//!    same way over 30 % of the rest; they are ranked by the best render's mel residual (a noise or
//!    sine layer where energy is missing, a filter where there is too much, `space` for a missing
//!    tail), removes last;
//! 4. `polish`: the best patch so far, small steps, whatever budget is left.
//!
//! `step(n)` runs at most `n` evaluations, so a host can report progress and cancel between calls.
//! Same target, templates, options and seed give the same result.

use std::collections::VecDeque;

use haa_core::catalog::{self, Category};
use haa_core::issues::{Severity, has_errors};
use haa_core::patch::{Patch, to_json};
use serde_json::{Value, json};

use crate::cmaes::{Gauss, Cma};
use crate::eval::{Renderer, prepare};
use crate::loss::{F0Mode, FAILED_LOSS, LossConfig, LossTerms, TargetFeatures, rms_of, round6};
use crate::mutate;
use crate::params::{MAX_DIMS, ParamSpace, round_sig};

/// Fitter settings. JSON keys (all optional): `budget`, `seed`, `topK`, `mutations`, `maxDims`,
/// `sigma`, `trimLead`, `align`, `f0` (`"auto"`, `true`, `false`), `weights` `{stft, mel, env, f0}`,
/// `ffts`.
#[derive(Clone, Debug, PartialEq)]
pub struct FitOptions {
    pub budget: usize,
    pub seed: u32,
    pub top_k: usize,
    pub mutations: usize,
    pub max_dims: usize,
    /// Multiplier of every dimension's initial step (`params::FitParam::step`).
    pub sigma: f64,
    /// Cut the target's leading silence (below −50 dB re its peak) and report it as `offset`.
    pub trim_lead: bool,
    /// Try the measured pitch/level alignment of each template while screening.
    pub align: bool,
    pub loss: LossConfig,
}

impl Default for FitOptions {
    fn default() -> FitOptions {
        FitOptions {
            budget: 600,
            seed: 1,
            top_k: 4,
            mutations: 3,
            max_dims: 24,
            sigma: 0.4,
            trim_lead: true,
            align: true,
            loss: LossConfig::default(),
        }
    }
}

impl FitOptions {
    /// Reads the options from a JSON object; unknown keys are ignored, bad values keep the default.
    pub fn from_json(v: &Value) -> FitOptions {
        let mut o = FitOptions::default();
        let num = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(x) = num("budget") {
            o.budget = x.max(1.0) as usize;
        }
        if let Some(x) = num("seed") {
            o.seed = x.max(0.0) as u32;
        }
        if let Some(x) = num("topK") {
            o.top_k = x.max(1.0) as usize;
        }
        if let Some(x) = num("mutations") {
            o.mutations = x.max(0.0) as usize;
        }
        if let Some(x) = num("maxDims") {
            o.max_dims = (x.max(1.0) as usize).min(MAX_DIMS);
        }
        if let Some(x) = num("sigma") {
            o.sigma = x.clamp(0.01, 4.0);
        }
        if let Some(b) = v.get("trimLead").and_then(Value::as_bool) {
            o.trim_lead = b;
        }
        if let Some(b) = v.get("align").and_then(Value::as_bool) {
            o.align = b;
        }
        match v.get("f0") {
            Some(Value::Bool(true)) => o.loss.f0 = F0Mode::On,
            Some(Value::Bool(false)) => o.loss.f0 = F0Mode::Off,
            _ => {}
        }
        if let Some(w) = v.get("weights") {
            let wf = |k: &str, d: f64| w.get(k).and_then(Value::as_f64).filter(|x| x.is_finite() && *x >= 0.0).unwrap_or(d);
            o.loss.w_stft = wf("stft", o.loss.w_stft);
            o.loss.w_mel = wf("mel", o.loss.w_mel);
            o.loss.w_env = wf("env", o.loss.w_env);
            o.loss.w_f0 = wf("f0", o.loss.w_f0);
        }
        if let Some(list) = v.get("ffts").and_then(Value::as_array) {
            let ffts: Vec<usize> = list
                .iter()
                .filter_map(Value::as_u64)
                .map(|n| n as usize)
                .filter(|n| n.is_power_of_two() && *n >= 16 && *n <= 65536)
                .collect();
            if !ffts.is_empty() {
                o.loss.ffts = ffts;
            }
        }
        o
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Screen,
    Optimize,
    Mutate,
    Polish,
    Done,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Screen => "screen",
            Stage::Optimize => "optimize",
            Stage::Mutate => "mutate",
            Stage::Polish => "polish",
            Stage::Done => "done",
        }
    }
}

struct TemplateState {
    id: String,
    prepared: Option<Patch>,
    error: Option<String>,
    score0: f64,
    aligned: Vec<String>,
    best: f64,
    evals: usize,
}

/// One optimisation: a base patch, its selected dims and a CMA run (restarted inside its budget).
struct Job {
    base: Patch,
    template: usize,
    mutations: Vec<String>,
    space: ParamSpace,
    cma: Cma,
    sigma: f64,
    /// Cumulative evaluation allowance; a race round extends it.
    budget: usize,
    used: usize,
    best_x: Vec<f64>,
    best_f: f64,
    restarts: usize,
    /// Start-point variants still to render before the CMA run (a mutant's level scan).
    scan: VecDeque<Patch>,
    scan_best: Option<(f64, Patch)>,
    prefer: Vec<String>,
}

/// A successive-halving race between jobs over one evaluation share.
struct Race {
    /// Evaluation count at which the race's share is spent.
    end: usize,
    round_share: usize,
    finished: Vec<Job>,
}

struct Best {
    patch: Patch,
    terms: LossTerms,
    template: usize,
    mutations: Vec<String>,
    values: Value,
}

/// A running or finished fit.
pub struct Fitter {
    target: TargetFeatures,
    target_pcm: Vec<f32>,
    renderer: Renderer,
    opts: FitOptions,
    gauss: Gauss,
    templates: Vec<TemplateState>,
    stage: Stage,
    screen_next: usize,
    pending_align: Option<(usize, Patch)>,
    queue: VecDeque<Job>,
    current: Option<Job>,
    race: Option<Race>,
    evals: usize,
    best: Option<Best>,
    offset: usize,
    sample_rate: f64,
    history: Vec<(usize, f64)>,
    mutate_share: usize,
    /// One line per optimisation run: stage, template, mutations, evaluations, best loss.
    trials: Vec<(String, Value)>,
}

impl Fitter {
    /// Starts a fit of mono `pcm` at `sample_rate` over template patch JSON texts.
    /// Err when the target is empty or silent, or when no template loads.
    pub fn new(pcm: &[f32], sample_rate: f64, templates: &[String], opts: FitOptions) -> Result<Fitter, String> {
        if !(sample_rate.is_finite() && (8000.0..=192_000.0).contains(&sample_rate)) {
            return Err(format!("bad sample rate {sample_rate}"));
        }
        let peak = pcm.iter().fold(0.0f32, |m, x| if x.is_finite() { m.max(x.abs()) } else { m });
        if pcm.is_empty() || peak <= 0.0 {
            return Err("target is empty or silent".to_string());
        }
        let offset = if opts.trim_lead { lead_offset(pcm, peak, sample_rate) } else { 0 };
        let target_pcm: Vec<f32> = pcm[offset..].iter().map(|x| if x.is_finite() { *x } else { 0.0 }).collect();
        let seconds = target_pcm.len() as f64 / sample_rate;
        let states: Vec<TemplateState> = templates.iter().enumerate().map(|(i, text)| load_template(i, text, seconds)).collect();
        if !states.iter().any(|t| t.prepared.is_some()) {
            return Err("no template loads".to_string());
        }
        let target = TargetFeatures::new(&target_pcm, sample_rate, &opts.loss);
        Ok(Fitter {
            renderer: Renderer::new(sample_rate, target_pcm.len()),
            target,
            target_pcm,
            gauss: Gauss::new(opts.seed.max(1)),
            opts,
            templates: states,
            stage: Stage::Screen,
            screen_next: 0,
            pending_align: None,
            queue: VecDeque::new(),
            current: None,
            race: None,
            evals: 0,
            best: None,
            offset,
            sample_rate,
            history: Vec::new(),
            mutate_share: 0,
            trials: Vec::new(),
        })
    }

    /// True once the fit has used its budget or run out of work.
    pub fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// Evaluations used so far.
    pub fn evals(&self) -> usize {
        self.evals
    }

    /// Best loss so far (`FAILED_LOSS` before the first playable render).
    pub fn best_loss(&self) -> f64 {
        self.best.as_ref().map_or(FAILED_LOSS, |b| b.terms.total)
    }

    /// Runs at most `n` evaluations; returns true while the fit is not done.
    pub fn step(&mut self, n: usize) -> bool {
        let mut left = n;
        while left > 0 && self.stage != Stage::Done {
            if self.evals >= self.opts.budget {
                self.stage = Stage::Done;
                break;
            }
            if self.advance() {
                left -= 1;
            }
        }
        self.stage != Stage::Done
    }

    /// Does one unit of work; true when it spent an evaluation.
    fn advance(&mut self) -> bool {
        match self.stage {
            Stage::Screen => self.screen(),
            Stage::Optimize | Stage::Mutate | Stage::Polish => {
                if self.current.is_none() {
                    match self.queue.pop_front() {
                        Some(job) => self.current = Some(job),
                        None => {
                            self.end_round();
                            return false;
                        }
                    }
                }
                let mut job = self.current.take().expect("job");
                if let Some(variant) = job.scan.pop_front() {
                    let terms = self.evaluate(&variant, job.template, &job.mutations, Value::Array(Vec::new()));
                    job.used += 1;
                    self.templates[job.template].evals += 1;
                    if job.scan_best.as_ref().is_none_or(|(f, _)| terms.total < *f) {
                        job.scan_best = Some((terms.total, variant));
                    }
                    if job.scan.is_empty() {
                        if let Some((f, base)) = job.scan_best.take() {
                            self.restart_from(&mut job, base);
                            job.best_f = f;
                        }
                    }
                    self.park(job);
                    return true;
                }
                let x = job.cma.ask().to_vec();
                let mut patch = job.base.clone();
                job.space.apply(&mut patch, &x);
                let values = job.space.values_json(&x);
                let terms = self.evaluate(&patch, job.template, &job.mutations, values);
                job.cma.tell(terms.total, &mut self.gauss);
                job.used += 1;
                if terms.total < job.best_f {
                    job.best_f = terms.total;
                    job.best_x = x;
                }
                let t = &mut self.templates[job.template];
                t.best = t.best.min(terms.total);
                t.evals += 1;
                if job.cma.at_generation_start() && job.cma.converged() {
                    // IPOP restart from the best point, population doubled.
                    let lambda = job.cma.lambda() * 2;
                    job.restarts += 1;
                    let steps = job.space.steps(self.target.uses_f0());
                    job.cma = Cma::new(&job.best_x, job.sigma, &steps, lambda, &mut self.gauss);
                }
                self.park(job);
                true
            }
            Stage::Done => false,
        }
    }

    /// Keeps a job running while it has budget, else hands it to the race.
    fn park(&mut self, job: Job) {
        let key = format!("{}|{}|{}", self.stage.name(), job.template, job.mutations.join("+"));
        let line = json!({
            "stage": self.stage.name(), "template": self.templates[job.template].id, "mutations": job.mutations,
            "dims": job.space.dims(), "evals": job.used, "restarts": job.restarts, "best": round6(job.best_f.min(FAILED_LOSS)),
        });
        match self.trials.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = line,
            None => self.trials.push((key, line)),
        }
        if job.used < job.budget {
            self.current = Some(job);
        } else if let Some(race) = &mut self.race {
            race.finished.push(job);
        }
    }

    /// Re-centres a job on a new base patch (its selected values become the new start point).
    fn restart_from(&mut self, job: &mut Job, base: Patch) {
        job.space = ParamSpace::select(&base, self.opts.max_dims, &job.prefer);
        let x0 = job.space.x0();
        job.cma = Cma::new(&x0, job.sigma, &job.space.steps(self.target.uses_f0()), 0, &mut self.gauss);
        job.best_x = x0;
        job.base = base;
    }

    /// Screening: the template as-is, then (when it changes anything) its measured alignment.
    fn screen(&mut self) -> bool {
        if let Some((i, aligned)) = self.pending_align.take() {
            let terms = self.evaluate(&aligned, i, &[], Value::Array(Vec::new()));
            let t = &mut self.templates[i];
            t.evals += 1;
            t.best = t.best.min(terms.total);
            if terms.total < t.score0 {
                t.score0 = terms.total;
                t.prepared = Some(aligned);
            } else {
                t.aligned.clear();
            }
            return true;
        }
        if self.screen_next >= self.templates.len() {
            self.plan_optimize();
            return false;
        }
        let i = self.screen_next;
        self.screen_next += 1;
        let Some(patch) = self.templates[i].prepared.clone() else { return false };
        let terms = self.evaluate(&patch, i, &[], Value::Array(Vec::new()));
        let t = &mut self.templates[i];
        t.score0 = terms.total;
        t.best = t.best.min(terms.total);
        t.evals += 1;
        if self.opts.align && terms.total < FAILED_LOSS {
            let mono = self.renderer.last().to_vec();
            let mut aligned = patch.clone();
            let mut notes = Vec::new();
            let target_f0 = self.target.target_f0();
            if self.target.uses_f0() && target_f0 > 0.0 {
                let f0 = self.target.median_f0(&mono);
                if f0 > 0.0 {
                    let ratio = (target_f0 / f0).clamp(0.125, 8.0);
                    if ratio.log2().abs() > 0.03 && transpose(&mut aligned, ratio) {
                        notes.push(format!("transpose x{}", round_sig(ratio)));
                    }
                }
            }
            let rms = rms_of(&mono);
            if rms > 1e-6 {
                let ratio = self.target.target_rms() / rms;
                if ratio.ln().abs() > 0.1 && scale_output(&mut aligned, ratio) {
                    notes.push(format!("output gain x{}", round_sig(ratio)));
                }
            }
            if !notes.is_empty() {
                self.templates[i].aligned = notes;
                self.pending_align = Some((i, aligned));
            }
        }
        true
    }

    fn remaining(&self) -> usize {
        self.opts.budget.saturating_sub(self.evals)
    }

    fn plan_optimize(&mut self) {
        let rest = self.remaining();
        self.mutate_share = if self.opts.mutations > 0 { rest * 3 / 10 } else { 0 };
        let polish_share = rest / 10;
        let optimize_share = rest - self.mutate_share - polish_share;
        let mut ranked: Vec<usize> = (0..self.templates.len()).filter(|i| self.templates[*i].prepared.is_some()).collect();
        ranked.sort_by(|a, b| {
            self.templates[*a].score0.partial_cmp(&self.templates[*b].score0).unwrap_or(core::cmp::Ordering::Equal).then(a.cmp(b))
        });
        ranked.truncate(self.opts.top_k.max(1));
        let mut jobs = Vec::new();
        for i in ranked {
            let base = self.templates[i].prepared.clone().expect("prepared");
            if let Some(job) = self.job(base, i, Vec::new(), &[], self.opts.sigma) {
                jobs.push(job);
            }
        }
        self.stage = Stage::Optimize;
        self.start_race(jobs, optimize_share);
    }

    /// Starts a successive-halving race: round r splits `share / rounds` among the jobs still in.
    /// Jobs come in rank order; the last ones are dropped until every job's first round holds at
    /// least three generations (plus its scan).
    fn start_race(&mut self, mut jobs: Vec<Job>, share: usize) {
        while jobs.len() > 1 {
            let k = jobs.len();
            let rounds = (usize::BITS - (k - 1).leading_zeros()) as usize + 1;
            let need = jobs.iter().map(|j| 3 * j.cma.lambda() + j.scan.len()).max().unwrap_or(0);
            if share / rounds / k >= need {
                break;
            }
            jobs.pop();
        }
        let k = jobs.len();
        if k == 0 || share == 0 {
            self.race = Some(Race { end: self.evals, round_share: 0, finished: Vec::new() });
            return;
        }
        let rounds = (usize::BITS - (k - 1).leading_zeros()) as usize + 1;
        let round_share = share / rounds;
        let end = self.evals + share;
        let per = if k == 1 { share } else { (round_share / k).max(1) };
        for mut job in jobs {
            job.budget = per;
            self.queue.push_back(job);
        }
        self.race = Some(Race { end, round_share, finished: Vec::new() });
    }

    /// The queue is empty: halve the race, or move to the next stage.
    fn end_round(&mut self) {
        let Some(mut race) = self.race.take() else {
            self.next_stage();
            return;
        };
        if self.stage != Stage::Polish && race.finished.len() > 1 && self.evals < race.end {
            race.finished.sort_by(|a, b| a.best_f.partial_cmp(&b.best_f).unwrap_or(core::cmp::Ordering::Equal));
            // The better half goes on, minus any job 1.5x worse than the leader.
            let lead = race.finished[0].best_f;
            let close = race.finished.iter().take_while(|j| j.best_f <= 1.5 * lead).count().max(1);
            let keep = race.finished.len().div_ceil(2).min(close);
            race.finished.truncate(keep);
            let left = race.end - self.evals;
            let per = if keep == 1 { left } else { (race.round_share / keep).clamp(1, left) };
            for mut job in race.finished.drain(..) {
                job.budget = job.used + per;
                self.queue.push_back(job);
            }
            self.race = Some(race);
            return;
        }
        self.next_stage();
    }

    fn next_stage(&mut self) {
        match self.stage {
            Stage::Optimize => {
                self.stage = Stage::Mutate;
                let jobs = self.mutation_jobs();
                let share = self.mutate_share;
                self.start_race(jobs, share);
            }
            Stage::Mutate => {
                self.stage = Stage::Polish;
                let budget = self.remaining();
                let job = self.best.as_ref().map(|b| (b.patch.clone(), b.template, b.mutations.clone()));
                let mut jobs = Vec::new();
                if let Some((patch, template, mutations)) = job {
                    if let Some(job) = self.job(patch, template, mutations, &[], self.opts.sigma * 0.3) {
                        jobs.push(job);
                    }
                }
                self.start_race(jobs, budget);
            }
            _ => self.stage = Stage::Done,
        }
    }

    fn mutation_jobs(&mut self) -> Vec<Job> {
        let Some(best) = &self.best else { return Vec::new() };
        if self.opts.mutations == 0 {
            return Vec::new();
        }
        let (base, template, done) = (best.patch.clone(), best.template, best.mutations.clone());
        // Rank the mutations by what the best render lacks or has too much of (mel residual).
        let Some(mono) = self.renderer.render(&base).map(<[f32]>::to_vec) else { return Vec::new() };
        self.target.loss(&mono);
        let residual = self.target.mel_residual_db();
        let tail = self.target.tail_residual_db(0.3);
        let pitched = self.target.uses_f0().then(|| self.target.target_f0()).filter(|f| *f > 0.0);
        let mut ranked: Vec<(f64, usize, mutate::MutationKind)> = mutate::candidates(&base)
            .into_iter()
            .enumerate()
            .map(|(i, k)| (mutation_score(&k, &residual, tail, pitched), i, k))
            .collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(core::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
        let mut picked = Vec::new();
        for (_, _, kind) in ranked {
            if picked.len() >= self.opts.mutations {
                break;
            }
            if let Some(m) = mutate::apply(&base, &kind) {
                picked.push(m);
            }
        }
        let mut jobs = Vec::new();
        for m in picked {
            let mut chain = done.clone();
            chain.push(m.label.clone());
            let scan = mutate::level_scan(&m);
            if let Some(mut job) = self.job(m.patch, template, chain, &m.added, self.opts.sigma) {
                job.scan = scan.into();
                jobs.push(job);
            }
        }
        jobs
    }

    fn job(&mut self, base: Patch, template: usize, mutations: Vec<String>, prefer: &[String], sigma: f64) -> Option<Job> {
        let space = ParamSpace::select(&base, self.opts.max_dims, prefer);
        if space.dims() == 0 {
            return None;
        }
        let x0 = space.x0();
        let cma = Cma::new(&x0, sigma, &space.steps(self.target.uses_f0()), 0, &mut self.gauss);
        Some(Job {
            base,
            template,
            mutations,
            space,
            cma,
            sigma,
            budget: 0,
            used: 0,
            best_x: x0,
            best_f: f64::INFINITY,
            restarts: 0,
            scan: VecDeque::new(),
            scan_best: None,
            prefer: prefer.to_vec(),
        })
    }

    fn evaluate(&mut self, patch: &Patch, template: usize, mutations: &[String], values: Value) -> LossTerms {
        self.evals += 1;
        let terms = match self.renderer.render(patch) {
            Some(mono) => self.target.loss(mono),
            None => LossTerms::failed(),
        };
        if terms.total < self.best_loss() {
            self.history.push((self.evals, round6(terms.total)));
            self.best = Some(Best { patch: patch.clone(), terms, template, mutations: mutations.to_vec(), values });
        }
        terms
    }

    /// Progress as JSON: stage, evaluations, budget, best loss and its template.
    pub fn progress_json(&self) -> String {
        let best = self.best.as_ref();
        json!({
            "stage": self.stage.name(),
            "done": self.done(),
            "evals": self.evals,
            "budget": self.opts.budget,
            "best": best.map(|b| round6(b.terms.total)),
            "template": best.map(|b| self.templates[b.template].id.clone()),
            "mutations": best.map(|b| b.mutations.clone()),
            "job": self.current.as_ref().map(|j| json!({
                "template": self.templates[j.template].id, "dims": j.space.dims(), "used": j.used,
                "budget": j.budget, "restarts": j.restarts, "sigma": round6(j.cma.sigma()),
            })),
        })
        .to_string()
    }

    /// The best patch as HAA patch JSON (2-space, trailing newline), or None before any render.
    pub fn best_patch_json(&self) -> Option<String> {
        self.best.as_ref().map(|b| to_json(&result_patch(b)))
    }

    /// The full result as JSON (works before `done` too: it then reports the best so far).
    pub fn result_json(&mut self) -> String {
        let report = self.best.as_ref().map(|b| b.patch.clone()).and_then(|patch| {
            let mono = self.renderer.render(&patch)?.to_vec();
            Some(crate::loss::compare(&self.target_pcm, &mono, self.sample_rate, 16))
        });
        let best = self.best.as_ref().map(|b| {
            let patch: Value = serde_json::from_str(&to_json(&result_patch(b))).unwrap_or(Value::Null);
            json!({
                "score": round6(b.terms.total), "terms": b.terms.to_json(), "template": b.template,
                "templateId": self.templates[b.template].id, "mutations": b.mutations, "params": b.values,
                "aligned": self.templates[b.template].aligned, "patch": patch,
            })
        });
        let templates: Vec<Value> = self
            .templates
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut v = json!({
                    "index": i, "id": t.id, "playable": t.prepared.is_some(), "evals": t.evals,
                    "score0": round6(t.score0), "best": round6(t.best), "aligned": t.aligned,
                });
                if let Some(e) = &t.error {
                    v["error"] = Value::from(e.clone());
                }
                v
            })
            .collect();
        json!({
            "done": self.done(),
            "evals": self.evals,
            "budget": self.opts.budget,
            "seed": self.opts.seed,
            "sampleRate": self.sample_rate,
            "offset": round6(self.offset as f64 / self.sample_rate),
            "seconds": round6(self.target_pcm.len() as f64 / self.sample_rate),
            "usesF0": self.target.uses_f0(),
            "targetF0": round6(self.target.target_f0()),
            "best": best,
            "templates": templates,
            "history": self.history.iter().map(|(e, l)| json!([e, l])).collect::<Vec<_>>(),
            "trials": self.trials.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            "report": report,
        })
        .to_string()
    }
}

fn load_template(i: usize, text: &str, seconds: f64) -> TemplateState {
    let state = |id: String, prepared: Option<Patch>, error: Option<String>| TemplateState {
        id,
        prepared,
        error,
        score0: FAILED_LOSS,
        aligned: Vec::new(),
        best: FAILED_LOSS,
        evals: 0,
    };
    match haa_core::load_patch(text) {
        Err(issue) => state(format!("#{i}"), None, Some(format!("{}: {}", issue.code, issue.message))),
        Ok((mut patch, issues)) => {
            let id = patch.id.clone();
            if has_errors(&issues) {
                let error = issues.iter().find(|i| i.severity == Severity::Error).map(|i| format!("{}: {}", i.code, i.message));
                state(id, None, error)
            } else {
                prepare(&mut patch, seconds);
                state(id, Some(patch), None)
            }
        }
    }
}

/// The patch the caller gets: the fitted copy, with `-fit` on its id and `(fit)` on its title.
fn result_patch(b: &Best) -> Patch {
    let mut p = b.patch.clone();
    if !p.id.ends_with("-fit") {
        p.id = format!("{}-fit", p.id);
    }
    if !p.title.ends_with("(fit)") {
        p.title = format!("{} (fit)", p.title);
    }
    p
}

/// Multiplies the `frequency`/`pitch` base value of every source node by `ratio` (clamped to the
/// catalog range). True when a value changed.
pub fn transpose(patch: &mut Patch, ratio: f64) -> bool {
    let mut changed = false;
    for node in &mut patch.graph.nodes {
        let Some(def) = catalog::find(&node.kind) else { continue };
        if def.category != Category::Source {
            continue;
        }
        for p in def.params {
            let catalog::ParamKind::Number { min, max, default, .. } = p.kind else { continue };
            if p.key != "frequency" && p.key != "pitch" {
                continue;
            }
            let v = node.params.get(p.key).and_then(Value::as_f64).unwrap_or(default);
            let next = round_sig((v * ratio).clamp(min, max));
            if next != v {
                node.params.insert(p.key.to_string(), Value::from(next));
                changed = true;
            }
        }
    }
    changed
}

/// Multiplies the first output node's `gain` by `ratio` (clamped to its catalog range).
pub fn scale_output(patch: &mut Patch, ratio: f64) -> bool {
    let Some(node) = patch.graph.nodes.iter_mut().find(|n| n.kind == "output" && n.muted != Some(true)) else { return false };
    let Some(def) = catalog::find("output") else { return false };
    let Some(catalog::ParamKind::Number { min, max, default, .. }) = def.params.iter().find(|p| p.key == "gain").map(|p| p.kind) else {
        return false;
    };
    let v = node.params.get("gain").and_then(Value::as_f64).unwrap_or(default);
    let next = round_sig((v * ratio).clamp(min, max));
    if next == v {
        return false;
    }
    node.params.insert("gain".to_string(), Value::from(next));
    true
}

/// First sample louder than −50 dB re peak, minus 2 ms.
fn lead_offset(pcm: &[f32], peak: f32, sample_rate: f64) -> usize {
    let threshold = peak * 0.003_162_3;
    let first = pcm.iter().position(|x| x.abs() >= threshold).unwrap_or(0);
    first.saturating_sub((sample_rate * 0.002) as usize)
}

/// How likely a mutation fixes the residual: positive where the candidate lacks energy (a layer
/// adds it), negative where it has too much (a filter removes it); removes and colour effects rank
/// low and keep their catalog order.
fn mutation_score(kind: &mutate::MutationKind, residual: &[(f64, f64)], tail_db: f64, pitched: Option<f64>) -> f64 {
    let band = |lo: f64, hi: f64| -> f64 {
        let v: Vec<f64> = residual.iter().filter(|(c, _)| *c >= lo && *c < hi).map(|(_, r)| *r).collect();
        if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
    };
    let (low, mid, high) = (band(0.0, 300.0), band(300.0, 3000.0), band(3000.0, f64::INFINITY));
    let pos = |x: f64| x.max(0.0);
    match kind {
        mutate::MutationKind::Layer("noise", overrides, _) if overrides.contains(&("color", "pink")) => pos(mid) + 0.5 * pos(low),
        mutate::MutationKind::Layer("noise", ..) => pos(high) + 0.5 * pos(mid),
        mutate::MutationKind::Layer(..) => pitched.map_or(0.0, |f0| pos(band(f0 / 1.2, f0 * 1.2))),
        mutate::MutationKind::Insert("filter", overrides, _) => {
            let mode = overrides.iter().find(|(k, _)| *k == "mode").map_or("", |(_, v)| *v);
            match mode {
                "lowpass" => pos(-high),
                "highpass" => pos(-low),
                _ => pos(-high).min(pos(-low)) * 2.0,
            }
        }
        mutate::MutationKind::Insert("space", ..) => pos(tail_db) / 2.0,
        mutate::MutationKind::Insert("equalizer", ..) => 0.25 * (low - high).abs(),
        mutate::MutationKind::Insert(..) => 0.1,
        mutate::MutationKind::Remove(_) => 0.05,
    }
}

/// Runs a whole fit in one call (native tools and tests; a browser host steps instead).
pub fn fit(pcm: &[f32], sample_rate: f64, templates: &[String], opts: FitOptions) -> Result<String, String> {
    let mut f = Fitter::new(pcm, sample_rate, templates, opts)?;
    while f.step(64) {}
    Ok(f.result_json())
}
