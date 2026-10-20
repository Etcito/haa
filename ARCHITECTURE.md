# HAA architecture

HAA ("Hear Anything Anywhere") is a procedural sound engine that works with any game. It ships as the TypeScript package `@etcito/haa`. `src/contract.ts` holds every public type of the API. Implementations build against those types and never rename a public type or field. Only the lead decides a contract change.

## 1. Deliverable and stack

- **Deliverable:** a library, a set of node tools (offline render, measurement report, MP3 bake) and a one-page browser demo.
  - Package `@etcito/haa`, repo `haa`, MIT licence, org Etcito, private.
  - It is not a deployed service and has no deploy artefact. The demo server is a local loopback dev tool.
  - Games consume it as a GitHub dependency (Kong will manage this later). Until then HAA is developed and proven in its own repo only.
- **Provenance:** the audio engine was extracted from an Etcito game and generalized. The origin is described in PROVENANCE.md; no task reads or modifies the origin repository.
  - Game-specific code and content stay in their games: vehicle, drivetrain and gameplay models, the game's director and its per-game families and presets.
  - HAA writes its own voice allocator and spatial code from the formulas in this document, not from the origin's director.
  - Every file extracted from the origin starts with a header that names PROVENANCE.md and lists the HAA changes.
- **Language:**
  - TypeScript strict, ES modules, package.json `"type": "module"`, `engines.node` `>=22.18`.
  - Node runs the .ts files directly with native type stripping, so only erasable syntax is allowed: no enum, no namespace, no constructor parameter properties, no `import =`.
  - Every relative import ends in `.ts`. Type-only imports use `import type`.
  - Extracted code gets every import rewritten, because the origin imports without extensions.
- **tsconfig.json:**
  - strict, target ES2022, module and moduleResolution `nodenext`;
  - allowImportingTsExtensions, verbatimModuleSyntax, erasableSyntaxOnly;
  - declaration + emitDeclarationOnly with outDir `out/types`;
  - lib `['ES2022', 'DOM']`, types `['node']`;
  - include src, scripts, test, demo, examples.
- **Distribution:** the TypeScript sources are exported directly. package.json `exports`:
  - `.` → `./src/index.ts` (browser-safe API);
  - `./node` → `./src/node.ts` (node-only API: fs, bake);
  - `./worklet` → `./worklet/haa-processors.js`.
  - The consuming game's bundler compiles the sources. The only committed build output is `worklet/haa-processors.js`, the esbuild bundle of `src/worklet/entry.ts`.
- **Dependencies:**
  - Runtime: exactly `"lamejs": "1.2.1"`, loaded only in node through `createRequire(import.meta.url)` (see Module: bake for the load shim). It is never patched, vendored or replaced.
  - devDependencies, pinned exactly: typescript 5.9.3, vitest 3.2.7, esbuild 0.28.2, @types/node 22.20.5.
  - No other dependency is ever added. package-lock.json is committed, and `npm ci` restores node_modules.
- **package.json scripts:**
  - `typecheck`: `tsc -p tsconfig.json --noEmit`; `build:types`: `tsc -p tsconfig.json`; `test`: `vitest run`;
  - `bake`: `node scripts/bake.ts`;
  - `build:worklet`: `node scripts/build-worklet.ts`; `build:demo`: `node scripts/build-demo.ts`; `serve`: `node scripts/serve.ts`; `gpu:run`: `node scripts/gpu-run.ts`;
  - `example:race`: the example pack's race script (run with `--conditions=haa-source`);
  - `proof:m1` … `proof:m5`: `node scripts/proof/mN.ts` (M6 is the example pack, see its README under `examples/`).
- **How to run:** `npm run typecheck`, `npm test`, `node scripts/proof/mN.ts [--only <pattern>] [--full]`, `node scripts/bake.ts`, `node scripts/build-demo.ts`, `node scripts/serve.ts`.

## 2. Layout (haa)

- `src/contract.ts`: constants, shared types and the frozen public API types.
- `src/dsp/`: DSP primitives.
- `src/gen/`: sound generators (building blocks and models).
- `src/music/`: instruments, the arranger and the music synth.
- `src/mix/`: the mix chain; presets live in `src/mix/presets/*.ts`.
- `src/content/`: the built-in families, beds and styles. Game and product content is not here: optional packs live in `examples/` and register themselves through the public registry.
- `src/bank/`: variant rendering, normalization and strip packing.
- `src/analysis/`: measurement and report builders.
- `src/offline/`: the WAV codec, offline render helpers and the offline scene.
- `src/registry/`: explicit registries.
- `src/bake/`: the node MP3 bake.
- `src/host/`: the sampler and bed hosts, run by the processors and offline.
- `src/worklet/`: the AudioWorklet processors and the bundle entry.
- `src/runtime/`: the conductor, voices and the browser `createAudio`.
- `src/spatial/`: spatial math and the spatial model.
- `src/node/` and `src/node.ts`: the node-only API.
- `src/index.ts`: the browser-safe public API.
- `worklet/haa-processors.js`: the generated, committed processor bundle.
- `demo/`: the demo page (index.html, main.ts, panel.ts, gpuOps.ts, panels/*.ts, nmc.gpu.json).
- `examples/`: typechecked usage examples.
- `scripts/`: node tools. Milestone proofs are in `scripts/proof/`, shared proof helpers in `scripts/proof/lib/`, and the example packs' scripts live under `examples/`.
- `test/`: Vitest files that mirror src (`test/<dir>/<name>.test.ts`).
- `proofs/mN/`: committed text results of browser runs (JSON only).
- `out/`: every generated file (WAV, MP3, reports, demo build, captures, types). It is git-ignored.
- `.nmc/artifacts.json`: the list of non-audio files under out/ that are reported as card outputs.

## 3. Isomorphic DSP and agnosticism

- **DSP-safe files:**
  - src/contract.ts, src/dsp, src/gen, src/music, src/mix, src/content, src/bank, src/analysis, src/host, src/spatial, src/registry;
  - src/offline/wav.ts, src/offline/render.ts, src/offline/scene.ts;
  - src/runtime/conductor.ts, src/runtime/voices.ts, src/runtime/flatSpatial.ts.
- **Rules for DSP-safe files:**
  - Runtime imports come only from other DSP-safe files. `import type` from anywhere in the package is allowed.
  - Never use Math.random, Date, performance, timers, DOM, the Web Audio API at runtime, fs, process or node builtins.
  - Type-only mentions of `BaseAudioContext` are allowed. Constructing or using an audio context as a value is not.
  - They run unchanged in node, on the browser main thread and in AudioWorkletGlobalScope.
- **Browser-only files:** src/runtime/createAudio.ts, src/runtime/bankLoader.ts, src/worklet/*, demo/*.
- **Node-only files:** src/bake/*, src/node.ts, src/node/*, scripts/*.
- **Agnostic:**
  - No file imports `three`. Positions are plain `{x,y,z}` and orientations plain `{x,y,z,w}`.
  - No file imports from `examples/` or uses an absolute path.
  - No file uses `import.meta.glob` or any bundler-specific import (`?url`, `?worker`).

## 4. Audio engineering rules

- **Units:** Hz, seconds, dB (dBFS, dBTP, LUFS, LU), linear gain, meters, m/s.
- **Rates:** offline renders and the bake use AUDIO_RATE = 48000. The worklet uses the context rate.
- **Blocks:** hosts render blocks of at most BLOCK = 128 frames. Processors never allocate in process(); every buffer is preallocated at construction.
- **Guards:** every host checks its block. A non-finite sample zeroes the block and resets the voice that produced it.
- **No raw oscillators:** a sine, triangle, square or saw is never routed straight to an output. Tonal content comes from:
  - additive partial banks with per-partial amplitude and drift;
  - excitation-driven resonators (modal, waveguide, filtered pulse trains);
  - FM/AM with modulated parameters.
- **Nonlinearities:** saturation that could alias audibly is oversampled at least 2x (dsp/shaper.ts).
- **Variation:**
  - each family holds 8 to 32 seeded variants;
  - every play adds seeded micro-variation (rate ±3 %, gain ±1 dB);
  - every play avoids the variant last played for that family.
- **Levels:**
  - generators output nominal peaks around -6 dBFS;
  - one-shot loudness is normalized per category at render and bake time (§7);
  - all balance between sounds lives in mix presets (bus levels, familyTrimDb, bedTrimDb), never in gains scattered through game code.
- **Determinism:** every random draw comes from AudioRng seeded through hashSeed. The same inputs give bit-identical PCM and a byte-identical MP3 and manifest.

## 5. Constants (src/contract.ts)

- `HAA_CONTRACT_VERSION = 0`; `AUDIO_RATE = 48000`; `BLOCK = 128`; `MAX_LISTENERS = 4`.
- **Buses:** `type BusId = 'world' | 'foley' | 'effects' | 'ambience' | 'music' | 'ui'`.
  - `BUSES` lists them in that order. `BUS_INDEX` gives world 0, foley 1, effects 2, ambience 3, music 4, ui 5.
  - `REVERB_INPUT = 6`, `MIX_INPUTS = 7`.
  - Mapping from the origin engine: engine → world (continuous source voices), tires → foley (contact and body sounds). The other buses keep their names.
- **Materials:** `type MaterialId = 'metal' | 'plastic' | 'wood' | 'concrete'`; `MATERIALS` lists them in that order.
- **Voice categories:** `type VoiceCategory = 'impact' | 'fx' | 'ui' | 'ambience' | 'sting'`; `VOICE_CATEGORIES` lists them in that order.
  - `VOICE_LIMITS = { impact: 12, fx: 16, ui: 4, ambience: 8, sting: 2 }`.
- `DEFAULT_USER_VOLUMES = { master: 0.8, music: 0.6, sfx: 0.8 }`.
- **Families and banks:** `FAMILY_VARIANTS_MIN = 8`, `FAMILY_VARIANTS_MAX = 32`, `FAMILY_MAX_SECONDS = 10`, `GAP_SAMPLES = 4800`, `ONSET_THRESHOLD = 1e-3`.
- **Processors:** `PROCESSOR_NAMES = { bed: 'haa-bed', sampler: 'haa-sampler', mix: 'haa-mix' } as const`, `SAMPLER_VOICES = 64`.
- **Safety and loudness:** `SAFETY = { maxSamplePeakDb: -1, maxAbsDc: 1e-3, minRmsDb: -60 } as const`, `NORMALIZE_PEAK_CAP_DB = -1.05`, `LOUDNESS_TOLERANCE_LU = 1`.
- `REVERB_SEND_BASE = 0.08`.
- `MICRO_VARIATION = { rate: 0.03, gainDb: 1 } as const`.

## 6. Geometry and spatial block

```ts
interface Vec3 { x: number; y: number; z: number }
interface Quat { x: number; y: number; z: number; w: number }
interface Listener { pos: Vec3; quat: Quat; vel?: Vec3 }
interface SourcePos { pos: Vec3; vel?: Vec3; range?: number; occlusion?: number }
type OcclusionFn = (src: Vec3, listener: number) => number   // 0..1
interface SpatialResult { sp: Float32Array; rate: number }       // sp has SP_LENGTH floats; rate = Doppler ratio
interface SpatialModel { evaluate(listeners: readonly Listener[], src: SourcePos | null, occlusion: OcclusionFn | null, out: SpatialResult): SpatialResult }
```

- **Spatial block `sp`:** a Float32Array of `SP_LENGTH = 14` (2 + 3 × MAX_LISTENERS). Layout: `SP = { COUNT: 0, SEND: 1, GAIN: 2, PAN: 6, LOWPASS: 10 } as const`.
  - Listener l (0..3) uses sp[SP.GAIN + l] (linear, 1/N factor INCLUDED), sp[SP.PAN + l] (-1..1) and sp[SP.LOWPASS + l] (Hz).
  - sp[SP.COUNT] is the active listener count N (1..4). sp[SP.SEND] is the reverb send (0..1).
- **Listener frame:** forward = q applied to (0,0,-1), right = q applied to (1,0,0), up = q applied to (0,1,0). World up is +Y.
- **Multi-listener mixing:** each source is spatialized per listener, and the results are summed into one stereo output with a per-listener gain of 1/N.
  - A source without position (listener-level) gets gain 1/N, pan 0 and lowpass 20000 for every listener.
  - A sound addressed to listener l gets 1/N for l and 0 for the others.
- **FLAT_SPATIAL** (used when no model is set): every listener l < N gets gain 1/N, pan 0, lowpass 20000 and send REVERB_SEND_BASE, with rate 1. Positions are ignored.
- **createSpatial formulas:**
  - **Constants:** SPEED_OF_SOUND 343, DOPPLER_AMOUNT 0.5, REF_DISTANCE 4, DEFAULT_RANGE 80, OPEN_LOWPASS_HZ 20000, REAR_LOWPASS_HZ 7000, AIR_ABSORPTION_M 400, OCCLUDED_MIN_HZ 900, OCCLUDED_GAIN_DB -9, SPLIT_PAN_WIDTH 0.5, REVERB_SEND_DIST 0.35, REVERB_SEND_DIST_M 60, DOPPLER_MIN 0.5, DOPPLER_MAX 2.
  - **Distance gain:** `distanceGain(d, ref, max)` = 1 for d ≤ ref; otherwise (ref / d) × (1 − smoothstep(0.7 max, max, d)); and 0 for d ≥ max.
  - **Distance:** d = |src − listener.pos|. When d < 1e-6 the result is gain 1, pan 0, lowpass 20000, send REVERB_SEND_BASE and Doppler 1.
  - **Direction:** dir = (src − pos)/d; x = dot(dir, right); z = dot(dir, forward); az = atan2(x, z).
  - **Pan:** pan = sin(az) × (N > 1 ? SPLIT_PAN_WIDTH : 1). The pan is equal-power: L = cos((pan+1)π/4), R = sin((pan+1)π/4).
  - **Lowpass:** rear = max(0, −cos(az)); lowpass = 20000 − rear × (20000 − 7000); then lowpass × clamp(1 − d / (400 × absorptionScale), 0.3, 1).
  - **Occlusion:** o = clamp(max(src.occlusion ?? 0, occlusionFn?.(src.pos, l) ?? 0), 0, 1). When o > 0: lowpass = max(900, lowpass × (1 − 0.95 o)), and gain × dB(−9 o).
  - **Gain:** gain_l = distanceGain(d, 4, src.range ?? 80) × occlusion gain × (1/N).
  - **Send:** send_l = REVERB_SEND_BASE + 0.35 × clamp(d / 60, 0, 1).
  - **Doppler:** u = (listener.pos − src.pos)/d, vs = dot(src.vel ?? 0, u), vl = −dot(listener.vel ?? 0, u); ratio = ((343 + vl) / (343 − vs))^0.5, clamped to 0.5..2.
  - **Dominant listener:** the one with the highest gain_l (ties go to the lowest index). sp[SP.SEND] and rate come from it.
  - **No listeners:** one listener at the origin with the identity quat is used.
- **Known-geometry table:** 1 listener at the origin, identity quat, absorptionScale 1, tolerance 1e-4.
  - src (0,0,−2): gain 1, pan 0, lowpass 19900, send 0.0916667.
  - (8,0,0): gain 0.5, pan 1, lowpass 19600, send 0.1266667.
  - (0,0,40): gain 0.1, |pan| < 1e-6, lowpass 6300.
  - (0,0,−68): gain 0.0294118.
  - (0,0,−2) with occlusion 1: gain 0.3548134, lowpass 995.
  - 2 listeners [origin, (100,0,0)], src (0,0,−8): g0 0.25, g1 0, pan0 0.
  - Doppler, src (0,0,−100): vel (0,0,34.3) gives 1.0540926; vel (0,0,−34.3) gives 0.9534626.

## 7. Content definitions and loudness

```ts
interface FamilyDef { id: string; bus: BusId; category: VoiceCategory; variants: number; maxSeconds: number; seed: number;
  render(out: Float32Array, sr: number, rng: AudioRng, v: number): number }   // out is zeroed, length ceil(maxSeconds*sr); ADDS mono; returns the frames written (<= out.length)
type FamilySpec = Omit<FamilyDef, 'seed'> & { seed?: number }   // seed default hashSeed('family', id)
interface BedVoice { process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void; reset(): void }   // ADDS stereo; reads params at p[offset..]
interface BedDef { id: string; bus: BusId; params: readonly string[]; defaults: readonly number[]; unsmoothed?: readonly string[]; positional: boolean; create(sr: number, seed: number): BedVoice }
interface MusicStyle { id: string; root: number; scale: readonly number[]; bpm: number;
  palette: { pad: number; bass: number; lead: number; pluck: number; drums: number; perc: number }; swing: number; progressions: readonly (readonly number[])[] }
interface ReverbParams { size: number; decay: number; damping: number; predelay: number; returnDb: number }   // size 0..1, decay RT60 s, damping Hz, predelay s
interface CompPreset { thresholdDb: number; ratio: number; kneeDb: number; attackMs: number; releaseMs: number; makeupDb: number }
interface EqPreset { lowShelfHz: number; lowShelfDb: number; peakHz: number; peakDb: number; peakQ: number; highShelfHz: number; highShelfDb: number }
interface BusPreset { levelDb: number; eq: EqPreset; comp: CompPreset }
interface DuckPreset { musicDb: number; ambienceDb: number; attackMs: number; holdMs: number; releaseMs: number }
interface MixPreset { id: string; priority: number; buses: Record<BusId, BusPreset>;
  master: { trimDb: number; glue: CompPreset; limiter: { ceilingDbtp: number; lookaheadMs: number; releaseMs: number } };
  ducks: Record<string, DuckPreset>; loudness: Record<VoiceCategory, number>; sceneLufs: number;
  familyTrimDb: Record<string, number>; bedTrimDb: Record<string, number>; reverb: ReverbParams }
```

- **Positional beds:** a bed with `positional: true` is spatialized like a one-shot. If its params include `doppler`, the host writes the Doppler ratio there.
- **Smoothing:** params named in `unsmoothed` are not smoothed. Every other param gets a 15 ms one-pole per block, with coefficient exp(−BLOCK / (0.015 sr)).
- **Limiter:** limiter.ceilingDbtp ≤ −1.2 in every preset.
- **Active mix:** `activeMix()` picks the highest priority, then the lowest id.
- **One-shot loudness**, `oneShotLufs(x, n, sr)`:
  - y = x[0..n) × √½ (the centered equal-power pan), zero-padded to at least 0.4 s;
  - the result is the LoudnessMeter (BS.1770-4) integrated loudness of the pair (y, y);
  - it is −Infinity when gated out.
- **Family normalization** (src/bank/render.ts):
  - render every raw variant;
  - median = the lower median of the finite variant LUFS values;
  - gain = min(dB(target − median), dB(NORMALIZE_PEAK_CAP_DB − the largest variant sample peak in dBFS)), where target = preset.loudness[category];
  - the same gain applies to every variant of the family, so variant dynamics are kept;
  - when no variant has a finite LUFS, the gain is 1.
- **Family check:** |median LUFS after normalization − target| ≤ 1 LU.
- **Default targets** (base preset): impact −21, fx −20, ui −24, ambience −26, sting −18 LUFS; sceneLufs −16. Only the lead can change these values. They are never changed to make a check pass.

## 8. Public API (types in src/contract.ts)

```ts
interface PlayOptions { pos?: Vec3; vel?: Vec3; gain?: number; pitch?: number; bus?: BusId; variant?: number; range?: number; occlusion?: number; listener?: number }
// gain linear (default 1); pitch in semitones (default 0); bus overrides the family bus; listener = only that listener hears it
interface Shot { readonly voice: number; setPos(pos: Vec3, vel?: Vec3): void; stop(fadeSec?: number): void }
interface BedHandle { readonly id: string; set(values: ArrayLike<number>): void; setParam(name: string, value: number): void; setPos(pos: Vec3, vel?: Vec3): void; stop(fadeSec?: number): void }
type MusicParam = 'intensity' | 'phase' | 'climax' | 'resolved' | 'won' | 'sparkle' | 'tension' | 'level'
interface MusicControl { readonly style: string | null; play(style: string, o?: { seed?: number; intensity?: number }): void; crossfade(style: string, bars: number): void; set(name: MusicParam, v: number): void; stop(fadeSec?: number): void }
interface UserVolumes { master: number; music: number; sfx: number }
interface AudioOptions { manifestUrl: string; processorUrl: string; mix?: string | MixPreset; maxListeners?: 1 | 2 | 3 | 4; spatial?: SpatialModel; seed?: number }
interface AudioControls { play(id: string, o?: PlayOptions): Shot | null; startBed(id: string, o?: { seed?: number; pos?: Vec3; vel?: Vec3; values?: ArrayLike<number> }): BedHandle | null;
  setListeners(ls: readonly Listener[]): void; setSpatial(m: SpatialModel | null): void; setOcclusion(fn: OcclusionFn | null): void;
  duck(kind: string, strength?: number): void; setVolumes(v: Partial<UserVolumes>): void; setReverb(p: ReverbParams): void; setMuffle(v: number): void; readonly music: MusicControl }
interface HaaAudio extends AudioControls { readonly ctx: BaseAudioContext; readonly meters: { mix: AnalyserNode; left: AnalyserNode; right: AnalyserNode } | null; ready(): Promise<void>; close(): Promise<void> }
interface StereoRender { name: string; left: Float32Array; right: Float32Array; sampleRate: number }
interface OfflineAudio extends AudioControls { readonly time: number; readonly stats: { played: number; dropped: number; stolen: number }; advance(seconds: number): void; take(name: string): StereoRender }
```

- **ready():** resolves once the bank loader has loaded (or fallback-rendered) every family. It never rejects.
- **setVolumes:** master scales the master stage before the limiter; music scales the music bus; sfx scales every other bus. Values are clamped to 0..1 and smoothed over 50 ms.
- **duck(kind, strength = 1):** triggers preset.ducks[kind] with its depths × strength. An unknown kind is ignored.
- **music:**
  - play starts the `music` bed with that style index, or sets the style if the bed is already running;
  - crossfade(style, bars) sets style to the new index and fadeBars to bars (clamped 1..16), and the switch is bar-quantized;
  - set writes the bed param.

## 9. Worklet nodes and ports

The offline scene mirrors this graph with the same host classes.

- **`haa-mix`:** one per context.
  - 7 stereo inputs (BUS_INDEX order, then REVERB_INPUT) and 1 stereo output.
  - processorOptions `{ preset: MixPreset }`.
  - The output feeds a ChannelSplitter that drives the left and right analysers, the mix analyser, and ctx.destination. The analysers use fftSize 2048.
- **`haa-sampler`:** one per context. 0 inputs; 7 stereo outputs go to mix inputs 0..6.
- **`haa-bed`:** one per started bed.
  - processorOptions `{ bed: string; seed: number; trimDb: number; values: number[] }`.
  - 0 inputs and 2 stereo outputs: output 0 goes to mix input BUS_INDEX[def.bus], output 1 to mix input REVERB_INPUT.
  - process returns false after `end` plus its fade.
- **Port messages:**

```ts
type SamplerMsg = { t: 'load'; id: string; variants: Float32Array[]; rate: number }   // buffers transferred
  | { t: 'play'; voice: number; id: string; variant: number; bus: number; gain: number; rate: number; sp: Float32Array }
  | { t: 'upd'; voice: number; rate: number; sp: Float32Array } | { t: 'stop'; voice: number; fade: number } | { t: 'stopAll'; fade: number }
type BedMsg = { t: 'p'; v: Float32Array } | { t: 'sp'; rate: number; sp: Float32Array } | { t: 'end'; fade: number }
type MixMsg = { t: 'preset'; p: MixPreset } | { t: 'user'; master: number; music: number; sfx: number } | { t: 'duck'; kind: string; strength: number } | { t: 'reverb'; p: ReverbParams } | { t: 'muffle'; v: number }
```

- **Sampler voice rendering:**
  - Playback is Hermite 4-point at rate × loadRate / sr.
  - For each listener l < sp[COUNT] whose gain is > 1e-5:
    1. a one-pole lowpass at sp[LOWPASS+l] (state per voice and listener);
    2. an equal-power pan at sp[PAN+l];
    3. × gain × sp[GAIN+l], added to the bus pair.
  - The send pair receives the voice's summed stereo × sp[SEND].
  - `upd` values are smoothed over 10 ms.
  - A `play` on an occupied slot replaces the old voice with a 5 ms fade. An unknown id or variant is ignored.

## 10. Seeds

- `hashSeed(...parts: (number | string)[]): number` is FNV-1a 32, extracted from the origin engine.
- Family variant v renders with `new AudioRng(hashSeed(def.seed, def.id, v))`.
- The audio seed is AudioOptions.seed ?? 1.
- The shot variation stream is hashSeed(seed, 'shots').
- Bed instance k uses seed hashSeed(seed, 'bed', bedId, k), unless startBed passes one.

## 11. Bank manifest

Written by bake to `<outDir>/manifest.json`.

```ts
interface BankManifest { version: 1; sampleRate: 48000; kbps: 128; encoder: 'lamejs 1.2.1'; preset: string; families: BankFamilyEntry[] }
interface BankFamilyEntry { id: string; file: string; bus: BusId; category: VoiceCategory; variants: { start: number; length: number }[]; onset0: number; pcmHash: string; mp3Sha256: string }
```

- **families:** sorted by id; file = `<id>.mp3`.
- **Strip:** a family's normalized variants in order, each followed by GAP_SAMPLES zeros. start and length are in samples at 48 kHz.
- **onset0:** the first strip index where |x| > ONSET_THRESHOLD.
- **pcmHash:** 8 lowercase hex digits of FNV-1a 32 over the strip's Float32 little-endian bytes.
- **mp3Sha256:** the hex sha256 of the MP3 file.
- **preset:** the id of the preset whose loudness targets were used.
- **Runtime alignment** (`alignVariants`, after decodeAudioData at context rate R):
  - k = R / 48000;
  - shift = (first decoded index with |x| > ONSET_THRESHOLD) − round(onset0 × k);
  - variant i = decoded[round(start × k) + shift, + round(length × k)), clamped. An empty slice gives a zero-length variant.

## 12. Measurement and reports (src/analysis)

```ts
interface Measure { frames: number; seconds: number; nonFinite: number; samplePeakDb: number; truePeakDb: number; dc: number; rmsDb: number; minWindowRmsDb: number;
  lufs: number; shortTermMaxLufs: number; bandsDb: number[] }
interface ReportEntry { group: string; id: string; file: string; title: string; measure: Measure; targetLufs?: number; note?: string }
interface ReportCheck { label: string; measured: string; threshold: string; pass: boolean }
interface Report { proof: string; entries: ReportEntry[]; checks: ReportCheck[] }
```

These types live in src/analysis and are re-exported from src/index.ts.

- **dc:** the largest per-channel |mean|.
- **minWindowRmsDb:** the minimum RMS over consecutive 1 s windows. It equals rmsDb when the signal is shorter than 2 s.
- **bandsDb:** the RMS dBFS per octave band at OCTAVE_BANDS = [31.5, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]. Each band uses two cascaded RBJ bandpass biquads with Q √2, on the mono sum.
- **Report triplet:** every proof that renders sound writes `report.json`, `report.md` and `index.html`. The HTML has an `<audio controls preload='none'>` per WAV, its measures and the CHECK lines.

## 13. Browser run results (committed under proofs/mN/)

```ts
interface BuildInfo { mainSha256: string; workletSha256: string; manifestSha256: string }   // out/demo/build.json
interface LevelsResult { version: 1; build: BuildInfo; ctxState: string; sampleRate: number; userAgent: string;
  actions: { id: string; baselineDb: number; maxRmsDb: number; peakDb: number }[]; captures: { file: string; sha256: string }[] }   // proofs/m4/levels.json
interface TraceResult { version: 1; build: BuildInfo; ctxState: string; sampleRate: number; action: string;
  points: { t: number; x: number; leftDb: number; rightDb: number }[]; captures: { file: string; sha256: string }[] }   // proofs/m5/trace.json
```

- **Page ops:** `window.nmcGpu.ops.render(steps)` takes these steps and returns `{ results: unknown[] }`:

  ```ts
  { op: 'start' } | { op: 'levels'; actions?: string[] } | { op: 'trace'; action: string; everyMs: number } | { op: 'snapshot' }
  ```

- **`start`:**
  - creates the AudioContext and races ctx.resume() against START_RESUME_MS = 2000 ms (a timeout is not an error);
  - if the state is `running`, it awaits audio.ready(); otherwise it returns at once;
  - it returns `{ ctxState, sampleRate, userAgent: navigator.userAgent }` on both paths.
- **`levels`:** measures each action through a dedicated AnalyserNode (fftSize 32768, one preallocated Float32Array) fed from audio.meters.mix:
  1. **Settle:** wait until the last-50 ms mix RMS has stayed below SILENCE_DB = −70 dBFS for BASELINE_MS = 200 ms in a row. Any reading ≥ SILENCE_DB restarts the wait. The total wait is at most SILENCE_MAX_MS = 3000 ms.
  2. **Baseline:** the max 50 ms RMS over the next BASELINE_MS.
  3. **Run** the action for action.seconds.
  4. **Measure** only the samples produced since the measurement began (capped at the buffer length): maxRmsDb is the max 50 ms RMS (hop 10 ms), and peakDb is the max absolute sample.
  5. **Stop** the action.
- **`trace`:** samples the left and right analysers.
- **`snapshot`:** calls host.drawMeter() synchronously (opaque `#111827` background) right before canvas.toDataURL. The client captures the result.
- **`demo/nmc.gpu.json`:** `{ version: 1, page: 'index.html', ops: ['render'], ready: 'window.nmcGpu.ready' }`, copied into out/demo/.

## 14. Registries

- Content is registered explicitly with `registerFamily(ies)`, `registerBed`, `registerMix` and `registerStyle`. There are no globs.
- `registerBuiltins()` registers the arrays exported by src/content/index.ts. It is idempotent.
- Ids are unique per kind. A duplicate throws `[haa] duplicate <kind> <id>`.
- Registry state is module-level, so each realm (main thread, worklet, node process) has its own.
- The worklet entry calls registerBuiltins() before registering the processors, so the prebuilt bundle knows every built-in bed and style.
- **Game content:**
  - A game with its own beds or styles bundles its own worklet entry (README recipe).
  - A game's own one-shot families need only register + bake.

## 15. Naming, style and errors

- **Naming:**
  - Files are camelCase.ts. Types and classes are PascalCase. Tunable constants are UPPER_SNAKE. Named exports only.
  - Family ids are lowercase dotted segments; a segment may be camelCase (`impact.metal.small`, `cue.placeUp`).
  - Bed, style and preset ids are lowercase with dashes.
- **Comments:** each file starts with a header comment stating its role (and its provenance when copied). Exports carry doc comments.
- **Errors in authored data:** invalid data in define*/register* throws `Error('[haa] <detail>')` when it is defined or registered.
- **Per-block paths:** process() and every per-block path never throw. They catch, output silence and reset.
- **Browser runtime:**
  - `createAudio` rejects with `Error('[haa] worklet unavailable: <reason>')` only when ctx.audioWorklet is missing or addModule or node creation fails.
  - After init nothing throws. A failed manifest, MP3 fetch or decode is logged once with `console.warn('[haa] ...')`, and that family falls back to an in-browser render with the same generators.
  - An unknown family or bed id returns null and warns once per id.
  - Every promise is caught.
- **Node tools:** they exit 1 with a one-line `[<tool>] <reason>`.

## 16. Data on disk and repository rules

- **WAV:** RIFF PCM 16-bit, 48 kHz, stereo unless stated otherwise; samples round half away from zero and are clamped.
- **MP3:** mono, 48 kHz, CBR 128 kbps (lamejs 1.2.1).
- **JSON:** 2-space indentation, a fixed key order, a trailing newline and no timestamps, so outputs are deterministic.
- **Output folders:**
  - `out/m1/`, `out/m2/`, `out/m3/`, `out/m5/`, `out/m6/`: proof captures and reports;
  - `out/bake/`: the default bake;
  - `out/demo/` and `out/demo-src/`: the demo build and generated demo sources;
  - `out/proofs/mN/`: browser screenshots;
  - `out/types/`.
- **Media records:**
  - Proofs that render audio write `media.json` next to their report: `{ proof, full, files: [{ path, bytes, sha256, ... }] }`, with repo-root-relative sorted paths and sha256 from node:crypto.
  - The m6 records also carry title, samplePeakDb, rmsDb, minWindowRmsDb, dc, nonFinite and safe.
  - `.nmc/artifacts.json` names only explicit non-audio files (index.html, media.json, report.json, report.md and similar). It never names a directory or an audio file.
- **No binary asset in git:** no .wav, .mp3, .ogg, .flac, .png, .jpg, .jpeg, .webp or .gif. WAV, MP3 and PNG stay under the git-ignored `out/`. `.gitignore` holds `/node_modules` and `/out`.
- **Secrets and commits:** no secrets. Tasks commit and never push.
- **Step discipline:**
  - No command waits more than 60 s.
  - Prove the smallest version first (one family, then all).
  - Technical questions go to the milestone lead. Only product questions (does this sound right for the game) reach the owner.
- **Ownership:**
  - Every file belongs to exactly one module (see Module ownership).
  - A milestone never edits another milestone's module, except for a minimal defect fix that its lead explicitly authorizes.
- **Append-only shared files** (owned by repo and content-core). Other milestones append exactly the items their module lists:
  - package.json `scripts` (nothing else in package.json);
  - src/index.ts and src/node.ts (export lines);
  - src/content/index.ts (array entries);
  - README.md (one section per milestone);
  - .nmc/artifacts.json (non-audio file entries).
- **Generated worklet bundle:**
  - `worklet/haa-processors.js` is generated.
  - Any change that reaches worklet code (a built-in bed, a style, a host, a DSP class they use) regenerates it with `node scripts/build-worklet.ts` in the same commit.
  - The freshness unit test fails otherwise.

## 17. Testing and proofs

- **Tests:**
  - Vitest, files `test/**/*.test.ts`. vitest.config.ts uses pool `forks` with maxWorkers = Number(process.env.NMC_CPUS) || 1.
  - The whole suite runs in at most 25 s at NMC_CPUS=1. Heavy renders belong in proofs, not tests.
  - Unit tests cover exact rules: math, formulas, protocols, validation, and determinism of small renders.
  - `test/repo/hygiene.test.ts` and `test/worklet/fresh.test.ts` run in the smoke at every milestone.
- **Proof command:** each milestone N has `scripts/proof/mN.ts`, built on scripts/proof/lib/proof.ts.
- **Targetable:** `--only <pattern>` runs only the checks and cases whose label contains the pattern (case-insensitive; `/regex/` allowed).
- **Output:**
  - Each check prints `CHECK <label>: <measured> <unit> (<op> <threshold>) PASS|FAIL`, or `CHECK <label>: <detail> PASS|FAIL` for a boolean check.
  - Aggregated checks print the worst case and its id.
  - A proof exits 1 at the first failure and ends with `ALL PASS mN`.
- **Cases:**
  - Independent cases run in child processes (`--case <id>`, exactly one `@@RESULT <json>` line on stdout), up to NMC_CPUS at a time (1 when unset).
  - Case labels (`family <id>`, `bed <id>`, `render <name>`) are what `--only` filters.
- **Size and modes:**
  - At most 5 checks per proof.
  - The default mode is quick. `--full` runs the long renders.
- **Wall budget:**
  - Default mode runs within 60 s at NMC_CPUS=1.
  - The budget is kept by design (variants cached per process, renders reused between checks), never by lowering a threshold.
  - There are no timing checks.
- **Closeout:**
  - A milestone's closeout runs its own proof plus the smoke (`npm run typecheck`, `npm test`).
  - Other milestones' proofs stay runnable on demand.
- **Check before tuning:** the check a tuning targets exists before the tuning.
- **Sound and look are review items:** WAVs, reports and screenshots go to the owner and the lead. No spectral metric decides anything.
- **Browser proofs (client GPU Chrome):**
  - The demo build includes `nmc.gpu.json` and exposes `window.nmcGpu = { version: 1, ready, ops: { render } }`.
  - scripts/gpu-run.ts publishes the uncommitted build and renders through `$NMC_GPU_CLIENT` without a shell. Tokens are never printed.
  - The text result (JSON) is committed under `proofs/mN/`. PNG captures stay in `out/proofs/mN/`, and the JSON records their sha256.
  - A committed result comes from ONE run on a freshly rebuilt demo, never picked from several runs.
  - Chrome must allow audio without a gesture (`--autoplay-policy=no-user-gesture-required` or a prior user activation). This is an operator setting, not a repo file.
  - No client, or a context that is not running, means a visible wait reported to the lead. It is never replaced by an offline result, and the `running` check is never relaxed.
  - The milestone proof (no network) checks the committed JSON against the current build hashes.

## 18. Quality bar

- **Q1 Technical safety** (proofs m1, m3, m6). Every family variant, bed render, music render and scene render has:
  - 0 NaN/Inf samples;
  - sample peak ≤ −1 dBFS;
  - |DC| < 1e-3;
  - RMS ≥ −60 dBFS (per variant, and per 1 s window for renders of 2 s or more).
- **Q2 Loudness consistency** (m1, m6):
  - each family's median one-shot loudness is within ±1 LU of its category target;
  - the 60 s scene's integrated loudness is within ±1 LU of preset.sceneLufs.
- **Q3 Determinism:**
  - identical pcmHash and byte-identical MP3s and manifest across two processes (m2);
  - a music render with the same seed has the same hash, and a different seed differs (m3);
  - the committed worklet bundle equals a fresh build (smoke, m4).
- **Q4 Isomorphic and agnostic** (hygiene test, m5):
  - the DSP-safe import rule holds;
  - no `three`, no import.meta.glob, no import from `examples/` and no absolute path;
  - no binary file is tracked.
- **Q5 Variation** (unit tests):
  - every family has 8..32 variants, and variant 0 differs from variant 1;
  - 100 plays never repeat a variant back to back;
  - micro-variation stays within rate ±3 % and gain ±1 dB.
- **Q6 Runs in a real browser** (m4, committed Chrome result): every demo action reaches max 50 ms RMS ≥ −40 dBFS, with baseline ≤ −70 dBFS, peak ≤ −1 dBFS and the context `running`.
- **Q7 Spatial correctness** (m5):
  - the known-geometry table matches within 1e-4;
  - each of N listeners contributes 1/N;
  - in Chrome, a source moving left to right gives right − left ≤ −3 dB in the first quarter and ≥ +3 dB in the last.
- **Q8 Sound character** (review by lead and owner):
  - materials read as their material;
  - UI sounds are soft and modern;
  - variants sound like the same sound performed again;
  - there are no clicks;
  - the two music styles have clearly different moods;
  - the wind sweep is smooth.
- **Q9 Example pack readability** (review of the example race soundtrack and its report):
  - the player's jumps, dives and grabs are audible;
  - an elimination reads as a cartoon fall, with the crowd's "aww" and a music dip;
  - the crowd sounds like a crowd;
  - the finish cheer feels rewarding;
  - no band is harsh.
- **Q10 Developer experience:**
  - examples/quickstart.ts typechecks;
  - the README shows real signatures, the provenance table, every command, and how a game adds its own family, bed and worklet entry.

## Module ownership

| Module | Owner milestone | Files |
|---|---|---|
| repo | M1 | package.json, package-lock.json, tsconfig.json, vitest.config.ts, .gitignore, LICENSE, README.md, src/index.ts, src/node.ts, .nmc/artifacts.json, test/repo/hygiene.test.ts |
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

## Module: repo

Owner M1. Consumes: nothing.

This module holds the repository scaffold and the two public entry points.

- **package.json:**
  - name `@etcito/haa`, version `0.1.0`, private, `"type": "module"`, MIT, engines.node `>=22.18`;
  - exports and scripts as in §1;
  - dependencies `{ "lamejs": "1.2.1" }`; devDependencies pinned exactly.
- **tsconfig.json and vitest.config.ts:** as in §1 and §17.
- **.gitignore:** `/node_modules`, `/out`.
- **LICENSE:** MIT, "Copyright (c) 2026 Etcito".
- **README.md** (Etcito brick house style):
  - What HAA is; status and contract version; install (GitHub dependency, Kong later);
  - the API sketch with the real signatures from src/contract.ts; commands; proofs (what each mN shows, where its captures go);
  - Provenance: "Extracted from the origin engine (PROVENANCE.md)", plus a changes table per file. Files tuned for m1 are marked "adapted: tuned for m1 safety/loudness";
  - what stays in each game;
  - milestone sections: Registries and bake, Music, Runtime and demo (with the custom worklet entry recipe), Spatial, example sound pack (outside the core).
- **src/index.ts** (browser-safe):
  - `export * from './contract.ts'`;
  - `dsp` and `gen` namespace barrels;
  - AudioRng, hashSeed;
  - renderVariantRaw, renderFamilyVariants, renderFamilyVariantsAsync, oneShotLufs;
  - measureStereo, measureMono, safetyVerdict, OCTAVE_BANDS and the Measure/Report types; reportMarkdown, reportHtml;
  - encodeWav, decodeWav, renderOffline, renderFamilyStrip, renderBedDemo;
  - MixChain, BASE_PRESET; BUILTIN_FAMILIES/BEDS/MIXES/STYLES;
  - the registry API and registerBuiltins; packStrip, pcmHash, alignVariants;
  - Arranger, MusicSynth, MUSIC_PARAMS, UPBEAT_STYLE, CALM_STYLE, MUSIC_BED;
  - createOfflineAudio, FLAT_SPATIAL, createAudio;
  - createSpatial, distanceGain, spatializeOne, dopplerRatio and the spatial constants;
- **src/node.ts:** writeWav, bake, encodeMp3.
- **.nmc/artifacts.json:** a JSON array of the explicit non-audio out/ files reported as card outputs (§16).
- **test/repo/hygiene.test.ts:**
  - (a) `git ls-files` lists no wav, mp3, ogg, flac, png, jpg, jpeg, webp or gif;
  - (b) no file under src/, demo/ or scripts/ contains `from 'three'`, `from "three"`, `'three/` or `import.meta.glob`;
  - (c) every DSP-safe file has only relative runtime imports that resolve to DSP-safe files, and does not match `Math\.random|Date\.|performance\.|setTimeout|setInterval|document\.|window\.|\b(?:Offline|webkit)?AudioContext\b|new\s+\w*AudioContext\b|from\s+['"]node:` (`BaseAudioContext` in type positions passes);
  - (d) no file imports an absolute path or from `examples/`.

## Module: contract

Owner M1. Consumes: nothing. DSP-safe.

- **Contents:** every constant, type and interface of §5–§11, written exactly as there:
  - HAA_CONTRACT_VERSION, AUDIO_RATE, BLOCK, MAX_LISTENERS;
  - BusId, BUSES, BUS_INDEX, REVERB_INPUT, MIX_INPUTS;
  - MaterialId, MATERIALS; VoiceCategory, VOICE_CATEGORIES, VOICE_LIMITS;
  - DEFAULT_USER_VOLUMES, FAMILY_VARIANTS_MIN/MAX, FAMILY_MAX_SECONDS, GAP_SAMPLES, ONSET_THRESHOLD;
  - PROCESSOR_NAMES, SAMPLER_VOICES, SAFETY, NORMALIZE_PEAK_CAP_DB, LOUDNESS_TOLERANCE_LU, REVERB_SEND_BASE, MICRO_VARIATION;
  - Vec3, Quat, Listener, SourcePos, OcclusionFn, SpatialResult, SpatialModel, SP, SP_LENGTH;
  - FamilyDef, FamilySpec, BedVoice, BedDef, MusicStyle, ReverbParams, CompPreset, EqPreset, BusPreset, DuckPreset, MixPreset;
  - BankManifest, BankFamilyEntry; SamplerMsg, BedMsg, MixMsg;
  - PlayOptions, Shot, BedHandle, MusicParam, MusicControl, UserVolumes, AudioOptions, AudioControls, HaaAudio, OfflineAudio, StereoRender.
- **Imports:** only `import type { AudioRng } from './dsp/rng.ts'`. `BaseAudioContext` appears in type positions only.
- **Helpers:** `spGain(l)` = SP.GAIN + l, `spPan(l)` = SP.PAN + l, `spLowpass(l)` = SP.LOWPASS + l, and `newSpatialResult()` (sp zeroed, rate 1).
- **Header:** says it is adapted from the origin's contract:
  - removed: the origin's game-specific kinds, layers and messages, and the SfxName import;
  - renamed: the buses;
  - added: the spatial block and the public API types.
- **Test:** the table values and the SP layout (SP_LENGTH = 2 + 3 × MAX_LISTENERS).

## Module: dsp

Owner M1. Consumes: contract. DSP-safe.

Extracted from the origin engine, with imports rewritten. The only changes are `.ts` imports, contract imports pointing at ../contract.ts, and the provenance header. Behaviour and signatures match the origin.

- **math.ts:** TAU, dbToGain, gainToDb (floor −200), clamp (NaN → lo), lerp, smoothstep, finiteOr, midiToHz, fastTanh, softClip.
- **rng.ts:** AudioRng (mulberry32: constructor(seed), next, range, int, gauss, fork, state); hashSeed (FNV-1a 32, the byte rules of the origin).
- **filters.ts:** BiquadType, Biquad (set, process, processBlock, magnitudeDb), Svf, OnePole, DcBlocker, Smoother, Crossover3.
- **noise.ts:** WhiteNoise, PinkNoise, BrownNoise, Crackle, VelvetNoise.
- **osc.ts:** Phasor, AdditiveBank.
- **shaper.ts:** Oversampler, Saturator.
- **env.ts:** Adsr, ExpDecay, EnvFollower.
- **modal.ts:** Mode, ModalBank.
- **delay.ts:** DelayLine, Waveguide.
- **reverb.ts:** FdnReverb.
- **dynamics.ts:**
  - Compressor.
  - TruePeakLimiter: output true peak ≤ ceiling + 0.2 dB.
  - Ducker: 8 slots, no sidechain, envelopes linear in dB. Members: trigger(depthDb, attackMs, holdMs, releaseMs), next() returning a linear gain, reset(). Overlapping triggers take the deepest.
  - TransientShaper: fast (1/30 ms) and slow (30/300 ms) peak followers. Members: set(attackDb, sustainDb) clamped to ±24 dB, process(x), reset(). Non-finite input returns 0.
- **meter.ts:** LoudnessMeter (integrated, momentaryMax, shortTermMax), TruePeakDetector, TP_OVERSAMPLE, TP_TAPS, TP_PHASE_TAPS, truePeakDb, samplePeakDb, rmsDb, windowRmsDb, maxNormXcorr, countNonFinite.
- **loop.ts:** LoopTable, makeSeamless, seamScore.
- **index.ts:** a barrel of every file above.
- **Tests:**
  - hashSeed('a', 1) is pinned, and AudioRng sequences repeat;
  - Biquad lowpass magnitudeDb(cutoff) ≈ −3 dB (±0.5) at Q 0.7071;
  - a 1 kHz sine at −20 dBFS peak in both channels for 5 s at 48 kHz measures −20 LUFS ± 0.1;
  - the limiter ceiling holds on a +6 dBFS burst;
  - clamp(NaN) = lo;
  - Ducker: idle gain 1, depth/hold/release shape, deepest wins, zero depth ignored, non-finite arguments safe, reset;
  - TransientShaper: identity at 0/0, onset boost, tail cut, NaN → 0, ±24 dB clamp.

## Module: generators

Owner M1. Consumes: contract, dsp. DSP-safe.

Extracted from the origin engine, with imports rewritten. 
- **impactModel.ts:** ImpactSize, MaterialSpec, MATERIAL_SPECS, SIZE_SCALE, ImpactOpts, and `renderImpact(o, sr, rng, out): number`, which adds into out and returns the end index.
- **uiKit.ts:** ClickMaterial, TickMaterial, SoftClickOpts, ChimeOpts, SwellOpts, TickOpts, and softClick, tick, chime, swell (each `(out, o, sr, rng): number`).
- **fxKit.ts:** pitchEnvelope, noiseSweep, harmonicStack, subTransient, resonantPing, modalHit, shapeBuffer, normalizePeak.
- **windModel.ts:** WindBand, WIND_BANDS, and `class WindModel implements BedVoice` (constructor(sr, seed); params speed, speed01, air, boost, slip, yaw, underwater, level).
- **reverbZones.ts:**
  - `type ReverbZoneId = 'open' | 'canyon' | 'snowfield' | 'cavern' | 'water' | 'city' | 'sky' | 'forest' | 'room' | 'underwater'`;
  - `REVERB_ZONES: Record<ReverbZoneId, ReverbParams>`, with the origin's biome values mapped meadow→open, desert→canyon, snow→snowfield, volcano→cavern, reef→water, neon→city, sky→sky, jungle→forest, test→room, underwater→underwater;
  - `ABSORPTION_SCALE` (snowfield 0.6, forest 0.7, sky 1.4, others 1);
  - `blendReverb(a, b, t, out)`.
- **src/music/instruments.ts:** Instrument, PadInstrument, BassInstrument, LeadInstrument (with the LeadVoice per-sample carrier renormalization), PluckInstrument, DRUM_NOTES, KickInstrument, SnareInstrument, HatInstrument, PercInstrument.
- **gen/index.ts:** a barrel of the gen files plus `export * from '../music/instruments.ts'`.
- **Test:** each generator renders 0.5 s with a fixed seed: finite, non-silent, and bit-identical on a second run.

## Module: mix

Owner M1. Consumes: contract, dsp. DSP-safe.

- **mixChain.ts** (adapted from the origin): `class MixChain(sr, preset)`.
  - `setPreset(p)`.
  - `setUser(master, music, sfx)`: clamped 0..1, smoothed 50 ms.
  - `duck(kind, strength)`: an unknown kind is ignored. It triggers preset.ducks[kind] × strength on the music and ambience buses; the deepest active duck wins.
  - `setReverb(p)`.
  - `setMuffle(v)`: v in 0..1 sets an Svf cutoff of 20000 × (700/20000)^v, smoothed 150 ms.
  - `process(inputs, outL, outR, n)`: takes MIX_INPUTS entries; a missing input is silence.
  - `meter: { truePeakDb; reductionDb }`; `reset()`.
- **Per-block chain:**
  1. Each bus: × level × user gain (music for `music`, sfx for the others) → EQ (lowshelf, peaking, highshelf) → Compressor → Ducker (music and ambience only) → sum.
  2. Reverb input → FdnReverb → × returnDb → sum.
  3. Master: × trim × user master → muffle → glue → TruePeakLimiter.
  4. The non-finite guard.
- **presets/base.ts:** `BASE_PRESET` `{ id: 'base', priority: 0 }`:
  - levelDb: world −8, foley −11, effects −6, ambience −15, music −12, ui −9;
  - EQ neutral except world (lowShelf 120 Hz +1.5 dB, highShelf 8 kHz −2 dB) and music (peak 2.5 kHz −2 dB, Q 0.8);
  - compressors: threshold −18, ratio 2.5, knee 6, attack 10 (3 for effects and ui), release 150, makeup 0;
  - master: trim 0; glue { −12, 1.8, 8, 20, 200, 0 }; limiter { ceilingDbtp −1.5, lookaheadMs 5, releaseMs 80 };
  - ducks: impact { −5, −4, 10/300/600 }, fx { −4, −3, 20/250/500 }, cue { −6, −4, 40/1200/800 }, sting { −8, −6, 60/3000/1500 };
  - loudness { impact −21, fx −20, ui −24, ambience −26, sting −18 }; sceneLufs −16;
  - empty trims;
  - reverb { size 0.5, decay 1.2, damping 6000, predelay 0.02, returnDb −14 }.
- **Test:**
  - +6 dBFS on every bus stays ≤ −1.3 dBTP;
  - duck('impact') lowers the music bus;
  - a NaN input zeroes the block.

## Module: content-core

Owner M1. Consumes: contract, dsp, generators, mix. DSP-safe.

The built-in content that works with any game, extracted from the origin's families and beds (impacts, ui, stings, wind).

- **Adaptations:**
  - the `sfx` alias fields and the SfxName import are removed;
  - `render` writes into the given zeroed `out` and returns the frames written;
  - the seeds stay the legacy seeds.
- **impacts.ts:** `IMPACT_FAMILIES`: 12 families `impact.<material>.<size>` (MATERIALS × small/medium/large). Bus `effects`, category `impact`, 10 variants, the origin MAX_SECONDS table, seed hashSeed('impact', material, size).
- **ui.ts:** `UI_FAMILIES`: bus `ui`, category `ui`, 8 variants, the origin recipes and seeds:
  - ui.move 0.14, ui.select 0.25, ui.back 0.20 s;
  - cue.countdown 0.30, cue.go 0.45, cue.lap 0.45, cue.placeUp 0.21, cue.placeDown 0.21, cue.itemGet 0.20, cue.roulette 0.08 s.
- **stings.ts:** `STING_FAMILIES`: bus `music`, category `sting`, 8 variants. Seeds keep the legacy id strings:
  - `sting.climax` (legacy id sting.finalLap, 1.2 s, seed hashSeed('sting', 'sting.finalLap'));
  - `sting.win` (legacy id sting.finishWin, 1.6 s);
  - `sting.end` (legacy id sting.finish, 0.95 s).
- **wind.ts:**
  - `WIND_FAMILIES`: [`wind.passby`, bus `effects`, category `fx`, 12 variants].
  - `WIND_BED` = `{ id: 'wind', bus: 'ambience', params: ['speed','speed01','air','boost','slip','yaw','underwater','level'], defaults: [0,0,0,0,0,0,0,1], positional: false }`.
  - create returns a wrapper around `new WindModel(sr, seed)`:
    - process copies the params into a scratch Float32Array preallocated at create (the caller's p is never mutated);
    - it multiplies `level` by a fixed dB trim times a smooth reduction from 30 m/s to 50 m/s and beyond;
    - it then calls WindModel.process. reset is forwarded.
- **index.ts** (append-only):
  - `BUILTIN_FAMILIES`: impact, ui, sting and wind families, sorted by id;
  - `BUILTIN_BEDS`: WIND_BED, MUSIC_BED, BEACON_BED;
  - `BUILTIN_MIXES`: BASE_PRESET;
  - `BUILTIN_STYLES`: UPBEAT_STYLE, CALM_STYLE.
- **Tuning rules:**
  - Families that fail m1 are fixed in their recipe only. Never change the loudness targets, SAFETY, NORMALIZE_PEAK_CAP_DB, LOUDNESS_TOLERANCE_LU, src/bank/render.ts, src/analysis or src/gen/windModel.ts.
  - Ids, seeds, variant counts and maxSeconds stay unchanged.
  - DC is removed at the source, or by passing the whole written length through DcBlocker before the recipe's final normalization or fade. Worst |DC| per normalized variant is < 5e-4.
  - Loudness misses (the gain hitting the −1.05 dBFS cap) are fixed by reducing the crest factor inside the recipe and keeping outlier peaks near the median.
  - The wind bed's m1 sweep keeps every 1 s window peak ≤ −2 dBFS and RMS ≥ −60 dBFS.
  - Every tuning carries a `// HAA tuning: ...` comment.
- **Test:**
  - ids are unique;
  - variants are within 8..32 and maxSeconds within 0..FAMILY_MAX_SECONDS;
  - variant 0 differs from variant 1 for every family.

## Module: bank-render

Owner M1. Consumes: contract, dsp. DSP-safe.

Adapted from the origin engine. 
- **`renderVariantRaw(def, v, sr): Float32Array`:**
  1. Allocate out of ceil(maxSeconds × sr).
  2. n = clamp(def.render(out, sr, new AudioRng(hashSeed(def.seed, def.id, v)), v), 0, out.length).
  3. Zero non-finite samples.
  4. Trim trailing samples below −70 dBFS.
  5. Apply a 5 ms cosine fade-out.
  6. Return the trimmed copy.
- **`oneShotLufs(x, n, sr)`:** follows §7.
- **`familyGain(raw, target, sr): { gain; medianLufs }`:** follows §7.
- **`renderFamilyVariants(def, sr, preset): Float32Array[]`:** the raw variants × familyGain with target preset.loudness[def.category]. Bit-deterministic.
- **`renderFamilyVariantsAsync(def, sr, preset, yieldEveryMs, now, yieldFn)`:** the same output, sliced cooperatively (one variant per slice check).
- **Tests:**
  - determinism;
  - a synthetic family lands within 0.05 LU of its target;
  - a peaky family is capped at NORMALIZE_PEAK_CAP_DB;
  - a render returning more than out.length is clamped.

## Module: analysis

Owner M1. Consumes: contract, dsp. DSP-safe.

- **measure.ts:**
  - `OCTAVE_BANDS`.
  - `measureStereo(l, r, sr): Measure`.
  - `measureMono(x, sr): Measure`: lufs is measured on (x√½, x√½); peaks, dc and rms on x.
  - `safetyVerdict(m): { ok; failures }`: checks nonFinite == 0, samplePeakDb ≤ −1, dc < 1e-3 and minWindowRmsDb ≥ −60, naming each failure. Types Measure, ReportEntry, ReportCheck and Report (§12).
- **report.ts:**
  - `reportMarkdown(r)`: one table per group (id, seconds, LUFS, target, sample peak, true peak, DC, min window RMS, the 10 band levels), then the CHECK table.
  - `reportHtml(r)`: for each entry, its title, an `<audio controls preload='none' src='<relative file>'>` and its measures, then the CHECK table. Plain HTML with inline CSS and no script.
- **Tests:**
  - DC 0.01 is detected;
  - silence fails;
  - a −20 dBFS 1 kHz sine peaks in the 1000 Hz band.

## Module: offline

Owner M1. Consumes: contract, dsp, bank-render.

- **wav.ts** (DSP-safe, from the origin):
  - `encodeWav(left, right | null, sampleRate, length?): Uint8Array`: PCM 16-bit, mono when right is null.
  - `decodeWav(bytes): { sampleRate; channels }`: PCM 16-bit only; anything else throws `[haa] bad wav`.
- **render.ts** (DSP-safe):
  - `renderOffline(fn(l, r, n, frame), seconds, sampleRate = AUDIO_RATE, name = 'render'): StereoRender`: calls fn per BLOCK with zeroed buffers.
  - `renderFamilyStrip(def, preset, o?: { gapSec?; sampleRate? })`: the normalized variants separated by gapSec (default 0.25), centered (× √½ on both channels).
  - `renderBedDemo(def, o: { seconds; seed; script(t, p); sampleRate?; name? })`:
    - one BedVoice; p starts as a copy of def.defaults;
    - script runs every BLOCK before process;
    - returns the raw bed output with no mix.
- **src/node/writeWav.ts** (node): `writeWav(path, render)` creates the parent directories and writes encodeWav.
- **Tests:**
  - WAV round trip within 1/32767;
  - header fields;
  - renderOffline length = round(seconds × sr).

## Module: proof-m1

Owner M1. Consumes: contract, dsp, generators, mix, content-core, bank-render, analysis, offline. Node.

- **scripts/proof/lib/proof.ts:**
  - `ProofArgs { only; full; caseId }`, `parseArgs(argv)`, `selected(label, a)`.
  - `ProofCase<R> { id; label; run(a) }`.
  - `runCases(script, cases, a)`: a child runs one case and prints `@@RESULT <json>`. The parent spawns `process.execPath <script> --case <id> [--full]` up to NMC_CPUS at a time, skipping unselected labels.
  - `check(label, measured, op, threshold, unit, worst?)` and `checkBool(label, ok, detail)`: print and record the line. A FAIL throws ProofFail, and the runner exits 1.
  - `proofMain(name, body)`: prints `ALL PASS <name>` and `wall <s> s`.
- **scripts/proof/lib/out.ts:** `writeReport(dir, r)` (the report triplet) and `sha256File(path)`.
- **scripts/proof/m1.ts:**
  - **Cases:**
    - `family <id>`, one per BUILTIN_FAMILIES entry: render the normalized variants with BASE_PRESET, measure each with measureMono, and write out/m1/families/<id>.wav.
    - `bed wind`: a speed sweep from 0 to 50 m/s (speed01 = speed/50) over 8 s (15 s with --full) to out/m1/beds/wind.wav, checked in 1 s windows.
  - **Checks:**
    1. `render: WAVs written and readable`;
    2. `safety: no NaN/Inf and |DC| < 1e-3`;
    3. `safety: sample peak <= -1 dBFS`;
    4. `safety: no silent variant (RMS >= -60 dBFS)`;
    5. `loudness: family median within ±1 LU of category target`.
  - **Media record:** it deletes a stale out/m1/media.json at start. After writeReport, and only when every check passed with no `--only`, it writes out/m1/media.json covering every WAV plus the report triplet, and prints `media out/m1/media.json (<N> files)`.
  - **Acceptance:** `node scripts/proof/m1.ts --full` passes all 5 checks. The script and thresholds are frozen. Failing cases go back to their recipe in content-core.

## Module: registry

Owner M2. Consumes: contract, dsp, content-core. DSP-safe.

Replaces the five origin `import.meta.glob` registries with explicit calls.

- **Define functions:** each returns a frozen definition or throws `[haa] bad <kind> <id>: <reason>`.
  - `defineFamily(spec)`:
    - the id follows the naming convention;
    - variants is an integer in 8..32;
    - bus is in BUSES and category in VOICE_CATEGORIES;
    - maxSeconds is finite in (0, FAMILY_MAX_SECONDS];
    - render is a function;
    - seed defaults to hashSeed('family', id).
  - `defineBed(spec)`:
    - lowercase dashed id;
    - params non-empty and unique; defaults finite with defaults.length == params.length;
    - unsmoothed ⊆ params; create is a function.
  - `defineMix(p)`:
    - every BusId is present;
    - limiter.ceilingDbtp ≤ −1.2;
    - loudness covers every category;
    - every number is finite.
  - `defineStyle(s)`:
    - lowercase dashed id;
    - scale non-empty, strictly ascending in 0..11, starting at 0;
    - bpm 40..240 and swing 0..0.3;
    - progressions of length 4 or 8 with indices < scale.length, at least one starting on 0.
- **Register functions:** `registerFamily`, `registerFamilies`, `registerBed`, `registerMix`, `registerStyle`. Each validates through define*, and a duplicate throws.
- **Lookups:**
  - `getFamily`, `listFamilies` (sorted by id);
  - `getBed`, `listBeds`; `getMix`, `listMixes`;
  - `activeMix()` (highest priority, then lowest id; throws `[haa] no mix preset` when empty);
  - `getStyle`, `listStyles()` (sorted by id; the index is the music `style` code), `styleIndex(id)` (−1 when absent).
- **Helpers:** `familyTrimDb(p, id)` and `bedTrimDb(p, id)` (0 when absent), `bankSeconds(defs?)` (Σ variants × maxSeconds), `clearRegistry()`.
- **builtins.ts:** `registerBuiltins()`, idempotent.
- **Tests:**
  - each validation error;
  - a duplicate throws;
  - ordering;
  - activeMix;
  - registerBuiltins is idempotent.

## Module: bank-pack

Owner M2. Consumes: contract. DSP-safe.

Extracted from the origin engine, with imports rewritten. 
- `packStrip(variants): { strip; entries; onset0 }`.
- `pcmHash(strip): string`.
- `alignVariants(decoded, entry, rate): Float32Array[]` (§11).
- **Tests:**
  - the strip layout with GAP_SAMPLES;
  - a stable hash;
  - 44.1 kHz alignment recovers the onsets within 1 sample.

## Module: bake

Owner M2. Consumes: contract, registry, bank-render, bank-pack, content-core. Node only.

- **mp3.ts:** `encodeMp3(pcm, 48000, 128): Uint8Array`.
  - **Encoding:** 1152-sample Int16 chunks (round half away from zero, clamped to ±32767), then flush, then concatenate.
  - **Why a shim:** lamejs 1.2.1 uses the globals MPEGMode, Lame and BitStream without requiring them.
  - **Shim:** a lazy loader runs once, on the first encoder creation (never at import). Through createRequire, it sets each of these globals that is still undefined to `require('lamejs/src/js/<name>.js')`. It then takes Mp3Encoder from `require('lamejs')` and creates `new Mp3Encoder(1, rate, kbps)`.
  - **Limits:** the globals stay set. lamejs is never patched, vendored or replaced. If encoding still fails, the task stops and asks the lead.
- **bake.ts:** `bake(families, { outDir; rate?; kbps?; preset?; log? }): BankManifest`.
  - Only 48000 Hz and 128 kbps are supported; any other rate or kbps throws a `[haa] bake supports 48000 Hz / 128 kbps ...` error.
  - preset defaults to activeMix().
  - Per family, sorted by id: renderFamilyVariants → packStrip → encodeMp3 → `<outDir>/<id>.mp3`.
  - It writes a deterministic manifest.json and logs one line per family plus a total.
- **scripts/bake.ts:** `node scripts/bake.ts [--out <dir>] [--families <prefixes>]`.
  - Default out is out/bake, with all builtins.
  - It prints the elapsed time and fails with `[bake] <reason>`.
- **Tests:**
  - a 0.5 s family bakes to a first frame of MPEG-1 Layer III, 48 kHz, 128 kbps, mono;
  - two bakes are byte-identical.

## Module: proof-m2

Owner M2. Consumes: proof-m1, registry, bank-pack, bake.

- **Cases:** `bake run1` and `bake run2` bake the impact., ui., cue. and sting. families into out/m2/run1 and out/m2/run2, in separate processes.
- **Checks:**
  1. `registry: no import.meta.glob and builtins registered`: the src scan finds 0 matches, the counts equal the BUILTIN_* lengths, and a duplicate throws.
  2. `bake: manifest valid`:
     - the schema matches and every file exists;
     - every MP3's first frame is 48 kHz/128 kbps/mono Layer III;
     - variant counts match the definitions;
     - the expected families are present (12 impact, 10 ui/cue, 3 sting).
  3. `bake: deterministic bytes`: 0 sha256 differences between the runs (MP3s and manifest).
  4. `bake: pcm hash matches a fresh render`: 0 mismatches.
- **Report:** out/m2/report.json and report.md, with per-file bytes and sha256 of both runs, pcmHash, variants and seconds.

## Module: music

Owner M3. Consumes: contract, dsp, generators, registry. DSP-safe.

Extracted from the origin engine, with imports rewritten. The algorithms are unchanged: Markov progressions, motif variation, intensity layers, bar-quantized sections with fills, a climax at tempo × 1.06 and +2 semitones, and resolves.

- **arranger.ts:**
  - **`Section`:** `'intro' | 'mainA' | 'mainB' | 'intense' | 'climax' | 'resolveWin' | 'resolve' | 'post'`.
  - **`MusicState`:** `{ style; styleB; styleMix; intensity; phase; climax; resolved; won; sparkle; tension }`. phase is 0 intro, 1 main, 2 resolved.
  - **`NoteEvent`:** `{ inst: 'pad'|'bass'|'lead'|'pluck'|'kick'|'snare'|'hat'|'perc'; midi; vel; step; len }`.
  - **`class Arranger(seed, styles)`:**
    - throws `[haa] no music style` when the list is empty;
    - members: `setState(s)`, `nextStep(out): number`, `section`, `bpm`, `styleIndex`;
    - pitched notes stay in the active scale.
- **musicSynth.ts:**
  - `MUSIC_PARAMS = ['style','styleB','styleMix','intensity','phase','climax','resolved','won','sparkle','tension','fadeBars','level'] as const`.
  - `class MusicSynth implements BedVoice (sr, seed, styles = listStyles())`:
    - a style change crossfades the palette over fadeBars bars (1..16);
    - sparkle adds a bright arpeggiated layer;
    - a DcBlocker per channel (dcL/dcR, reset in resetVoices()) filters the compressed stereo before the level-scaled sum.
- **styles.ts:** both styles are genre-neutral; the 8-long progression is the climax one.
  - `UPBEAT_STYLE`: { id 'upbeat', root 60, scale MAJOR, bpm 128, palette { pad 0.45, bass 0.5, lead 0.6, pluck 0.55, drums 0.55, perc 0.4 }, swing 0.06, progressions [[0,4,5,3],[0,5,3,4],[3,4,0,5],[5,3,0,4],[0,4,5,3,0,4,3,4]] }.
  - `CALM_STYLE`: { id 'calm', root 57, scale DORIAN [0,2,3,5,7,9,10], bpm 96, palette { pad 0.75, bass 0.4, lead 0.45, pluck 0.7, drums 0.3, perc 0.3 }, swing 0.12, progressions [[0,3,4,0],[0,6,3,4],[3,6,0,4],[0,3,6,4,0,3,4,0]] }.
- **musicBed.ts:** `MUSIC_BED` = `{ id: 'music', bus: 'music', params: MUSIC_PARAMS, defaults: [0,0,0,0.5,1,0,0,0,0,0,2,1], unsmoothed: ['style','styleB','phase','climax','resolved','won','fadeBars'], positional: false, create: (sr, seed) => new MusicSynth(sr, seed) }`.
- **Tests:**
  - notes stay in scale over 64 bars per style;
  - the same seed gives the same NoteEvent stream;
  - section switches land on bar lines.

## Module: proof-m3

Owner M3. Consumes: proof-m1, music, mix, content-core, analysis, offline, bank-pack.

Renders through MixChain(BASE_PRESET) on the music bus at DEFAULT_USER_VOLUMES, writing out/m3/*.wav, the report triplet and media.json.

- **Cases:**
  - `render upbeat` and `render calm`: 20 s at intensity 0.6, phase 1 (60 s with --full);
  - `render arc-upbeat`: intro 4 s → main 12 s (intensity 0.3→0.9) → climax 8 s → resolveWin 6 s → post 4 s;
  - `render crossfade`: upbeat for 8 s, then a crossfade to calm over 2 bars, 20 s total;
  - `render stings`: the STING_FAMILIES strips.
- **Checks:**
  1. `safety: music renders`: the Q1 criteria, worst case printed.
  2. `music: notes in scale`: 0 out-of-scale notes over 64 bars per style.
  3. `music: deterministic`: same seed gives the same pcmHash, a different seed differs.
  4. `music: crossfade lands`: ≤ 3 bars from the request until Arranger.styleIndex == calm.

## Module: hosts

Owner M4. Consumes: contract, dsp. DSP-safe.

- **sampler.ts** (adapted from the origin with a per-listener spatial block):
  - `SamplerPlay { voice; id; variant; bus; gain; rate; sp }`.
  - `class Sampler(sr)` with SAMPLER_VOICES slots. Members:
    - `load(id, variants, rate)`, `has(id)`, `variantCount(id)`;
    - `play(c)`, `update(voice, rate, sp)`;
    - `stop(voice, fade)`, `stopAll(fade)`;
    - `process(out, n)`, which OVERWRITES the MIX_INPUTS pairs following §9;
    - `playing`, `voiceDone(voice)`.
- **bedHost.ts:** `class BedHost(sr, def, seed, trimDb = 0)`. Members:
  - `setParams(v)`: smoothed per §7.
  - `setSpatial(rate, sp)`.
  - `process(bus, send, n)`, which OVERWRITES:
    - non-positional: voice stereo × dB(trimDb) goes to bus, and send = bus × sp[SEND] (FLAT one-listener defaults);
    - positional: mono (L+R)/2 gets per-listener lowpass, pan and gain as in the sampler, and rate is written into `doppler` when that param exists.
  - `end(fadeSec)`, `done`, `reset()`.
  - The non-finite guard resets the voice.
- **Tests:**
  - one listener at pan 0 outputs the variant × √½ on both channels (±1e-6);
  - 2 listeners at g 0.5 with pans ±1 split left and right;
  - a stop fade reaches 0;
  - a positional BedHost pans.

## Module: conductor

Owner M4. Consumes: contract, dsp, registry. DSP-safe.

Written for HAA; nothing is ported from the origin director.

- **voices.ts:**
  - `class VoiceAllocator(limits)`:
    - `request(cat, score, now, duration): { id; stolen } | null`, with ids 0..SAMPLER_VOICES−1;
    - when the category is full, a higher score steals the lowest-score voice of that category; otherwise the request returns null (dropped). stolen is −1 when nothing was stolen;
    - `expire(now)`, `active(cat)`, `dropped`, `stolenCount`.
  - `class VariationPicker(seed)`:
    - `pick(id, count)`: uniform, excluding the last pick when count > 1;
    - `jitter(out)`: rate = 1 + u(−0.03, 0.03), gainDb = u(−1, 1).
- **flatSpatial.ts:** `FLAT_SPATIAL` (§6).
- **conductor.ts:** the brain that does not depend on the realm.
  - `ConductorBackend { sampler(m); mix(m); startBed(handle, def, seed, trimDb, values): boolean; bed(handle, m) }`.
  - `class Conductor implements AudioControls({ preset, maxListeners, seed, spatial, backend, now, loaded, variantLength, loadRate })`.
  - **play(id, o):**
    1. Unknown id: warn once and return null.
    2. Not loaded: return null and count it as dropped.
    3. variant = o.variant ?? pick.
    4. Apply jitter: rate = 2^(pitch/12) × jitter × Doppler; gain = (o.gain ?? 1) × dB(familyTrimDb + jitterDb).
    5. sp = (spatial ?? FLAT_SPATIAL).evaluate(...). With o.listener set, only that listener keeps its gain.
    6. Allocate a voice by category with score = gain × max listener gain and duration = variantLength / (loadRate × rate). A steal posts `stop` with a 0.03 s fade.
    7. Post `play` and return a Shot. Shot.setPos posts `upd`; Shot.stop posts `stop`.
  - **Other members:**
    - `setListeners(ls)`: caps at maxListeners, re-evaluates live positional shots and beds, and drops expired shots.
    - `startBed(id, o)`: defaults overridden by o.values. A positional bed posts `sp`. BedHandle.set and setParam post `p` with the full array; stop posts `end`.
    - `duck`, `setVolumes`, `setReverb`, `setMuffle`: post a MixMsg.
    - `music`: drives one `music` bed (§8).
    - `stats: { played; dropped; stolen }`.
    - No allocation per play beyond the posted messages.
- **Tests:**
  - allocator limit and steal;
  - no immediate repeat in 100 picks;
  - jitter bounds;
  - 2 listeners with no model get 0.5 each;
  - a listener-local play gives 0.5/0;
  - duck posts the right message.

## Module: offline-scene

Owner M4. Consumes: contract, hosts, conductor, mix, bank-render, registry, offline. DSP-safe.

- **`createOfflineAudio(o?: { sampleRate?; mix?; maxListeners?; spatial?; seed?; volumes? }): OfflineAudio`:**
  - calls registerBuiltins();
  - mix defaults to activeMix(); a string goes through getMix, and an unknown id throws `[haa] unknown mix <id>`;
  - families load lazily and synchronously on first play through renderFamilyVariants, cached per process by id + sr + preset id;
  - a direct backend drives Sampler, BedHost and MixChain;
  - `advance(seconds)` renders BLOCKs into growable, reused buffers;
  - `take(name)` returns the render so far; `time` is in seconds;
  - volumes default to DEFAULT_USER_VOLUMES.
- **Test:**
  - play('impact.wood.small') then advance(1) is non-silent and ≤ −1 dBFS;
  - the same script twice gives an identical pcmHash.

## Module: worklet

Owner M4. Consumes: contract, hosts, mix, registry, music. AudioWorkletGlobalScope.

- **processors.ts:** `registerHaaProcessors()` registers the 3 PROCESSOR_NAMES, guarded against double registration.
  - `haa-mix` wraps MixChain and handles MixMsg.
  - `haa-sampler` wraps Sampler and handles SamplerMsg.
  - `haa-bed` wraps BedHost(getBed(bed), seed, trimDb) with the initial values. It handles BedMsg and returns false after end plus its fade. An unknown bed is silent and returns false.
  - No allocation in process(). Every exception is caught and gives silence.
- **entry.ts:** `registerBuiltins(); registerHaaProcessors()`. Games with custom beds or styles copy this file, add their register calls, and bundle it with esbuild.
- **globals.d.ts:** declares AudioWorkletProcessor, registerProcessor, sampleRate and currentTime.
- **scripts/build-worklet.ts:** `node scripts/build-worklet.ts [--check]`.
  - esbuild options: bundle, esm, es2022, unminified, legalComments none, charset utf8.
  - Banner: `// @etcito/haa prebuilt AudioWorklet processors. Generated by scripts/build-worklet.ts from src/worklet/entry.ts. Do not edit.`
  - Output: worklet/haa-processors.js. `--check` exits 1 when the committed file is stale.
  - Exports `buildWorkletText()`.
- **test/worklet/fresh.test.ts:** buildWorkletText() equals the committed bytes.

## Module: runtime

Owner M4. Consumes: contract, conductor, registry, bank-render, bank-pack, worklet. Browser.

- **bankLoader.ts:** `class BankLoader(ctx, manifestUrl, preset, sink)`.
  - `ensure(ids?)` is idempotent and never rejects:
    - it fetches the manifest once (resolved against document.baseURI) and each `new URL(entry.file, manifestAbsUrl)`;
    - it decodes, aligns with alignVariants, and calls sink with the buffers transferred.
  - **Fallback:** on a missing manifest or family, a fetch or decode error, or a registered family absent from the manifest, it renders with `renderFamilyVariantsAsync(def, ctx.sampleRate, preset, 8, () => performance.now(), () => new Promise(r => setTimeout(r, 0)))`. It warns once per cause.
  - `loaded(id)`, `variantLength(id, v)`.
- **createAudio.ts:** `createAudio(ctx, o): Promise<HaaAudio>`:
  1. registerBuiltins(), then `addModule(o.processorUrl)`.
  2. Wire mix → splitter → analysers (when ctx is an AudioContext; meters are null otherwise) → destination, and sampler → mix inputs 0..6.
  3. Post the preset and the user volumes.
  4. Build a Conductor whose backend posts to the ports and creates `haa-bed` nodes per §9.
  5. Start BankLoader.ensure() without awaiting; ready() returns that promise.
  - It rejects with `[haa] worklet unavailable: <reason>`.
  - `close()` stops everything (stopAll 0.05, bed end 0.05), disconnects after 100 ms, and resolves.
- **examples/quickstart.ts:** typechecked usage of createAudio, play, startBed and music (not executed).

## Module: demo

Owner M4. Consumes: contract, runtime, registry, bake, worklet. Plain DOM bundled with esbuild.

- **demo/panel.ts:**
  - `DemoAction { id; label; seconds; run(audio): (() => void) | void }`;
  - `DemoPanel { id; title; order; actions(); mount(root, audio) }`.
- **demo/panels/core.ts:** `PANEL { id: 'core', order: 0 }`.
  - Actions:
    - `family:<id>` for every built-in family (seconds = maxSeconds + 0.3);
    - `bed:wind` (3 s; speed 0→40 m/s through setParam every 50 ms);
    - `music:upbeat` and `music:calm` (4 s each).
  - Controls: buttons, a wind slider, a style select with a crossfade button, and a volume slider.
- **demo/main.ts:**
  - the Start button creates `new AudioContext()` and calls `createAudio(ctx, { manifestUrl: './audio/manifest.json', processorUrl: './haa-processors.js' })`;
  - mounts the panels from the generated `out/demo-src/panels.gen.ts` (sorted by order, then id);
  - `drawMeter()` paints one opaque frame (`#111827` background, then the Mix/L/R bars and the status line). The rAF loop calls it;
  - calls installGpuOps with drawMeter.
- **demo/gpuOps.ts:** `installGpuOps(get)` sets `window.nmcGpu = { version: 1, ready, ops: { render } }` with the steps of §13. `GpuOpsHost` provides audio, start, actions, canvas and drawMeter. ready resolves once the panels are mounted.
- **demo/index.html:** a title, the Start button, `<canvas id='meter' width='960' height='240'>`, `<div id='panels'>`, `<script type='module' src='./main.js'>`.
- **scripts/build-demo.ts:** `node scripts/build-demo.ts [--out out/demo]`, exported as `buildDemo(out): Promise<BuildInfo>`. It runs in under 60 s.
  1. registerBuiltins, then bake every family into `<out>/audio`.
  2. Copy the worklet bundle, index.html and nmc.gpu.json.
  3. Generate panels.gen.ts from demo/panels/*.ts, sorted by file name.
  4. Bundle main.js with esbuild (esm, es2022, unminified, no sourcemap).
  5. Write build.json.
- **scripts/serve.ts:** `node scripts/serve.ts [--dir out/demo] [--port 8787]`, exported as `startServer(dir, port)` (port 0 = ephemeral).
  - Binds 127.0.0.1.
  - MIME types: .html text/html, .js text/javascript, .json application/json, .mp3 audio/mpeg, .wav audio/wav, .png image/png.
  - Returns 404 for missing paths and 403 for paths that escape the directory.
- **scripts/gpu-run.ts:** `--input <json> --out <result.json> --captures <dir>`.
  - It requires NMC_GPU_CLIENT and runs `publish out/demo/`, then `render <input> --page-ref <REF> -o <captures>`, through execFile.
  - It builds a LevelsResult or TraceResult, taking userAgent from the start step, plus build.json and the capture sha256 values.
  - Exit codes: 2 with `[gpu] no client: waiting`; 3 with `[gpu] AudioContext not running: <state>`.

## Module: proof-m4

Owner M4. Consumes: proof-m1, demo, worklet, offline-scene, runtime.

- **proofs/m4/input.json:** [start, levels (all core actions), snapshot].
- **proofs/m4/levels.json:** the LevelsResult of one gpu-run on a fresh demo build. The screenshot stays in out/proofs/m4/.
- **Checks:**
  1. `worklet: committed bundle is fresh`.
  2. `demo: builds`: in a child; index.html, main.js, haa-processors.js, nmc.gpu.json, build.json and audio/manifest.json exist; the manifest has listFamilies().length families.
  3. `offline: every demo action sounds`: through createOfflineAudio, the worst max 50 ms RMS is ≥ −40 dBFS and the worst peak ≤ −1 dBFS.
  4. `server: serves the demo with correct types`: /, /main.js, /haa-processors.js, /audio/manifest.json and the first MP3 return 200 with the right MIME type; /../package.json is refused.
  5. `browser: committed Chrome levels`:
     - the build equals the fresh build.json; ctxState is `running`; userAgent is non-empty;
     - the action ids equal the core actions;
     - every maxRmsDb ≥ −40, baselineDb ≤ −70 and peakDb ≤ −1.

## Module: spatial

Owner M5. Consumes: contract. DSP-safe, plain vectors.

- **spatial.ts:**
  - the §6 constants;
  - `distanceGain(d, ref = REF_DISTANCE, max = DEFAULT_RANGE)`;
  - `rotate(q, v, out)`, `listenerAxes(q, right, up, fwd)`;
  - `OneListener { gain; pan; lowpass; send; doppler; dist }`;
  - `spatializeOne(l, src, count, occlusion, env: { absorptionScale }, out)`: the §6 formulas without the 1/N factor;
  - `dopplerRatio(l, src)`.
- **model.ts:** `createSpatial(o?: { absorptionScale? }): SpatialModel`.
  - It applies spatializeOne per listener, the 1/N factor, the dominant-listener rule and the null-source rule.
  - It allocates nothing after creation.
- **Tests:**
  - the full known-geometry table (±1e-4);
  - a quaternion yaw of 90° moves a front source to pan ±1 by the right-hand rule;
  - 1..4 co-located listeners with a null source sum to 1.

## Module: spatial-demo

Owner M5. Consumes: contract, dsp, spatial, demo, registry.

- **src/content/beacon.ts** (DSP-safe): `class BeaconModel implements BedVoice`.
  - A glass-like ModalBank ping (modes near 880, 2215 and 3960 Hz with seeded jitter), excited `rate` times per second by noise bursts.
  - A soft Svf band-passed pink-noise hum at 300 Hz underneath.
  - Mode frequencies × doppler. No raw oscillator.
  - `BEACON_BED` = `{ id: 'beacon', bus: 'effects', params: ['level','rate','doppler'], defaults: [1,4,1], positional: true }`.
- **demo/panels/spatial.ts:** `PANEL { id: 'spatial', order: 10 }`. On mount it calls `audio.setSpatial(createSpatial())`.
  - `spatial:sweep` (4 s): a beacon from x = −20 to +20 at z = −5, vel (10,0,0), updated every 20 ms; the listener is at the origin with the identity quat.
  - `spatial:flyby` (4 s): along z from −60 to +60 at x = 3.
  - A top-down view shows the source and the listener.
- **Test:** the beacon is finite and non-silent; doppler 2 raises the spectral centroid.

## Module: proof-m5

Owner M5. Consumes: proof-m1, spatial, spatial-demo, offline-scene, demo.

- **proofs/m5/input.json:** [start, trace { action: 'spatial:sweep', everyMs: 50 }, snapshot].
- **proofs/m5/trace.json:** the TraceResult from client Chrome. The capture stays in out/proofs/m5/.
- **Review captures:** out/m5/sweep.wav and flyby.wav, rendered through createOfflineAudio + createSpatial, plus the report and media.json.
- **Checks:**
  1. `geometry: known attenuation table`: max error ≤ 1e-4.
  2. `listeners: each of N contributes 1/N`: for N = 1..4, the error of the summed null-source gains vs 1, and of an equidistant positional gain vs distanceGain/N, is ≤ 1e-6.
  3. `doppler: approach and recede ratios`: error vs 1.0540926 and 0.9534626 ≤ 1e-4.
  4. `package: no three import`: 0 matches in src, demo and scripts, and no `three` in package.json dependencies.
  5. `browser: moving source pans left to right`:
     - the build equals a fresh buildDemo and ctxState is `running`;
     - the first-quarter mean (rightDb − leftDb) is ≤ −3 dB and the last-quarter mean ≥ +3 dB.

## Milestone proofs

- M1 Core extracted and proven in node: every family rendered to WAV with a safety and loudness report: `node scripts/proof/m1.ts`
- M2 Explicit registries and a deterministic MP3 bake: `node scripts/proof/m2.ts`
- M3 Genre-neutral procedural music rendered offline: `node scripts/proof/m3.ts`
- M4 Browser runtime: prebuilt worklet, createAudio and a demo page measured in Chrome: `node scripts/proof/m4.ts`
- M5 Multi-listener spatial audio with plain {x,y,z}: `node scripts/proof/m5.ts`
- M6 example sound pack (under `examples/`, outside the core) and its race soundtrack (commands in its README under `examples/`)
