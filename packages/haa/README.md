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

## Usage: play a HAA Studio graph

Export the graph from HAA Studio (**Project menu → Download .haa.json**) and serve it with your game. In the browser
(Vite shown; any bundler that gives asset URLs works):

```ts
import { createPlayer, loadHaaModule } from '@etcito/haa';
import wasmUrl from '@etcito/haa/wasm?url';
import workletUrl from '@etcito/haa/worklet?url';
import speechWorkerUrl from '@etcito/haa/speech-worker?url';

// Call it after a click or key press: browsers keep audio silent until the page had one.
export async function startEngineSound() {
  const ctx = new AudioContext();
  const player = await createPlayer(ctx, {
    module: await loadHaaModule(wasmUrl),
    processorUrl: workletUrl,
    patch: await (await fetch('/sounds/engine.haa.json')).text(),
    inputs: { speed: 0.5 },                   // the graph's published inputs, set before it starts
    speech: { workerUrl: speechWorkerUrl },   // used only if the graph has a speak node
  });
  player.set('speed', 0.8);                   // from your game loop; player.trigger('hit') for triggers
  return player;
}
```

A graph with `speak` nodes needs nothing more: the voice model (tiny-voca, 8 MB) is downloaded from Hugging Face on
first use, kept in the Cache API and never bundled (`speech: { onProgress }` for a loading bar). A WAV:
`npx haa-node render engine.haa.json --out engine.wav` (`--tinyvoca tiny-voca-js.gguf` for speak nodes). Offline in a
Worker or Node: `renderPatch` / `renderPatchWithSpeech` from `@etcito/haa/offline`.
