# Provenance of this crate

Vendored subset of **chimera-vocal-synth** (https://github.com/jamesondh/chimera-vocal-synth), crate ,
MIT License, Copyright (c) 2026 Jameson Hodge (see LICENSE in this folder).

- Source revision: e780f7dd1159e82bb1aed70f273b4b5b69cf74aa (main, 2026-11-13: denormals zeroed at control rate, airway stability at low device rates and pressed-low tongues, IPA marks, Korean and Russian front ends), plus one local addition in 
  (`Whistle::next_stable`, `next_shaped`, `acoustic_phase` and their test) written for the HAA whistle node.
- Taken: the library sources under `src/` that `haa-core` uses (anatomy, airway, control, phonetics, sources, voice, engine, creatures, math, physics).
- Left out: the CLI and wasm crates, the integration tests and golden data, and the modules nothing in HAA calls
  (analysis/fft, analysis/pitch, util/optimize, phonetics/songs, airway/frequency); phonetics/mod.rs drops its `songs` module.
- Why: haa-core path-depended on a checkout outside this repository, so it did not build from its commits.
