# HAA architecture

HAA ("Hear Anything Anywhere") is a procedural sound engine that works with any game. It ships as the TypeScript package `@etcito/haa`. `src/contract.ts` holds every public type of the API. Implementations build against those types and never rename a public type or field. Only the lead decides a contract change.

## 1. Deliverable and stack

HAA is the procedural sound engine that the owner's games and HAA Studio drive. It uses Sona's graph model: a patch is Sona's graph JSON plus a header (§7). The published inputs are the game's whole contract with a sound (§8).

- **Deliverable:** the engine library `@etcito/haa` v0.2.0 (MIT, org Etcito, private). It has:
  - a Rust workspace: `crates/haa-core` (engine), `crates/haa-wasm` (wasm build), `crates/haa-cli` (native build);
  - a native CLI `haa`;
  - the wasm engine `pkg/haa.wasm`;
  - a JS host (AudioWorklet player, offline renderer for Worker and Node, WAV writer, Node CLI);
  - an A/B listening page against Sona's JS engine.
- **No deploy:** HAA is not a deployed service and has no deploy artefact. The listen server is a loopback dev tool.
- **Consumers:**
  - haa-studio uses `"@etcito/haa": "file:../haa/packages/haa"`;
  - games use a GitHub release tgz built locally (decision 13);
  - Royal Fiesta keeps its v0.1.1 pin until a later phase switches it.
- **Repos:** `haa` is the engine. There is no DOM and no Vue in the engine or the package. Sona (the owner's Sona v11 studio, SOUND_REVISION 8) is a read-only reference. haa-studio, NodeGraph, CRM atoms and Royal Fiesta are out of scope.
- **Rust:** `rust-toolchain.toml` pins channel `1.96` with target `wasm32-unknown-unknown`. Edition 2024, workspace resolver `3`.
  - Crate dependencies: only `serde` (derive) and `serde_json`, declared once in `[workspace.dependencies]`. No wasm-bindgen, libm, rand, hound, inventory or any other crate.
  - `[profile.release]`: `panic = "abort"`, `lto = true`, `codegen-units = 1`, `opt-level = 3`. `Cargo.lock` is committed.
- **TypeScript:** strict, ES2022, module/moduleResolution `NodeNext`.
  - Erasable syntax only: no `enum`, no `namespace`, no constructor parameter properties, no `import =`.
  - Every relative import ends in `.ts`. Type-only imports use `import type`.
  - esbuild bundles the JS. tsc only typechecks and emits `.d.ts`.
- **Node and npm:** Node `>=22.18`. The root `package.json` is an npm workspace root (`workspaces: ["packages/haa"]`).
  - devDependencies are pinned exactly: typescript 5.9.3, esbuild 0.28.2, @types/node 22.20.5. No other npm dependency.
  - `@etcito/haa` has zero runtime dependencies. `package-lock.json` is committed, and `npm ci` restores `node_modules`.
- **Wasm ABI:** plain `extern "C"` exports over linear memory, with no wasm-bindgen and no JS imports (§13). wasm-bindgen glue needs `TextDecoder`, which `AudioWorkletGlobalScope` lacks.
- **Wasm artefact:** `packages/haa/pkg/haa.wasm` is git-ignored. `npm run build` and `npm run release:pack` produce it.
- **Commands** (from the repo root):
  - `npm ci`: install the dev tools.
  - `npm run build`: `node scripts/build-wasm.mjs` (cargo release build of `haa-wasm`, copied to `packages/haa/pkg/haa.wasm`), then `npm run build -w @etcito/haa` (esbuild bundles plus tsc declarations into `packages/haa/dist/`).
  - `npm run typecheck`: `tsc -p tsconfig.json --noEmit` over `packages/haa/src`, `packages/haa/test` and `tools/listen`.
  - `npm test`: `cargo test --workspace --quiet`.
  - `npm run test:host`: `node --test packages/haa/test/`.
  - `npm run render -- <patch.json> --out <file.wav> [flags]`: `cargo run -q --release -p haa-cli -- render`.
  - `npm run inputs -- <patch.json> [--out <file.d.ts>]`: `cargo run -q --release -p haa-cli -- inputs`.
  - `npm run render:node -- <patch.json> --out <file.wav> [flags]`: `node packages/haa/dist/cli.js render` (needs `npm run build` first).
  - `npm run listen`: `node tools/listen/serve.mjs`. The port comes from env `HAA_LISTEN_PORT`, default 8010.
  - `npm run export:sona`: `node tools/export-sona-presets.mjs`.
  - `npm run release:pack`: `node scripts/release-pack.mjs [--tag v<version>]`.
  - Only the first cold `npm run build`, which compiles the wasm, takes more than 60 s.
- **Releases:** local only.
  1. `npm run release:pack` builds the wasm and the JS, then packs `release/etcito-haa-<version>.tgz` from `packages/haa`.
  2. The owner runs `gh release create v<version> release/etcito-haa-<version>.tgz`. The packer prints that command and never runs it. There is no GitHub CI.
- **Provenance:** the engine semantics and every node are ports of Sona v11. The v0.1 TypeScript engine, extracted from an Etcito game, stays readable under `legacy/`. It is not built, not exported and not typechecked, and it is deleted when Phase 4 closes. PROVENANCE.md records both.

## 2. Layout (haa)

- `Cargo.toml` (workspace, members `crates/*`), `Cargo.lock`, `rust-toolchain.toml`.
- `crates/haa-core/`: patch types and loader, validation, catalog, interpreter, DSP and every node.
  - `build.rs` generates the node and DSP module lists (§14).
  - `src/{lib,consts,catalog,node,rng,patch,validate,issues,graph,render}.rs`, `src/dsp/*.rs`, `src/nodes/<type>.rs`, `tests/*.rs`.
- `crates/haa-wasm/`: a `cdylib` with the §13 exports (`src/lib.rs` for the generic exports, `src/voice.rs` for the voice exports).
- `crates/haa-cli/`: the native binary `haa` (`render`, `inputs`).
- `packages/haa/`: the npm package `@etcito/haa`.
  - `package.json`, `build.mjs`, `tsconfig.json`, `README.md`, `LICENSE`;
  - `src/*.ts`, `test/*.test.ts`;
  - `dist/` (built, ignored) and `pkg/haa.wasm` (built, ignored).
- `scripts/build-wasm.mjs`, `scripts/release-pack.mjs`.
- `tools/listen/`: the A/B listening page and its static server.
- `tools/export-sona-presets.mjs`: turns Sona presets into patch files.
- `reference/sona/`: a verbatim copy of Sona v11 `dist/*.js`, used as the listening reference engine. Never edited.
- `patches/examples/`: small hand-written patches, each isolating one feature or node.
- `patches/sona/`: every Sona preset as a patch file.
- `legacy/`: the v0.1 TypeScript sources, tests, worklet bundle, demo, examples and TS scripts, moved as-is. Not built.
- `dist/` (root): leftover v0.1 build output that is still tracked. Nothing builds or reads it.
- `out/`, `target/`, `release/`, `node_modules/`, `packages/haa/dist/`, `packages/haa/pkg/`: generated and ignored.

## 3. Isomorphic DSP and agnosticism

- **haa-core is pure.** Its runtime code may not use:
  - `std::fs`, `std::io`, `std::net`, `std::time`, `std::thread`, `std::env` or `std::process`;
  - `print!`/`println!`/`eprintln!`/`dbg!`, `static mut` or interior-mutable statics;
  - any randomness other than `rng.rs`.
  - It uses only `core`/`alloc` APIs plus std float math (`sin`, `exp`, `powf`…), so it can become `no_std` + `libm` later without a redesign.
  - It allocates only during loading, validation, `Voice::new`, `Voice::load`, `Node` creation and `configure`. `Voice::process` and `Node::process_sample` never allocate.
- **haa-wasm:** no JS imports (the import object is `{}`). One global voice table. Single-threaded.
- **JS host realms:**
  - Realm-neutral (valid in AudioWorkletGlobalScope, Worker, Node and the main thread): `types.ts`, `utf8.ts`, `engine.ts`, `offline.ts`, `wav.ts`. They use no DOM, `fetch`, `TextEncoder`/`TextDecoder`, timers or Node builtins.
  - AudioWorkletGlobalScope only: `worklet.ts`.
  - Browser main thread: `player.ts`, `load.ts`.
  - Node only: `node.ts`, `cli.ts`.
- **Agnostic:**
  - No game code and no `three`. Positions are plain `{x,y,z}`.
  - `crates/` and `packages/` contain no absolute path and no import from `legacy/` or `reference/`.
  - `tools/listen` may load `reference/sona/` at runtime by URL.

## 4. Audio engineering rules

- **Units:** Hz, seconds, dB, linear gain. Published inputs use game units (rpm, m/s, people, metres).
- **Rates:**
  - A voice runs at the sample rate given when it is created. In the worklet, that is the context rate.
  - Offline renders (CLI, Node, Worker) default to `OFFLINE_RATE = 48000`, like Sona's export.
- **Blocks:** at most `BLOCK = 128` frames per `process` call. The interpreter runs per sample, like Sona's `GraphDSP`.
- **Precision:** f64 for all state and math; f32 for the output buffers.
- **Signal chain per sample** (Sona order; the constants are copied from Sona `dsp.js` into `consts.rs`, with the Sona name in a comment):
  1. Clock.
  2. Input smoothing.
  3. Nodes in Kahn order:
     - effective params = 5 ms smoothed base + Σ (source value × edge-smoothed amount), clamped as Sona does;
     - audio input = Σ (source l/r × edge-smoothed amount);
     - mute and bypass applied as in Sona.
  4. Sum of the `output` nodes.
  5. Master gain.
  6. DC blocker.
  7. 0.92 peak limiter.
  8. Start and end fades.
  9. Non-finite guard.
- **Guard:** a non-finite sample zeroes the block, resets every node and increments `guard_trips`. Processing then continues.
- **Live graph swap:** `load(json, keep_state = true)` keeps the `Node` instances whose id and type are unchanged (`configure` gets the new params). It also keeps the clock, like Sona `setGraph(graph, true)`.
- **Levels:** the sound balance lives in the patch (`gain` nodes, `master`). The host adds no hidden gain.
- **Determinism:** the same patch, seed, sample rate and input script give the same sound. This is checked by listening, never by hash or sample comparison.

## 5. Constants (crates/haa-core/src/consts.rs)

- `PATCH_FORMAT: u32 = 1`: the `haa` header value.
- `OFFLINE_RATE: u32 = 48_000`.
- `BLOCK: usize = 128`.
- `MAX_NODES: usize = 256` and `MAX_CONNECTIONS: usize = 1024` (Sona uses 64/256).
- `PARAM_SMOOTH_S: f64 = 0.005`.
- `LIMIT_PEAK: f64 = 0.92`.
- `TRACE_HZ: f64 = 25.0`.
- `DEFAULT_INPUT_SMOOTH_S: f64 = 0.05`.
- `LIVE_DEFAULT_SECONDS: f64 = 10.0`.
- `SONA_SOUND_REVISION: u32 = 8`.
- `LEGACY_TYPES: [&str; 3] = ["engine", "reed", "bubble"]`.
- `RESERVED_INPUTS: [(&str, f64); 3] = [("_dist", 0.0), ("_pan", 0.0), ("_doppler", 1.0)]`.
- **Copied from Sona `dsp.js`**, each with its Sona name in a comment:
  - `EDGE_SMOOTH_S = 0.005`: edge-amount smoothing;
  - `DC_BLOCK_HZ = 5.0` and `dc_blocker_coefficient(sr) = exp(−τ·5/sr)`;
  - `START_FADE_S = 0.005`, `TIMELINE_START_FADE_S = 0.003`, `END_FADE_S = 0.006`.
- **TS mirror:** `packages/haa/src/types.ts` mirrors `BLOCK`, `OFFLINE_RATE`, `TRACE_HZ` and `LIVE_DEFAULT_SECONDS` with the same values, and defines `PLAYER_PROCESSOR = 'haa-player'`.

## 6. Geometry and spatial block

These are reference formulas for the per-voice spatial stage of decisions 10 and 11, which a later phase implements. No current module implements them. Source: `legacy/src/spatial/` (v0.1).

- **Types:** `Vec3 {x,y,z}`, `Quat {x,y,z,w}`, `Listener { pos, quat, vel? }`, `SourcePos { pos, vel?, range?, occlusion? }`.
- **Listener frame:** forward = q·(0,0,−1), right = q·(1,0,0), up = q·(0,1,0). World up is +Y.
- **Constants:**
  - SPEED_OF_SOUND 343, DOPPLER_AMOUNT 0.5, REF_DISTANCE 4, DEFAULT_RANGE 80;
  - OPEN_LOWPASS_HZ 20000, REAR_LOWPASS_HZ 7000, AIR_ABSORPTION_M 400;
  - OCCLUDED_MIN_HZ 900, OCCLUDED_GAIN_DB −9, SPLIT_PAN_WIDTH 0.5;
  - REVERB_SEND_DIST 0.35, REVERB_SEND_DIST_M 60, DOPPLER_MIN 0.5, DOPPLER_MAX 2.
- **distanceGain(d, ref, max):** 1 for d ≤ ref; (ref/d)·(1 − smoothstep(0.7·max, max, d)) otherwise; 0 for d ≥ max.
- **Distance:** when d < 1e−6, the result is gain 1, pan 0, lowpass 20000, Doppler 1.
- **Direction:** dir = (src − pos)/d; x = dot(dir, right); z = dot(dir, forward); az = atan2(x, z).
- **Pan:** pan = sin(az) × (N > 1 ? 0.5 : 1). It is equal-power: L = cos((pan+1)π/4), R = sin((pan+1)π/4).
- **Lowpass:** rear = max(0, −cos az); lowpass = 20000 − rear·13000; then × clamp(1 − d/400, 0.3, 1).
- **Occlusion:** o in 0..1. Lowpass = max(900, lowpass·(1 − 0.95·o)) and gain × dB(−9·o).
- **Doppler:** u = (listener − src)/d, vs = dot(src.vel, u), vl = −dot(listener.vel, u); ratio = ((343 + vl)/(343 − vs))^0.5, clamped to 0.5..2.
- **Multi-listener:** each listener contributes with gain 1/N. The dominant listener (highest gain, lowest index on ties) gives the send and the Doppler.
- **Known values** (1 listener at the origin, identity quat):
  - src (0,0,−2): gain 1, pan 0, lowpass 19900;
  - (8,0,0): gain 0.5, pan 1, lowpass 19600;
  - (0,0,40): gain 0.1, lowpass 6300;
  - (0,0,−68): gain 0.0294118;
  - occlusion 1 at (0,0,−2): gain 0.3548134, lowpass 995;
  - Doppler src (0,0,−100) with vel (0,0,34.3): 1.0540926; with vel (0,0,−34.3): 0.9534626.
- **Perspective (decision 11):** a patch can read these results through the reserved inputs `_dist` (metres), `_pan` (−1..1) and `_doppler` (ratio).

## 7. Patch format

On disk a patch is UTF-8 JSON:

```json
{ "haa": 1, "id": "siren", "title": "Siren",
  "inputs": [ { "name": "pitch", "kind": "continuous", "unit": "Hz", "min": 200, "max": 1200, "default": 440, "smooth": 0.05, "curve": "log" } ],
  "graph": { "duration": 4, "loop": true, "mode": "live",
    "nodes": [ { "id": "in1", "type": "input", "x": 40, "y": 80, "label": "Pitch", "params": { "name": "pitch" } } ],
    "connections": [ { "id": "c1", "from": "in1", "to": "osc1", "target": "frequency", "amount": 1 } ],
    "performance": null },
  "master": 1 }
```

- **Header:**
  - `haa`: the format number; 1 = `PATCH_FORMAT`.
  - `id`: lowercase with dashes. `title`: free text.
  - `inputs`: §8; default `[]`.
  - `graph`: required.
  - `master`: Sona's project `master` value, copied as-is and applied the way Sona applies its project master; default unity.
  - Unknown top-level keys are kept (round trip).
- **graph:** Sona's graph, unchanged.
  - `duration`: seconds, > 0. `loop`: bool, default false.
  - `mode`: `"timeline" | "live"`, default `"timeline"`. This key is new in HAA.
  - `nodes`, `connections` (default `[]`), `performance?`. Unknown keys are kept.
- **node:** `{ id, type, x, y, label, params, curve?, compact?, muted?, bypass? }`.
  - `params` maps a param key to a number, a string (enum or text) or a bool.
  - `curve` holds Sona's breakpoint data. It is kept as JSON and parsed by the nodes that use it (`curve`, `map`).
  - Unknown keys are kept.
- **connection:** `{ id, from, to, target, amount }`.
  - `target` is `"audio"` for an audio wire, or the key of a param of `to` for a control wire. A control wire adds `value × amount` to that param's base value.
  - `amount` defaults to 1. Unknown keys are kept.
- **performance:** Sona's one-pedal macro `{ node, parameter, curveNode, edge, mode: "curve" | "live", manualValue }`. It is kept as-is and played with Sona's semantics.
- **Accepted top-level shapes** (`load_patch`):
  1. A HAA patch: an object with a numeric `haa`. `haa > 1` gives the error `format-too-new`.
  2. A Sona v2 project `{ version, recipeRevision, title, titleCustom, presetId, graph, master }` without `haa`. It is wrapped:
     - `id` = slug(presetId, else title, else `"patch"`), `title` = title or `"Untitled"`, `master` = master;
     - `{ version, recipeRevision, presetId, titleCustom }` are kept under the top-level key `sona`;
     - info issue `wrapped-sona-project`.
  3. A bare Sona graph: an object with `nodes` and `duration`. It is wrapped with id `"patch"`, title `"Untitled"` and info issue `wrapped-sona-graph`.
- **Pedal conversion:** when `graph.performance` is present and `inputs` has no `pedal`, the loader appends `{ name: "pedal", kind: "continuous", unit: "", min, max, default: manualValue, smooth: DEFAULT_INPUT_SMOOTH_S, curve: "lin" }` and issues info `performance-converted`.
  - `min`/`max` are the range that Sona's pedal bar uses for `manualValue`.
  - At run time, the value Sona takes from `manualValue` comes from the `pedal` input. In `"curve"` mode the patch plays as in Sona, driven by its curve node.
- **Loading is permissive, playing is strict.**
  - `load_patch` returns the patch plus issues. A field of the wrong type is defaulted with the error `bad-field`.
  - `validate` refuses any error. Only a validated plan plays, renders or exports.
- **Saving:** serializing a loaded patch writes the header and keeps every unknown key, so Sona presets and project files round-trip.

## 8. Published inputs and time

- **InputDef:** `{ name, kind, unit, min, max, default, smooth, curve, values? }`.
  - `kind`: `"continuous" | "trigger" | "enum"`.
  - `unit`: free text, default `""`.
  - `min`/`max`: game units, with min < max. `log` needs min > 0.
  - `default`: clamped to the range; for enum, an index into `values`.
  - `smooth`: one-pole time constant in seconds for continuous inputs, default 0.05. 0 means unsmoothed.
  - `curve`: `"lin" | "log"`, the slider and `map` axis; default `"lin"`.
  - `values`: required and non-empty for enum; the value is the index.
  - Names are lowerCamel game words (`rpm`, `speed`, `pedal`). They are unique, and names starting with `_` are reserved.
- **Reserved inputs** `_dist`, `_pan`, `_doppler`: always present after the header inputs, with defaults 0, 0 and 1. They can be set like any input and are never listed in the header.
- **Values:**
  - Continuous: `set(v)` clamps to min..max and smooths toward v.
  - Enum: `set(index)`, or `set(name)` from the host; no smoothing.
  - Trigger: `trigger()` raises a one-sample pulse (1 on the next sample, then 0).
- **`input` node** (control, params `name: text`): outputs the current value of input `name`, which is the smoothed game value, the enum index or the trigger pulse. An unknown name gives the error `unknown-input`.
- **`map` node** (control): turns a value into another through breakpoints drawn against the input range.
  - Params: `in` (a modulatable base value, normally wired from an `input` node), `inMin`, `inMax`, `scale: lin|log`, `outMin`, `outMax`.
  - It uses the node's `curve` breakpoints with Sona's curve shape, x and y normalized to 0..1.
  - Output = outMin + (outMax − outMin) × curveEval(norm(in)). norm is linear or logarithmic over inMin..inMax, clamped to 0..1. Breakpoints are linearly interpolated. With no breakpoints, the curve is the identity.
- **graph.mode:**
  - `timeline` (Sona): normalized time t = seconds/duration. At the end the patch loops (wrap, `looped` flag) or ends (end fade, then silence and `ended`).
  - `live`: a continuous clock with no end and no end fade. Curves read t = (seconds mod duration)/duration, so they loop. Inputs drive the sound.
- **Trigger clock:** `curve` and `envelope` have `clock: enum["patch", "trigger"]` (default `"patch"` = Sona) and `retrigger` (a modulatable number, not smoothed).
  - With `clock = "trigger"`, the node keeps its own local time.
  - The local time restarts at 0 on each rising edge of `retrigger` (previous sample ≤ 0.5, current > 0.5), runs over `duration`, then holds at the end. Before the first trigger it sits at the end.
  - When a trigger input's `input` node is wired to `retrigger`, one patch works as both a one-shot and a live instrument.
- **Render rules** (shared by `haa render`, `renderPatch` and `dist/cli.js`):
  - `seconds` defaults to `duration` in timeline mode and to `LIVE_DEFAULT_SECONDS` in live mode.
  - The sample rate defaults to `OFFLINE_RATE`; the seed defaults to 0.
  - `inputs` are set before the first block.
  - `ramps name=a:b` move the input linearly from a to b over the render, set at every block start.
  - `triggers` fire at the first block whose start is ≥ `at` seconds.
  - The output always has `round(seconds × rate)` frames, with silence after `ended`.
- **inputs.d.ts** (`haa inputs`), with P = PascalCase(patch id):
  - header line `// Generated by haa inputs from <file name>. Do not edit.`;
  - `export interface PInputs { <name>?: number; ... }` for continuous inputs, each with a `/** <unit> <min>..<max>, default <default> */` doc;
  - enum inputs typed as the union of their `values` literals;
  - `export type PTrigger = '<name>' | ...` (or `never`);
  - `export const P_PATCH_ID = '<id>';`.

## 9. Worklet processor and messages

- **Processor:** `haa-player` (`PLAYER_PROCESSOR`), bundled self-contained as `packages/haa/dist/worklet.js`.
  - 0 inputs and 1 stereo output (`outputChannelCount: [2]`).
  - It instantiates the compiled `WebAssembly.Module` synchronously with `new WebAssembly.Instance(module, {})`.
  - It runs one voice and processes at most `BLOCK` frames per call.
  - Apart from the trace packets, it allocates nothing in `process()`.
- **processorOptions** (`PlayerProcessorOptions`): `{ module: WebAssembly.Module, patch: Uint8Array /* UTF-8 JSON */, seed: number, inputs: Record<string, number | string>, trace: boolean }`.
- **main → processor** (`PlayerInMsg`):
  - `{ t: 'set', name, value: number | string }`, `{ t: 'trigger', name }`;
  - `{ t: 'patch', req: number, bytes: Uint8Array, keepState: boolean }`;
  - `{ t: 'restart' }`, `{ t: 'trace', on: boolean }`, `{ t: 'dispose' }`.
- **processor → main** (`PlayerOutMsg`):
  - `{ t: 'ready', playable, issues: PatchIssue[], info: VoiceInfo }`;
  - `{ t: 'loaded', req, playable, issues, info }`;
  - `{ t: 'trace', time, seconds, values: Float32Array }`: one last control value per `info.nodes` entry (plan order). It is posted at `TRACE_HZ` while trace is on, like Sona's modulation packets.
  - `{ t: 'ended' }`: once, when a timeline voice ends.
  - `{ t: 'error', message }`: once per trap.
- **Types** (`packages/haa/src/types.ts`):
  - `VoiceInfo = { nodes: string[]; inputs: HaaInput[]; mode: 'timeline' | 'live'; duration: number; loop: boolean }`.
  - `PatchIssue` (with `PatchIssueCode`) is §12.
  - `HaaPatch`, `HaaInput`, `SonaGraph`, `SonaNode`, `SonaConnection` and `SonaPerformance` mirror §7/§8 field for field. Unknown keys are typed `[key: string]: unknown`.

## 10. Seeds

- **Node seed:** `fnv1a32(node.id) ^ voice_seed`.
  - FNV-1a 32 is ported exactly as Sona hashes the node id, with the same iteration over the string's char codes and the same zero-seed fallback.
  - The per-node generator is Sona's xorshift, with Sona's state update and output scaling.
- **Voice seed:** default 0, which gives Sona's exact sound. Variants of a one-shot are voice seeds (`--seed`, `createPlayer({ seed })`, `renderPatch({ seed })`).
- **Randomness:** nodes draw only from their own generator, created from `NodeInit.seed`.
- **Checks:** seeds are checked by listening (Q8). The `rng.rs` unit tests pin the first values of one seed's sequence. This is a math test, not a parity check.

## 11. Node catalog JSON

`catalog_json()` and the wasm export `haa_catalog` return:

```json
{ "format": 1, "soundRevision": 8, "types": [ {
  "type": "oscillator", "category": "source", "label": { "en": "Oscillator", "fr": "Oscillateur" },
  "ports": { "audioIn": false, "audioOut": true, "controlOut": false }, "usesCurve": false, "hidden": false,
  "params": [
    { "key": "frequency", "kind": "number", "min": 20, "max": 20000, "default": 220, "step": 1, "scale": "log", "unit": "Hz", "label": { "en": "Frequency", "fr": "Fréquence" }, "modulatable": true, "smooth": true },
    { "key": "waveform", "kind": "enum", "values": ["sine", "saw"], "default": "sine", "label": { "en": "Waveform", "fr": "Forme" }, "modulatable": false, "smooth": false } ] } ] }
```

- **Categories:** `source | control | effect | output`.
- **Param kinds:** `number` (min, max, default, step, scale `lin|log`, unit), `enum` (values, default as a string), `bool` (default), `text` (default).
- **Order:** types are sorted by `type`. Params are in declaration order, which is the index order of the interpreter's `params` slice.
- **Values:** the values above are only illustrative. Every real value comes from Sona's `catalog.js`. The TS shape is `CatalogJson` in `types.ts`.
- **Legacy:** the legacy types `engine`, `reed` and `bubble` are not in the catalog.
- **Use:** the studio generates its NodeGraph descriptors from this JSON (decision 8).

## 12. Validation issues

`Issue { severity: "error" | "warning" | "info", code, path, node?, message }`, serialized as JSON with those keys (`node` is omitted when absent).

- `path` is a dotted path with indices, e.g. `graph.nodes[3].params.cutoff` or `inputs[0].max`.
- **Codes** (closed list):
  - Errors:
    - `not-json`, `not-object`, `format-too-new`, `bad-field`;
    - `bad-duration` (duration ≤ 0 or not finite);
    - `duplicate-node-id`, `duplicate-connection-id`, `unknown-type`, `legacy-type`;
    - `bad-enum` (value not in values);
    - `unknown-node` (connection endpoint);
    - `bad-port`: an audio target on a node without audio input, a source without the matching output, or a param target that is unknown or not modulatable;
    - `cycle`, `too-many-nodes`, `too-many-connections`;
    - `no-output` (no `output` node);
    - `unknown-input`, `duplicate-input`, `bad-input` (min ≥ max, log with min ≤ 0, enum without values, reserved name).
  - Warnings: `unknown-param` (kept in JSON, ignored), `param-clamped` (base value outside its range, clamped).
  - Info: `wrapped-sona-project`, `wrapped-sona-graph`, `performance-converted`.
- **Order:** errors follow document order. Kahn ordering runs only when there is no structural error.
- **Plan order:** Kahn's algorithm over audio and control edges. The queue is seeded with the zero-in-degree nodes in `graph.nodes` order, exactly as in Sona's `validateGraph`.

## 13. Wasm ABI (crates/haa-wasm)

All exports are `extern "C"` with `#[unsafe(no_mangle)]`.

- **Arguments:** pointers and lengths are `u32` offsets into `memory`.
- **Voice handles:** `u32 ≥ 1`. With handle 0 or an unknown handle, every call is a no-op that returns 0/−1.
- **Exports:**
  - `haa_version() -> u32` = `PATCH_FORMAT`.
  - `haa_alloc(len: u32) -> u32` and `haa_free(ptr: u32, len: u32)`: byte buffers that the host fills.
  - `haa_result_ptr() -> u32` and `haa_result_len() -> u32`: the last UTF-8 JSON result.
  - `haa_catalog() -> u32`: writes the §11 JSON into the result and returns its length.
  - `haa_voice_new(sample_rate: f64, seed: u32) -> u32`; `haa_voice_free(h: u32)`.
  - `haa_voice_load(h, ptr, len, keep_state: u32) -> i32`: 1 = playable; 0 = loaded but not playable (the voice is silent); −1 = bad handle. The issues JSON array goes into the result.
  - `haa_voice_info(h) -> u32`: writes the `VoiceInfo` JSON (§9) into the result.
  - `haa_voice_input_index(h, ptr, len) -> i32`: −1 if unknown. Reserved inputs follow the header inputs.
  - `haa_voice_set_input(h, index: u32, value: f64)`; `haa_voice_trigger(h, index)`; `haa_voice_restart(h)`.
  - `haa_voice_process(h, frames: u32) -> u32`: frames ≤ BLOCK. Returns 1 while playing and 0 once ended.
  - `haa_voice_left(h) -> u32`, `haa_voice_right(h) -> u32`: pointers to the voice's `BLOCK` f32 output buffers.
  - `haa_voice_trace(h) -> u32`, `haa_voice_trace_len(h) -> u32`: the last f32 control value per plan node.
  - `haa_voice_time(h) -> f64` (normalized time) and `haa_voice_seconds(h) -> f64`.
- **Memory views:** calls that may allocate (`alloc`, `catalog`, `voice_new`, `voice_load`, `voice_info`) may grow memory, so the host re-creates its typed views whenever `memory.buffer` changes. `haa_voice_process` never allocates.
- **Text:** text crosses the boundary only as UTF-8 bytes. The host encodes and decodes them with its own `utf8.ts`, because AudioWorkletGlobalScope has no `TextEncoder`/`TextDecoder`.
- **Traps:** a Rust panic aborts, which traps in wasm. The JS host catches the trap, outputs silence, posts `{ t: 'error' }` once and rebuilds the instance from the same module on the next `patch` message.

## 14. Registries

- **Node registry:** `crates/haa-core/build.rs` scans `src/nodes/*.rs` and `src/dsp/*.rs` (non-recursive, sorted by file name) and writes two files:
  - `$OUT_DIR/nodes_gen.rs`: one `#[path = <absolute path, Debug-formatted>] pub mod <stem>;` per node file, plus `pub static ALL: &[&crate::catalog::NodeTypeDef] = &[&<stem>::DEF, ...];`;
  - `$OUT_DIR/dsp_gen.rs`: the same module lines for `src/dsp`.
  - `lib.rs` includes them inside `pub mod nodes { ... }` and `pub mod dsp { ... }`.
  - The script prints `cargo:rerun-if-changed=src/nodes` and `src/dsp`.
- **Adding a node** means adding one file, `src/nodes/<type>.rs`, that defines `pub static DEF: NodeTypeDef` with `type_name == <type>`. No other file changes. A unit test in `catalog.rs` asserts that type names are unique and equal their file stems.
- **DSP helpers:** a helper shared by several nodes is a new file `src/dsp/<name>.rs`. A helper used by only one node stays private in that node's file.
- **Studio:** the catalog JSON (§11) is the only node registry that the studio and MCP see.
- **Append-only files:** later work only appends lines to root `package.json` `scripts`, `packages/haa/src/index.ts` (export lines), `crates/haa-wasm/src/lib.rs` (`mod <name>;` lines) and `README.md` (one section per milestone).

## 15. Naming, style and errors

- **Headers:** each file starts with a header comment that states its role. Ported files add `Ported from Sona v11 <file>` and list their deviations. Exports carry doc comments.
- **Rust naming:** files `snake_case.rs`, types `PascalCase`, functions `snake_case`, constants `UPPER_SNAKE`. A node file is named after its Sona type.
- **TypeScript naming:** files `kebab-case.ts` or one word, types `PascalCase`, functions `camelCase`, constants `UPPER_SNAKE`. Named exports only.
- **Patch ids:** lowercase with dashes (`osc-curve`, `v12-live`).
- **Sona names are never renamed:** node types, param keys, enum values, connection `target` keys. New params and node types are added next to them.
- **Porting a Sona node:**
  1. Copy params, ranges, defaults, steps, units and enums from `catalog.js`, and the FR/EN labels from Sona's translations.
  2. Port the per-sample math line by line, in f64: the same formulas, constants, order of operations, state updates and seed use.
  3. Comment every deviation that Rust forces as `// sona: <what differs and why>`.
  4. Allocate only in `create`/`configure`.
- **Errors:**
  - haa-core: loading never panics, and every problem is an `Issue`. `Voice::process` and `Node::process_sample` never panic and never allocate.
  - JS host: `createPlayer` rejects only with `Error('[haa] worklet unavailable: <reason>')`. A patch that is not playable still creates a silent player that carries its issues. `renderPatch` throws `Error('[haa] patch not playable: <code> <message>')` with the first error issue. Every promise is caught.
  - CLIs and tools (`haa`, `dist/cli.js`, `scripts/*.mjs`, `tools/**/*.mjs`): exit 1 with exactly one line `[<tool>] <reason>` on stderr.
- **Logs:** no logging in haa-core or in the worklet's process path.

## 16. Data on disk and repository rules

- **WAV:** RIFF PCM 16-bit stereo, 48 kHz unless `--rate` says otherwise. Samples are scaled by 32767, rounded half away from zero and clamped. The Rust writer (`crates/haa-cli/src/wav.rs`) and the JS writer (`packages/haa/src/wav.ts`) follow the same rule.
- **Patches:** `patches/examples/<id>.json` (hand-written) and `patches/sona/<preset-id>.json` (exported). The file name equals the header `id`.
- **JSON written by tools:** 2-space indentation, stable key order, a trailing newline, no timestamps.
- **Generated files** go under the git-ignored `out/`: WAVs (`out/render/`, `out/smoke/`) and the listen bundle (`out/listen/`).
- **References:** reference recordings never live in git. The labelled asset store is a later phase.
- **No binary in git:** no wav, mp3, ogg, flac, png, jpg, jpeg, webp, gif or wasm.
- **`.gitignore`:** `/node_modules`, `/out`, `/target`, `/release`, `/.nmc`, `/packages/haa/dist`, `/packages/haa/pkg`, `/legacy/examples/*/out/`.
- **Commits:** no secrets. Tasks commit and never push.
- **Throwaway work:** `/root/nmc/temp/<topic>/`, never committed.
- **Ownership:** every file belongs to exactly one module.

## 17. Testing and listening

- **Rust unit tests:** inline `#[cfg(test)] mod tests` at the bottom of the file under test. Names are snake_case sentences (`silence_in_gives_silence_out`). Each node or file has two or three focused tests, and no test renders more than 2 s of audio. Integration tests live in `crates/haa-core/tests/*.rs`.
- **Per node:** silence in gives silence out where that applies, outputs stay finite at every param extreme, and params are clamped to range.
- **Host tests:** `packages/haa/test/*.test.ts`, run with `node --test` and native type stripping. They cover only pure logic (UTF-8, WAV encoding) and never need a browser.
- **Banned checks:** no hash, SHA or sample-diff check, no timing, benchmark or baseline check, no numeric Sona-vs-HAA comparison, and no GitHub CI.
- **Milestone close:** the branch checks (`npm run typecheck`, `npm run build`) plus the project smoke. There is no proof script, capture, pin, hash or record.
- **Listening acceptance (decision 7):**
  - Every port is accepted by ear on `tools/listen/` (`npm run listen`).
  - Steps: open the patch, press Play (both engines start together), press Match levels, toggle A/B (Sona JS vs HAA wasm), move the input sliders and watch the traces.
  - The lead (or the owner) answers the Q5–Q7 questions.
  - The audio judge is not used in this chain.
- **Mismatch:** a mismatch heard on the listen page is fixed in the port by following Sona's code, never by tuning HAA toward a number.

## 18. Quality bar

- **Q1 Builds from a clean clone.** `npm ci && npm run build` writes `packages/haa/pkg/haa.wasm` and `packages/haa/dist/*.js|.d.ts`, and `npm run typecheck` passes. Checked by the branch checks.
- **Q2 A patch becomes a WAV natively.** `haa render patches/examples/osc-curve.json --out out/smoke/osc-curve.wav` writes a non-empty 48 kHz 16-bit stereo WAV. Checked by the smoke.
- **Q3 Engine safety.** No NaN or Inf ever leaves a voice, and peaks never exceed the 0.92 limiter. Every node keeps its outputs finite at its param extremes and gives silence for silence where that applies. Checked by each node's unit tests (`cargo test`).
- **Q4 Sona files open.** Every Sona preset and every Sona v2 project file loads (permissively, with issues). Every exported Sona preset validates strictly with zero errors. Checked by `crates/haa-core/tests/sona_presets.rs`.
- **Q5 Sounds like Sona.** On the listen page, after matching levels, A/B shows no audible difference in timbre, level, pitch, motion or timing, and HAA adds no click, zipper or noise. Review criterion.
- **Q6 Published inputs drive the sound live.** Moving an input slider (siren `pitch`, V12 `pedal`) changes the sound smoothly while it plays, with no clicks or steps. Review criterion.
- **Q7 Real-time playback.** The wasm worklet plays at 128-frame blocks with no dropouts or crackle, and the traces move with the sound. Review criterion. Code rule: no allocation in `process`.
- **Q8 Seeds give stable variants.** The same patch and seed sound the same on every play, and voice seed 0 is Sona's sound. Another seed gives the same sound performed again, not a different sound. Checked by listening.
- **Q9 Developer experience.** The README shows the real commands, the patch format, the CLI flags and the JS host signatures. `haa inputs` produces a `.d.ts` that tsc accepts.

## Module ownership

| Module | Files | Consumes |
|---|---|---|
| repo | `package.json`<br>`package-lock.json`<br>`tsconfig.json`<br>`.gitignore`<br>`LICENSE`<br>`README.md`<br>`PROVENANCE.md`<br>`Cargo.toml`<br>`Cargo.lock`<br>`rust-toolchain.toml`<br>`scripts/build-wasm.mjs`<br>`scripts/release-pack.mjs`<br>`packages/haa/package.json`<br>`packages/haa/build.mjs`<br>`packages/haa/tsconfig.json`<br>`packages/haa/README.md`<br>`packages/haa/LICENSE`<br>`packages/haa/src/index.ts`<br>`packages/haa/src/version.ts` | — |
| contract | M1 | src/contract.ts, test/contract.test.ts |
| dsp | M1 | src/dsp/{math, rng, filters, noise, osc, shaper, env, modal, delay, reverb, dynamics, meter, loop, index}.ts, test/dsp/{math, rng, filters, meter, dynamics}.test.ts |
| generators | M1 | src/gen/{impactModel, uiKit, fxKit, windModel, reverbZones, index}.ts, src/music/instruments.ts, test/gen/generators.test.ts |
| mix | M1 | src/mix/mixChain.ts, src/mix/presets/base.ts, test/mix/mixChain.test.ts |
| content-core | M1 | src/content/{impacts, ui, stings, wind, index}.ts, test/content/core.test.ts |
| bank-render | M1 | src/bank/render.ts, test/bank/render.test.ts |
| analysis | M1 | src/analysis/measure.ts, src/analysis/report.ts, test/analysis/measure.test.ts |
| offline | M1 | src/offline/wav.ts, src/offline/render.ts, src/node/writeWav.ts, test/offline/wav.test.ts, test/offline/render.test.ts |
| proof-m1 | M1 | scripts/proof/lib/proof.ts, scripts/proof/lib/out.ts, scripts/proof/m1.ts |
| registry | M2 | src/registry/registry.ts, src/registry/builtins.ts, test/registry/registry.test.ts |
| bank-pack | M2 | src/bank/pack.ts, test/bank/pack.test.ts |
| bake | M2 | src/bake/bake.ts, src/bake/mp3.ts, scripts/bake.ts, test/bake/bake.test.ts |
| proof-m2 | M2 | scripts/proof/m2.ts |
| music | M3 | src/music/arranger.ts, src/music/musicSynth.ts, src/content/styles.ts, src/content/musicBed.ts, test/music/arranger.test.ts, test/music/musicSynth.test.ts |
| proof-m3 | M3 | scripts/proof/m3.ts |
| hosts | M4 | src/host/sampler.ts, src/host/bedHost.ts, test/host/sampler.test.ts, test/host/bedHost.test.ts |
| conductor | M4 | src/runtime/conductor.ts, src/runtime/voices.ts, src/runtime/flatSpatial.ts, test/runtime/conductor.test.ts, test/runtime/voices.test.ts |
| offline-scene | M4 | src/offline/scene.ts, test/offline/scene.test.ts |
| worklet | M4 | src/worklet/processors.ts, src/worklet/entry.ts, src/worklet/globals.d.ts, worklet/haa-processors.js, scripts/build-worklet.ts, test/worklet/fresh.test.ts |
| runtime | M4 | src/runtime/createAudio.ts, src/runtime/bankLoader.ts, examples/quickstart.ts |
| demo | M4 | demo/index.html, demo/main.ts, demo/panel.ts, demo/gpuOps.ts, demo/panels/core.ts, demo/nmc.gpu.json, scripts/build-demo.ts, scripts/serve.ts, scripts/gpu-run.ts |
| proof-m4 | M4 | scripts/proof/m4.ts, proofs/m4/input.json, proofs/m4/levels.json |
| spatial | M5 | src/spatial/spatial.ts, src/spatial/model.ts, test/spatial/spatial.test.ts |
| spatial-demo | M5 | src/content/beacon.ts, demo/panels/spatial.ts, test/content/beacon.test.ts |
| proof-m5 | M5 | scripts/proof/m5.ts, proofs/m5/input.json, proofs/m5/trace.json |
| legacy | `legacy/README.md` | — |
| core-model | `crates/haa-core/Cargo.toml`<br>`crates/haa-core/build.rs`<br>`crates/haa-core/src/lib.rs`<br>`crates/haa-core/src/consts.rs`<br>`crates/haa-core/src/catalog.rs`<br>`crates/haa-core/src/node.rs`<br>`crates/haa-core/src/rng.rs` | — |
| core-patch | `crates/haa-core/src/patch.rs`<br>`crates/haa-core/src/validate.rs`<br>`crates/haa-core/src/issues.rs` | core-model |
| core-dsp | `crates/haa-core/src/dsp/smoother.rs`<br>`crates/haa-core/src/dsp/dc_blocker.rs`<br>`crates/haa-core/src/dsp/biquad.rs`<br>`crates/haa-core/src/dsp/delay_line.rs` | core-model |
| core-graph | `crates/haa-core/src/graph.rs`<br>`crates/haa-core/src/render.rs` | core-model, core-patch, core-dsp |
| nodes-base | `crates/haa-core/src/nodes/output.rs`<br>`crates/haa-core/src/nodes/gain.rs`<br>`crates/haa-core/src/nodes/oscillator.rs`<br>`crates/haa-core/src/nodes/curve.rs`<br>`patches/examples/osc-curve.json` | core-model, core-dsp |
| wasm-crate | `crates/haa-wasm/Cargo.toml`<br>`crates/haa-wasm/src/lib.rs` | core-model |
| cli | `crates/haa-cli/Cargo.toml`<br>`crates/haa-cli/src/main.rs`<br>`crates/haa-cli/src/wav.rs`<br>`crates/haa-cli/src/inputs_dts.rs` | core-patch, core-graph |
| wasm-voice-abi | `crates/haa-wasm/src/voice.rs` | wasm-crate, core-graph |
| host-engine | `packages/haa/src/types.ts`<br>`packages/haa/src/utf8.ts`<br>`packages/haa/src/engine.ts`<br>`packages/haa/test/utf8.test.ts` | repo, wasm-voice-abi |
| host-worklet | `packages/haa/src/worklet.ts`<br>`packages/haa/src/worklet-globals.d.ts`<br>`packages/haa/src/player.ts`<br>`packages/haa/src/load.ts` | host-engine |
| host-offline | `packages/haa/src/offline.ts`<br>`packages/haa/src/wav.ts`<br>`packages/haa/src/node.ts`<br>`packages/haa/src/cli.ts`<br>`packages/haa/test/wav.test.ts` | host-engine |
| nodes-inputs | `crates/haa-core/src/nodes/input.rs`<br>`crates/haa-core/src/nodes/map.rs`<br>`patches/examples/siren.json` | core-model, core-graph |
| listen | `tools/listen/index.html`<br>`tools/listen/style.css`<br>`tools/listen/main.ts`<br>`tools/listen/sona.ts`<br>`tools/listen/haa.ts`<br>`tools/listen/traces.ts`<br>`tools/listen/serve.mjs`<br>`reference/sona/README.md` | host-worklet, host-engine |
| nodes-controls | `crates/haa-core/src/nodes/math.rs`<br>`crates/haa-core/src/nodes/lfo.rs`<br>`crates/haa-core/src/nodes/random.rs`<br>`crates/haa-core/src/nodes/envelope.rs`<br>`patches/examples/lfo-vibrato.json` | core-model, core-graph |
| nodes-response | `crates/haa-core/src/nodes/response.rs` | core-model, core-graph |
| nodes-motion | `crates/haa-core/src/nodes/motion.rs`<br>`patches/examples/v12-pedal-chain.json` | core-model, core-graph |
| nodes-noise | `crates/haa-core/src/nodes/noise.rs`<br>`patches/examples/noise-colours.json` | core-model, core-graph, core-dsp |
| nodes-pulse | `crates/haa-core/src/nodes/pulse.rs`<br>`patches/examples/pulse-train.json` | core-model, core-graph |
| nodes-grain | `crates/haa-core/src/nodes/grain.rs`<br>`patches/examples/grain-impact.json` | core-model, core-graph, core-dsp |
| nodes-filters | `crates/haa-core/src/nodes/filter.rs`<br>`crates/haa-core/src/nodes/equalizer.rs`<br>`crates/haa-core/src/nodes/drive.rs`<br>`patches/examples/fx-filters.json` | core-model, core-graph, core-dsp |
| nodes-resonant | `crates/haa-core/src/nodes/comb.rs`<br>`crates/haa-core/src/nodes/resonator.rs`<br>`crates/haa-core/src/nodes/formant.rs`<br>`patches/examples/fx-resonant.json` | core-model, core-graph, core-dsp |
| nodes-time-fx | `crates/haa-core/src/nodes/delay.rs`<br>`crates/haa-core/src/nodes/compressor.rs`<br>`crates/haa-core/src/nodes/space.rs`<br>`crates/haa-core/src/nodes/reverb.rs`<br>`crates/haa-core/src/dsp/acoustic_space.rs`<br>`patches/examples/fx-time.json` | core-model, core-graph, core-dsp |
| sona-presets | `tools/export-sona-presets.mjs`<br>`crates/haa-core/tests/sona_presets.rs` | core-patch, core-graph, listen |

## Module: repo

Owner M1. Consumes: nothing.

The workspace scaffold, build and release.

- **Root `package.json`:** `{ name: "haa-workspace", private: true, type: "module", license: "MIT", engines: { node: ">=22.18" }, workspaces: ["packages/haa"] }`.
  - devDependencies exactly typescript 5.9.3, esbuild 0.28.2, @types/node 22.20.5.
  - scripts (append-only): `build`, `typecheck`, `test`, `render`, `inputs`, `release:pack`, `render:node`, `test:host`, `listen`, `export:sona`, as in §1.
- **Root `tsconfig.json`:** strict, ES2022, NodeNext. allowImportingTsExtensions, verbatimModuleSyntax, erasableSyntaxOnly, noEmit. lib `["ES2022", "DOM"]`, types `["node"]`. Includes `packages/haa/src`, `packages/haa/test` and `tools/listen`.
- **`Cargo.toml`:**
  - `members = ["crates/*"]`, `resolver = "3"`;
  - `[workspace.package]` edition 2024, rust-version 1.96, MIT, version 0.2.0;
  - `[workspace.dependencies]` serde and serde_json;
  - the release profile of §1.
- **`rust-toolchain.toml`:** channel `1.96`, targets `["wasm32-unknown-unknown"]`. **`.gitignore`:** as in §16.
- **`scripts/build-wasm.mjs`:** `cargo build --release --target wasm32-unknown-unknown -p haa-wasm`, then copies `target/wasm32-unknown-unknown/release/haa_wasm.wasm` to `packages/haa/pkg/haa.wasm`. Errors: `[build-wasm] <reason>`.
- **`scripts/release-pack.mjs`:**
  - It reads `packages/haa/package.json` (the name must be `@etcito/haa`); `--tag v<version>` must match the version.
  - It runs `npm run build` and `npm pack -w @etcito/haa --pack-destination release/`.
  - It checks the tar entries with a pure-Node reader: `package/package.json`, `package/dist/{index.js,index.d.ts,offline.js,offline.d.ts,node.js,node.d.ts,worklet.js,cli.js}`, `package/pkg/haa.wasm`, `package/README.md`, `package/LICENSE`.
  - It prints `release-pack: etcito-haa-<v>.tgz ok` and the `gh release create` command, without running it.
- **`packages/haa/package.json`:**
  - `@etcito/haa` 0.2.0, private, module, MIT;
  - `files: ["dist", "pkg", "README.md", "LICENSE"]`, `bin: { "haa-node": "./dist/cli.js" }`;
  - `exports`: `.`, `./offline`, `./node` (each types + default from `dist/`), `./worklet` → `./dist/worklet.js`, `./wasm` → `./pkg/haa.wasm`, `./package.json`;
  - `scripts.build: "node build.mjs"`. No dependencies.
- **`packages/haa/build.mjs`:** esbuild bundles (esm, es2022, no minify, no sourcemap):
  - `index` (browser), `offline` (neutral), `worklet` (self-contained, generated banner), `node` (node), `cli` (node, shebang);
  - an entry whose source is missing is skipped with `[haa build] skip <entry>`.
  - It then runs `tsc -p packages/haa/tsconfig.json` (declarations only) and rewrites `./x.ts` specifiers in the `.d.ts` files to `./x.js`.
  - It warns when `pkg/haa.wasm` is missing.
- **`packages/haa/src/version.ts`:** `HAA_PACKAGE_VERSION = '0.2.0'`, `HAA_WASM_FILE = 'pkg/haa.wasm'`.
- **`packages/haa/src/index.ts`:** the append-only export file: `version`, `types`, `engine`, `player`, `load`.
- **README.md:** what HAA is, the layout, every command, the patch format summary, the local release flow and one status section per milestone. **PROVENANCE.md:** Sona v11 node semantics, plus the v0.1 extraction history kept under `legacy/`. **`packages/haa/README.md`, `packages/haa/LICENSE`:** a short usage note and the MIT licence.

## Module: legacy

Owner M1. Consumes: nothing.

The v0.1 TypeScript engine, moved as-is and kept readable for later ports (crowd model, wind bands, spatial stage). It is not built, not typechecked and not exported.

- **Contents:** `legacy/src/`, `legacy/test/`, `legacy/worklet/`, `legacy/demo/`, `legacy/examples/`, `legacy/scripts/` (bake, build-demo, build-worklet, build-dist, serve, gpu-run, proof), `legacy/vitest.config.ts`, `legacy/tsconfig.build.json`.
- **`legacy/README.md`:** what is here, that it is not built, which later ports read which folder (`legacy/src/spatial` for decisions 10–11, the crowd and wind models), and that the folder is deleted when Phase 4 closes.
- No file outside `legacy/` references it at build time.

## Module: core-model

Owner M1. Consumes: nothing.

The skeleton of the `haa-core` crate: constants, node trait, catalog and rng.

- **Cargo.toml:** `haa-core`, with workspace fields and serde + serde_json.
- **build.rs:** §14.
- **lib.rs:** `pub mod consts, catalog, node, rng, issues, patch, validate, graph, render`, and the generated `nodes`/`dsp` modules. Re-exports `Patch`, `Issue`, `Voice`, `load_patch`, `validate`, `catalog_json`.
- **consts.rs:** §5.
- **node.rs:**
  - `SampleCtx<'a> { sample_rate, dt, t /* normalized clock */, seconds, duration, mode: GraphMode, live_frames: u64 /* Sona totalFrames */, looped, perf_rebase, inputs: &[f64], triggers: &[bool] }`.
  - `NodeOut { l, r, value }`.
  - `NodeInit<'a> { id, seed: u32, sample_rate, params: &[f64] /* DEF.params order; enum = index, bool = 0/1, text = 0 */, texts: &[Option<String>], curve: Option<&Value>, inputs: &[InputDef], duration }`.
  - `trait Node { fn process_sample(&mut self, ctx: &SampleCtx, input: (f64, f64), params: &[f64]) -> NodeOut; fn configure(&mut self, init: &NodeInit) {} fn reset(&mut self); }`.
  - A source returns `value = 0` and a control returns `l = r = 0`. `params` holds the effective values (smoothed, modulated, clamped).
- **catalog.rs:**
  - `Category`, `Scale`, `ParamKind { Number { min, max, default, step, scale }, Enum { values, default }, Bool { default }, Text { default } }`;
  - `ParamDef { key, kind, unit, label_en, label_fr, modulatable, smooth }`;
  - `NodeTypeDef { type_name, category, label_en, label_fr, audio_in, audio_out, control_out, uses_curve, hidden, params, create: fn(&NodeInit) -> Box<dyn Node> }`;
  - `all()`, `find(type_name)`, `param_index(def, key)`, `catalog_json()` (§11).
- **rng.rs:** `fnv1a32(s)` (Sona's variant over char codes), `Rng` (Sona's xorshift: `new(seed)` with the zero fallback, `next_u32`, `next_f64`), `node_seed(node_id, voice_seed)`.

## Module: core-patch

Owner M1. Consumes: core-model.

The patch format of §7–§8, the permissive loader and strict validation.

- **issues.rs:** `Severity { Error, Warning, Info }` (serialized lowercase), `Issue { severity, code, path, node: Option<String>, message }`, `has_errors(&[Issue])`, `issues_json(&[Issue])`.
- **patch.rs:** serde types. Each has `#[serde(flatten)] extra` so that unknown keys round-trip.
  - `Patch { haa, id, title, inputs: Vec<InputDef>, graph: Graph, master: Value, extra }`.
  - `InputDef { name, kind: InputKind, unit, min, max, default, smooth, curve: InputCurve, values }`.
  - `Graph { duration, loop, mode: GraphMode { Timeline, Live }, nodes: Vec<NodeSpec>, connections: Vec<Connection>, performance: Option<Performance>, extra }`.
  - `NodeSpec { id, type, x, y, label, params, curve, compact, muted, bypass, extra }`, `Connection { id, from, to, target, amount /* default 1 */, extra }`.
  - `Performance { node, parameter, curveNode, edge, mode: PerfMode { Curve, Live }, manualValue, extra }`.
  - `load_patch(json) -> Result<(Patch, Vec<Issue>), Issue>`: Err only for `not-json`/`not-object`. It accepts the three shapes of §7 and converts the pedal. Fields of the wrong type are defaulted with `bad-field`.
  - `to_json(&Patch)`: 2-space pretty, trailing newline.
  - `param_value_f64(def, v) -> (f64, Option<String>)`.
- **validate.rs:**
  - `Plan { nodes: Vec<PlanNode> /* Kahn order */, inputs, duration, loop, mode, performance: Option<PlanPerformance>, master: f64 }`;
  - `PlanNode { id, def, params, texts, curve, muted, bypass, audio_in: Vec<Wire>, mods: Vec<Mod> }`;
  - `Wire { from, amount, conn_id }` and `Mod { from, param, amount, conn_id }`, with indices in plan order;
  - `PlanPerformance`: the resolved node, param, curve node, edge, mode and `pedal` input index.
  - `validate(&Patch) -> Result<(Plan, Vec<Issue>), Vec<Issue>>` applies Sona's rules plus §8/§12. Warnings come back with Ok.

## Module: core-dsp

Owner M1. Consumes: core-model.

Shared DSP primitives, ported from the Sona code that uses them (`dsp.js`, `effects.js`, `simple-effects.js`).

- **smoother.rs:** `Smoother` with `new(time_s, sr, initial)`, `set_time`, `set_target`, `next`, `jump`, `value`. One-pole with coefficient `exp(-1/(time_s*sr))`; time 0 means no smoothing. It handles param, edge and input smoothing.
- **dc_blocker.rs:** `DcBlocker` with `new(sr)`, `process(x)`, `reset()`, using Sona's coefficient.
- **biquad.rs:** `BiquadKind { Lowpass, Highpass, Bandpass, Notch, Peaking, Lowshelf, Highshelf, Allpass }` and `Biquad` (mono, direct form as in Sona) with `set(kind, freq_hz, q, gain_db, sr)` (RBJ, Sona's frequency clamp), `process`, `reset`.
- **delay_line.rs:** `DelayLine` with `new(max_seconds, sr)` (which allocates), `write(x)`, `read(delay_samples)` (Sona's interpolation), `reset()`.

## Module: core-graph

Owner M1. Consumes: core-model, core-patch, core-dsp.

The interpreter with Sona `GraphDSP` semantics (§4), the clocks and inputs of §8, and the offline render.

- **graph.rs:**
  - `LoadOutcome { playable, issues }`.
  - `Voice` with:
    - `new(sample_rate, voice_seed)`;
    - `load(json, keep_state)` and `load_patch(&Patch, keep_state)`: a patch that is not playable makes the voice silent;
    - `issues`, `playable`, `info_json`;
    - `input_index(name)`, `set_input(index, value)`, `trigger(index)`, `restart()`;
    - `process(left, right) -> bool`: len ≤ BLOCK; returns false once ended;
    - `node_ids`, `trace`, `time`, `seconds`, `ended`, `guard_trips`.
  - **Per sample:**
    1. Clock.
    2. Inputs.
    3. The performance macro, with the manual value read from `pedal` and `perf_rebase` set on a mode change.
    4. Nodes in plan order, with smoothed and modulated params and audio wires.
    5. Output sum, master, DC blocker, limiter, fades, guard.
  - **keep_state:** nodes with the same id and type keep their instance and get `configure`; the clock and the smoothers are kept.
  - **Allocation:** buffers are allocated in `load`, never in `process`.
- **render.rs:**
  - `RenderOptions { seconds: Option<f64>, sample_rate, seed, inputs: Vec<(String, InputValue)>, ramps: Vec<(String, f64, f64)>, triggers: Vec<(String, f64)> }`, with `InputValue { Number, Name }`;
  - `StereoRender { left, right, sample_rate, issues }`;
  - `render(json, &RenderOptions) -> Result<StereoRender, Vec<Issue>>` applies the §8 render rules. It returns Err when the patch is not playable or an input is unknown (`unknown-input`).

## Module: nodes-base

Owner M1. Consumes: core-model, core-dsp.

The first Sona node types, ported per §15.

- **output.rs** (`output`, Output): passes its input through. The interpreter sums the `output` nodes.
- **gain.rs** (`gain`, Effect): Sona's gain node.
- **oscillator.rs** (`oscillator`, Source): Sona's polyBLEP oscillator with every waveform.
- **curve.rs** (`curve`, Control, uses_curve): Sona's breakpoint curve over normalized time, plus `clock` and `retrigger` (§8). In live mode it reads the looping `ctx.t`.
- **patches/examples/osc-curve.json:** timeline, 3 s, no loop. An oscillator goes through a gain driven by a rise-then-fall curve. Plays on both engines; used by the smoke.

## Module: wasm-crate

Owner M1. Consumes: core-model.

- **Cargo.toml:** `haa-wasm`, `crate-type = ["cdylib", "rlib"]`, deps haa-core and serde_json.
- **src/lib.rs:** the global result buffer and the generic §13 exports (`haa_version`, `haa_alloc`, `haa_free`, `haa_result_ptr`, `haa_result_len`, `haa_catalog`).
  - It exposes `pub(crate) fn set_result(s: String)`.
  - It is append-only for `mod <name>;` lines (`mod voice;`).

## Module: cli

Owner M1. Consumes: core-patch, core-graph.

The native binary `haa`, with hand-parsed arguments.

- **`haa render <patch.json> --out <file.wav> [--seconds S] [--seed N] [--rate HZ] [--input name=value]... [--ramp name=a:b]... [--trigger name@seconds]...`:**
  - calls `haa_core::render::render`;
  - prints each warning or error issue as `[haa] <severity> <code> <path>: <message>`;
  - writes the WAV (§16), creating parent directories;
  - prints `[haa] wrote <path> (<frames> frames, <rate> Hz)`.
  - Exits 1 on a patch that is not playable or a bad argument. An enum input value may be a name or an index.
- **`haa inputs <patch.json> [--out <file.d.ts>]`:** writes the §8 `.d.ts` to the file or to stdout.
- **wav.rs:** `encode_wav(left, right, sample_rate) -> Vec<u8>`, `write_wav(path, ...)`.
- **inputs_dts.rs:** `inputs_dts(patch, file_name) -> String`.

## Module: wasm-voice-abi

Owner M2. Consumes: wasm-crate, core-graph.

- **crates/haa-wasm/src/voice.rs:** the §13 voice exports over a global `Vec<Option<Voice>>` table (handle = index + 1).
- `haa_voice_load` treats invalid UTF-8 as a `not-json` issue.
- `haa_voice_process` clamps frames to BLOCK, renders into the voice's preallocated f32 buffers and never allocates.

## Module: host-engine

Owner M2. Consumes: repo, wasm-voice-abi.

The realm-neutral wasm wrapper shared by the worklet, the offline renders and Node.

- **types.ts:** the §5 constant mirror and `PLAYER_PROCESSOR`. It also defines `PatchIssue`, `PatchIssueCode`, `HaaInput`, `HaaPatch`, `SonaGraph`, `SonaNode`, `SonaConnection`, `SonaPerformance`, `VoiceInfo`, `LoadResult = { playable; issues }`, `TracePacket = { time; seconds; values: Float32Array }`, the `Catalog*` types (§11), `PlayerProcessorOptions`, `PlayerInMsg` and `PlayerOutMsg` (§9).
- **utf8.ts:** pure `encodeUtf8(s)` and `decodeUtf8(b)`, with surrogate pairs handled. No TextEncoder/TextDecoder.
- **engine.ts:**
  - `HaaExports`: the typed §13 exports plus `memory`.
  - `HaaEngine.fromModule(module)` (`new WebAssembly.Instance(module, {})`), with `exports`, `catalog()` and `createVoice(sampleRate, seed?)`.
  - `HaaVoice` with:
    - `handle`, `load(bytes, keepState?) -> LoadResult`, `info()`;
    - `set(name, value) -> boolean`: enum names are resolved from `info().inputs`;
    - `trigger(name)`, `restart()`, `process(frames) -> boolean`;
    - `left()`/`right()`/`trace()`: views refreshed when `memory.buffer` changes;
    - `time()`, `seconds()`, `free()`.
    - Input indices are cached per name after each load.
  - `patchBytes(patch: HaaPatch | string | Uint8Array): Uint8Array`.
- **test/utf8.test.ts:** round trip of ASCII, accents, CJK and emoji.

## Module: host-worklet

Owner M2. Consumes: host-engine.

The browser playback path.

- **worklet-globals.d.ts:** declares `AudioWorkletProcessor`, `registerProcessor`, `sampleRate` and `currentTime`.
- **worklet.ts:** registers `PLAYER_PROCESSOR`, guarded against double registration.
  - It builds the engine from `processorOptions.module`, creates one voice at `sampleRate`, loads the patch, applies `inputs` and posts `ready`.
  - `process()` renders `outputs[0]` in blocks of up to 128 frames.
  - It posts `ended` once, and posts `trace` every `round(sampleRate / TRACE_HZ)` frames while trace is on.
  - It handles every §9 message. On a trap it outputs silence, posts `error` once and rebuilds on the next `patch`.
- **load.ts:** `loadHaaModule(url)` uses `WebAssembly.compileStreaming(fetch(url))`, with an arrayBuffer fallback when the MIME type is wrong.
- **player.ts:**
  - `PlayerOptions { module, processorUrl, patch, seed?, inputs?, output?, trace? }`.
  - `HaaPlayer { node, info, issues, playable, set, trigger, restart, setPatch(patch, keepState?) -> Promise<LoadResult>, onTrace(cb) -> unsubscribe, onEnded(cb) -> unsubscribe, dispose }`.
  - `createPlayer(ctx, o)`:
    - runs addModule once per context (WeakMap);
    - creates the node with `numberOfInputs 0`, `outputChannelCount [2]`;
    - awaits `ready` and connects to `o.output ?? ctx.destination`;
    - rejects only with `[haa] worklet unavailable: <reason>`.

## Module: host-offline

Owner M2. Consumes: host-engine.

Offline renders in a Worker and in Node, plus the WAV writer.

- **offline.ts** (realm-neutral):
  - `RenderOptions { sampleRate?, seconds?, seed?, inputs?, ramps?: Record<string, [number, number]>, triggers?: { name; at }[] }`;
  - `StereoRender { left, right, sampleRate, issues }`;
  - `renderPatch(module, patch, o?)` applies the §8 rules and throws `[haa] patch not playable: <code> <message>` or `[haa] unknown input <name>`;
  - re-exports `encodeWav` and the types.
- **wav.ts:** `encodeWav({ left, right, sampleRate }): Uint8Array` (§16).
- **node.ts:**
  - `loadModule(path?)`, defaulting to `../pkg/haa.wasm` relative to the module;
  - `renderPatchFile(patchPath, o?)`;
  - `writeWav(path, r)`, which creates directories;
  - re-exports offline.ts.
- **cli.ts:** `node dist/cli.js render`, with the same flags and messages as `haa render`. Errors: `[haa-node] <reason>`, exit 1.
- **test/wav.test.ts:** WAV header, frame count, and rounding of ±1 and ±0.5/32767.

## Module: nodes-inputs

Owner M2. Consumes: core-model, core-graph.

The two HAA control nodes of decision 4 (§8).

- **input.rs** (`input`, Control; en `Input`, fr `Entrée`): param `name: text`. It resolves the input index at create/configure, including reserved names, and outputs `ctx.inputs[i]`. A trigger input outputs its pulse.
- **map.rs** (`map`, Control, uses_curve; en `Map`, fr `Courbe de réponse`): params `in` (wide modulatable range, default 0), `inMin`, `inMax`, `scale: lin|log`, `outMin`, `outMax`. It parses Sona's breakpoints at configure.
- **patches/examples/siren.json:** live mode, with published `pitch` (continuous, Hz, 200..1200, default 440, log). The chain is input → map → oscillator `frequency` → gain → output. HAA-only.

## Module: listen

Owner M2. Consumes: host-worklet, host-engine.

The A/B listening page of decision 7, with Sona's JS engine as the reference.

- **reference/sona/:** a verbatim copy of every top-level `*.js` file of Sona v11 `dist/`. Never edited. `README.md` names the source checkout and commit, the copy date and the refresh rule (copy again).
- **serve.mjs** (`npm run listen`):
  - It requires `packages/haa/dist/worklet.js` and `packages/haa/pkg/haa.wasm`, else it fails with `[listen] run npm run build first`.
  - It bundles `main.ts` into `out/listen/main.js` and serves the repo root on `127.0.0.1:${HAA_LISTEN_PORT || 8010}`.
  - Routes: `/` redirects to `/tools/listen/index.html`; `GET /__patches` returns the sorted list of `patches/**/*.json` paths.
  - It serves MIME types including `application/wasm`, returns 404 for missing files and 403 for paths that escape the root.
  - It exports `startListenServer(port)`.
- **index.html, style.css:** one dark page with:
  - a patch select and a file picker (Sona project files accepted);
  - Play/Stop;
  - the A/B toggle (key `a`) and Match levels with the trim in dB;
  - the issue list and one control per published input;
  - the traces panel.
- **haa.ts:** `createHaaSide(ctx, patchBytes, out)` uses `loadHaaModule` and `createPlayer` (trace on), imported from `packages/haa/src/index.ts`.
- **sona.ts:** `createSonaSide(ctx, graph, out)` loads `/reference/sona/audio-worklet.js` (`sonalab-graph` node). `setPedal(v)` drives the manual value. It is available only when every node type is a Sona type; otherwise the page shows `Sona: n/a (HAA-only nodes)`.
- **main.ts:**
  - One AudioContext; both sides start together into their own GainNodes.
  - A/B switches between them with 5 ms ramps.
  - Match levels compares 1 s RMS through AnalyserNodes and trims B.
  - `pedal` also drives Sona.
- **traces.ts:** one canvas per control node showing the last 8 s: HAA in solid cyan, Sona in dashed amber.

## Module: nodes-controls

Owner M3. Consumes: core-model, core-graph.

- **math.rs:** port of Sona `control-math.js` (× + − ÷ min max smoothstep).
- **lfo.rs, random.rs, envelope.rs:** ports from Sona `dsp.js`. `envelope` also gets `clock` and `retrigger` (§8).
- **patches/examples/lfo-vibrato.json:** an lfo modulates an oscillator's frequency, and an envelope shapes the gain. Plays on both engines.

## Module: nodes-response

Owner M3. Consumes: core-model, core-graph.

- **response.rs:** port of Sona `control-response.js`: follow, rise and fall envelopes of a control, with delay, attack, release, shape and edge window.

## Module: nodes-motion

Owner M3. Consumes: core-model, core-graph.

- **motion.rs:** port of Sona `control-motion.js`, an inertial drive with acceleration, resistance, drag, thresholds, stages, hold, inertia and rest. In live mode it uses `ctx.live_frames` where Sona uses `totalFrames`, and it rebases the macro on `ctx.perf_rebase`.
- **patches/examples/v12-pedal-chain.json:** the V12 pedal control chain (curve → response → motion → math) with a `performance` block, which loads as the `pedal` input. The chain drives a quiet oscillator.

## Module: nodes-noise

Owner M4. Consumes: core-model, core-graph, core-dsp.

- **noise.rs:** port of Sona `noise.js`: five colours and stereo width, with Sona's seed use.
- **patches/examples/noise-colours.json:** the five colours in sequence through a gain curve.

## Module: nodes-pulse

Owner M4. Consumes: core-model, core-graph.

- **pulse.rs:** port of Sona `primitives.js` `pulse`: rate, width, phase, jitter, shimmer, and rounded and exponential shapes.
- **patches/examples/pulse-train.json:** pulse → gain → output, with a rate curve.

## Module: nodes-grain

Owner M4. Consumes: core-model, core-graph, core-dsp.

- **grain.rs:** port of Sona `grains.js`: sine, noise and filtered-noise grains with density, decay, spread, glide, delay, width and level compensation. Rate 0 means one grain.
- **patches/examples/grain-impact.json:** a one-shot grain-cloud impact (timeline, no loop).

## Module: nodes-filters

Owner M5. Consumes: core-model, core-graph, core-dsp.

- **filter.rs:** Sona's trapezoidal state-variable filter (lowpass/bandpass/highpass), ported line by line. It is not an RBJ biquad.
- **equalizer.rs:** a 170 Hz low shelf, a peaking band (RBJ q 0.8) and a 4800 Hz high shelf, using `Biquad` per channel and per band.
- **drive.rs:** tanh with first-order antiderivative antialiasing (logCosh).
- **patches/examples/fx-filters.json:** noise through filter → equalizer → drive, each with a modulated param.

## Module: nodes-resonant

Owner M5. Consumes: core-model, core-graph, core-dsp.

- **comb.rs** (Sona `simple-effects.js`), **resonator.rs** and **formant.rs** (Sona `effects.js`).
- **patches/examples/fx-resonant.json:** noise and pulses through comb, resonator and formant.

## Module: nodes-time-fx

Owner M5. Consumes: core-model, core-graph, core-dsp.

- **delay.rs, compressor.rs, space.rs, reverb.rs:** ports from Sona `effects.js`. Their buffers are allocated at create from the maximum times.
- **dsp/acoustic_space.rs:** `AcousticSpace`, Sona's `acousticSpace` (8-line FDN plus early reflections), shared by `space` (legacy = false) and `reverb` (legacy = true).
  - `new(sr)`, `reset()`;
  - `process(in_l, in_r, size, damping, mix, width, distance, frame, legacy) -> (f64, f64)`.
- **patches/examples/fx-time.json:** a grain or pulse source through delay → compressor → space → reverb.

## Module: sona-presets

Owner M6. Consumes: core-patch, core-graph, listen.

- **tools/export-sona-presets.mjs** (`npm run export:sona`):
  - It imports `../reference/sona/presets.js` and runs Sona's `buildPresets` in Node, stubbing only the globals it needs.
  - It writes one `patches/sona/<preset-id>.json` per preset: `{ haa: 1, id, title (EN), inputs, graph, master }`. The V12 one-pedal preset carries an explicit `pedal` input and keeps its `performance` block.
  - It prints `[export-sona] wrote <n> patches`.
- **patches/sona/*.json:** the exported presets (elephant, engine-start, footsteps-st2, gravel, light-rain, rain-city-1/2, snow, v12, v12-live, water, water-detail, water-hollow, wind).
- **crates/haa-core/tests/sona_presets.rs:** every file loads and validates with zero errors, and a voice renders 0.5 s of finite audio.

## Milestone proofs

- M1 Core extracted and proven in node: every family rendered to WAV with a safety and loudness report: `node scripts/proof/m1.ts`
- M2 Explicit registries and a deterministic MP3 bake: `node scripts/proof/m2.ts`
- M3 Genre-neutral procedural music rendered offline: `node scripts/proof/m3.ts`
- M4 Browser runtime: prebuilt worklet, createAudio and a demo page measured in Chrome: `node scripts/proof/m4.ts`
- M5 Multi-listener spatial audio with plain {x,y,z}: `node scripts/proof/m5.ts`
- M6 example sound pack (under `examples/`, outside the core) and its race soundtrack (commands in its README under `examples/`)
