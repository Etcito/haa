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
    ("pebbles: \"sature\" (thrown 5 m away, quiet machine)", "haa-d564b6c_studio-a8e68b9", "pebbles", ["crackle", "sature"]),
    ("pebbles: \"sature\" (8 handfuls at the feet)", "haa-d564b6c_studio-a8e68b9", "pebbles-close", ["crackle", "sature"]),
]


def heard(texts, cls):
    """In how many descriptions a sentence affirms the class (negated sentences do not count)."""
    rx = CLASSES.get(cls) or CLASSES_EXTRA.get(cls)
    if not rx:
        return 0
    return sum(1 for t in texts if any(re.search(rx, s) and not NEGATION.search(s) for s in sentences(t)))


def base_rates(ctl):
    """Per class, the share of blind descriptions of real recordings (positive controls) that affirm it: a word the
    model also says of real sounds says little about ours."""
    texts = [t["text"] for x in ctl if x.get("kind") == "positive" and x["mode"] == "blind" for t in x["answers"]]
    classes = set(CLASSES) | set(CLASSES_EXTRA)
    return {c: (heard(texts, c) / len(texts) if texts else 0.0) for c in classes}, len(texts)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--judged", default="/mnt/q/temp_nomorechat/haa/judge")
    ap.add_argument("--controls")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    ctl = json.load(open(a.controls)) if a.controls and os.path.exists(a.controls) else []
    base, nbase = base_rates(ctl)
    lines = ["# Judge calibration on the owner's verdicts", "",
             "A case is heard by the model when at least 2 of its 3 blind descriptions affirm the owner's word (or its class) "
             f"and that rate is 0.25 above the class's rate on real recordings ({nbase} blind descriptions of the positive controls); "
             "by the detectors when a scoped detector flags it.", "",
             "| owner verdict | capture | blind: his class named | same class on real recordings | model | detectors | score 0-10 | reproduced |",
             "|---|---|---|---|---|---|---|---|"]
    rows = []
    for words, run, scen, cls in CASES:
        where = f"{run.replace('haa-', '').replace('_studio-', '/')} {scen}"
        p = os.path.join(a.judged, run, "judge.json")
        r = next((x for x in json.load(open(p))["rows"] if x["name"] == scen), None) if os.path.exists(p) else None
        if not r:
            lines.append(f"| {words} | {where} | not captured / judged | | | | | |")
            continue
        if r.get("invalid"):
            lines.append(f"| {words} | {where} | invalid take ({r['invalid']}): retake on a quiet machine | | | | | |")
            rows.append({"words": words, "run": run, "scenario": scen, "invalid": r["invalid"], "reproduced": None})
            continue
        # "Too long" is a judgment against what the place should do: it is read in the answers told the intent
        # (protocol step b), the other words in the blind ones.
        texts = r["rate"] if "too long" in cls else r["blind"]
        n = len(texts)
        best = max(cls, key=lambda c: (heard(texts, c) / n if n else 0) - base.get(c, 0))
        named = heard(texts, best)
        rate = named / n if n else 0
        model_ok = n > 0 and named >= 2 and rate >= base.get(best, 0) + 0.25
        flags = [f for f in r["detectors"].get("flags", []) if f in cls]
        ok = model_ok or bool(flags)
        rows.append({"words": words, "run": run, "scenario": scen, "classes": cls, "class": best, "named": named, "runs": n,
                     "base": round(base.get(best, 0), 2), "model": model_ok, "flags": flags, "score": r["score"], "reproduced": ok,
                     "blind": r["blind"]})
        lines.append(f"| {words} | {where} | {named}/{n} ({best}) | {base.get(best, 0):.2f} | {'yes' if model_ok else 'no'} | "
                     f"{', '.join(flags) or '-'} | {r['score'] if r['score'] is not None else '-'} | {'yes' if ok else 'NO'} |")
    if ctl:
        lines += ["", "## Controls (real recordings)", "", "| control | kind | blind names | score 0-10 |", "|---|---|---|---|"]
        by = {}
        for x in ctl:
            by.setdefault(x["id"], {})[x["mode"]] = x
        pos, neg = [], []
        for cid, m in by.items():
            texts = [t["text"] for t in m.get("blind", {}).get("answers", [])]
            scores = [t.get("score") for t in m.get("rate", {}).get("answers", []) if t.get("score") is not None]
            sc = round(sum(scores) / len(scores), 1) if scores else None
            first = m.get("blind", m.get("rate"))
            what = first.get("is")
            named = f"{heard(texts, what)}/{len(texts)} name {what}" if what else ("; ".join(sorted({k for t in texts for k in classes_of(t)})) or "-")
            (pos if first["kind"] == "positive" else neg).append(sc)
            lines.append(f"| {cid.split(':', 1)[1]} | {first['kind']} | {named} | {sc if sc is not None else '-'} |")
        mean = lambda v: round(sum(x for x in v if x is not None) / max(1, len([x for x in v if x is not None])), 1)  # noqa: E731
        lines += ["", f"Mean realism score: real recordings told their true intent {mean(pos)}, wrong sounds told our intent {mean(neg)}, "
                      f"the owner's failing captures {mean([r.get('score') for r in rows])}."]
        lines += ["", "Class rates on real recordings (blind): " + ", ".join(f"{c} {v:.2f}" for c, v in sorted(base.items()) if v > 0)]
    judged = [r for r in rows if r.get("reproduced") is not None]
    lines += ["", f"Reproduced: {sum(1 for r in judged if r['reproduced'])}/{len(judged)} of the owner's verdicts "
                  f"(model {sum(1 for r in judged if r.get('model'))}, detectors {sum(1 for r in judged if r.get('flags'))}); "
                  f"{len(rows) - len(judged)} invalid takes to retake."]
    open(a.out, "w").write("\n".join(lines) + "\n")
    json.dump(rows, open(a.out.replace(".md", ".json"), "w"), indent=1)
    print("\n".join(lines))


if __name__ == "__main__":
    main()
