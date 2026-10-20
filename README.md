# HAA — Hear Anything Anywhere

## What HAA is

HAA is a game-agnostic procedural sound engine for authored sound families, music, ambience and spatial audio. It is developed and proven in this repository; it is not a deployed service.

## Status and contract version

HAA is milestone M6 complete in this repository: the core (M1 extraction and node proof, M2 registries and bake, M3 music, M4 runtime and demo, M5 spatial audio) and an example sound pack that lives outside the core (M6). The public API contract is version 0 (`HAA_CONTRACT_VERSION = 0`) and is frozen in `src/contract.ts`. The core ships no game content.

| milestone | what it adds | state |
| --- | --- | --- |
| M1 | core extraction, DSP, one-shot families, offline WAV, node proof | done |
| M2 | explicit registries, bake to MP3 with a manifest | done |
| M3 | music arranger and synth, registered styles, music bed | done |
| M4 | browser runtime (AudioWorklet), demo, prebuilt worklet | done |
| M5 | spatial audio: listeners, distance, Doppler, occlusion, reverb sends | done |
| M6 | example sound pack outside the core (`examples/`), race proof | done, not part of the package |

## Install

HAA is not on a registry yet (`"private": true`). The package installs from a tarball built from this repository. These commands were run and verified on Windows with Node 24.22.0, from an empty folder outside the repository:

```
npm ci                                          # from the checkout: development dependencies
npm run build                                   # dist/ (JS ESM and .d.ts); prepack runs it too
npm pack --pack-destination ../haa-pack         # writes ../haa-pack/etcito-haa-0.1.1.tgz
mkdir my-app && cd my-app && npm init -y
npm install ../haa-pack/etcito-haa-0.1.1.tgz    # installs /haa and its lamejs dependency
```

The tarball contains `dist/`, the prebuilt worklet, this README, PROVENANCE.md and LICENSE. Node >= 22.18 is required.

## Releases

Each release is a tag `v<version>`. The GitHub Release for that tag carries the built tarball and `SHA256SUMS`
(built and attached by `.github/workflows/release.yml`; the tarball contains `dist/` and the worklet, so nothing is
built on install):

```
https://github.com/Etcito/haa/releases/download/v<version>/etcito-haa-<version>.tgz
https://github.com/Etcito/haa/releases/download/v<version>/SHA256SUMS
```

Install a release in your game (npm also installs `lamejs`):

```sh
npm install https://github.com/Etcito/haa/releases/download/v0.1.1/etcito-haa-0.1.1.tgz
```

To cut a release: set the same `version` in `package.json`, commit, then `git tag v<version>` and
`git push origin v<version>`. The workflow fails if the tag is not `v` plus the package version. To build and check
the tarball locally first, run `npm run release:pack`; it writes `release/` (gitignored).

## Example

```ts
import { createOfflineAudio } from '/haa';
import { writeWav } from '/haa/node';

const audio = createOfflineAudio({ seed: 1 });        // seeded offline scene, no browser needed
audio.play('impact.wood.small', { gain: 0.8 });       // a generic one-shot family from the core
audio.advance(1);                                     // render one second of scene time
writeWav('out/wood.wav', audio.take('wood'));         // 16-bit PCM WAV, 48 kHz
```

Checked against the installed tarball (see Install): it writes a 48000-frame WAV.

## API sketch

```ts
// Everything below is exported by src/index.ts (browser-safe). Node-only: writeWav from src/node.ts.
const HAA_CONTRACT_VERSION = 0, AUDIO_RATE = 48000, BLOCK = 128, MAX_LISTENERS = 4

interface FamilyDef { id: string; bus: BusId; category: VoiceCategory; variants: number; maxSeconds: number; seed: number;
  render(out: Float32Array, sr: number, rng: AudioRng, v: number): number }   // ADDS mono, returns frames written
type FamilySpec = Omit<FamilyDef, 'seed'> & { seed?: number; families?: readonly string[] }
interface BedDef { id: string; bus: BusId; params: readonly string[]; defaults: readonly number[]; unsmoothed?: readonly string[];
  positional: boolean; create(sr: number, seed: number): BedVoice }
interface MixPreset { id: string; priority: number; buses: Record<BusId, BusPreset>; master: {...}; ducks: Record<string, DuckPreset>;
  loudness: Record<VoiceCategory, number>; sceneLufs: number; familyTrimDb: Record<string, number>; bedTrimDb: Record<string, number>; reverb: ReverbParams }

// Rendering and measuring (node and browser)
renderVariantRaw / renderFamilyVariants / renderFamilyVariantsAsync(def, preset)   // seeded variants, loudness-normalized per category
oneShotLufs(x: Float32Array, n: number, sr: number): number
renderFamilyStrip(def, preset), renderBedDemo(def, ...), renderOffline(...)   // src/offline/render.ts
encodeWav(...) / decodeWav(bytes)                                              // RIFF PCM16
measureMono(x, sr) / measureStereo(l, r, sr): Measure; safetyVerdict(m): string[]; OCTAVE_BANDS
class MixChain; const BASE_PRESET: MixPreset
const BUILTIN_FAMILIES, BUILTIN_BEDS, BUILTIN_MIXES, BUILTIN_STYLES
namespace-style exports: `dsp` (filters, modal, noise, shaper, reverb, ...) and `gen` (impactModel, uiKit, fxKit, windModel, reverbZones, instruments)
AudioRng, hashSeed(...parts: (number | string)[]): number

// Public runtime API (types frozen in v0, implemented in M4/M5)
interface PlayOptions { pos?: Vec3; vel?: Vec3; gain?: number; pitch?: number; bus?: BusId; variant?: number; range?: number; occlusion?: number; listener?: number }
interface AudioControls { play(id: string, o?: PlayOptions): Shot | null;
  startBed(id: string, o?: { seed?: number; pos?: Vec3; vel?: Vec3; values?: ArrayLike<number> }): BedHandle | null;
  setListeners(ls: readonly Listener[]): void; setSpatial(m: SpatialModel | null): void; setOcclusion(fn: OcclusionFn | null): void;
  duck(kind: string, strength?: number): void; setVolumes(v: Partial<UserVolumes>): void; setReverb(p: ReverbParams): void;
  setMuffle(v: number): void; readonly music: MusicControl }
interface HaaAudio extends AudioControls { readonly ctx: BaseAudioContext; ready(): Promise<void>; close(): Promise<void>; ... }
interface OfflineAudio extends AudioControls { readonly time: number; advance(seconds: number): void; take(name: string): StereoRender; ... }
```

The authoritative signatures are in `src/contract.ts`.

## Commands

```
npm install                                   # once; package-lock.json is committed (npm ci restores it)
npm run typecheck                             # tsc --noEmit
npm run build:types                           # declarations into out/types
npx vitest run                                # unit tests (npm test)
node scripts/proof/m1.ts [--only <pattern>] [--full]   # M1 proof (npm run proof:m1)
```

`--only` filters cases by label (case-insensitive, `/regex/` allowed), e.g. `--only 'family impact.metal'` or `--only 'bed wind'`. `--full` runs the long renders (15 s wind sweep). Node >= 22.18 runs the `.ts` files directly.

## Proofs

- **M1** renders every built-in family (all variants, one WAV per family) and the wind-bed speed sweep, then prints five CHECK lines with the measured worst values (WAVs readable; no NaN/Inf and |DC| < 1e-3; sample peak <= -1 dBFS; no silent variant; each family's median LUFS within ±1 LU of its category target) and `ALL PASS m1` with the wall time.
- A passing **full** run (`node scripts/proof/m1.ts --full`, no `--only`) writes into `out/m1/`: `families/<id>.wav`, `beds/wind.wav`, the report triplet `report.json`, `report.md`, `index.html` (an `<audio controls>` per WAV with its measures and the CHECK lines), and `media.json`: `{ proof, files: [{ path, bytes, sha256 }] }` for every WAV, read by the owner's media gate.
- WAVs and everything under `out/` stay untracked (`.gitignore`); no binary asset is committed. `media.json` and the report triplet are text and can be regenerated deterministically.
- Later milestone proofs record their captures under `out/mN/`; browser proof JSON is committed under `proofs/mN/`.

## Provenance

HAA was extracted from the audio engine of an Etcito game and generalized. What was taken, what was written and the changes made are described in [PROVENANCE.md](PROVENANCE.md).

Generated audio, images and build output stay out of git. The committed worklet bundle is the sole prebuilt output.

## Registries and bake (M2)

M2 makes content registration and the offline MP3 bank explicit. The registry is new HAA code (the only new module of this milestone): it replaces the five glob-based registries of the origin engine and has no provenance row of its own (see PROVENANCE.md).

### Why the registries are explicit

A content definition is only ever reachable through a `define*`/`register*` call. There is no bundler glob import anywhere in HAA (the Vite glob helper is never used, and neither are `?url` or `?worker`), so every family, bed, mix and style is visible in the source that registers it, and the same module-level registry works unchanged in node, on the browser main thread and in the worklet realm. Ids are unique per kind: `register*` validates through `define*` and a duplicate throws `[haa] duplicate family|bed|mix|style <id>`. `registerBuiltins()` registers the built-in arrays of `src/content/index.ts` and is idempotent.

A game defines its own family, registers it next to the built-ins, and bakes the whole list — no worklet rebuild is needed for a one-shot family:

```ts
import { defineFamily, registerFamily, registerBuiltins, listFamilies } from '@etcito/haa'
import { bake } from '@etcito/haa/node'

registerBuiltins()

const crate = defineFamily({
  id: 'impact.crate.wood',
  bus: 'world',
  category: 'impact',
  variants: 12,          // 8..32, validated
  maxSeconds: 1.2,       // (0, 10]
  render(out, sr, rng, v) {
    // fill `out` with mono samples for seeded variant `v`; return the frames written
    return out.length
  },
})
registerFamily(crate)    // a duplicate id throws '[haa] duplicate family <id>'

const manifest = bake(listFamilies(), { outDir: 'out/bake' })   // returns the BankManifest
```

Real signatures (from `src/registry/registry.ts` and `src/bake/bake.ts`):

```ts
function defineFamily(spec: FamilySpec): FamilyDef            // frozen copy; seed defaults to hashSeed('family', id)
function registerFamily(def: FamilySpec): void
function registerBuiltins(): void                              // idempotent
function listFamilies(): readonly FamilyDef[]                  // sorted by id
function bake(families: readonly FamilyDef[], opts: { outDir: string; rate?: 48000; kbps?: 128; preset?: MixPreset;
  log?: (line: string) => void }): BankManifest                // node only; preset defaults to activeMix()
```

### Commands

```
node scripts/bake.ts [--out <dir>] [--families <prefixes>]     # npm run bake; default out/bake
node scripts/proof/m2.ts [--only <pattern>]                    # npm run proof:m2
```

`scripts/bake.ts` calls `registerBuiltins()`, selects the built-ins whose id starts with one of the comma-separated prefixes (all of them when `--families` is absent), bakes into `--out` (default `out/bake`) and prints one line per family plus the elapsed time. Example: `node scripts/bake.ts --families sting. --out out/bake-readme`.

### manifest.json, the strip and the MP3

`bake()` writes `<outDir>/<id>.mp3` for every family (by id) and one deterministic `manifest.json` (2-space indentation, fixed key order, trailing newline, no timestamp):

```json
{
  "version": 1, "sampleRate": 48000, "kbps": 128, "encoder": "lamejs 1.2.1", "preset": "<active mix id>",
  "families": [
    { "id": "impact.metal.small", "file": "impact.metal.small.mp3", "bus": "world", "category": "impact",
      "variants": [ { "start": 0, "length": 12345 } ], "onset0": 12, "pcmHash": "1a2b3c4d", "mp3Sha256": "<hex>" }
  ]
}
```

- Families are sorted by id and each entry names its file, bus, category, variant slots, `onset0`, `pcmHash` and `mp3Sha256`.
- The strip is the family's normalized variants concatenated in order, each followed by `GAP_SAMPLES = 4800` zeros; `start`/`length` are in samples at 48 kHz. `onset0` is the first strip index with `|x| > ONSET_THRESHOLD` (1e-3, 0 when silent), and the runtime `alignVariants` uses it to re-align a strip decoded at the context rate.
- `pcmHash` is 8 lowercase hex digits of FNV-1a 32 over the strip's Float32 little-endian bytes; `mp3Sha256` is the hex sha256 of the MP3 file. Both are the determinism anchors.
- The MP3 is mono 48 kHz CBR 128 kbps, encoded by `lamejs` pinned at exactly 1.2.1 and loaded through `createRequire(import.meta.url)` — the package is never patched, vendored or substituted (the load shim only defines lamejs's own missing `MPEGMode`, `Lame` and `BitStream` globals from its `src/js` modules). `BankManifest.encoder` stays `'lamejs 1.2.1'`.
- Determinism: the same families, preset and rate give byte-identical MP3 files and manifest, in every process and run.

### The M2 proof

`node scripts/proof/m2.ts` runs two child cases, `bake run1` and `bake run2`, each baking the `impact.`, `ui.`, `cue.` and `sting.` built-ins (25 families) into `out/m2/run1` and `out/m2/run2` in its own process, then four CHECK lines: no bundler glob token and the builtins registered (counts match `BUILTIN_*`, duplicate registration throws); the manifest valid (schema, sorted families, files present, MPEG-1 Layer III 48 kHz 128 kbps mono first frames, variant counts and the 12/10/3 family split); 0 files differing between run1 and run2 (MP3s and manifest by sha256); and 0 `pcmHash` mismatches against a fresh `packStrip(renderFamilyVariants(...))` render in the parent. It prints `ALL PASS m2` and writes `out/m2/report.json` (per file: both runs' bytes and sha256, `pcmHash`, variants, seconds) and `out/m2/report.md` (the same table plus the CHECK lines). `--only <pattern>` filters cases by label (e.g. `--only 'bake run1'`); checks whose runs were skipped print nothing. Everything under `out/` stays untracked.

## Music (M3)

### The music bed

The music bed is the arranger and `MusicSynth` extracted from the origin engine and generalized from biomes to registered styles. The algorithms are unchanged (Markov progressions, motif variation, intensity layers, bar-quantized sections with fills, the climax at tempo × 1.06 and +2 semitones, resolves). The bed is `MUSIC_BED` (id `music`, bus `music`); the classes are `Arranger` and `MusicSynth`.

### Built-in styles

`UPBEAT_STYLE` (`upbeat`: C major, 128 bpm) and `CALM_STYLE` (`calm`: A dorian, 96 bpm) are genre-neutral demo styles. The `style` param code is the index in `listStyles()` (sorted by id), so always use `styleIndex('calm')`, never a hard-coded number.

### Parameters

`MUSIC_PARAMS` is, in order, the param vector of the bed:

| param | meaning |
| --- | --- |
| `style` | `biome` | style index (`styleIndex(id)`) |
| `styleB` | `biomeB` | second style index |
| `styleMix` | `biomeMix` | blend 0..1 between `style` and `styleB` |
| `intensity` | `intensity` | 0..1, adds layers |
| `phase` | `phase` | 0 intro, 1 main, 2 resolved |
| `climax` | `finalLap` | 0/1, tempo × 1.06 and +2 semitones |
| `resolved` | `finished` | 0/1, plays the resolve |
| `won` | `won` | 0/1, picks `resolveWin` over `resolve` |
| `sparkle` | `star` | the bright arpeggiated layer |
| `tension` | `place01` | 0..1 pressure |
| `fadeBars` | new | palette crossfade length on a style change, clamped to 1..16 bars (the origin fixed 2) |
| `level` | unchanged | output level |

Section names (`Section`): `intro`, `mainA`/`mainB`, `intense`, `climax`, `resolveWin`, `resolve`, `post`. Sections switch on bar lines. `style`, `styleB`, `phase`, `climax`, `resolved`, `won` and `fadeBars` are unsmoothed.

### Adding your own style

Define and register the style before the bed is created; the bed reads `listStyles()` when it is built.

```ts
import { defineStyle, registerStyle, registerBuiltins, styleIndex } from '@etcito/haa'

registerBuiltins()
registerStyle(defineStyle({ id: 'my-theme', root: 62, scale: [0, 2, 4, 7, 9], bpm: 110, swing: 0.05,
  palette: { pad: 0.5, bass: 0.5, lead: 0.5, pluck: 0.5, drums: 0.4, perc: 0.3 },
  progressions: [[0, 2, 3, 1], [0, 3, 2, 4]] }))
const code = styleIndex('my-theme')   // the value of the `style` param
```

Style indexes follow the sorted id order, so read them with `styleIndex` after all registrations. A game with its own styles bundles its own worklet entry (see Registries and bake (M2)): the prebuilt `worklet/haa-processors.js` only knows the built-in styles.

### Offline usage

```ts
import { registerBuiltins, MUSIC_BED, MUSIC_PARAMS, styleIndex, renderBedDemo, renderOffline } from '@etcito/haa'

registerBuiltins()
const P = (n: string) => MUSIC_PARAMS.indexOf(n as (typeof MUSIC_PARAMS)[number])
const music = renderBedDemo(MUSIC_BED, {
  seconds: 20, seed: 1,
  script: (_t, p) => { p[P('style')] = styleIndex('upbeat'); p[P('intensity')] = 0.6; p[P('phase')] = 1 },
})   // raw stereo bed output: { name, left, right, sampleRate }
// or drive a voice yourself: const v = MUSIC_BED.create(48000, 1); renderOffline((l, r, n) => v.process(p, 0, l, r, n), 20)
```

### Commands and the M3 proof

```
node scripts/proof/m3.ts [--only <pattern>] [--full]    # npm run proof:m3
```

The cases (`render upbeat`, `render calm`, `render arc-upbeat`, `render crossfade`, `render stings`) render through `MixChain(BASE_PRESET)` on the music bus input at `DEFAULT_USER_VOLUMES`; `--full` renders 60 s instead of 20 s for upbeat and calm. The four CHECK labels are `safety: music renders` (0 NaN/Inf, |DC| < 1e-3, sample peak <= -1 dBFS, minimum 1 s window RMS >= -60 dBFS, worst printed), `music: notes in scale` (0 out-of-scale pitched notes over 64 bars per style), `music: deterministic` (same seed, same pcmHash; a different seed differs) and `music: crossfade lands` (<= 3 bars from the request until `Arranger.styleIndex` is calm). It ends with `ALL PASS m3`.

`out/m3/` holds `upbeat.wav`, `calm.wav`, `arc-upbeat.wav`, `crossfade.wav`, the stings WAVs (`stings/<id>.wav`), `report.json`, `report.md` and `index.html`.

### M3 provenance

The arranger and synth are extracted from the origin engine and generalized (registered styles, generic params, the `fadeBars` crossfade). `src/content/styles.ts` and `src/content/musicBed.ts` are new. Details are in PROVENANCE.md.

## Runtime and demo (M4)

### API

```ts
import { createAudio, createOfflineAudio, FLAT_SPATIAL } from '@etcito/haa'

// Browser only. Rejects Error('[haa] worklet unavailable: <reason>') when ctx.audioWorklet is missing
// or addModule / node creation fails; an unknown mix id rejects '[haa] unknown mix <id>'.
function createAudio(ctx: BaseAudioContext, o: AudioOptions): Promise<HaaAudio>
interface AudioOptions { manifestUrl: string; processorUrl: string; mix?: string | MixPreset; maxListeners?: 1 | 2 | 3 | 4; spatial?: SpatialModel; seed?: number; families?: readonly string[] }
interface HaaAudio extends AudioControls {
  readonly ctx: BaseAudioContext
  readonly meters: { mix: AnalyserNode; left: AnalyserNode; right: AnalyserNode } | null   // null unless ctx is an AudioContext
  ready(): Promise<void>   // every family loaded from the manifest or fallback-rendered; never rejects
  close(): Promise<void>   // stopAll 0.05 s, beds end 0.05 s, disconnects after 100 ms
}

// DSP-safe (node, main thread, worklet): the offline mirror of the runtime graph (same Sampler, BedHost, MixChain).
function createOfflineAudio(o?: { sampleRate?: number; mix?: string | MixPreset; maxListeners?: 1 | 2 | 3 | 4;
  spatial?: SpatialModel; seed?: number; volumes?: Partial<UserVolumes> }): OfflineAudio
interface OfflineAudio extends AudioControls {
  readonly time: number
  readonly stats: { played: number; dropped: number; stolen: number }
  advance(seconds: number): void
  take(name: string): StereoRender
}

const FLAT_SPATIAL: SpatialModel   // used when no model is set: gain 1/N, pan 0, lowpass 20000, send REVERB_SEND_BASE, rate 1
```

After init the browser runtime never throws: a failed manifest, MP3 fetch or decode warns once and falls back to the in-browser render of that family; an unknown family or bed id returns null and warns once. Offline, a play of a not-yet-loaded family renders it synchronously (cached per process) and plays; in the browser it is dropped until loaded. `examples/quickstart.ts` is a typechecked usage example (play, a wind bed, music, an offline render).

### Commands

```
node scripts/build-worklet.ts [--check]   # npm run build:worklet; --check exits 1 when the committed bundle is stale
node scripts/build-demo.ts [--out out/demo]   # npm run build:demo; bake + worklet + esbuild demo, writes build.json
node scripts/serve.ts [--dir out/demo] [--port 8787]   # npm run serve; 127.0.0.1 only
node scripts/gpu-run.ts --input proofs/m4/input.json --out proofs/m4/levels.json --captures out/proofs/m4   # npm run gpu:run; needs NMC_GPU_CLIENT
node scripts/proof/m4.ts [--only <pattern>] [--full]   # npm run proof:m4
```

The M4 proof prints five CHECK labels and ends with `ALL PASS m4`:

1. `worklet: committed bundle is fresh`
2. `demo: builds` (case `render demo`)
3. `offline: every demo action sounds` (cases `family <id>`, `bed wind`, `music upbeat`, `music calm`; worst max 50 ms RMS >= -40 dBFS, worst peak <= -1 dBFS)
4. `server: serves the demo with correct types`
5. `browser: committed Chrome levels` (needs `out/demo` from check 2 in the same run)

Run checks 1-4 without the browser result with `node scripts/proof/m4.ts --only '/worklet|demo|offline|server|family|bed|music/'`, or a single action with e.g. `--only 'family impact.wood'`.

### The prebuilt worklet

`worklet/haa-processors.js` (export `@etcito/haa/worklet`) is the only prebuilt, committed file: the esbuild bundle of `src/worklet/entry.ts` (`registerBuiltins(); registerHaaProcessors()`). `test/worklet/fresh.test.ts` runs in `npm test` and fails when the committed bundle differs from a fresh build, so any change that reaches worklet code (a host, a built-in bed or style) regenerates it with `node scripts/build-worklet.ts` in the same commit.

### Custom worklet entry (own beds or styles)

Limitation: the published package ships `dist/` and not `src/`, so the relative `src/` import paths below resolve only from a source checkout. Exporting the processor registration for installed consumers is not done yet.

The prebuilt bundle only knows the built-in beds and styles. A game with its own copies `src/worklet/entry.ts`, adds its register calls before `registerHaaProcessors()`, bundles it with the options of `scripts/build-worklet.ts`, and passes the bundle URL as `processorUrl`:

```ts
// game/audio/worklet.ts: a copy of src/worklet/entry.ts. registerHaaProcessors is not in the package
// exports, so the copy imports the HAA sources by relative path (esbuild resolves them).
import { registerBuiltins } from '../../node_modules/@etcito/haa/src/registry/builtins.ts'
import { registerBed, registerStyle } from '../../node_modules/@etcito/haa/src/registry/registry.ts'
import { registerHaaProcessors } from '../../node_modules/@etcito/haa/src/worklet/processors.ts'
import { MY_BED, MY_STYLE } from './content.ts'

registerBuiltins()
registerBed(MY_BED)
registerStyle(MY_STYLE)
registerHaaProcessors()
```

```ts
// game build script (node)
import { build } from 'esbuild'
await build({
  entryPoints: ['game/audio/worklet.ts'], outfile: 'public/game-processors.js',
  bundle: true, format: 'esm', target: 'es2022', minify: false, legalComments: 'none', charset: 'utf8',
})
// runtime: createAudio(ctx, { manifestUrl: './audio/manifest.json', processorUrl: './game-processors.js' })
```

Register the same beds and styles on the main thread too (before `createAudio`), so ids and style indexes agree. One-shot families need no custom entry: register and bake them (Registries and bake (M2)).

### Demo and browser results

`build:demo` writes `out/demo/` (index.html, main.js, haa-processors.js, nmc.gpu.json, build.json, audio/). `gpu:run` publishes it to the client GPU Chrome and runs `proofs/m4/input.json` (start, levels of every core action, snapshot). The browser result is committed as text in `proofs/m4/levels.json` (LevelsResult with the build hashes and each capture's sha256); the screenshots stay in `out/proofs/m4/` and are never committed.

### M4 provenance

The host sampler and the worklet processors are extracted from the origin engine (per-listener spatial rendering replaces its pan and gain; no game voice; guarded registration). The conductor, voices, flat spatial model, offline scene, bank loader, `createAudio` and the demo are written for HAA.

## Spatial (M5)

### Usage

The spatial model is optional: pass it to `createAudio` or install it later with `setSpatial`. `setListeners` takes 1..4 listeners; with none, the model uses one at the origin with the identity quaternion. Positions are plain `{x,y,z}` and orientations plain `{x,y,z,w}` — HAA never imports three.js. A game copies the numbers out of its own vectors itself:

```ts
import { createAudio, createSpatial } from '@etcito/haa'

const audio = await createAudio(ctx, { manifestUrl, processorUrl, spatial: createSpatial() })
audio.setListeners([
  { pos: { x: 0, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } },   // 1..4 listeners
])
audio.setSpatial(createSpatial({ absorptionScale: 1 }))               // or null to use FLAT_SPATIAL

const pos = { x: body.x, y: body.y, z: body.z }   // one-line adapter: a game copies x, y, z (quat likewise {x,y,z,w})
```

Real signatures (from `src/spatial/spatial.ts` and `src/spatial/model.ts`):

```ts
function createSpatial(o?: { absorptionScale?: number }): SpatialModel
function distanceGain(d: number, ref?: number, max?: number): number
function spatializeOne(l: Listener, src: SourcePos, count: number, occlusion: number,
  env: { absorptionScale: number }, out: OneListener): OneListener
function dopplerRatio(l: Listener, src: SourcePos): number
interface SpatialModel { evaluate(listeners: readonly Listener[], src: SourcePos | null,
  occlusion: OcclusionFn | null, out: SpatialResult): SpatialResult }
```

### The listener frame and yaw

`quat` rotates the local frame: forward = q·(0,0,-1), right = q·(1,0,0), up = q·(0,1,0); world up is +Y. The right-hand rule about +Y gives yaw: a listener yawed +90° (`quat = (0, sin45°, 0, cos45°)`) turns counter-clockwise seen from above, so forward becomes -X and right becomes -Z. A source at (0,0,-5), in front of the unrotated listener, is then on its right and gets pan +1; yaw -90° puts it on the left and gives pan -1. This is the rule documented in `test/spatial/spatial.test.ts` (the 'spatial/spatial right-hand rule' block).

### The formulas

- **Distance attenuation:** d = |src − listener.pos|; gain is 1 for d ≤ `REF_DISTANCE`, otherwise `(ref/d) × (1 − smoothstep(0.7 max, max, d))`, and 0 at d ≥ max (`max = src.range ?? DEFAULT_RANGE`).
- **Pan:** dir = (src − pos)/d; az = atan2(dot(dir, right), dot(dir, forward)); pan = sin(az), narrowed to `SPLIT_PAN_WIDTH = 0.5` when N > 1. Equal-power: L = cos((pan+1)π/4), R = sin((pan+1)π/4).
- **Rear and air-absorption lowpass:** rear = max(0, −cos az); lowpass = 20000 − rear × (20000 − 7000), then × clamp(1 − d / (400 × absorptionScale), 0.3, 1).
- **Occlusion:** o = clamp(max(src.occlusion ?? 0, occlusionFn?.(src.pos, l) ?? 0), 0, 1); when o > 0, lowpass = max(900, lowpass × (1 − 0.95 o)) and gain ×= dB(−9 o).
- **Reverb send:** 0.08 + 0.35 × clamp(d / 60, 0, 1).
- **Doppler:** u = (listener.pos − src.pos)/d; vs = dot(src.vel ?? 0, u); vl = −dot(listener.vel ?? 0, u); ratio = ((343 + vl)/(343 − vs))^0.5, clamped to 0.5..2.
- **Multi-listener:** each listener l gets gain × 1/N, and the per-listener results are summed into one stereo output. The dominant listener (highest gain, ties lowest index) supplies `sp[SP.SEND]` and the playback rate. A null source gets 1/N, pan 0 and lowpass 20000 per listener, send 0.08 and rate 1; a source at d < 1e-6 gets gain 1, pan 0, lowpass 20000, send 0.08 and Doppler 1.

Constants (exported by `src/spatial/spatial.ts` unless noted):

| constant | value | meaning |
| --- | --- | --- |
| `SPEED_OF_SOUND` | 343 m/s | Doppler numerator/denominator |
| `DOPPLER_AMOUNT` | 0.5 | Doppler ratio exponent |
| `REF_DISTANCE` | 4 m | gain is 1 within this distance |
| `DEFAULT_RANGE` | 80 m | gain is 0 at and beyond this distance |
| `OPEN_LOWPASS_HZ` | 20000 Hz | unfiltered cutoff |
| `REAR_LOWPASS_HZ` | 7000 Hz | cutoff straight behind |
| `AIR_ABSORPTION_M` | 400 m | air-absorption scale |
| `OCCLUDED_MIN_HZ` | 900 Hz | lowpass floor under occlusion |
| `OCCLUDED_GAIN_DB` | −9 dB | gain of a fully occluded source |
| `SPLIT_PAN_WIDTH` | 0.5 | pan width when N > 1 |
| `REVERB_SEND_BASE` | 0.08 | base send (from `src/contract.ts`) |
| `REVERB_SEND_DIST` | 0.35 | extra send at full distance |
| `REVERB_SEND_DIST_M` | 60 m | distance of the full extra send |
| `DOPPLER_MIN` / `DOPPLER_MAX` | 0.5 / 2 | rate clamp |

### The beacon bed and the spatial demo panel

`BEACON_BED` (id `beacon`, effects bus, positional, params `level`, `rate`, `doppler`, defaults 1/4/1) is a continuous, easy-to-locate demo source: a ModalBank glass-like ping around 880, 2215 and 3960 Hz with seeded jitter, excited `rate` times per second by short Hann-windowed noise bursts, plus a soft band-passed pink-noise hum at 300 Hz so energy never drops out between pings. Every mode frequency is multiplied by the Doppler ratio written by the bed host; there is no raw oscillator.

`demo/panels/spatial.ts` (id `spatial`, order 10) calls `audio.setSpatial(createSpatial())` on mount, draws a top-down view of the listener and source, and offers two 4 s actions updated every 20 ms: `spatial:sweep` moves the beacon from x = −20 to +20 at z = −5 with vel (10, 0, 0), and `spatial:flyby` moves it from z = −60 to +60 at x = 3 with vel (0, 0, 30) for Doppler.

### Commands and the M5 proof

```
node scripts/proof/m5.ts [--only <pattern>]    # npm run proof:m5
```

The proof runs cases `render demo`, `render sweep` and `render flyby` (up to NMC_CPUS children, 1 when unset) and prints five CHECK labels: the shared known-geometry table (gain, pan, lowpass, send; 1e-4), each of N = 1..4 listeners contributing 1/N (1e-6), the Doppler approach/recede ratios (1e-4), zero three imports in `src`, `demo` and `scripts`, and the committed Chrome trace panning from ≤ −3 dB to ≥ +3 dB right-minus-left over the sweep. The renders are review captures, not extra checks.

Refresh the committed browser trace:

```
node scripts/build-demo.ts
node scripts/gpu-run.ts --input proofs/m5/input.json --out proofs/m5/trace.json --captures out/proofs/m5
```

`proofs/m5/input.json` is `start` → `trace` of `spatial:sweep` every 50 ms → `snapshot`. Captures go to `out/m5/` (`sweep.wav`, `flyby.wav`, and the report triplet `report.json`, `report.md`, `index.html`) and the browser screenshots stay in `out/proofs/m5/`; nothing under `out/` is committed.

### M5 provenance

The spatial code (`src/spatial/spatial.ts`, `src/spatial/model.ts`) is new HAA code written from the contract formulas ('createSpatial formulas'). No code was copied from the origin engine's director or any other of its modules. `src/content/beacon.ts` and `demo/panels/spatial.ts` are likewise new HAA code. The extraction base is described in PROVENANCE.md.

## Example sound pack (M6)

M6 is a complete example sound pack kept outside the core: one-shot families built from the generator kits, a continuous crowd bed, a mix preset, a scripted 60 s race soundtrack and its offline proof. It lives under `examples/`, with its own README that lists the families, the bed, the preset and the race. The core builds in none of it: the pack imports HAA through the public API and registers its content with the public registry (`registerFamily`, `registerBed`, `registerMix`). That is the pattern for any game's sound pack.

The example is an optional pack: its commands (the race render and the proof) are in the README of the pack under `examples/`. The core `npm test` runs the pack's tests too.

The proof checks family and bed safety, family loudness within ±1 LU of the category target, the race's safety and integrated loudness (±1 LU of the scene target) and that every scripted sound played.
