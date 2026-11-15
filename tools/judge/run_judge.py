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
from clap import VALIDATION as CLAP_VALID, class_of as clap_class  # noqa: E402

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
    "bell": r"\bbells?\b|chime|ringing|tinkl|\bding\b",
    "water": r"water|stream|trickl|splash|pour|liquid|drip",
    "toy": r"\btoy|cap gun|popgun|cork",
    "crackle": r"crackl|distort|static|clipp|glitch",
    "nail": r"hammer|nail|knock|tapping|\btap\b|chisel",
    "wet cloth": r"\bwet\b|slap|splat|squish|cloth|towel|\bmud",
    "too long": r"long (reverb|echo|tail|decay)|lingering|excessive (reverb|echo)|echo(es)? (go(es)? on|too)|(reverb\w*|echo\w*|decay|tail) (is|are|was|were|seems?|sounds?) too|too much (reverb|echo)",
    "synthetic": r"synthe|artificial|electronic|computer|digital|unnatural|fake",
    # Broader, for the calibration: a small object heard as a blast; a tail heard as something pitched.
    "boom": r"explosion|explod|\bboom|blast|firework|\bbomb",
    "tonal": r"organ|drone|\bhum\b|tonal|chord|guitar|piano|music|siren|\bbell|chime|ringing|\btone\b|\bnote\b|whistl|resonan",
}


# The words that name each scenario's event (by name prefix), to check that the model recognises it: on the real
# references (a scenario whose real recordings it does not recognise is one it cannot decide) and on our capture.
EXPECT = [
    ("balls-", r"\bballs?\b|bounc"), ("ball-", r"\bballs?\b|bounc|rubber"), ("grenade-explode", r"explo|blast|bomb|boom"),
    ("grenade-land", r"metal|steel|clank|clang|drop|thud|object|can\b|tool"), ("shot-", r"gun|shot|firearm|pistol|rifle|bullet"),
    ("casings", r"casing|shell|brass|metal|gun|shot"), ("walk-", r"step|walk|foot"), ("pebbles", r"pebble|stone|gravel|rock|grit"),
    ("smg", r"gun|shot|fire|machine"), ("shotgun", r"gun|shot|blast"), ("bottle-", r"glass|bottle|shatter|break"),
    ("crate-drop", r"crate|box|wood|thud|drop|fall"), ("crate-slide", r"drag|slid|scrap|push|furniture"), ("rain", r"rain|drizzl|downpour|storm"),
]


def expect_of(name):
    return next((rx for p, rx in EXPECT if name.startswith(p)), None)


def recognised(texts, name):
    """In how many descriptions a sentence names the scenario's event (negated sentences do not count)."""
    rx = expect_of(name)
    if not rx:
        return None
    return sum(1 for t in texts if any(re.search(rx, x) and not NEGATION.search(x) for x in sentences(t)))


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
    ap.add_argument("--no-ref-listen", action="store_true", help="do not have the model describe the real references")
    ap.add_argument("--reuse", action="store_true", help="keep detectors.json, clap.json and listen.json when present")
    a = ap.parse_args()
    os.makedirs(os.path.join(a.out, "refs"), exist_ok=True)
    index = json.load(open(os.path.join(a.captures, "index.json")))
    caps = sorted(index["captures"], key=lambda c: c["name"])
    all_caps = caps
    if a.only:
        keep = set(a.only.split(","))
        caps = [c for c in caps if c["name"] in keep]
    wav = {c["name"]: os.path.join(a.captures, c["name"] + ".wav") for c in all_caps}
    refs_spec = json.load(open(os.path.join(HERE, "refs.json")))
    root = os.environ.get("JUDGE_REFS", "/root/nmc/temp")
    # Several real references per scenario (refs.json): refs/<name>-<k>.wav, each built once.
    ref = {}
    for c in all_caps:
        spec = refs_spec.get(c["name"])
        specs = spec if isinstance(spec, list) else ([spec] if spec else [])
        paths = []
        for k, sp in enumerate(specs):
            p = os.path.join(a.out, "refs", f"{c['name']}-{k}.wav")
            if not os.path.exists(p) or c in caps:
                try:
                    sf.write(p, build_ref(sp, root, c["seconds"]), 48000, subtype="FLOAT")
                except Exception as e:  # a missing reference is left out, nothing else
                    print(f"[judge] no reference {k} for {c['name']}: {e}", file=sys.stderr)
                    continue
            paths.append(p)
        if paths:
            ref[c["name"]] = paths
    mine = [wav[c["name"]] for c in caps] + [p for c in caps for p in ref.get(c["name"], [])]
    det_path = os.path.join(a.out, "detectors.json")
    det = json.load(open(det_path)) if os.path.exists(det_path) else {}
    if not (a.reuse and all(p in det for p in mine)):
        run([PY, os.path.join(HERE, "detectors.py"), *mine, "--json", det_path + ".new"])
        det.update(json.load(open(det_path + ".new")))
        os.replace(det_path + ".new", det_path + ".last")
        json.dump(det, open(det_path, "w"), indent=1)
    clap = {}
    if not a.no_clap:
        clap_path = os.path.join(a.out, "clap.json")
        clap = json.load(open(clap_path)) if os.path.exists(clap_path) else {}
        if not (a.reuse and all(p in clap for c in caps for p in [wav[c["name"]], *ref.get(c["name"], [])])):
            # Our capture and each real reference, told the scenario's intent (is the real one recognised at all?).
            items = [(wav[c["name"]], c["intent"]) for c in caps] + [(p, c["intent"]) for c in caps for p in ref.get(c["name"], [])]
            args = [PY, os.path.join(HERE, "clap.py"), *[i[0] for i in items], "--json", clap_path + ".new"]
            for _, intent in items:
                args += ["--intent", intent]
            run(args)
            clap.update(json.load(open(clap_path + ".new")))
            os.replace(clap_path + ".new", clap_path + ".last")
            json.dump(clap, open(clap_path, "w"), indent=1)
    listen = []
    if not a.no_listen:
        jobs = []
        for c in caps:
            n = c["name"]
            if invalid_take(c):
                continue
            jobs.append({"id": n, "mode": "blind", "wav": wav[n]})
            jobs.append({"id": n, "mode": "rate", "wav": wav[n], "intent": c["intent"]})
            if n in ref and not a.no_ab:
                for k, rp in enumerate(ref[n][:2]):
                    jobs.append({"id": n, "mode": f"ab{k}" if k else "ab", "wav": wav[n], "ref": rp, "intent": c["intent"]})
        jp = os.path.join(a.out, "listen-jobs.json")
        lp = os.path.join(a.out, "listen.json")
        listen = json.load(open(lp)) if os.path.exists(lp) else []
        done = {(x["id"], x["mode"]) for x in listen}
        if jobs and not (a.reuse and all((j["id"], j["mode"]) in done for j in jobs)):
            json.dump(jobs, open(jp, "w"), indent=1)
            run([PY, os.path.join(HERE, "listen.py"), jp, lp + ".new", "--runs", str(a.runs)])
            new = json.load(open(lp + ".new"))
            fresh = {(x["id"], x["mode"]) for x in new}
            listen = [x for x in listen if (x["id"], x["mode"]) not in fresh] + new
            os.replace(lp + ".new", lp + ".last")
            json.dump(listen, open(lp, "w"), indent=1)
    elif os.path.exists(os.path.join(a.out, "listen.json")):
        listen = json.load(open(os.path.join(a.out, "listen.json")))
    # The real references heard blind (2 runs each): a scenario whose real recordings the model does not recognise
    # is one it cannot decide.
    rl = os.path.join(a.out, "listen-refs.json")
    ref_listen = json.load(open(rl)) if os.path.exists(rl) else []
    if not a.no_listen and not a.no_ref_listen:
        done = {x["wav"] for x in ref_listen}
        rjobs = [{"id": f"ref:{c['name']}:{k}", "mode": "blind", "wav": p} for c in caps for k, p in enumerate(ref.get(c["name"], []))
                 if p not in done or not a.reuse]
        if rjobs:
            rj = os.path.join(a.out, "listen-refs-jobs.json")
            json.dump(rjobs, open(rj, "w"), indent=1)
            run([PY, os.path.join(HERE, "listen.py"), rj, rl + ".new", "--runs", "2"])
            new = json.load(open(rl + ".new"))
            fresh = {x["wav"] for x in new}
            ref_listen = [x for x in ref_listen if x["wav"] not in fresh] + new
            os.replace(rl + ".new", rl + ".last")
            json.dump(ref_listen, open(rl, "w"), indent=1)
    rows = summarize(all_caps, wav, ref, det, clap, listen, ref_listen)
    json.dump({"index": index, "rows": rows}, open(os.path.join(a.out, "judge.json"), "w"), indent=1)
    write_table(rows, os.path.join(a.out, "table.md"), index)
    write_page(rows, a.out, a.captures, index)
    print(f"[judge] {os.path.join(a.out, 'index.html')}", file=sys.stderr)


def invalid_take(c):
    """A take captured on a busy machine (audio underruns, a slow scene load) says nothing about the sound: it is
    shown, never scored (the listening model does not hear it)."""
    busy = max(8.0, 0.4 * (c.get("cpus") or 20))
    load = max(c.get("loadAvg") or [0])
    if c.get("underruns", 0) >= 10:
        return f"{c['underruns']} underruns" + (f", load {load}" if load else "")
    if (c.get("sceneReadyS") or 0) > 120 and (c.get("sceneLoadAvg") or busy) >= busy:
        return f"scene load {c['sceneReadyS']} s under load {c.get('sceneLoadAvg', '?')}"
    if load > busy:
        return f"machine busy during the take (load {load})"
    return None


def summarize(caps, wav, ref, det, clap, listen, ref_listen=()):
    by = {}
    for r in listen:
        by.setdefault(r["id"], {})[r["mode"]] = r
    ref_texts = {}
    for r in ref_listen:
        ref_texts[r["wav"]] = [x["text"] for x in r["answers"]]
    rows = []
    for c in caps:
        n = c["name"]
        L = by.get(n, {})
        blind = [x["text"] for x in L.get("blind", {}).get("answers", [])]
        scores = [x["score"] for x in L.get("rate", {}).get("answers", []) if x.get("score") is not None]
        rate_texts = [x["text"] for x in L.get("rate", {}).get("answers", [])]
        ab = [x for m in ("ab", "ab1") for x in L.get(m, {}).get("answers", [])]
        heard = {}
        for t in blind:
            for k in classes_of(t):
                heard[k] = heard.get(k, 0) + 1
        refs = ref.get(n, [])
        d = dict(det.get(wav[n], {}))
        d["flags"] = D.flags_for(n, d, c, det.get(refs[0], {}) if refs else {})
        # Are the real references recognised for what they are? By the model (blind) and by CLAP (intent in its top 3).
        rt = [t for p in refs for t in ref_texts.get(p, [])]
        rm = recognised(rt, n)
        ref_model = round(rm / len(rt), 2) if rt and rm is not None else None
        rc = [clap.get(p, {}).get("intent_rank") for p in refs]
        rc = [x for x in rc if x is not None]
        ref_clap = round(sum(1 for x in rc if x <= 3) / len(rc), 2) if rc else None
        om = recognised(blind, n)
        ours_model = round(om / len(blind), 2) if blind and om is not None else None
        decidable = (ref_model is not None and ref_model >= 0.5) or (ref_clap is not None and ref_clap >= 0.5)
        undecided = None if decidable or not refs else (
            f"real references not recognised (model {ref_model if ref_model is not None else '-'}, CLAP {ref_clap if ref_clap is not None else '-'})")
        bad = invalid_take(c)
        if bad:
            # Nothing of an invalid take is scored: not the model's answers of an earlier run, not the detectors.
            blind, scores, rate_texts, ab, heard = [], [], [], [], {}
            d["flags"] = []
        rows.append({
            "invalid": bad, "undecided": undecided, "ref_model": ref_model, "ref_clap": ref_clap, "ours_model": ours_model,
            "refs": [os.path.basename(p) for p in refs],
            "name": n, "intent": c["intent"], "capture": c, "detectors": d, "ref_detectors": det.get(refs[0], {}) if refs else {},
            "has_ref": bool(refs), "clap": clap.get(wav[n], {}), "blind": blind, "heard": heard, "rate": rate_texts,
            "score": round(float(np.mean(scores)), 1) if scores else None, "scores": scores,
            "ab_correct": sum(1 for x in ab if x.get("correct")), "ab_runs": len(ab), "ab_texts": [x["text"] for x in ab],
        })
    return rows


def clap_cell(r):
    """CLAP's top answers, or why they do not count: on real recordings of this kind of event CLAP rarely names it
    (clap_validation.json)."""
    v = CLAP_VALID.get(clap_class(r["name"]) or "")
    top = ", ".join(r["clap"].get("top3", [])[:2]) or "-"
    return f"(CLAP cannot judge this class: {top})" if v and not v["judgeable"] else top


def write_table(rows, path, index):
    lines = [f"# Judge: haa {index.get('haa')} / studio {index.get('studio')} ({index.get('at')})", "",
             "| scenario | event named: real refs / ours (model) | blind hears (runs) | score 0-10 | real picked (A/B) | detector flags | CLAP top | capture | voices that played |",
             "|---|---|---|---|---|---|---|---|---|"]
    for r in rows:
        heard = ", ".join(f"{k} {v}/{len(r['blind'])}" for k, v in sorted(r["heard"].items())) or "-"
        flags = ", ".join(r["detectors"].get("flags", [])) or "-"
        c = r["capture"]
        cap = f"underruns {c.get('underruns')}, thinned {c.get('thinned')}, peak {r['detectors'].get('peak_dbfs')} dBFS"
        ab = f"{r['ab_correct']}/{r['ab_runs']}" if r["ab_runs"] else "-"
        if r.get("invalid"):
            lines.append(f"| {r['name']} | INVALID take ({r['invalid']}): not scored, capture again on a quiet machine | | | | | | {cap} | {c.get('summary', '-')} |")
            continue
        named = f"{r['ref_model'] if r['ref_model'] is not None else '-'} / {r['ours_model'] if r['ours_model'] is not None else '-'}"
        if r.get("undecided"):
            lines.append(f"| {r['name']} | {named}: judge cannot decide ({r['undecided']}) | {heard} | | | {flags} | {clap_cell(r)} | {cap} | {c.get('summary', '-')} |")
            continue
        lines.append(f"| {r['name']} | {named} | {heard} | {r['score'] if r['score'] is not None else '-'} | {ab} | {flags} | {clap_cell(r)} | {cap} | {c.get('summary', '-')} |")
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
        refp = "".join(f'<audio controls preload="none" src="refs/{e(x)}"></audio>' for x in r.get("refs", [])) or "<em>no reference</em>"
        blind = "".join(f"<li>{e(t)}</li>" for t in r["blind"])
        rate = "".join(f"<li>{e(t)}</li>" for t in r["rate"])
        ab = "".join(f"<li>{e(t)}</li>" for t in r["ab_texts"])
        det = ", ".join(f"{k} {v}" for k, v in d.items() if k not in ("flags",))
        parts.append(f"""
<section>
  <h2>{e(r['name'])}</h2>
  <p class="intent">{e(r['intent'])}</p>
  <p class="intent">voices: {e(str(r['capture'].get('summary', '-')))}</p>
  {f'<p class="flag">INVALID take ({e(r["invalid"])}): not scored</p>' if r.get('invalid') else ''}
  {f'<p class="flag">judge cannot decide: {e(r["undecided"])}</p>' if r.get('undecided') else ''}
  <p class="intent">event named by the model: real references {r.get('ref_model')}, ours {r.get('ours_model')}; CLAP recognises the real references {r.get('ref_clap')}</p>
  <div class="players">
    <figure><figcaption>scene capture</figcaption><audio controls preload="none" src="{e(rel_cap)}/{e(r['name'])}.wav"></audio></figure>
    <figure><figcaption>real recordings</figcaption>{refp}</figure>
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
