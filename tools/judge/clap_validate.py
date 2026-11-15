"""Where CLAP can judge: on real recordings of each class, how often the right prompt is in its top 3.

    clap_validate.py <refs fs dir> [--per-class 8] --json clap_validation.json

A class whose real recordings CLAP does not recognise (right prompt in the top 3 for under half of them) is
marked `judgeable: false`; the judge's table then says CLAP cannot judge that scenario. The defect classes
(the owner's thunder, organ, bell, water, toy gun, nail, wet rag) are checked the same way on the nogo-* folders.
"""
import argparse
import glob
import json
import os
import sys

os.environ.setdefault("OMP_NUM_THREADS", "4")
os.nice(15)

import librosa  # noqa: E402
import numpy as np  # noqa: E402
import torch  # noqa: E402

torch.set_num_threads(4)
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from clap import DEFECTS, EVERYDAY, features  # noqa: E402

CLASS_PROMPT = {
    "ball": "a rubber ball bouncing on a concrete floor", "ballroll": "a ball rolling on concrete",
    "manyballs": "many balls bouncing on the ground", "pebbles": "small pebbles scattering on the ground",
    "footstep-snow": "footsteps on snow", "footstep-concrete": "footsteps on a concrete floor indoors",
    "footstep-metal": "footsteps on a metal floor", "footstep-wood": "footsteps on a wooden floor",
    "pistol": "a gunshot outdoors", "smg": "a submachine gun burst", "shotgun": "a shotgun blast",
    "bottle-break": "a glass bottle shattering", "wood-crate": "a wooden crate falling on the ground",
    "rain": "rain falling on a roof", "rain-metal": "rain falling on a metal roof",
    "slide": "something heavy being dragged across a floor", "scrape": "a metal object scraping on a steel plate",
    "casing": "brass bullet casings falling on concrete", "explosion": "an explosion",
    "nogo-thunder": DEFECTS["thunder"], "nogo-organ": DEFECTS["organ"], "nogo-bell": DEFECTS["bell"],
    "water": DEFECTS["water"], "nogo-toy": DEFECTS["toy gun"], "nogo-nail": DEFECTS["nail"], "nogo-wetrag": DEFECTS["wet cloth"],
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("refs")
    ap.add_argument("--per-class", type=int, default=8)
    ap.add_argument("--json", required=True)
    a = ap.parse_args()
    from transformers import ClapModel, ClapProcessor
    model = ClapModel.from_pretrained("laion/clap-htsat-unfused").eval()
    proc = ClapProcessor.from_pretrained("laion/clap-htsat-unfused")
    prompts = list(DEFECTS.values()) + EVERYDAY
    with torch.no_grad():
        t = proc(text=prompts, return_tensors="pt", padding=True)
        temb = features(model.get_text_features(**t))
        temb = temb / temb.norm(dim=-1, keepdim=True)
    out = {}
    for cls, prompt in CLASS_PROMPT.items():
        k = prompts.index(prompt)
        ranks = []
        for f in sorted(glob.glob(os.path.join(a.refs, cls, "*.wav")))[: a.per_class]:
            y, _ = librosa.load(f, sr=48000, mono=True, duration=10)
            if np.abs(y).max() > 1e-6:
                y = y / np.abs(y).max() * 0.7
            with torch.no_grad():
                inp = proc(audio=[y], sampling_rate=48000, return_tensors="pt")
                aemb = features(model.get_audio_features(**inp))
                aemb = aemb / aemb.norm(dim=-1, keepdim=True)
            order = torch.argsort(-(aemb @ temb.T)[0]).tolist()
            ranks.append(order.index(k) + 1)
        if not ranks:
            continue
        top3 = sum(1 for r in ranks if r <= 3) / len(ranks)
        out[cls] = {"prompt": prompt, "n": len(ranks), "top3": round(top3, 2), "median_rank": float(np.median(ranks)), "judgeable": top3 >= 0.5}
        print(f"{cls:18s} top3 {top3:.2f} median rank {np.median(ranks):.0f}/{len(prompts)}  {'ok' if top3 >= 0.5 else 'CLAP cannot judge'}", file=sys.stderr)
    json.dump(out, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
