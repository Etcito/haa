# Royal Fiesta sound pack (example)

A party-game sound pack for [`@etcito/haa`](../../README.md): six `fx` one-shot families, a
crowd cheer/aww family set, a continuous crowd bed, a `fiesta` mix preset and a scripted 60 s
race soundtrack. It is an **example and a proof**, not part of the HAA core: the core ships no
game content, and nothing here is registered until your code calls `registerRoyalFiesta()`.

It imports HAA only through its public API (`@etcito/haa`, `@etcito/haa/node`). The
`scripts/proof-m6.ts` proof reuses the core proof helpers in `scripts/proof/lib/`, which are
development tooling and not part of the package.

## Use it

```ts
import { createOfflineAudio, createSpatial } from '@etcito/haa'
import { registerRoyalFiesta } from './examples/royal-fiesta-pack/src/index.ts'

registerRoyalFiesta()                                   // families, 'crowd' bed, 'fiesta' mix; idempotent
const audio = createOfflineAudio({ mix: 'fiesta', spatial: createSpatial(), maxListeners: 1 })
audio.startBed('crowd')?.setParam('excitement', 0.8)
audio.play('fiesta.jump', { pos: { x: 3, y: 0, z: -10 } })
audio.advance(1)
const wav = audio.take('demo')                          // StereoRender
```

In the browser, the prebuilt `@etcito/haa/worklet` bundle does not know these ids. Build a custom
worklet entry that calls `registerRoyalFiesta()` before `registerHaaProcessors()` (see the
"Custom worklet entry" section of the core README), and register the same pack on the main thread
before `createAudio`.

## Families

Each family has 8 seeded variants and is built from fxKit/uiKit layers.

| id | length | bus / category | layers |
| --- | --- | --- | --- |
| `fiesta.jump` | 0.35 s | effects / fx | springy rising harmonicStack glide, short upward noiseSweep, small subTransient |
| `fiesta.dive` | 0.5 s | effects / fx | falling whoosh noiseSweep, plastic modalHit body thud, cloth flutter |
| `fiesta.grab` | 0.25 s | effects / fx | filtered-noise cloth transient, low resonantPing, rubber squeak harmonicStack |
| `fiesta.eliminate` | 1.2 s | effects / fx | descending slide-whistle harmonicStack, poof noise burst, low boom subTransient |
| `fiesta.wobble` | 0.6 s | effects / fx | jelly harmonicStack with 4..9 Hz decaying AM, rubbery resonantPing |
| `fiesta.squash` | 0.3 s | effects / fx | squelch (band-passed noise, fast downward sweep), subTransient |
| `crowd.cheer.small` | 1.5 s | ambience / sting | `renderCrowdShot`, mood 'cheer', small crowd |
| `crowd.cheer.big` | 3.0 s | ambience / sting | `renderCrowdShot`, mood 'cheer', full crowd, long swell |
| `crowd.aww` | 1.8 s | ambience / sting | `renderCrowdShot`, mood 'aww' (o/u vowels, falling pitch, no claps) |

## The 'crowd' bed

`CROWD_BED` (id `crowd`, ambience bus, not positional) is a `CrowdModel` with params `size`,
`excitement`, `swell` and `level`, defaults 0.6 / 0.3 / 0 / 1.

- **babble:** up to 48 talkers (active count = round(6 + 42 × size)), each a glottal filtered pulse
  train (seeded f0 110..260 Hz with jitter) through three Svf formant bandpasses that glide between
  the vowels a, e, i, o, u every 0.15..0.5 s, with a 3..6 Hz syllable AM and a seeded pan.
- **murmur:** pink noise through two bandpasses (400 and 1500 Hz).
- **claps:** velvet-noise bursts band-passed at 1..2.5 kHz with an exponential decay; the rate
  follows `excitement` and starts above 0.3.
- **whistles:** above excitement 0.6, rare FM-modulated harmonicStack glides.
- `excitement` raises density, pitch, brightness and level (+6 dB at 1); `swell` (0.3 s attack) adds
  +9 dB and brightness for cheers; a short internal FdnReverb sits on the whole bed.

## The 'fiesta' preset

`FIESTA_PRESET` (id `fiesta`, priority -10) is selected with `mix: 'fiesta'`. Its priority is below
`base`, so `activeMix()` stays `'base'`. Bus levels are the base ones except effects -5 dB, ambience
-13 dB and music -13 dB (world -8, foley -11, ui -9 stay). Ducks are the base ducks plus
`eliminate` (music -6 dB, ambience -2 dB, 20/600/900 ms attack/hold/release), `cheer` (-4, 0,
40/1500/1200) and `qualify` (-5, -3, 30/1000/1000). All balance between sounds lives in the
preset's `familyTrimDb` and `bedTrimDb` (`src/mix.ts`).

## The race

`scripts/raceScript.ts` exports `FIESTA_RACE`: 60 s, 8 players, generated deterministically from its
seed. Player 0 is followed by the listener (2 m behind, 1.5 m up, facing -Z); the others run along
lanes at ±2..8 m. The script holds a countdown, music 'upbeat' with an intensity ramp and a climax
at 45 s, crowd excitement with swells, jumps, dives and grabs, wobble and squash on hits,
eliminations at 20, 35 and 48 s and a qualify at 55 s. Every event resolves to a registered id.

## Commands (run from the repository root)

```
node --conditions=haa-source examples/royal-fiesta-pack/scripts/fiesta-race.ts [--listeners 2] [--out <dir>]
node --conditions=haa-source examples/royal-fiesta-pack/scripts/proof-m6.ts [--only <pattern>] [--full]
npm test                                    # includes this pack's tests (examples/*/test)
```

The race writes to `out/m6/` under this folder by default. `--only` takes a substring or `/regex/`
(for example `--only "family fiesta."` or `--only /crowd/`); `--full` adds the 2-listener render.

`--conditions=haa-source` makes `@etcito/haa` resolve to `src/` (the package `exports` map).
The proof checks: families safety, families loudness within ±1 LU of the category target, race
safety, race integrated loudness against `FIESTA_PRESET.sceneLufs` (±1 LU), and every scripted
sound played. Outputs (WAV, event log, report triplet) go to `out/m6/` under this folder and are
not committed.
