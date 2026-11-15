"""Judge a folder of scenario captures: references, detectors, CLAP, the listening model, a table and a page.

    run_judge.py --captures <dir> --out <dir> [--runs 3] [--only a,b] [--no-listen] [--no-clap]

<captures> is what haa-studio's scripts/scene-judge.ts wrote (<name>.wav, <name>.json, index.json).
Writes into <out>: refs/<name>.wav (the real recording for each scenario, refs.json), detectors.json, clap.json,
listen.json, table.md and index.html (players side by side; local only, the references are never deployed).
"""
import argparse
import glob
import html
import json
import os
import re
import subprocess
import sys

import librosa
import numpy as np
import soundfile as sf
from scipy.signal import fftconvolve

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import detectors as D  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
# Gentle on a shared machine: niced, every step with 4 CPU threads.
os.environ.setdefault("OMP_NUM_THREADS", "4")
os.environ.setdefault("MKL_NUM_THREADS", "4")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "4")
try:
    os.nice(15)
except OSError:
    pass
PY = sys.executable

# The defect words the owner used, as a blind description would say them.
CLASSES = {
    "thunder": r"thunder|rumbl|storm",
    "organ": r"organ|drone|\bhum\b|humming|tonal|whistl|sustained (tone|note|pitch)|chord|resonan|feedback",
    "bell": r"\bbell|chime|ringing|tinkl|ding",
    "water": r"water|stream|trickl|splash|pour|liquid|drip",
    "toy": r"\btoy|cap gun|popgun|cork",
    "crackle": r"crackl|distort|static|clipp|glitch",
    "nail": r"hammer|nail|knock|tapping|\btap\b|chisel",
    "wet cloth": r"\bwet\b|slap|splat|squish|cloth|towel|\bmud",
    "too long": r"long (reverb|echo|tail|decay)|lingering|excessive (reverb|echo)|echo(es)? (go(es)? on|too)",
    "synthetic": r"synthe|artificial|electronic|computer|digital|unnatural|fake",
    # Broader, for the calibration: a small object heard as a blast; a tail heard as something pitched.
    "boom": r"explosion|explod|\bboom|blast|firework|\bbomb",
    "tonal": r"organ|drone|\bhum\b|tonal|chord|guitar|piano|music|siren|\bbell|chime|ringing|\btone\b|\bnote\b|whistl|resonan",
}


NEGATION = re.compile(r"\b(no|not|nothing|none|without|neither|nor|lack|lacks)\b|n't\b")


def sentences(text):
    return [s for s in re.split(r"(?<=[.!?;])\s+|\n", text.lower()) if s]


def classes_of(text):
    """The defect classes a description affirms: a sentence that negates ("nothing sounds unnatural", "no ringing")
    does not count."""
    found = set()
    for s in sentences(text):
        if NEGATION.search(s):
            continue
        found.update(k for k, rx in CLASSES.items() if re.search(rx, s))
    return sorted(found)


def load_mono(path, sr=48000):
    y, _ = librosa.load(path, sr=sr, mono=True)
    return y.astype(np.float32)


def build_ref(spec, root, seconds):
    dry = load_mono(os.path.join(root, spec["dry"]))
    if "start" in spec:
        dry = dry[int(spec["start"] * 48000):]
    if "seconds" in spec:
        dry = dry[: int(spec["seconds"] * 48000)]
    y = dry
    if "ir" in spec:
        hits = glob.glob(os.path.join(root, spec["ir"]), recursive=True)
        hits = [h for h in hits if "/._" not in h]
        if not hits:
            raise FileNotFoundError(spec["ir"])
        ir = load_mono(hits[0])
        ir = ir[np.argmax(np.abs(ir)) - 48:] if np.argmax(np.abs(ir)) > 48 else ir
        ir = ir / (np.sqrt((ir ** 2).sum()) + 1e-12)
        y = fftconvolve(dry, ir)[: len(dry) + len(ir)]
    y = y[: int(max(seconds, len(dry) / 48000) * 48000) + 48000]
    y = y / (np.abs(y).max() + 1e-9) * 10 ** (-3 / 20)
    return np.stack([y, y], 1)


def run(cmd):
    print("[judge] $", " ".join(cmd[:3]), "...", file=sys.stderr, flush=True)
    subprocess.run(cmd, check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--captures", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--only")
    ap.add_argument("--no-listen", action="store_true")
    ap.add_argument("--no-clap", action="store_true")
    ap.add_argument("--no-ab", action="store_true")
    ap.add_argument("--reuse", action="store_true", help="keep detectors.json, clap.json and listen.json when present")
    a = ap.parse_args()
    os.makedirs(os.path.join(a.out, "refs"), exist_ok=True)
    index = json.load(open(os.path.join(a.captures, "index.json")))
    caps = sorted(index["captures"], key=lambda c: c["name"])
    if a.only:
        keep = set(a.only.split(","))
        caps = [c for c in caps if c["name"] in keep]
    wav = {c["name"]: os.path.join(a.captures, c["name"] + ".wav") for c in caps}
    refs_spec = json.load(open(os.path.join(HERE, "refs.json")))
    root = os.environ.get("JUDGE_REFS", "/root/nmc/temp")
    ref = {}
    for c in caps:
        spec = refs_spec.get(c["name"])
        if not spec:
            continue
        p = os.path.join(a.out, "refs", c["name"] + ".wav")
        try:
            sf.write(p, build_ref(spec, root, c["seconds"]), 48000, subtype="FLOAT")
            ref[c["name"]] = p
        except Exception as e:  # a missing reference leaves the A/B out, nothing else
            print(f"[judge] no reference for {c['name']}: {e}", file=sys.stderr)
    det_path = os.path.join(a.out, "detectors.json")
    if not (a.reuse and os.path.exists(det_path)):
        run([PY, os.path.join(HERE, "detectors.py"), *wav.values(), *ref.values(), "--json", det_path])
    det = json.load(open(det_path))
    clap = {}
    if not a.no_clap:
        clap_path = os.path.join(a.out, "clap.json")
        args = [PY, os.path.join(HERE, "clap.py"), *wav.values(), "--json", clap_path]
        for c in caps:
            args += ["--intent", c["intent"]]
        if not (a.reuse and os.path.exists(clap_path)):
            run(args)
        clap = json.load(open(clap_path))
    listen = []
    if not a.no_listen:
        jobs = []
        for c in caps:
            n = c["name"]
            jobs.append({"id": n, "mode": "blind", "wav": wav[n]})
            jobs.append({"id": n, "mode": "rate", "wav": wav[n], "intent": c["intent"]})
            if n in ref and not a.no_ab:
                jobs.append({"id": n, "mode": "ab", "wav": wav[n], "ref": ref[n], "intent": c["intent"]})
        jp = os.path.join(a.out, "listen-jobs.json")
        lp = os.path.join(a.out, "listen.json")
        if not (a.reuse and os.path.exists(lp)):
            json.dump(jobs, open(jp, "w"), indent=1)
            run([PY, os.path.join(HERE, "listen.py"), jp, lp, "--runs", str(a.runs)])
        listen = json.load(open(lp))
    rows = summarize(caps, wav, ref, det, clap, listen)
    json.dump({"index": index, "rows": rows}, open(os.path.join(a.out, "judge.json"), "w"), indent=1)
    write_table(rows, os.path.join(a.out, "table.md"), index)
    write_page(rows, a.out, a.captures, index)
    print(f"[judge] {os.path.join(a.out, 'index.html')}", file=sys.stderr)


def summarize(caps, wav, ref, det, clap, listen):
    by = {}
    for r in listen:
        by.setdefault(r["id"], {})[r["mode"]] = r
    rows = []
    for c in caps:
        n = c["name"]
        L = by.get(n, {})
        blind = [x["text"] for x in L.get("blind", {}).get("answers", [])]
        scores = [x["score"] for x in L.get("rate", {}).get("answers", []) if x.get("score") is not None]
        rate_texts = [x["text"] for x in L.get("rate", {}).get("answers", [])]
        ab = L.get("ab", {}).get("answers", [])
        heard = {}
        for t in blind:
            for k in classes_of(t):
                heard[k] = heard.get(k, 0) + 1
        d = dict(det.get(wav[n], {}))
        d["flags"] = D.flags_for(n, d, c, det.get(ref.get(n, ""), {}))
        rows.append({
            "name": n, "intent": c["intent"], "capture": c, "detectors": d, "ref_detectors": det.get(ref.get(n, ""), {}),
            "has_ref": n in ref, "clap": clap.get(wav[n], {}), "blind": blind, "heard": heard, "rate": rate_texts,
            "score": round(float(np.mean(scores)), 1) if scores else None, "scores": scores,
            "ab_correct": sum(1 for x in ab if x.get("correct")), "ab_runs": len(ab), "ab_texts": [x["text"] for x in ab],
        })
    return rows


def write_table(rows, path, index):
    lines = [f"# Judge: haa {index.get('haa')} / studio {index.get('studio')} ({index.get('at')})", "",
             "| scenario | blind hears (runs) | score 0-10 | real picked (A/B) | detector flags | CLAP top | capture | voices that played |",
             "|---|---|---|---|---|---|---|---|"]
    for r in rows:
        heard = ", ".join(f"{k} {v}/{len(r['blind'])}" for k, v in sorted(r["heard"].items())) or "-"
        flags = ", ".join(r["detectors"].get("flags", [])) or "-"
        c = r["capture"]
        cap = f"underruns {c.get('underruns')}, thinned {c.get('thinned')}, peak {r['detectors'].get('peak_dbfs')} dBFS"
        ab = f"{r['ab_correct']}/{r['ab_runs']}" if r["ab_runs"] else "-"
        lines.append(f"| {r['name']} | {heard} | {r['score'] if r['score'] is not None else '-'} | {ab} | {flags} | {', '.join(r['clap'].get('top3', [])[:2]) or '-'} | {cap} | {c.get('summary', '-')} |")
    lines += ["", "## Blind descriptions", ""]
    for r in rows:
        lines.append(f"### {r['name']}")
        for t in r["blind"]:
            lines.append(f"- {t}")
        lines.append("")
    open(path, "w").write("\n".join(lines))


def write_page(rows, out, captures, index):
    rel_cap = os.path.relpath(captures, out)
    e = html.escape
    parts = []
    for r in rows:
        d = r["detectors"]
        flags = " ".join(f'<span class="flag">{e(f)}</span>' for f in d.get("flags", [])) or '<span class="ok">none</span>'
        heard = " ".join(f'<span class="flag">{e(k)} {v}/{len(r["blind"])}</span>' for k, v in sorted(r["heard"].items())) or '<span class="ok">-</span>'
        refp = f'<audio controls preload="none" src="refs/{e(r["name"])}.wav"></audio>' if r["has_ref"] else "<em>no reference</em>"
        blind = "".join(f"<li>{e(t)}</li>" for t in r["blind"])
        rate = "".join(f"<li>{e(t)}</li>" for t in r["rate"])
        ab = "".join(f"<li>{e(t)}</li>" for t in r["ab_texts"])
        det = ", ".join(f"{k} {v}" for k, v in d.items() if k not in ("flags",))
        parts.append(f"""
<section>
  <h2>{e(r['name'])}</h2>
  <p class="intent">{e(r['intent'])}</p>
  <p class="intent">voices: {e(str(r['capture'].get('summary', '-')))}</p>
  <div class="players">
    <figure><figcaption>scene capture</figcaption><audio controls preload="none" src="{e(rel_cap)}/{e(r['name'])}.wav"></audio></figure>
    <figure><figcaption>real recording</figcaption>{refp}</figure>
  </div>
  <p><b>Blind, the model hears:</b> {heard} &nbsp; <b>score</b> {r['score'] if r['score'] is not None else '-'} &nbsp;
     <b>A/B real picked</b> {f"{r['ab_correct']}/{r['ab_runs']}" if r['ab_runs'] else '-'} &nbsp; <b>detectors</b> {flags}</p>
  <details><summary>blind descriptions</summary><ul>{blind}</ul></details>
  <details><summary>told the intent: rating</summary><ul>{rate}</ul></details>
  <details><summary>A/B against the real recording</summary><ul>{ab}</ul></details>
  <details><summary>measures</summary><p class="mono">{e(det)}</p><p class="mono">CLAP: {e(', '.join(r['clap'].get('top3', [])))}</p></details>
</section>""")
    page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sound judge</title>
<style>
:root {{ --bg: #fafaf9; --fg: #1c1917; --muted: #78716c; --line: #e7e5e4; --flag: #b91c1c; --ok: #15803d; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg: #1c1917; --fg: #f5f5f4; --muted: #a8a29e; --line: #44403c; --flag: #f87171; --ok: #4ade80; }} }}
body {{ background: var(--bg); color: var(--fg); font: 15px/1.5 system-ui, sans-serif; margin: 0 auto; max-width: 980px; padding: 16px; }}
section {{ border-top: 1px solid var(--line); padding: 12px 0; }}
h2 {{ margin: 0 0 4px; font-size: 18px; }} .intent {{ color: var(--muted); margin: 0 0 8px; }}
.players {{ display: flex; flex-wrap: wrap; gap: 16px; }} figure {{ margin: 0; }} figcaption {{ font-size: 13px; color: var(--muted); }}
audio {{ width: min(440px, 100%); }} .flag {{ color: var(--flag); font-weight: 600; margin-right: 6px; }} .ok {{ color: var(--ok); }}
.mono {{ font: 12px ui-monospace, monospace; overflow-wrap: anywhere; }}
</style></head><body>
<h1>Sound judge</h1>
<p>haa {e(str(index.get('haa')))} / haa-studio {e(str(index.get('studio')))}, captured {e(str(index.get('at')))}. Scene output after the master limiter, 48 kHz.
Local only: the real recordings are CC0 / CC BY / research-use references, not for deployment.</p>
{''.join(parts)}
</body></html>"""
    open(os.path.join(out, "index.html"), "w").write(page)


if __name__ == "__main__":
    main()
