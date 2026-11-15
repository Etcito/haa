"""Detectors on synthetic signals: each fires on its defect and stays quiet on the natural counterpart.

    /root/nmc/venvs/judge/bin/python -m pytest tools/judge/test_detectors.py -q   (or run the file)
"""
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import detectors as D  # noqa: E402

SR = 48000
rng = np.random.default_rng(1)


def decaying_noise(seconds, t60):
    t = np.arange(int(seconds * SR)) / SR
    return rng.standard_normal(len(t)) * 10 ** (-3 * t / t60)


def test_organ_tail_vs_a_diffuse_tail():
    burst = np.concatenate([rng.standard_normal(2400) * 4, np.zeros(SR // 10)])
    diffuse = np.concatenate([burst, decaying_noise(3, 2.0)])
    t = np.arange(int(3 * SR)) / SR
    tones = sum(np.sin(2 * np.pi * f * t) for f in (440, 660, 880)) * 0.3 * 10 ** (-3 * t / 4.0)
    organ = np.concatenate([burst, decaying_noise(3, 2.0) + tones])
    assert D.tail_tonality(diffuse, SR)[0] < D.THRESHOLDS["tail_tonality_db"]
    assert D.tail_tonality(organ, SR)[0] >= D.THRESHOLDS["tail_tonality_db"]


def test_bell_vs_a_dry_knock():
    t = np.arange(int(1.5 * SR)) / SR
    knock = rng.standard_normal(len(t)) * np.exp(-t / 0.01)
    bell = knock + 0.5 * np.sin(2 * np.pi * 2900 * t) * np.exp(-t / 0.6)
    assert D.ringing(knock, SR)[0] < D.THRESHOLDS["ringing_s"]
    assert D.ringing(bell, SR)[0] >= D.THRESHOLDS["ringing_s"]


def test_thunder_vs_a_bounce():
    t = np.arange(int(4 * SR)) / SR
    bounce = rng.standard_normal(len(t)) * np.exp(-t / 0.02)
    from scipy.signal import butter, sosfilt
    rumble = sosfilt(butter(2, 120, "low", fs=SR, output="sos"), rng.standard_normal(len(t))) * 10 * np.exp(-t / 3.0)
    thunder = bounce + rumble
    fb = D.flags_for("ball-outdoor", dict(zip(("lf_hang_s", "lf_tail_db"), D.lf_measures(bounce, SR))))
    ft = D.flags_for("ball-outdoor", dict(zip(("lf_hang_s", "lf_tail_db"), D.lf_measures(thunder, SR))))
    assert "thunder" not in fb and "thunder" in ft


if __name__ == "__main__":
    for name, f in list(globals().items()):
        if name.startswith("test_"):
            f()
            print("ok", name)
