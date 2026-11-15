"""The judge's ears: an audio-language model that listens to a wav and answers in words.

    listen.py jobs.json results.json [--model Qwen/Qwen2-Audio-7B-Instruct] [--runs 3] [--cpu]

jobs.json: [{"id": ..., "mode": "blind" | "rate" | "ab", "wav": path, "intent": str, "ref": path}, ...]
  blind: "describe exactly what you hear" (the model is never told what it should be);
  rate:  told the intended event, a realism score 0-10 and the defects;
  ab:    two clips (ours and a real recording, order shuffled per run): which one is the real recording, what differs.
Each job runs `runs` times with sampling (temperature 0.7, seeds 0..runs-1); the answers are kept verbatim.

The model loads once, 4-bit (bitsandbytes) on the GPU (about 6 GB), or bf16 on the CPU with --cpu.
One listen job runs on the machine at a time ($JUDGE_LOCK_DIR/listen.lock, default /root/work/haa/run/judge; it waits),
niced, with 4 CPU threads.
Audio goes in as the model expects it: mono, 16 kHz, peak-normalised to -3 dBFS (the scene's levels are
reported by the detectors, not by the model).
"""
import argparse
import fcntl
import json
import os
import random
import re
import sys
import time

# Gentle on a shared machine: niced, 4 CPU threads (owner, 2026-11-15: "les agents utilisent trop le CPU").
os.environ.setdefault("OMP_NUM_THREADS", "4")
os.environ.setdefault("MKL_NUM_THREADS", "4")
os.nice(15)

import librosa  # noqa: E402
import numpy as np  # noqa: E402
import torch  # noqa: E402

torch.set_num_threads(4)

BLIND = ("Listen carefully. Describe exactly what you hear: what object or objects, what event or action, and what kind of "
         "place (outdoors, small room, large hall...). Then say whether anything sounds unnatural, synthetic, or out of place, "
         "and what it sounds like instead.")
RATE = ("This recording is supposed to be: {intent}. Rate how realistic it sounds as that, from 0 (not at all) to 10 "
        "(indistinguishable from a real recording). Answer on the first line as 'Score: N', then list every defect you hear.")
AB = ("Two recordings follow. One is a real recording, the other is synthesized; both are supposed to be: {intent}. "
      "Which one is the real recording, Audio 1 or Audio 2? Answer on the first line as 'Real: 1' or 'Real: 2', then say "
      "what differs between them.")


def load_audio(path, sr):
    y, _ = librosa.load(path, sr=sr, mono=True)
    peak = float(np.abs(y).max()) if len(y) else 0.0
    if peak > 1e-6:
        y = y * (10 ** (-3 / 20) / peak)
    return y.astype(np.float32)


class Ears:
    def __init__(self, model_id, cpu=False):
        from transformers import AutoProcessor, Qwen2AudioForConditionalGeneration
        self.processor = AutoProcessor.from_pretrained(model_id)
        if cpu:
            self.model = Qwen2AudioForConditionalGeneration.from_pretrained(model_id, torch_dtype=torch.bfloat16, device_map="cpu")
        else:
            from transformers import BitsAndBytesConfig
            q = BitsAndBytesConfig(load_in_4bit=True, bnb_4bit_compute_dtype=torch.float16, bnb_4bit_quant_type="nf4")
            self.model = Qwen2AudioForConditionalGeneration.from_pretrained(model_id, quantization_config=q, device_map="cuda:0")
        self.model.eval()
        self.sr = self.processor.feature_extractor.sampling_rate

    def ask(self, wavs, prompt, seed):
        content = []
        for i, w in enumerate(wavs):
            if len(wavs) > 1:
                content.append({"type": "text", "text": f"Audio {i + 1}:"})
            content.append({"type": "audio", "audio_url": w})
        content.append({"type": "text", "text": prompt})
        conv = [{"role": "user", "content": content}]
        text = self.processor.apply_chat_template(conv, add_generation_prompt=True, tokenize=False)
        audios = [load_audio(w, self.sr) for w in wavs]
        inputs = self.processor(text=text, audio=audios, sampling_rate=self.sr, return_tensors="pt", padding=True)
        inputs = {k: (v.to(self.model.device) if hasattr(v, "to") else v) for k, v in inputs.items()}
        if "input_features" in inputs:
            inputs["input_features"] = inputs["input_features"].to(self.model.dtype if self.model.dtype != torch.float32 else torch.float32)
        torch.manual_seed(seed)
        with torch.no_grad():
            out = self.model.generate(**inputs, max_new_tokens=220, do_sample=True, temperature=0.7, top_p=0.9)
        out = out[:, inputs["input_ids"].shape[1]:]
        return self.processor.batch_decode(out, skip_special_tokens=True)[0].strip()


def score_of(text):
    m = re.search(r"score\s*[:=]?\s*(\d+(?:\.\d+)?)", text, re.I)
    return float(m.group(1)) if m else None


def real_of(text):
    m = re.search(r"real\s*[:=]?\s*(?:audio\s*)?([12])", text, re.I)
    return int(m.group(1)) if m else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("jobs")
    ap.add_argument("results")
    ap.add_argument("--model", default="Qwen/Qwen2-Audio-7B-Instruct")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--cpu", action="store_true")
    a = ap.parse_args()
    jobs = json.load(open(a.jobs))
    # One listening job on the machine at a time (the GPU and the CPU are shared): wait for the lock.
    lock_dir = os.environ.get("JUDGE_LOCK_DIR", "/root/work/haa/run/judge")
    os.makedirs(lock_dir, exist_ok=True)
    lock = open(os.path.join(lock_dir, "listen.lock"), "w")
    print(f"[listen] waiting for {lock.name}", file=sys.stderr, flush=True)
    fcntl.flock(lock, fcntl.LOCK_EX)
    t0 = time.time()
    ears = Ears(a.model, a.cpu)
    print(f"[listen] {a.model} loaded in {time.time() - t0:.0f} s", file=sys.stderr, flush=True)
    results = []
    for job in jobs:
        answers = []
        for run in range(a.runs):
            if job["mode"] == "blind":
                text = ears.ask([job["wav"]], BLIND, run)
                answers.append({"text": text})
            elif job["mode"] == "rate":
                text = ears.ask([job["wav"]], RATE.format(intent=job["intent"]), run)
                answers.append({"text": text, "score": score_of(text)})
            else:
                # Order shuffled per run (seeded): position bias is measured, not hidden.
                ours_first = random.Random(run * 7919 + len(job["wav"])).random() < 0.5
                pair = [job["wav"], job["ref"]] if ours_first else [job["ref"], job["wav"]]
                text = ears.ask(pair, AB.format(intent=job["intent"]), run)
                pick = real_of(text)
                real_pos = 2 if ours_first else 1
                answers.append({"text": text, "picked": pick, "real_was": real_pos, "correct": None if pick is None else pick == real_pos})
        results.append({**job, "answers": answers})
        print(f"[listen] {job['id']} {job['mode']}: {answers[0]['text'][:120]!r}", file=sys.stderr, flush=True)
        json.dump(results, open(a.results, "w"), indent=1)
    print(f"[listen] {len(jobs)} jobs x {a.runs} runs in {time.time() - t0:.0f} s", file=sys.stderr)


if __name__ == "__main__":
    main()
