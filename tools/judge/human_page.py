"""The fallback where no model can decide: a page for a human ear, ours next to real recordings, level-matched.

    human_page.py <judge output dir> [--all] [--only a,b]

From a judge run (judge.json, captures/, refs/), for the scenarios the judge cannot decide and the model's blind
spots (footsteps, the pistol and the other shots, casings) - or every scenario with --all - writes
<dir>/human/index.html with, per scenario, our capture and its 2-3 real references, level-matched: every clip of a
scenario at the same RMS over its active part (-23 dBFS, or lower when a clip's peaks would pass -1 dBFS there), as
16-bit wavs under human/<scenario>/. The
listener marks each scenario "as real" or "worse" with a word on what it sounds like; the answers stay in the
browser and a button copies them all as text to send back. Local only: the references are never deployed.
"""
import argparse
import html
import json
import os

import numpy as np
import soundfile as sf

SPECS = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "refs.json")))
BLIND_SPOTS = ("walk-", "shot-", "casings", "smg", "shotgun")
TARGET_DB = -23.0


def active_rms_db(y, sr):
    """RMS over the 100 ms frames within 30 dB of the loudest: the level of the event, not of its silence."""
    m = y.mean(1) if y.ndim > 1 else y
    hop = sr // 10
    n = len(m) // hop
    if n == 0:
        return -120.0
    e = (m[: n * hop].reshape(n, hop) ** 2).mean(1)
    db = 10 * np.log10(e + 1e-12)
    act = e[db > db.max() - 30]
    return float(10 * np.log10(act.mean() + 1e-12))


def reachable_db(src):
    """The loudest active level this clip reaches with its peak at -1 dBFS."""
    y, sr = sf.read(src, always_2d=True, dtype="float32")
    peak_db = 20 * np.log10(float(np.abs(y).max()) + 1e-12)
    return active_rms_db(y, sr) + (-1.0 - peak_db)


def matched(src, dst, target_db):
    """Write the clip at `target_db` active RMS (16-bit)."""
    y, sr = sf.read(src, always_2d=True, dtype="float32")
    g = 10 ** ((target_db - active_rms_db(y, sr)) / 20)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    sf.write(dst, np.clip(y * g, -1, 1), sr, subtype="PCM_16")
    return round(20 * np.log10(g), 1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--only")
    a = ap.parse_args()
    j = json.load(open(os.path.join(a.dir, "judge.json")))
    rows = j["rows"]
    only = set(a.only.split(",")) if a.only else None
    pick = [r for r in rows if (only and r["name"] in only) or (not only and (a.all or r.get("undecided") or r["name"].startswith(BLIND_SPOTS)))]
    pick = [r for r in pick if r.get("refs")]
    e = html.escape
    parts = []
    for r in pick:
        n = r["name"]
        # Each reference says where it comes from: the owner's own pick (a target) or a game sound library.
        specs = SPECS.get(n, []) if isinstance(SPECS.get(n), list) else []
        def label(k):
            sp = specs[k] if k < len(specs) else {}
            if sp.get("target"):
                return f"real {k + 1}: your pick"
            return f"real {k + 1}" + (": game SFX" if "game" in sp.get("source", "") else "")
        clips = [("ours", os.path.join(a.dir, "captures", n + ".wav"))] + [(label(k), os.path.join(a.dir, "refs", f)) for k, f in enumerate(r["refs"][:3])]
        clips = [(label, src) for label, src in clips if os.path.exists(src)]
        # One level for the whole scenario: -23 dBFS active RMS, or lower if a clip's peaks would pass -1 dBFS there.
        target = min([TARGET_DB] + [reachable_db(src) for _, src in clips])
        players = []
        for label, src in clips:
            rel = f"{n}/{label.split(':')[0].replace(' ', '-')}.wav"
            matched(src, os.path.join(a.dir, "human", rel), target)
            players.append(f'<figure><figcaption>{e(label)}</figcaption><audio controls preload="none" src="{e(rel)}"></audio></figure>')
        why = r.get("undecided") or "the model cannot hear this kind of sound (blind spot)"
        if r.get("ref_note"):
            why += f"; references: {r['ref_note']}"
        parts.append(f"""
<section data-name="{e(n)}">
  <h2>{e(n)}</h2>
  <p class="intent">{e(r['intent'])}</p>
  <p class="why">Why by ear: {e(why)}</p>
  <div class="players">{''.join(players)}</div>
  <div class="verdict">
    <label><input type="radio" name="v-{e(n)}" value="as real"> as good as the real ones</label>
    <label><input type="radio" name="v-{e(n)}" value="worse"> worse</label>
    <label><input type="checkbox" class="flag" value="echo before shot"> echo before the shot</label>
    <label><input type="checkbox" class="flag" value="distant"> distant</label>
    <input type="text" class="note" placeholder="what it sounds like (a word or two)">
  </div>
</section>""")
    index = j.get("index", {})
    page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Judge by ear</title>
<style>
:root {{ --bg: #fafaf9; --fg: #1c1917; --muted: #78716c; --line: #e7e5e4; --accent: #0f766e; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg: #1c1917; --fg: #f5f5f4; --muted: #a8a29e; --line: #44403c; --accent: #5eead4; }} }}
body {{ background: var(--bg); color: var(--fg); font: 15px/1.5 system-ui, sans-serif; margin: 0 auto; max-width: 980px; padding: 16px; }}
section {{ border-top: 1px solid var(--line); padding: 12px 0; }}
h2 {{ margin: 0 0 4px; font-size: 18px; }} .intent, .why {{ color: var(--muted); margin: 0 0 6px; }}
.players {{ display: flex; flex-wrap: wrap; gap: 12px; }} figure {{ margin: 0; }} figcaption {{ font-size: 13px; color: var(--muted); }}
audio {{ width: min(300px, 100%); }}
.verdict {{ display: flex; flex-wrap: wrap; gap: 12px; align-items: center; margin-top: 8px; }}
.note {{ flex: 1; min-width: 180px; padding: 4px 6px; background: transparent; color: var(--fg); border: 1px solid var(--line); border-radius: 4px; }}
button {{ background: var(--accent); color: var(--bg); border: 0; border-radius: 4px; padding: 6px 12px; font: inherit; cursor: pointer; }}
#out {{ width: 100%; min-height: 6em; margin-top: 8px; background: transparent; color: var(--fg); border: 1px solid var(--line); }}
</style></head><body>
<h1>Judge by ear</h1>
<p>Scene build haa {e(str(index.get('haa')))} / haa-studio {e(str(index.get('studio')))}. Each scenario: our capture of the scene's output and real
recordings of the same event, all at the same loudness. About a minute each: play them, mark ours, say what it sounds like if worse.
Your answers stay in this browser; the button at the end copies them to send. Local only.</p>
{''.join(parts)}
<section><button id="copy">Copy my answers</button><textarea id="out" readonly></textarea></section>
<script>
const KEY = 'judge-by-ear:' + {json.dumps(str(index.get('haa')) + '/' + str(index.get('studio')))};
let saved = {{}};
try {{ saved = JSON.parse(localStorage.getItem(KEY) || '{{}}'); }} catch (e) {{}}
const store = () => {{ try {{ localStorage.setItem(KEY, JSON.stringify(saved)); }} catch (e) {{}} }};
for (const s of document.querySelectorAll('section[data-name]')) {{
  const n = s.dataset.name, v = saved[n] || {{}};
  for (const r of s.querySelectorAll('input[type=radio]')) {{
    r.checked = r.value === v.verdict;
    r.addEventListener('change', () => {{ saved[n] = {{ ...saved[n], verdict: r.value }}; store(); }});
  }}
  for (const c of s.querySelectorAll('input.flag')) {{
    c.checked = (v.flags || []).includes(c.value);
    c.addEventListener('change', () => {{
      saved[n] = {{ ...saved[n], flags: [...s.querySelectorAll('input.flag')].filter((x) => x.checked).map((x) => x.value) }};
      store();
    }});
  }}
  const t = s.querySelector('.note');
  t.value = v.note || '';
  t.addEventListener('input', () => {{ saved[n] = {{ ...saved[n], note: t.value }}; store(); }});
}}
document.getElementById('copy').addEventListener('click', () => {{
  const lines = Object.entries(saved).map(([n, v]) => n + ': ' + (v.verdict || '-') + ((v.flags || []).length ? ' [' + v.flags.join(', ') + ']' : '') + (v.note ? ' (' + v.note + ')' : ''));
  const out = document.getElementById('out');
  out.value = lines.join('\\n');
  out.select();
  try {{ navigator.clipboard.writeText(out.value); }} catch (e) {{}}
}});
</script>
</body></html>"""
    os.makedirs(os.path.join(a.dir, "human"), exist_ok=True)
    open(os.path.join(a.dir, "human", "index.html"), "w").write(page)
    print(f"{len(pick)} scenarios -> {os.path.join(a.dir, 'human', 'index.html')}")


if __name__ == "__main__":
    main()
