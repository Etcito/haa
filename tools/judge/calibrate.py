"""Calibrate the judge on the owner: his verdicts are labelled failing cases, the real recordings are controls.

    calibrate.py --judged <root of judge outputs> --controls <controls listen.json> --out calibration.md

A case reproduces when the blind model names the owner's defect class in at least 2 of its runs, or a scoped
detector flags it. The owner's rule: what he rated 3/10 must fail, so the model's realism score on his failing
cases is shown next to its score on the real recordings.
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_judge import CLASSES, NEGATION, classes_of, sentences  # noqa: E402

CLASSES_EXTRA = {"weak": r"weak|thin|\bsmall\b|tiny|faint|\btoy|\bpop\b|cap gun|firecracker|cork|click"}

# (his words, the commits where he heard it, scenario, the classes that say the same)
CASES = [
    ("ball on the ground: \"comme un coup de tonnerre\"", "haa-733aaa7_studio-f3be2ac", "ball-outdoor", ["thunder", "boom"]),
    ("many balls landing: \"un coup de tonnerre\"", "haa-733aaa7_studio-f3be2ac", "balls-30", ["thunder", "boom"]),
    ("pebbles: \"de l'eau qui ruisselle\"", "haa-733aaa7_studio-f3be2ac", "pebbles", ["water"]),
    ("the first pistol: \"ridicule\"", "haa-733aaa7_studio-f3be2ac", "shot-outdoor", ["toy", "weak"]),
    ("concrete steps indoors: \"un clou qui ne veut pas rentrer dans du béton\"", "haa-01dcc76_studio-6f780aa", "walk-concrete-hall", ["nail"]),
    ("snow steps: \"un chiffon mouillé contre un mur\"", "haa-01dcc76_studio-6f780aa", "walk-snow", ["wet cloth"]),
    ("red ball rolling after landing: \"fait de l'orage\"", "haa-d564b6c_studio-a8e68b9", "ball-outdoor", ["thunder", "boom"]),
    ("echo in the buildings: \"dure trop longtemps\" (hall)", "haa-d564b6c_studio-a8e68b9", "shot-hall", ["too long"]),
    ("echo in the buildings: \"dure trop longtemps\" (big)", "haa-d564b6c_studio-a8e68b9", "shot-big-empty", ["too long"]),
    ("grenade landing: \"un bruit de clochette\"", "haa-d564b6c_studio-a8e68b9", "grenade-land-outdoor", ["bell", "tonal"]),
    ("big hall: \"on dirait un orgue l'écho\"", "haa-d564b6c_studio-a8e68b9", "shot-big-empty", ["organ", "tonal"]),
    ("pebbles: \"sature\"", "haa-d564b6c_studio-a8e68b9", "pebbles-close", ["crackle", "sature"]),
]


def heard(texts, cls):
    """In how many descriptions a sentence affirms the class (negated sentences do not count)."""
    rx = CLASSES.get(cls) or CLASSES_EXTRA.get(cls)
    if not rx:
        return 0
    return sum(1 for t in texts if any(re.search(rx, s) and not NEGATION.search(s) for s in sentences(t)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--judged", default="/mnt/q/temp_nomorechat/haa/judge")
    ap.add_argument("--controls")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    lines = ["# Judge calibration on the owner's verdicts", "",
             "| owner verdict | capture | blind: his class named (runs) | detectors | score 0-10 | reproduced |",
             "|---|---|---|---|---|---|"]
    rows = []
    for words, run, scen, cls in CASES:
        p = os.path.join(a.judged, run, "judge.json")
        if not os.path.exists(p):
            lines.append(f"| {words} | {run}/{scen} | not judged yet | | | |")
            continue
        r = next((x for x in json.load(open(p))["rows"] if x["name"] == scen), None)
        if not r:
            lines.append(f"| {words} | {run}/{scen} | no capture | | | |")
            continue
        n = len(r["blind"])
        named = max((heard(r["blind"], c) for c in cls), default=0)
        flags = [f for f in r["detectors"].get("flags", []) if f in cls]
        ok = named >= 2 or bool(flags)
        rows.append({"words": words, "run": run, "scenario": scen, "classes": cls, "named": named, "runs": n, "flags": flags,
                     "score": r["score"], "reproduced": ok, "blind": r["blind"]})
        lines.append(f"| {words} | {run.replace('haa-', '').replace('_studio-', '/')} {scen} | {named}/{n} ({', '.join(cls)}) | "
                     f"{', '.join(flags) or '-'} | {r['score'] if r['score'] is not None else '-'} | {'yes' if ok else 'NO'} |")
    if a.controls and os.path.exists(a.controls):
        ctl = json.load(open(a.controls))
        lines += ["", "## Controls (real recordings)", "", "| control | kind | blind names | score 0-10 |", "|---|---|---|---|"]
        by = {}
        for x in ctl:
            by.setdefault(x["id"], {})[x["mode"]] = x
        pos, neg = [], []
        for cid, m in by.items():
            texts = [t["text"] for t in m.get("blind", {}).get("answers", [])]
            scores = [t.get("score") for t in m.get("rate", {}).get("answers", []) if t.get("score") is not None]
            sc = round(sum(scores) / len(scores), 1) if scores else None
            kind = m.get("blind", m.get("rate"))["kind"]
            what = m.get("blind", m.get("rate")).get("is")
            named = ""
            if what:
                c = {"wet cloth": "wet cloth", "toy": "toy", "nail": "nail"}.get(what, what)
                named = f"{heard(texts, c)}/{len(texts)} name {what}"
            else:
                named = "; ".join(sorted({k for t in texts for k in classes_of(t)})) or "-"
            (pos if kind == "positive" else neg).append(sc)
            lines.append(f"| {cid.split(':', 1)[1]} | {kind} | {named} | {sc if sc is not None else '-'} |")
        mean = lambda v: round(sum(x for x in v if x is not None) / max(1, len([x for x in v if x is not None])), 1)  # noqa: E731
        lines += ["", f"Mean score: real recordings told their true intent {mean(pos)}, wrong sounds told our intent {mean(neg)}, "
                      f"the owner's failing captures {mean([r['score'] for r in rows])}."]
    rep = sum(1 for r in rows if r["reproduced"])
    lines += ["", f"Reproduced: {rep}/{len(rows)} of the owner's verdicts."]
    open(a.out, "w").write("\n".join(lines) + "\n")
    json.dump(rows, open(a.out.replace(".md", ".json"), "w"), indent=1)
    print("\n".join(lines))


if __name__ == "__main__":
    main()
