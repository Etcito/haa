"""Signal detectors for the defect classes the owner named, run on captures of the scene's output.

    detectors.py <wav> [<wav> ...] [--json out.json]

Each returns a number and the detector's threshold flags it (thresholds validated on real recordings and on the
owner's failing captures: tools/judge/validate.py):
- tail_tonality_db  "organ": the late reverb tail's narrow stationary peaks, prominence (dB) over the local spectral
                    median of the tail's averaged spectrum, kept only if the peak stands out in most tail frames.
- ringing_s         "bell": the longest-lived narrow partial above 400 Hz (s), a peak held at the same frequency.
- lf_hang_s         "thunder": how long after the onset the band under 250 Hz stays within 25 dB of its own maximum (s).
- lf_tail_db        the energy under 250 Hz after the first 250 ms, against the whole first 250 ms (dB).
- decay_s           "too long": from the last moment within 10 dB of the peak to 60 dB under it (100 ms RMS), extrapolated
                    from the slope when the clip ends first.
- audible_s         how long the output stays above -70 dBFS after the onset (what a listener at full volume hears).
- clipped / clicks  samples held at the ceiling (|x| >= 0.97 for 3+ samples) and isolated discontinuities per s (reported;
                    real recordings have many, so "sature" comes from the take's underruns and thinned blocks).
flags_for() turns these into the owner's words, scoped to the scenario (APPLIES).
"""
import argparse
import json
import sys

import numpy as np
import soundfile as sf
from scipy.signal import stft

# Validated (validate.py, tonality on 95 real reverberant tails, ringing on 226 real recordings incl. 17 metal hits,
# the low band on 70 real small impacts and rolls): real p95 / max, then the owner's failing captures.
THRESHOLDS = {
    "tail_tonality_db": 11.5,  # real reverberant tails: median 5.7, p95 11.0, max 17.6 (a stone shaft); 3/95 over
    "ringing_s": 0.6,          # real: median 0.05, p95 0.48; 2/17 real metal hits over (1.1 s)
    "lf_hang_s": 1.2,          # with lf_tail_db >= 5: a low band that holds after a small object's impact
    "lf_tail_db": 5.0,
}

# Which detector speaks for which scenario (by name prefix): a reverb tail's tonality only where there is a room,
# a held low band only after a small object's impact, a ringing partial only where nothing should ring.
APPLIES = {
    "organ": ("shot-hall", "shot-big", "shot-vault", "shot-shed", "grenade-explode-big", "ball-hall", "ball-big", "walk-concrete"),
    "thunder": ("ball-", "balls-", "grenade-land", "pebbles", "crate-"),
    "bell": ("ball-", "balls-", "grenade-land", "pebbles", "crate-", "walk-"),
}


def flags_for(name, r, meta=None, ref=None):
    """The defect flags of a capture: the scoped detectors, `too long` against the real reference's decay, and
    `sature` from the take itself (audio underruns, voice-blocks frozen while a worker was behind, clipping)."""
    f = []
    on = lambda k: name.startswith(APPLIES[k])  # noqa: E731
    if on("organ") and r.get("tail_tonality_db", 0) >= THRESHOLDS["tail_tonality_db"]:
        f.append("organ")
    if on("thunder") and r.get("lf_hang_s", 0) >= THRESHOLDS["lf_hang_s"] and r.get("lf_tail_db", -99) >= THRESHOLDS["lf_tail_db"]:
        f.append("thunder")
    if on("bell") and r.get("ringing_s", 0) >= THRESHOLDS["ringing_s"]:
        f.append("bell")
    if ref and ref.get("decay_s") and r.get("decay_s", 0) > max(1.0, 1.5 * ref["decay_s"]):
        f.append("too long")
    if meta and (meta.get("underruns", 0) > 0 or meta.get("thinned", 0) > 0) or r.get("clipped", 0) > 0:
        f.append("sature")
    return f


def mono(path):
    x, sr = sf.read(path, always_2d=True, dtype="float32")
    return x.mean(1), x, sr


def env_db(m, sr, win=0.01):
    hop = max(1, int(sr * win))
    n = len(m) // hop
    if n == 0:
        return np.array([-120.0]), hop
    e = (m[: n * hop].reshape(n, hop) ** 2).mean(1)
    return 10 * np.log10(e + 1e-12), hop


def local_median_db(spec_db, freqs, frac=1 / 6):
    out = np.empty_like(spec_db)
    for i, f in enumerate(freqs):
        lo, hi = f * 2 ** (-frac), f * 2 ** frac
        sel = (freqs >= lo) & (freqs <= hi)
        out[i] = np.median(spec_db[sel]) if sel.sum() > 2 else spec_db[i]
    return out


def tail_segment(m, sr):
    e, hop = env_db(m, sr, 0.01)
    peak = e.max()
    strong = np.where(e >= peak - 10)[0]
    if not len(strong):
        return None
    start = strong[-1] + 20  # 200 ms after the last strong moment
    below = np.where(e[start:] < peak - 55)[0]
    end = start + (below[0] if len(below) else len(e) - start)
    end = min(end, start + 600)
    if end - start < 40:
        return None
    return m[start * hop: end * hop]


def tail_tonality(m, sr):
    seg = tail_segment(m, sr)
    if seg is None:
        return 0.0, 0.0
    f, _, Z = stft(seg, sr, nperseg=4096, noverlap=3072)
    P = np.abs(Z) ** 2 + 1e-20
    band = (f >= 100) & (f <= 8000)
    f, P = f[band], P[band]
    if P.shape[1] < 4:
        return 0.0, 0.0
    # Each frame normalised (the tail decays), then averaged.
    Pn = P / P.sum(0, keepdims=True)
    avg_db = 10 * np.log10(Pn.mean(1))
    prom = avg_db - local_median_db(avg_db, f)
    frames_db = 10 * np.log10(Pn)
    med_frames = np.stack([local_median_db(frames_db[:, k], f) for k in range(0, P.shape[1], max(1, P.shape[1] // 12))], 1)
    sub = frames_db[:, :: max(1, P.shape[1] // 12)][:, : med_frames.shape[1]]
    persistent = ((sub - med_frames) > 6).mean(1) >= 0.6
    cand = np.where(persistent)[0]
    if not len(cand):
        return float(max(0.0, prom.max())) * 0.0, 0.0
    k = cand[np.argmax(prom[cand])]
    return float(prom[k]), float(f[k])


def ringing(m, sr):
    f, t, Z = stft(m, sr, nperseg=2048, noverlap=1536)
    P = 10 * np.log10(np.abs(Z) ** 2 + 1e-20)
    sel = (f >= 400) & (f <= 12000)
    f, P = f[sel], P[sel]
    frame_max = P.max(0)
    loud = frame_max > frame_max.max() - 45
    best, best_f = 0.0, 0.0
    dt = t[1] - t[0] if len(t) > 1 else 0.01
    # A partial: a bin at least 15 dB over the median of the 41 bins around it (about +-470 Hz), within 30 dB of the
    # frame's loudest bin.
    from scipy.ndimage import median_filter
    med = median_filter(P, size=(41, 1), mode="nearest")
    peaks = (P - med > 15) & (P > frame_max[None, :] - 30) & loud[None, :]
    for i in range(len(f)):
        row = peaks[max(0, i - 1): i + 2].any(0)
        run = cur = 0
        for v in row:
            cur = cur + 1 if v else 0
            run = max(run, cur)
        if run * dt > best:
            best, best_f = run * dt, f[i]
    return float(best), float(best_f)


def lf_measures(m, sr):
    from scipy.signal import butter, sosfilt
    sos = butter(4, 250, "low", fs=sr, output="sos")
    lf = sosfilt(sos, m)
    e, hop = env_db(m, sr, 0.01)
    el, _ = env_db(lf, sr, 0.01)
    onset_idx = np.where(e > e.max() - 30)[0]
    if not len(onset_idx):
        return 0.0, -120.0
    o = onset_idx[0]
    lmax = el[o:].max()
    hang = float((el[o:] > lmax - 25).sum() * 0.01)
    head = (m[o * hop:(o + 25) * hop] ** 2).sum()
    tail = (lf[(o + 25) * hop:] ** 2).sum()
    return hang, float(10 * np.log10((tail + 1e-20) / (head + 1e-20)))


def decay(m, sr):
    e, _ = env_db(m, sr, 0.1)
    if len(e) < 3:
        return 0.0
    peak = e.max()
    last = np.where(e >= peak - 10)[0][-1]
    below = np.where(e[last:] < peak - 60)[0]
    if len(below):
        return float(below[0] * 0.1)
    seg = e[last:]
    if len(seg) < 3:
        return float(len(seg) * 0.1)
    slope = np.polyfit(np.arange(len(seg)) * 0.1, seg, 1)[0]
    return float(60 / -slope) if slope < -1 else 99.0


def audible(m, sr):
    e, _ = env_db(m, sr, 0.05)
    on = np.where(e > -70)[0]
    return float((on[-1] - on[0] + 1) * 0.05) if len(on) else 0.0


def crackle(x, sr):
    a = np.abs(x)
    clipped = 0
    for ch in range(x.shape[1]):
        hot = a[:, ch] >= 0.97
        run = 0
        for v in hot:
            run = run + 1 if v else 0
            if run == 3:
                clipped += 1
    # A click: one sample of the signal above 6 kHz standing 10x over the RMS of the 20 ms around it (the 2 ms at the
    # sample left out). An impact's high band is dense for milliseconds, so its onset is not a click; a dropout
    # or a discontinuity in a smooth tail is.
    from scipy.signal import butter, sosfilt
    m = x.mean(1)
    hp = sosfilt(butter(4, 6000, "high", fs=sr, output="sos"), m)
    p = hp ** 2
    w, g = int(sr * 0.02), int(sr * 0.002)
    if len(p) < w * 3:
        return clipped, 0.0
    c = np.cumsum(np.concatenate([[0.0], p]))
    i = np.arange(len(p))
    lo, hi = np.clip(i - w // 2, 0, len(p)), np.clip(i + w // 2, 0, len(p))
    glo, ghi = np.clip(i - g // 2, 0, len(p)), np.clip(i + g // 2, 0, len(p))
    around = (c[hi] - c[lo] - (c[ghi] - c[glo])) / np.maximum(1, (hi - lo) - (ghi - glo))
    clicks = (p > 100 * around) & (np.abs(hp) > 10 ** (-60 / 20))
    ev = np.sum(np.diff(clicks.astype(np.int8)) == 1)
    return int(clipped), float(ev / (len(m) / sr))


def measure(path):
    m, x, sr = mono(path)
    tt, tf = tail_tonality(m, sr)
    rs, rf = ringing(m, sr)
    hang, lft = lf_measures(m, sr)
    clipped, cps = crackle(x, sr)
    r = {
        "tail_tonality_db": round(tt, 1), "tail_tonality_hz": round(tf), "ringing_s": round(rs, 2), "ringing_hz": round(rf),
        "lf_hang_s": round(hang, 2), "lf_tail_db": round(lft, 1), "decay_s": round(decay(m, sr), 2), "audible_s": round(audible(m, sr), 2),
        "clipped": clipped, "clicks_per_s": round(cps, 2), "peak_dbfs": round(float(20 * np.log10(np.abs(x).max() + 1e-12)), 1),
    }
    r["flags"] = [k for k in ("tail_tonality_db", "ringing_s", "lf_hang_s") if r.get(k, 0) >= THRESHOLDS[k]]
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wavs", nargs="+")
    ap.add_argument("--json")
    a = ap.parse_args()
    out = {}
    for p in a.wavs:
        try:
            out[p] = measure(p)
        except Exception as e:  # a bad file is reported, the batch goes on
            out[p] = {"error": str(e)}
        print(p.split("/")[-1], out[p], file=sys.stderr)
    if a.json:
        json.dump(out, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
