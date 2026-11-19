# @etcito/haa

The HAA procedural sound engine: a Rust core compiled to wasm (`pkg/haa.wasm`) with a small JS host.
MIT. Source, docs and releases: https://github.com/Etcito/haa.

## Install

```sh
npm install @etcito/haa
# or, from the GitHub release tarball:
npm install https://github.com/Etcito/haa/releases/download/v0.3.0/etcito-haa-0.3.0.tgz
```

The package ships the prebuilt wasm (`pkg/*.wasm`), the bundled JS (`dist/`), its `.d.ts` and the AudioWorklet
processor: nothing to compile. To build from source, see the repository README.

## Exports

| Specifier | File |
| --- | --- |
| `@etcito/haa` | `dist/index.js` (+ `dist/index.d.ts`) |
| `@etcito/haa/offline` | `dist/offline.js` (+ `.d.ts`) |
| `@etcito/haa/node` | `dist/node.js` (+ `.d.ts`) |
| `@etcito/haa/worklet` | `dist/worklet.js` (AudioWorklet processor) |
| `@etcito/haa/speech-worker` | `dist/speech-worker.js` (module Worker for `speak` nodes) |
| `@etcito/haa/speech` | `dist/speech.js` (tiny-voca host: weights, `createSpeech`, `patchSpeechClips`) |
| `@etcito/haa/wasm` | `pkg/haa.wasm` |
| `@etcito/haa/package.json` | `package.json` |

The `haa-node` bin runs `dist/cli.js`.

## Usage

```ts
import { HAA_PACKAGE_VERSION, HAA_WASM_FILE } from '@etcito/haa';
```

Today only the version constants are exported. The host API (`createPlayer`, `renderPatch`,
the offline/node/worklet entries and the Node CLI) arrives in milestone 2.
