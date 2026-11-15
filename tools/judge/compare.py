"""What changed between two judge runs, per scenario.

    compare.py <old judge dir> <new judge dir> [--all]

Per scenario: the model's verdict on our capture against the real references ("named like the real ones" when
it names the event at least as often as for the real ones or in 2 of 3 runs, "not named" when never, "partly"
between, "cannot decide" when it does not recognise the real ones, "invalid" for a take from a busy machine),
the mix's frozen blocks, the underruns, the limiter, the tail-tonality (organ) flag and the other detector flags.
Only the scenarios where something changed, unless --all.
"""
import argparse
import json
import os


def verdict(r):
    if r.get("invalid"):
        return "invalid"
    if r.get("undecided"):
        return "cannot decide"
    ours, real = r.get("ours_model"), r.get("ref_model")
    if ours is None:
        return "-"
    if ours >= 0.66 or (real is not None and ours >= real):
        return "named like the real ones"
    return "not named" if ours == 0 else "partly named"


def facts(r):
    c, d = r["capture"], r["detectors"]
    return {
        "verdict": verdict(r), "named": f"{r.get('ref_model')}/{r.get('ours_model')}",
        "frozen": c.get("thinned", 0), "underruns": c.get("underruns", 0), "limiter": round(c.get("limiterMinDb") or 0, 1),
        "organ": "organ" in d.get("flags", []), "flags": ",".join(f for f in d.get("flags", []) if f != "organ") or "-",
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--all", action="store_true")
    a = ap.parse_args()
    old = {r["name"]: facts(r) for r in json.load(open(os.path.join(a.old, "judge.json")))["rows"]}
    new = {r["name"]: facts(r) for r in json.load(open(os.path.join(a.new, "judge.json")))["rows"]}
    keys = ("verdict", "frozen", "underruns", "limiter", "organ", "flags")
    print("| scenario | verdict (real/ours named) | frozen blocks | underruns | limiter dB | tonality flag | other flags |")
    print("|---|---|---|---|---|---|---|")
    for n in sorted(new):
        o, w = old.get(n), new[n]
        if not a.all and o and all(o[k] == w[k] for k in keys):
            continue
        cell = lambda k: (f"{o[k]} -> {w[k]}" if o and o[k] != w[k] else f"{w[k]}")  # noqa: E731
        v = (f"{o['verdict']} ({o['named']}) -> " if o and o["verdict"] != w["verdict"] else "") + f"{w['verdict']} ({w['named']})"
        print(f"| {n} | {v} | {cell('frozen')} | {cell('underruns')} | {cell('limiter')} | {cell('organ')} | {cell('flags')} |")


if __name__ == "__main__":
    main()
