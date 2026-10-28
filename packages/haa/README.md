# @etcito/haa

The HAA procedural sound engine: a Rust core compiled to wasm (`pkg/haa.wasm`) with a small JS host.
MIT, private (not on the npm registry).

## Install

From a GitHub release tgz (built locally with `npm run release:pack` in the haa repo):

```sh
npm install ./etcito-haa-0.2.0.tgz
```

Or from a sibling checkout of the haa repo (run `npm run build` there first):

```json
"dependencies": { "@etcito/haa": "file:../haa/packages/haa" }
```

## Exports

| Specifier | File |
| --- | --- |
| `@etcito/haa` | `dist/index.js` (+ `dist/index.d.ts`) |
| `@etcito/haa/offline` | `dist/offline.js` (+ `.d.ts`) |
| `@etcito/haa/node` | `dist/node.js` (+ `.d.ts`) |
| `@etcito/haa/worklet` | `dist/worklet.js` (AudioWorklet processor) |
| `@etcito/haa/wasm` | `pkg/haa.wasm` |
| `@etcito/haa/package.json` | `package.json` |

The `haa-node` bin runs `dist/cli.js`.

## Usage

```ts
import { HAA_PACKAGE_VERSION, HAA_WASM_FILE } from '@etcito/haa';
```

Today only the version constants are exported. The host API (`createPlayer`, `renderPatch`,
the offline/node/worklet entries and the Node CLI) arrives in milestone 2.
