# Provenance of this crate

A Rust port of the inference of **tiny-voca** (https://github.com/n4ze3m/tiny-voca), Apache License 2.0
(see LICENSE in this folder). This crate is distributed under the same licence.

- Source revision: 0eea2be213b53ec0b1949c169b80e611544718e1 (2026-11-19).
- Ported: `js/tiny-voca.js` (the GGUF reader, the transformer, the duration predictor, the source-filter head and
  `gaussianNoise`), checked against `src/tiny_voca/model.py` (`encode`, `predict_durations`, `expand`,
  `predict_prosody`, `hard_prosody`, `decode`, `widen_pitch`, `pitch_wobble`, `SourceFilter.forward`, `synthesize`).
- Changed: the model runs in stages with plain arrays between them; rotary tables, durations and the prosody
  clamps follow model.py's float32 arithmetic; the voice is a blend of the two speaker rows plus `post` edits.
- Written for HAA: `edit.rs` (the voice's post edits; the envelope edit is a port of the offline `tvd.py`
  `warp_cepstrum` of the HAA voice work, not of tiny-voca), `perf.rs`, `voice.rs`, the shared model and the
  resampler in `lib.rs`.
- Not included: weights. The host passes in the bytes of an official GGUF from https://huggingface.co/n4ze3m/tiny-voca
  (`gguf/tiny-voca-orig.gguf`, `-f16`, `-q8_0`, `-q4_0` or `-js`); nothing is bundled or committed.
- Not ported: the dizione phonemizer (a separate crate) and training.
