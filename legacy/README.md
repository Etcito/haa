# legacy: the v0.1 TypeScript engine

This folder holds the v0.1 TypeScript engine, extracted from an Etcito game and moved here
as-is. It is kept readable on purpose: later ports read its math and its data, but nothing in
the repository depends on it.

It is **not built**, **not typechecked** and **not exported**. No package, manifest, tsconfig
or build script points at it. Do not import from `legacy/` in `crates/` or `packages/`.

## What is here

- `src/`: the engine sources (contract, dsp, gen, mix, spatial, content, host, worklet, ...).
- `test/`: the v0.1 tests.
- `worklet/`: the prebuilt AudioWorklet processor bundle.
- `demo/` and `examples/`: the demo app and the quickstart example.
- `scripts/`: the TypeScript scripts that used to build and run the above
  (`bake.ts`, `build-demo.ts`, `build-worklet.ts`, `build-dist.ts`, `serve.ts`,
  `gpu-run.ts`) plus `scripts/proof/`.
- `vitest.config.ts` and `tsconfig.build.json`: the v0.1 test and build configuration.

The two current scripts that stayed at the repository root, `scripts/build-wasm.mjs` and
`scripts/release-pack.mjs`, are not part of this legacy tree.

## What later ports read here

- `legacy/src/spatial/`: the spatial stage of decisions 10 and 11 (listener frame, distance
  gain, pan, lowpass, occlusion, Doppler, multi-listener rules). See the Architecture
  contract, section 6, which restates these formulas.
- `legacy/src/gen/windModel.ts` and the related content: the wind bands and the crowd
  models, together with the content sets under `legacy/src/content/` that drive them.

## Lifetime

This folder is deleted when Phase 4 closes.
