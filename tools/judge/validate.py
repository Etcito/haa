"""Validate the detectors: real recordings must not be flagged, the owner's named defects (the nogo-* real
recordings of thunder, organ, bell...) must fire their own detector, and so must his failing captures.

    validate.py --refs </root/work/haa/refs/sfx-targets/fs> [--irs <dir with real IR tails>] [--failing name=wav ...] --json out.json

Prints, per detector, the distribution on real recordings (median, 95th percentile, max, share flagged) and
what it says of each control.
"""
import argparse
import glob
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import detectors as D  # noqa: E402

# Which detector each nogo class should fire (the owner's words).
EXPECT = {"nogo-organ": "tail_tonality_db", "nogo-bell": "ringing_s", "nogo-thunder": "lf_hang_s"}
# Real recordings whose own nature trips a detector by design (not counted as false alarms of that detector).
NATURAL = {"lf_hang_s": ("rain", "water", "ballroll", "roll", "snowroll", "explosion", "grenade", "slide", "scrape"),
           "ringing_s": ("metal-impact", "steeldrop", "glass-clink", "casing", "nogo-bell")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--refs", required=True)
    ap.add_argument("--failing", nargs="*", default=[])
    ap.add_argument("--json", required=True)
    ap.add_argument("--per-class", type=int, default=6)
    a = ap.parse_args()
    real, nogo = {}, {}
    for d in sorted(glob.glob(os.path.join(a.refs, "*"))):
        if not os.path.isdir(d):
            continue
        cls = os.path.basename(d)
        if cls == "wrong":
            continue
        files = sorted(glob.glob(os.path.join(d, "*.wav")))[: a.per_class]
        for f in files:
            try:
                r = D.measure(f)
            except Exception as e:
                print("skip", f, e, file=sys.stderr)
                continue
            (nogo if cls.startswith("nogo-") else real).setdefault(cls, []).append((f, r))
    out = {"thresholds": D.THRESHOLDS, "real": {}, "nogo": {}, "failing": {}}
    for key, th in D.THRESHOLDS.items():
        vals, fp = [], []
        for cls, items in real.items():
            for f, r in items:
                v = r.get(key, 0)
                vals.append(v)
                if v >= th and not cls.startswith(NATURAL.get(key, ("\0",))):
                    fp.append(f"{cls}/{os.path.basename(f)}={v}")
        vals = np.array(vals, dtype=float)
        out["real"][key] = {"median": float(np.median(vals)), "p95": float(np.percentile(vals, 95)), "max": float(vals.max()),
                            "n": int(len(vals)), "false_alarms": fp}
        print(f"{key:18s} real: median {np.median(vals):.2f} p95 {np.percentile(vals, 95):.2f} max {vals.max():.2f} "
              f"false alarms {len(fp)}/{len(vals)} {fp[:6]}", file=sys.stderr)
    for cls, items in nogo.items():
        key = EXPECT.get(cls)
        res = [{"file": os.path.basename(f), "value": r.get(key) if key else None, "flags": r["flags"]} for f, r in items]
        hit = sum(1 for x in res if key and key in x["flags"])
        out["nogo"][cls] = {"detector": key, "fired": hit, "n": len(res), "items": res}
        print(f"{cls:14s} expects {key}: fired {hit}/{len(res)}  {[x['value'] for x in res]}", file=sys.stderr)
    for spec in a.failing:
        name, path = spec.split("=", 1)
        r = D.measure(path)
        out["failing"][name] = r
        print(f"failing {name:28s} flags {r['flags']}  tonality {r['tail_tonality_db']} ringing {r['ringing_s']} lf_hang {r['lf_hang_s']} "
              f"lf_tail {r['lf_tail_db']} decay {r['decay_s']} clicks {r['clicks_per_s']} clipped {r['clipped']}", file=sys.stderr)
    json.dump(out, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
