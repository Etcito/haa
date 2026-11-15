"""The judge's controls, from real recordings (the owner's own comparisons).

    controls.py <refs fs dir> <jobs.json>

Positive controls: real recordings told their true intent (must be described right and score high).
Negative controls: real recordings of what the owner said our sounds were like (thunder, organ, bell,
running water, a toy gun, a nail, a wet rag), told the intent our sound had (must be named for what they are,
blind, and score low). Run with listen.py like the captures.
"""
import json
import sys

POSITIVE = [
    ("ball/650982.wav", "a rubber ball bouncing on pavement outdoors"),
    ("ball/267882.wav", "a rubber ball bouncing on a hard floor"),
    ("pistol/233318.wav", "a single 9 mm pistol shot"),
    ("pistol/402789.wav", "a small pistol shot fired indoors"),
    ("footstep-snow/613849.wav", "footsteps of a person in boots walking on snow"),
    ("footstep-concrete/813622.wav", "footsteps on a concrete floor"),
    ("pebbles/787857.wav", "a handful of gravel thrown onto the ground"),
    ("grenade/516914.wav", "an explosion outdoors"),
    ("shotgun/473846.wav", "one shotgun shot"),
    ("bottle-break/212698.wav", "a glass bottle breaking on a hard floor"),
    ("steeldrop/259553.wav", "a small steel object dropped on a hard floor"),
    ("rain-tile/333509.wav", "rain falling on a roof"),
    ("smg/390663.wav", "a submachine gun burst"),
]
NEGATIVE = [
    ("nogo-thunder/234736.wav", "a rubber ball rolling on concrete outdoors", "thunder"),
    ("nogo-thunder/347562.wav", "many rubber balls landing on concrete", "thunder"),
    ("nogo-organ/522566.wav", "a pistol shot reverberating in a large empty concrete building", "organ"),
    ("nogo-bell/553214.wav", "a metal hand grenade landing on concrete", "bell"),
    ("water/189672.wav", "small pebbles thrown onto concrete", "water"),
    ("nogo-toy/215847.wav", "a 9 mm pistol shot", "toy"),
    ("nogo-nail/202231.wav", "footsteps on a concrete floor indoors", "nail"),
    ("nogo-wetrag/134710.wav", "footsteps on snow", "wet cloth"),
]


def main():
    root, out = sys.argv[1], sys.argv[2]
    jobs = []
    for f, intent in POSITIVE:
        jobs.append({"id": f"pos:{f}", "mode": "blind", "wav": f"{root}/{f}", "kind": "positive"})
        jobs.append({"id": f"pos:{f}", "mode": "rate", "wav": f"{root}/{f}", "intent": intent, "kind": "positive"})
    for f, intent, what in NEGATIVE:
        jobs.append({"id": f"neg:{f}", "mode": "blind", "wav": f"{root}/{f}", "kind": "negative", "is": what})
        jobs.append({"id": f"neg:{f}", "mode": "rate", "wav": f"{root}/{f}", "intent": intent, "kind": "negative", "is": what})
    json.dump(jobs, open(out, "w"), indent=1)
    print(len(jobs), "jobs", file=sys.stderr)


if __name__ == "__main__":
    main()
