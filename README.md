# HAA — Hear Anything Anywhere

HAA is a procedural sound engine. Its graph model and every node are ported from Sona v11 (the owner's sound studio), and a game drives a patch through **published inputs** (`rpm`, `speed`, `pedal`, ...). The engine is Rust (`haa-core`), compiled to wasm and to a native CLI; a small TypeScript host (`@etcito/haa`) plays and renders it. Nothing here is a deployed service.

## Layout

- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`: Rust workspace (`crates/*`), Rust 1.96, edition 2024.
- `crates/haa-core/`: patch types and loader, validation, catalog, interpreter, DSP and every node.
- `crates/haa-wasm/`: the `cdylib` with the `extern "C"` wasm ABI.
- `crates/haa-cli/`: the native binary `haa` (`render`, `inputs`).
- `packages/haa/`: the npm package `@etcito/haa`. Built output goes to `dist/` and `pkg/haa.wasm` (both ignored).
- `scripts/`: `build-wasm.mjs`, `release-pack.mjs`.
- `tools/listen/`: the A/B listening page (later milestone).
- `reference/sona/`: a verbatim copy of Sona v11 `dist/*.js` (later milestone).
- `patches/examples/`: small hand-written patches; `patches/sona/`: Sona presets as patches.
- `legacy/`: the v0.1 TypeScript engine, moved as-is, not built (see `legacy/README.md`).
- `out/`, `target/`, `release/`, `node_modules/`: generated and ignored.

## Commands

Run from the repo root (Node >= 22.18, Rust 1.96).

```sh
npm ci                  # install the dev tools
npm run build           # wasm release build into packages/haa/pkg, then the JS bundles and .d.ts
npm run typecheck       # tsc over the TypeScript sources
npm test                # cargo test --workspace --quiet
npm run render -- <patch.json> --out <file.wav> [flags]
npm run inputs -- <patch.json> [--out <file.d.ts>]
npm run release:pack    # see "Releasing"
```

### `haa render`

```sh
npm run render -- patches/examples/osc-curve.json --out out/render/osc-curve.wav
```

```
haa render <patch.json> --out <file.wav>
           [--seconds S] [--seed N] [--rate HZ]
           [--input name=value]... [--ramp name=a:b]... [--trigger name@seconds]...
```

- `--out`: required; parent folders are created. The WAV is 16-bit stereo PCM, 48 kHz unless `--rate` is given.
- `--seconds`: render length; defaults to the graph `duration` (timeline) or 10 s (live).
- `--seed`: voice seed, default 0 (Sona's exact sound).
- `--input name=value`: set an input before the first block. A number sets the value; any other text is an enum value name. Repeatable.
- `--ramp name=a:b`: move the input linearly from `a` to `b` over the render.
- `--trigger name@seconds`: fire a trigger input at the first block starting at or after that time.

Warnings and errors print as `[haa] <severity> <code> <path>: <message>` on stderr, then `[haa] wrote <path> (<frames> frames, <rate> Hz)`. A bad argument, an unreadable file or a non-playable patch prints one `[haa] <reason>` line and exits 1.

### `haa inputs`

```sh
npm run inputs -- patches/examples/osc-curve.json            # .d.ts on stdout
npm run inputs -- <patch.json> --out out/<id>.inputs.d.ts    # or to a file
```

It writes a TypeScript declaration of the patch's published inputs (an `<Id>Inputs` interface, a trigger-name union and a `<ID>_PATCH_ID` constant).

## Patch format

A patch is UTF-8 JSON: Sona's graph plus a header.

```json
{ "haa": 1, "id": "siren", "title": "Siren",
  "inputs": [ { "name": "pitch", "kind": "continuous", "unit": "Hz", "min": 200, "max": 1200, "default": 440, "smooth": 0.05, "curve": "log" } ],
  "graph": { "duration": 4, "loop": true, "mode": "live",
    "nodes": [ { "id": "in1", "type": "input", "x": 40, "y": 80, "label": "Pitch", "params": { "name": "pitch" } } ],
    "connections": [ { "id": "c1", "from": "in1", "to": "osc1", "target": "frequency", "amount": 1 } ],
    "performance": null },
  "master": 1 }
```

- **Header:** `haa` (format number, 1), `id` (lowercase-dash, equals the file name), `title`, `inputs` (default `[]`), `graph` (required), `master` (Sona's project master, default unity). Unknown keys are kept.
- **graph:** `duration` (seconds, > 0), `loop` (default false), `mode` (`"timeline"` default, or `"live"`), `nodes`, `connections` (default `[]`), `performance?`.
- **node:** `{ id, type, x, y, label, params, curve?, compact?, muted?, bypass? }`. `params` maps a param key to a number, string or bool; `curve` is Sona's breakpoint data. Node types, param keys and enum values are Sona's, never renamed.
- **connection:** `{ id, from, to, target, amount }`. `target` is `"audio"` for an audio wire, or a param key of `to` for a control wire (adds `value × amount` to that param). `amount` defaults to 1.
- **Three accepted shapes:** a HAA patch (numeric `haa`; a higher number is the error `format-too-new`); a Sona v2 project without `haa` (wrapped, info `wrapped-sona-project`); a bare Sona graph with `nodes` and `duration` (wrapped, id `patch`, info `wrapped-sona-graph`).
- **Pedal conversion:** a patch with `graph.performance` and no `pedal` input gets a continuous `pedal` input (default = `manualValue`) and the info issue `performance-converted`; the pedal input then plays the role of `manualValue`.
- **Inputs:** `{ name, kind, unit, min, max, default, smooth, curve, values? }`. `kind` is `continuous` (clamped, one-pole smoothed over `smooth` seconds, default 0.05), `enum` (`values` required, the value is the index) or `trigger` (a one-sample pulse). `curve` is `lin` or `log` (`log` needs min > 0). The `input` node outputs the current value of the named input.
- **Reserved inputs:** `_dist`, `_pan` (defaults 0) and `_doppler` (default 1) always exist after the header inputs and are never listed in the header. Names starting with `_` are reserved.
- **graph.mode:** `timeline` plays once (or loops) over `duration` with Sona's end fade; `live` has a continuous clock with no end, and curves loop over `duration`.

Loading is permissive and reports issues; only a patch with no error issue plays, renders or exports.

## Releasing

Releases are local; there is no CI.

```sh
npm run release:pack    # builds the wasm and JS, packs release/etcito-haa-<version>.tgz, checks its contents
```

It then prints the command below, which it never runs. The owner runs it:

```sh
gh release create v<version> release/etcito-haa-<version>.tgz
```

## Status

- **M0.1 Rust workspace: done.** Cargo and npm workspaces, patch loader and validation, interpreter, offline render, `haa render` and `haa inputs`, `patches/examples/osc-curve.json`.
- M0.2 and later: planned. Each later milestone appends one section below.
