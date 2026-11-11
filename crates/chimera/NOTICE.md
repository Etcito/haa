# Provenance of this crate

Vendored subset of **chimera-vocal-synth** (https://github.com/jamesondh/chimera-vocal-synth), crate ,
MIT License, Copyright (c) 2026 Jameson Hodge (see LICENSE in this folder).

- Source revision: eed4a5ff2695ad151df2210d012b2a4ab584f099, plus one local addition in 
  (`Whistle::next_stable`, `next_shaped`, `acoustic_phase` and their test) written for the HAA whistle node.
- Taken: the library sources under `src/` that `haa-core` uses (anatomy, airway, control, phonetics, sources, voice, engine, creatures, math, physics).
- Left out: the CLI and wasm crates, the integration tests and golden data, and the modules nothing in HAA calls
  (analysis/fft, analysis/pitch, util/optimize, phonetics/songs, airway/frequency).
- Why: haa-core path-depended on a checkout outside this repository, so it did not build from its commits.
