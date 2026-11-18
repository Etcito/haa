//! Articulator curves from the sound, with no text: acoustic-to-articulatory inversion.
//!
//! SPARC (Cho et al. 2024, "Coding Speech through Vocal Tract Kinematics", arXiv 2406.12998; Berkeley Speech Group,
//! Speech-Articulatory-Coding) reads 12 EMA coordinates at 50 Hz off WavLM-Large's 9th layer through one linear head:
//! tongue dorsum, blade and tip, lower incisor (the jaw), upper and lower lip, each x (forward) and y (up), in the
//! z-scored frame of the MNGU0 speaker. The network runs as ONNX (the caller's runtime); what follows is Rust:
//!
//! - [`ema_lowpass`]: SPARC's own 10 Hz Butterworth lowpass (order 5, zero phase, scipy `filtfilt` padding). Its head
//!   is linear, so filtering the 12 outputs equals filtering the 1024 features as SPARC does;
//! - [`fit_linear`]: ridge least squares, used once to calibrate the EMA -> chimera map by analysis by synthesis (chimera
//!   poses rendered, inverted by the same SPARC, the poses regressed on the EMA it read);
//! - [`ema_to_artic`]: that map, the chimera `articulate` mode's articulator inputs per EMA frame;
//! - [`syllable_bounds`] and [`artic_keyframes`]: each articulator per phrase as a few keyframes per syllable
//!   (Ramer-Douglas-Peucker inside each syllable, at most `max_per` points each).

/// SPARC's EMA channels, in its output order.
pub const EMA_NAMES: [&str; 12] = ["TDX", "TDY", "TBX", "TBY", "TTX", "TTY", "LIX", "LIY", "ULX", "ULY", "LLX", "LLY"];
/// SPARC's EMA rate (Hz): one frame per 320 samples at 16 kHz.
pub const EMA_RATE: f64 = 50.0;

/// scipy `butter(5, 10, fs=50)`: SPARC's articulatory lowpass.
const BUTTER_B: [f64; 6] = [0.02193962068846417, 0.10969810344232085, 0.2193962068846417, 0.2193962068846417, 0.10969810344232085, 0.02193962068846417];
const BUTTER_A: [f64; 6] = [1.0, -0.9853252392792379, 0.9738493318367641, -0.38635655864844876, 0.111163840578342, -0.011263512456565873];
/// scipy `lfilter_zi(b, a)`: the filter's state for a unit step.
const BUTTER_ZI: [f64; 5] = [0.9780603793115357, -0.1169629634100231, 0.6374901615420994, 0.0317373960090089, 0.033203133145030046];

/// One pass of the Butterworth (direct form II transposed), its state started at `zi · x[0]`.
fn lfilter(x: &[f64]) -> Vec<f64> {
    let Some(&x0) = x.first() else { return Vec::new() };
    let mut z = BUTTER_ZI.map(|v| v * x0);
    x.iter()
        .map(|&v| {
            let y = BUTTER_B[0] * v + z[0];
            for i in 0..4 {
                z[i] = BUTTER_B[i + 1] * v + z[i + 1] - BUTTER_A[i + 1] * y;
            }
            z[4] = BUTTER_B[5] * v - BUTTER_A[5] * y;
            y
        })
        .collect()
}

/// SPARC's lowpass on `ema` (`frames` rows of 12, row-major): zero phase, as scipy `filtfilt` (odd extension of
/// 18 frames each side, fewer for a shorter clip).
pub fn ema_lowpass(ema: &[f32], frames: usize) -> Vec<f32> {
    let n = frames.min(ema.len() / 12);
    let mut out = vec![0.0f32; n * 12];
    if n == 0 {
        return out;
    }
    let pad = 18.min(n - 1);
    for c in 0..12 {
        let x: Vec<f64> = (0..n).map(|i| f64::from(ema[i * 12 + c])).collect();
        let mut ext = Vec::with_capacity(n + 2 * pad);
        for i in (1..=pad).rev() {
            ext.push(2.0 * x[0] - x[i]);
        }
        ext.extend_from_slice(&x);
        for i in 1..=pad {
            ext.push(2.0 * x[n - 1] - x[n - 1 - i]);
        }
        let mut y = lfilter(&ext);
        y.reverse();
        let mut y = lfilter(&y);
        y.reverse();
        for i in 0..n {
            out[i * 12 + c] = y[pad + i] as f32;
        }
    }
    out
}

/// Ridge least squares: `y` (`n` rows of `dy`) ≈ `[x, 1]` (`n` rows of `dx`, plus a constant) times the returned
/// coefficients, `dy` rows of `dx + 1` (the constant last). `ridge` is added to the diagonal of the normal equations
/// (relative to their mean diagonal), the constant left free.
pub fn fit_linear(x: &[f64], y: &[f64], n: usize, dx: usize, dy: usize, ridge: f64) -> Vec<f64> {
    let d = dx + 1;
    let mut g = vec![0.0; d * d];
    let mut r = vec![0.0; d * dy];
    let mut row = vec![0.0; d];
    for s in 0..n {
        row[..dx].copy_from_slice(&x[s * dx..(s + 1) * dx]);
        row[dx] = 1.0;
        for i in 0..d {
            for j in 0..d {
                g[i * d + j] += row[i] * row[j];
            }
            for k in 0..dy {
                r[i * dy + k] += row[i] * y[s * dy + k];
            }
        }
    }
    let mean_diag = (0..dx).map(|i| g[i * d + i]).sum::<f64>() / dx.max(1) as f64;
    for i in 0..dx {
        g[i * d + i] += ridge * mean_diag;
    }
    // Gaussian elimination with partial pivoting, all right-hand sides at once
    for col in 0..d {
        let p = (col..d).max_by(|&a, &b| g[a * d + col].abs().total_cmp(&g[b * d + col].abs())).unwrap_or(col);
        if p != col {
            for j in 0..d {
                g.swap(col * d + j, p * d + j);
            }
            for k in 0..dy {
                r.swap(col * dy + k, p * dy + k);
            }
        }
        let piv = g[col * d + col];
        if piv.abs() < 1e-12 {
            continue;
        }
        for i in 0..d {
            if i == col {
                continue;
            }
            let f = g[i * d + col] / piv;
            if f == 0.0 {
                continue;
            }
            for j in col..d {
                g[i * d + j] -= f * g[col * d + j];
            }
            for k in 0..dy {
                r[i * dy + k] -= f * r[col * dy + k];
            }
        }
    }
    let mut coef = vec![0.0; dy * d];
    for k in 0..dy {
        for i in 0..d {
            let piv = g[i * d + i];
            coef[k * d + i] = if piv.abs() < 1e-12 { 0.0 } else { r[i * dy + k] / piv };
        }
    }
    coef
}

/// The chimera `articulate`-mode inputs the map writes, with the node's ranges.
pub const ARTIC_INPUTS: [(&str, f64, f64); 8] = [
    ("jaw", -1.5, 1.5),
    ("tongueHeight", -2.0, 2.0),
    ("tongueFront", -2.0, 2.0),
    ("lipRound", -1.0, 1.0),
    ("lipOpen", -1.5, 1.5),
    ("lipSpread", -1.0, 1.0),
    ("tipUp", -2.0, 4.0),
    ("tipFront", -2.0, 2.0),
];

/// The EMA -> chimera map: per input, 12 EMA weights (in [`EMA_NAMES`] order) then a constant; lowpassed SPARC EMA in,
/// node units out. Calibrated by analysis by synthesis: 120 renders of the male chimera body in `articulate` mode
/// (2880 held poses: phoneme-table vowels, consonants over vowels, uniform random poses; pitch 100-160 Hz) inverted
/// by the same SPARC, ridge least squares ([`fit_linear`], 1e-3) on the 7668 audible frames inside the holds.
/// Cross-validated (5 folds by render) RMSE, node units (share of the poses' variance explained): jaw 0.46 (32%),
/// tongueHeight 0.41 (61%), tongueFront 0.38 (75%), lipRound 0.35 (33%), lipOpen 0.40 (57%), lipSpread 0.32 (34%),
/// tipUp 0.87 (42%), tipFront 0.54 (16%). Much of the rest is the model's own redundancy (a jaw against a tongue
/// height, a rounding against a lip opening sound alike); a quadratic map gained 2-7 points.
pub const ARTIC_MAP: [[f64; 13]; 8] = [
    [0.271502, -0.208067, -0.428390, -0.047649, 0.185511, 0.150008, 0.071440, -0.201457, 0.025280, -0.031344, -0.046994, 0.056425, 0.267058], // jaw
    [0.198201, 0.243516, -0.334531, 0.186599, 0.276962, 0.033482, -0.012603, -0.125163, -0.104476, 0.045335, 0.043437, -0.022281, -0.145636], // tongueHeight
    [-0.238640, -0.847921, -0.096420, 1.002458, 0.118363, -0.148730, 0.125417, 0.150910, 0.063668, -0.225272, -0.159671, -0.310645, -0.275598], // tongueFront
    [0.095965, -0.022055, -0.207052, -0.017870, 0.149508, 0.043891, 0.008129, 0.181726, -0.237689, -0.021366, 0.035949, -0.173827, 0.023776], // lipRound
    [0.133809, 0.120946, -0.204724, -0.249336, 0.091655, 0.163864, -0.320355, -0.090150, -0.028372, 0.165081, 0.373670, -0.189351, 0.386822], // lipOpen
    [0.024483, 0.060041, -0.024732, -0.021114, -0.028261, -0.002263, -0.014944, -0.103812, 0.162765, 0.010650, -0.052738, -0.028984, 0.088492], // lipSpread
    [0.011600, 0.435522, -0.487626, -0.979417, 0.547945, 0.980483, -0.179397, -0.163053, 0.249345, 0.171711, 0.027399, 0.090838, 1.076273], // tipUp
    [0.144712, 0.021851, -0.317086, -0.195260, -0.025230, -0.091119, -0.141132, 0.167545, -0.051773, 0.059599, 0.299453, 0.111516, 0.054651], // tipFront
];

/// The map on lowpassed EMA (`frames` rows of 12): per frame the 8 inputs of [`ARTIC_INPUTS`], within their ranges.
pub fn ema_to_artic(ema: &[f32], frames: usize) -> Vec<[f64; 8]> {
    ema_to_artic_with(ema, frames, &ARTIC_MAP, 1.0)
}

/// [`ema_to_artic`] with any map, its EMA-driven part times `gain` (1 = as fitted; a least-squares map shrinks toward
/// the calibration's mean what the EMA leaves ambiguous, so > 1 gives some of that movement back).
pub fn ema_to_artic_with(ema: &[f32], frames: usize, map: &[[f64; 13]; 8], gain: f64) -> Vec<[f64; 8]> {
    (0..frames.min(ema.len() / 12))
        .map(|k| {
            let e = &ema[k * 12..k * 12 + 12];
            let mut v = [0.0; 8];
            for (i, row) in map.iter().enumerate() {
                let x = row[12] + gain * (0..12).map(|j| row[j] * f64::from(e[j])).sum::<f64>();
                v[i] = x.clamp(ARTIC_INPUTS[i].1, ARTIC_INPUTS[i].2);
            }
            v
        })
        .collect()
}

/// Syllables of a phrase from its sonority, per `hop` frame: the level (dB) plus how open the mouth is (`opening`,
/// in the EMA's z units: the jaw lowered and the lips apart), smoothed over 50 ms. A syllable nucleus is a sonority
/// peak standing at least `prominence` above the dips on both sides; the syllables are cut at the deepest dip between
/// two nuclei, at least `min_len` s apart. Returns the cut frames, the phrase's first and last frame included.
pub fn syllable_bounds(level_db: &[f32], opening: &[f32], hop: f64, prominence: f64, min_len: f64) -> Vec<usize> {
    let n = level_db.len().min(opening.len());
    if n < 3 {
        return if n == 0 { Vec::new() } else { vec![0, n - 1] };
    }
    let raw: Vec<f64> = (0..n).map(|i| f64::from(level_db[i]) / 6.0 + f64::from(opening[i])).collect();
    let w = ((0.025 / hop).round() as usize).max(1);
    let s: Vec<f64> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(w), (i + w).min(n - 1));
            raw[a..=b].iter().sum::<f64>() / (b - a + 1) as f64
        })
        .collect();
    let gap = ((min_len / hop).round() as usize).max(1);
    // nuclei: local maxima, merged left to right while the dip between two is too shallow or they are too close
    let mut peaks: Vec<usize> = (1..n - 1).filter(|&i| s[i] >= s[i - 1] && s[i] > s[i + 1]).collect();
    let mut kept: Vec<usize> = Vec::new();
    for p in peaks.drain(..) {
        let Some(&q) = kept.last() else {
            kept.push(p);
            continue;
        };
        let dip = s[q..=p].iter().copied().fold(f64::INFINITY, f64::min);
        if p - q < gap || s[q] - dip < prominence || s[p] - dip < prominence {
            if s[p] > s[q] {
                *kept.last_mut().unwrap() = p;
            }
        } else {
            kept.push(p);
        }
    }
    let mut bounds = vec![0];
    for w in kept.windows(2) {
        let (a, b) = (w[0], w[1]);
        let cut = (a..=b).min_by(|&x, &y| s[x].total_cmp(&s[y])).unwrap_or(a);
        if cut > *bounds.last().unwrap() && cut < n - 1 {
            bounds.push(cut);
        }
    }
    bounds.push(n - 1);
    bounds
}

/// Keyframes of one articulator curve `y` (one value per frame) cut into syllables at `bounds` (frames, first and
/// last included): inside each syllable Ramer-Douglas-Peucker from its two ends, the farthest frame from the line
/// through its neighbours added until every frame is within `tol` or the syllable holds `max_per` points (its first
/// frame counted, its last belonging to the next). Returns the kept frames in order.
pub fn artic_keyframes(y: &[f32], bounds: &[usize], max_per: usize, tol: f32) -> Vec<usize> {
    let n = y.len();
    if n == 0 {
        return Vec::new();
    }
    let mut out: Vec<usize> = Vec::new();
    let cuts: Vec<usize> = if bounds.len() >= 2 { bounds.iter().map(|&b| b.min(n - 1)).collect() } else { vec![0, n - 1] };
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b <= a {
            continue;
        }
        let mut keep = vec![a, b];
        // the syllable's own points: its first frame plus what is added (its last frame is the next one's first)
        while keep.len() - 1 < max_per.max(1) {
            let mut worst = (0.0f32, 0usize);
            for k in keep.windows(2) {
                let (p, q) = (k[0], k[1]);
                for i in p + 1..q {
                    let t = (i - p) as f32 / (q - p) as f32;
                    let e = (y[i] - (y[p] + t * (y[q] - y[p]))).abs();
                    if e > worst.0 {
                        worst = (e, i);
                    }
                }
            }
            if worst.0 <= tol {
                break;
            }
            let pos = keep.partition_point(|&k| k < worst.1);
            keep.insert(pos, worst.1);
        }
        if out.last() == Some(&a) {
            out.pop();
        }
        out.extend_from_slice(&keep);
    }
    out
}

/// A phrase's articulator curves (see [`artic_curves`]).
pub struct ArticCurves {
    /// per input of [`ARTIC_INPUTS`], its value per hop frame from the phrase's start
    pub frames: [Vec<f64>; 8],
    /// per input, its keyframes (hop frames)
    pub keys: [Vec<usize>; 8],
    /// the syllable cuts (hop frames, the first and last frame included)
    pub bounds: Vec<usize>,
    /// how open the mouth is per hop frame (z units: the jaw lowered, the lips apart)
    pub opening: Vec<f64>,
}

/// The articulation gain [`artic_curves`] plays the map at (see [`ema_to_artic_with`]).
pub const ARTIC_GAIN: f64 = 1.0;
/// Each input's keyframe tolerance, as a share of its range.
pub const KEY_TOL: f64 = 0.02;
/// Syllables: the sonority dip between two nuclei (level / 6 dB + opening), and their least distance (s).
pub const SYLLABLE_PROMINENCE: f64 = 0.5;
pub const SYLLABLE_MIN: f64 = 0.1;

/// The mouth's opening from one lowpassed EMA row: the jaw lowered (the lower incisor's y down) and the lips apart
/// (upper lip y minus lower lip y), in SPARC's z units.
pub fn ema_opening(e: &[f32]) -> f64 {
    0.5 * -f64::from(e[7]) + 0.25 * (f64::from(e[9]) - f64::from(e[11]))
}

/// One phrase's articulator curves from SPARC's raw EMA of a clip (`ema` rows of 12 at [`EMA_RATE`], row 0 at
/// `offset` s; a row stands for the 20 ms from its time): SPARC's lowpass, the map to the `articulate` inputs, both read
/// at each `hop` frame of [`start`, `end`] (patch s) `lead` s later (the tract's dynamics lag their targets), the
/// syllables from the sonority (`level`: amplitude 0..1 per hop frame from `start`, and the mouth's opening), and per
/// input at most `max_per` keyframes per syllable within [`KEY_TOL`] of its range.
pub fn artic_curves(ema: &[f32], offset: f64, start: f64, end: f64, hop: f64, level: &[f32], max_per: usize, lead: f64) -> ArticCurves {
    artic_curves_with(ema, offset, start, end, hop, level, max_per, lead, &ARTIC_MAP, ARTIC_GAIN)
}

/// [`artic_curves`] with any map and gain (see [`ema_to_artic_with`]).
#[allow(clippy::too_many_arguments)]
pub fn artic_curves_with(ema: &[f32], offset: f64, start: f64, end: f64, hop: f64, level: &[f32], max_per: usize, lead: f64, map: &[[f64; 13]; 8], gain: f64) -> ArticCurves {
    let rows = ema.len() / 12;
    let lp = ema_lowpass(ema, rows);
    let mapped = ema_to_artic_with(&lp, rows, map, gain);
    let n = (((end - start) / hop).round() as usize + 1).max(2);
    // fractional EMA row of patch time t (a row's centre is 10 ms after its start, the analysis window's middle)
    let at = |t: f64| ((t + lead - offset) * EMA_RATE - 0.5).clamp(0.0, (rows.max(1) - 1) as f64);
    let lerp = |r: f64, f: &dyn Fn(usize) -> f64| {
        let i = r.floor() as usize;
        let j = (i + 1).min(rows.max(1) - 1);
        let w = r - i as f64;
        f(i) * (1.0 - w) + f(j) * w
    };
    let frames: [Vec<f64>; 8] = std::array::from_fn(|k| (0..n).map(|i| lerp(at(start + i as f64 * hop), &|q| mapped.get(q).map_or(0.0, |m| m[k]))).collect());
    let opening: Vec<f64> = (0..n).map(|i| lerp(at(start + i as f64 * hop), &|q| if q < rows { ema_opening(&lp[q * 12..q * 12 + 12]) } else { 0.0 })).collect();
    let level_db: Vec<f32> = (0..n).map(|i| 20.0 * level.get(i).copied().unwrap_or(0.0).max(1e-4).log10()).collect();
    let open32: Vec<f32> = opening.iter().map(|&v| v as f32).collect();
    let bounds = syllable_bounds(&level_db, &open32, hop, SYLLABLE_PROMINENCE, SYLLABLE_MIN);
    let keys: [Vec<usize>; 8] = std::array::from_fn(|k| {
        let y: Vec<f32> = frames[k].iter().map(|&v| v as f32).collect();
        let (_, lo, hi) = ARTIC_INPUTS[k];
        artic_keyframes(&y, &bounds, max_per, (KEY_TOL * (hi - lo)) as f32)
    });
    ArticCurves { frames, keys, bounds, opening }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn articulator_curves_keep_a_few_keyframes_per_syllable_within_range() {
        // 3 s of EMA at 50 Hz: the jaw and tip moving at 4 Hz (a syllable each 250 ms), the level following the opening
        let rows = 150;
        let mut ema = vec![0.0f32; rows * 12];
        for r in 0..rows {
            let ph = (r as f32 / 50.0 * 4.0 * std::f32::consts::TAU).sin();
            ema[r * 12 + 7] = -2.0 * ph; // LIY: the jaw opens
            ema[r * 12 + 5] = 2.0 * ph; // TTY
            ema[r * 12 + 9] = 0.5 * ph;
        }
        let hop = 0.01;
        let level: Vec<f32> = (0..=250).map(|i| 0.55 + 0.45 * (i as f32 * hop as f32 * 4.0 * std::f32::consts::TAU).sin()).collect();
        let c = artic_curves(&ema, 0.0, 0.25, 2.75, hop, &level, 6, 0.03);
        let syllables = c.bounds.len() - 1;
        assert!((7..=12).contains(&syllables), "{syllables} syllables");
        for (k, keys) in c.keys.iter().enumerate() {
            for w in c.bounds.windows(2) {
                let n = keys.iter().filter(|&&q| q >= w[0] && q < w[1]).count();
                assert!(n <= 6 + 1, "{} has {n} keyframes in one syllable", ARTIC_INPUTS[k].0);
            }
            assert!(c.frames[k].iter().all(|v| (ARTIC_INPUTS[k].1..=ARTIC_INPUTS[k].2).contains(v)));
        }
        // the lowpass is scipy's filtfilt: a constant goes through unchanged
        let flat = ema_lowpass(&vec![1.5f32; 60 * 12], 60);
        assert!(flat.iter().all(|v| (v - 1.5).abs() < 1e-4));
    }
}
