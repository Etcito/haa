"""A cheap second opinion: CLAP (laion/clap-htsat-unfused) scores each clip against free-text prompts.

    clap.py <wav> [<wav> ...] --json out.json [--intent "<text>" ...]

For every clip: the probabilities (softmax over the prompt set, CLAP's own logit scale) of the intended event
(when given), of each defect the owner named, and of every other everyday description; the top 3.
CPU is enough (about 0.3 s a clip).
"""
import argparse
import json
import os
import sys

import librosa
import numpy as np
import torch

DEFECTS = {
    "thunder": "thunder rumbling in the distance",
    "organ": "a church organ holding a sustained chord",
    "drone": "a tonal electronic hum or drone",
    "bell": "a small bell or chime ringing",
    "water": "water running or trickling in a stream",
    "toy gun": "a plastic toy cap gun clicking",
    "nail": "a hammer striking a nail into concrete",
    "wet cloth": "a wet cloth slapped against a wall",
    "distortion": "harsh distorted crackling noise",
}
EVERYDAY = [
    "a rubber ball bouncing on a concrete floor", "a ball rolling on concrete", "many balls bouncing on the ground",
    "a hand grenade landing on concrete", "an explosion", "a gunshot outdoors", "a gunshot echoing in a large hall",
    "footsteps on snow", "footsteps on a concrete floor indoors", "small pebbles scattering on the ground",
    "a submachine gun burst", "a shotgun blast", "a glass bottle shattering", "a wooden crate falling on the ground",
    "rain falling on a roof", "rain falling on a metal roof", "silence",
    "something heavy being dragged across a floor", "a wooden crate sliding and scraping on concrete",
    "a metal object scraping on a steel plate", "footsteps on a metal floor", "footsteps on a wooden floor",
    "a glass bottle rolling on the floor", "brass bullet casings falling on concrete", "a single dry click or clack",
]

# Which real-reference class validates each scenario (clap_validate.py writes clap_validation.json: per class,
# how often CLAP puts the right prompt in its top 3 on real recordings; under half = CLAP cannot judge it).
SCENARIO_CLASS = [
    ("ball-metal", "ball"), ("balls-", "manyballs"), ("ball-", "ball"), ("grenade-explode", "explosion"),
    ("shot-", "pistol"), ("casings", "casing"), ("walk-snow", "footstep-snow"), ("walk-concrete", "footstep-concrete"),
    ("walk-metal", "footstep-metal"), ("walk-wood", "footstep-wood"), ("pebbles", "pebbles"), ("smg", "smg"),
    ("shotgun", "shotgun"), ("bottle-concrete", "bottle-break"), ("crate-slide", "slide"), ("crate-drop", "wood-crate"),
    ("rain-shed", "rain-metal"), ("rain", "rain"),
]


_VP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "clap_validation.json")
VALIDATION = json.load(open(_VP)) if os.path.exists(_VP) else {}


def class_of(scenario):
    return next((c for p, c in SCENARIO_CLASS if scenario.startswith(p)), None)


def features(out):
    """transformers 5 returns an output object from get_*_features, 4.x a tensor."""
    return getattr(out, "pooler_output", out) if not torch.is_tensor(out) else out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wavs", nargs="+")
    ap.add_argument("--json", required=True)
    ap.add_argument("--intent", action="append", default=[])
    a = ap.parse_args()
    from transformers import ClapModel, ClapProcessor
    model = ClapModel.from_pretrained("laion/clap-htsat-unfused").eval()
    proc = ClapProcessor.from_pretrained("laion/clap-htsat-unfused")
    out = {}
    for i, path in enumerate(a.wavs):
        intent = a.intent[i] if i < len(a.intent) else None
        prompts = ([intent] if intent else []) + list(DEFECTS.values()) + EVERYDAY
        y, _ = librosa.load(path, sr=48000, mono=True)
        peak = float(np.abs(y).max())
        if peak > 1e-6:
            y = y / peak * 0.7
        with torch.no_grad():
            inp = proc(text=prompts, audio=[y], sampling_rate=48000, return_tensors="pt", padding=True)
            logits = model(**inp).logits_per_audio[0]
            p = torch.softmax(logits, -1).numpy()
        names = (["intent"] if intent else []) + list(DEFECTS) + EVERYDAY
        r = {"p": {n: round(float(v), 3) for n, v in zip(names, p)}}
        order = np.argsort(-p)
        r["top3"] = [names[k] for k in order[:3]]
        r["intent_rank"] = int(np.where(order == 0)[0][0]) + 1 if intent else None
        r["defect_top"] = max(DEFECTS, key=lambda d: r["p"][d])
        # Whether CLAP recognises this kind of event at all on real recordings (clap_validation.json).
        cls = class_of(os.path.splitext(os.path.basename(path))[0])
        v = VALIDATION.get(cls) if cls else None
        r["class"] = cls
        r["judgeable"] = None if v is None else bool(v["judgeable"])
        out[path] = r
        print(path.split("/")[-1], r["top3"], "intent rank", r["intent_rank"], file=sys.stderr)
    json.dump(out, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
