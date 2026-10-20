# Provenance

HAA started as the audio engine of an Etcito game. The engine was extracted into this
repository at revision `0972c2c1a0ad` of that game's audio module and then generalized so that
it carries no game content.

## What was taken and what was written

- **Taken and adapted:** the DSP primitives (filters, envelopes, delays, reverb, dynamics,
  noise, oscillators, RNG), the generator kits (impact, UI, FX, wind), the mix chain and the
  balance presets, the offline WAV path, the music instruments, arranger and synth, and the
  worklet processors. Each file's header says what changed.
- **Written for HAA:** the registry, the content registration, the spatial model, the
  conductor and voice allocation, the host and bank packaging, the measurement and report
  tools, and the public API in `src/index.ts`.
- **Not taken:** the game's vehicle, track and gameplay models and their content, its
  director, its per-game registries and its game-specific sound families and presets. Any
  game-specific content that HAA ships as an example lives under `examples/`, outside the core.

## Changes made to the extracted code

- Imports use explicit `.ts` extensions and point at the HAA layout.
- Ids and bus names are generic (for example `effects`, `ambience`, `music`); the
  seed strings of the extracted one-shot families are kept as they were, so renders stay
  bit-identical.
- Game-specific duck kinds, surface kinds, layers and messages were removed.

## Rules for new code

- Core code (`src/`, `demo/`, `scripts/`) does not import from `examples/` and uses no
  absolute paths (enforced by `test/repo/hygiene.test.ts`).
- Game content is registered through the public registry, never built into the core.
