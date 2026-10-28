# Provenance

## HAA engine (Rust)

The engine semantics and every node are ported from Sona v11, the owner's own studio (SOUND_REVISION 8). Each ported file carries a `Ported from Sona v11 <file>` header and lists its deviations. Sona remains a read-only reference.

## Legacy TypeScript engine (v0.1)

`legacy/` holds the v0.1 TypeScript engine, extracted from an Etcito game. It is kept readable for later ports, is not built, and is deleted when Phase 4 closes. HAA v0.1 started as the audio engine of that game, extracted at revision `0972c2c1a0ad` of the game's audio module and then generalized so that it carries no game content. Paths below are under `legacy/`.

### What was taken and what was written

- **Taken and adapted:** the DSP primitives (filters, envelopes, delays, reverb, dynamics,
  noise, oscillators, RNG), the generator kits (impact, UI, FX, wind), the mix chain and the
  balance presets, the offline WAV path, the music instruments, arranger and synth, and the
  worklet processors. Each file's header says what changed.
- **Written for HAA:** the registry, the content registration, the spatial model, the
  conductor and voice allocation, the host and bank packaging, the measurement and report
  tools, and the public API in `legacy/src/index.ts`.
- **Not taken:** the game's vehicle, track and gameplay models and their content, its
  director, its per-game registries and its game-specific sound families and presets. Any
  game-specific content that HAA ships as an example lives under `legacy/examples/`, outside the core.

### Changes made to the extracted code

- Imports use explicit `.ts` extensions and point at the HAA layout.
- Ids and bus names are generic (for example `effects`, `ambience`, `music`); the
  seed strings of the extracted one-shot families are kept as they were, so renders stay
  bit-identical.
- Game-specific duck kinds, surface kinds, layers and messages were removed.

### Rules for the legacy code

- Core code (`legacy/src/`, `legacy/demo/`, `legacy/scripts/`) does not import from `legacy/examples/` and uses no
  absolute paths (enforced by `legacy/test/repo/hygiene.test.ts`).
- Game content is registered through the public registry, never built into the core.
