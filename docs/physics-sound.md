# Physics sound (`@etcito/haa/sfx`)

Game sound effects driven by physics. A sound **material** names, for each kind of interaction, the
preset it plays. A physics **adapter** turns contacts into **events**. The **engine** picks the
presets from the two materials of a contact, drives their published inputs, and places each voice
in 3D using the scene geometry. The graphs are authored presets: interactions never generate them.

Source: `packages/haa/src/sfx/`. It runs in the browser (Web Audio + AudioWorklet) and does not
depend on three.js or Rapier.

## Contract with the presets

Every sfx preset can read these published inputs through an `input` node. Each one is 0..1:

| input      | from physics                                                                |
|------------|-----------------------------------------------------------------------------|
| `energy`   | kinetic energy ½·m_eff·v_n², log-scaled from 10⁻⁴ J (0) to 10³ J (1)        |
| `size`     | the striking body's mass, log-scaled from 1 g (0) to 1 t (1)                |
| `hardness` | the hardness of the *other* material of the contact                         |
| `depth`    | penetration (÷ 10 cm) × softness of the surface, or the sinking given by the game (footsteps) |
| `speed`    | tangential slip speed ÷ 10 m/s                                              |

- A one-shot preset (`impact`, `step`, `break`) is a timeline graph. It is restarted for each event
  and stopped after one pass, even when the patch loops. Its inputs are set before the restart, and
  the engine's `restart()` places continuous inputs on their target values without gliding.
- A continuous preset (`slide`, `roll`, and loops started with `loop()`) runs until it is stopped
  and gets new inputs every frame. A timeline graph without `loop` is restarted when it ends.
- A preset that declares an input with another range receives the 0..1 value mapped onto
  `min..max` (log when `curve` is `log`). A preset that does not read `energy` is scaled by level
  instead: 10^(−2·(1 − energy)), so 0 dB at 1 and −40 dB at 0.
- Reserved input `_dist`: a preset that declares it receives the distance in metres. Direction is
  handled by an HRTF panner, so `_pan` and `_doppler` are not sent.

## API

```ts
import { createSfxEngine, createRapierSfx } from '@etcito/haa/sfx';

const sfx = await createSfxEngine({ ctx, module, processorUrl: workletUrl,
  loadPreset: (name) => fetch(`/sfx/${name}.json`).then((r) => r.ok ? r.text() : null) });
const snow = sfx.defineMaterial({ name: 'snow', hardness: 0.05, absorption: 0.6,
  sounds: { step: 'footstep-snow', impact: 'footstep-snow' } });
const glass = sfx.defineMaterial({ name: 'glass', hardness: 0.95, absorption: 0.03, breakEnergy: 2,
  sounds: { impact: 'glass-shards', break: 'glass-bottle-break' } });

const physics = createRapierSfx({ world, rapier: RAPIER, materialOf: (collider) => byHandle.get(collider.handle) ?? null,
  emit: (e) => sfx.handle(e), onBreak: (collider, e) => shatter(collider, e.position) });
sfx.setRaycast(physics.rayCast((c) => c.parent()?.isFixed() ?? true)); // walls occlude and reflect

// each physics step
physics.beforeStep(); world.step(queue); physics.afterStep(queue, dt);
// each frame: the camera is the listener
sfx.update({ position, forward, up });

sfx.play('explosion', point, { energy: 1, size: 1 });               // a one-shot from game code
const rain = sfx.loop('rain-surface', above, { energy: 0.6 });       // a continuous voice
sfx.handle({ kind: 'step', position, striker: boot, surface: snow,
             inputs: { energy: 0.5, depth: 0.6, speed: 0.4, size: 0.55 } });
```

- **Material slots**: `impact`, `slide`, `roll`, `step`, `break`. A slot is a preset name, or
  `{ preset, gain, inputs }`, where `inputs` forces fixed values (for example a tin can is
  `size: 0.1`). `hardness` is 0..1 and `absorption` is the surface's Sabine α.
- **Which presets play** (`resolveInteraction`):
  - impact: both sides, each receiving the other's hardness, −3 dB each. When both name the same
    preset, it plays once.
  - slide: the surface's slot, else the striker's.
  - roll: the striker's slot, else the surface's.
  - step: the ground's slot.
  - break: the breaking body's slot.
- **Rapier adapter**: works with the 0.21 API (`rapier3d-compat`, or the deterministic build that
  royal-fiesta uses) and is typed structurally.
  - Set `ActiveEvents.COLLISION_EVENTS` on the colliders. `CONTACT_FORCE_EVENTS` is optional: those
    events make the adapter re-check a pair that is held in contact.
  - Velocities are taken before the step, because after it the solver has already stopped the
    body.
  - Impacts below 0.35 m/s normal speed are dropped. Each pair waits 60 ms and each collider 20 ms
    between impacts. While in contact, a new impact needs an impulse spike of 4× the running mean.
  - A game that polls contacts, or the Rust engine's `ege.contact` events, can feed
    `contactStarted` and `contactStopped` instead of the EventQueue.
  - `surfaceBelow(p)` returns the material under a foot.
- **Voices**: one AudioWorkletNode (`haa-sfx-pool`, in `@etcito/haa/worklet`) runs one wasm
  instance with N slots (16 by default), one stereo output per slot.
  - A slot keeps its last patch, and a new voice goes to a slot that already holds its preset.
  - Admission is a CPU budget: each preset's render cost is measured once on the main thread, the
    3D chain adds 0.6 %, and the default budget is 60 % of the audio thread.
  - When the budget or the slots are full, the voice with the lowest priority is stolen. Priority is
    energy ÷ (1 + d/8), +0.25 for a continuous voice. A one-shot estimated below −60 dB is not
    started.

## Acoustic model

Everything comes from a few Rapier ray casts against the fixed colliders. Each hit carries the
material's α.

- **Direct path**: a PannerNode with the HRTF model at the source. Gain is 1/r beyond 2 m. Air
  absorption is a low-pass of 20 kHz / (1 + d/50).
- **Occlusion**: three rays from the listener, one to the source and two to points 0.35 m beside and
  above it. The last 0.2 m of each ray is ignored, so the surface the sound comes from does not
  count. With a blocked fraction b, the gain is 1 − 0.84·b (−16 dB when fully blocked) and the
  low-pass cutoff is 20 kHz^(1−b)·700 Hz^b. The reverb send loses only half of that.
- **Early reflections**: first-order image sources.
  - 14 rays from the source (6 axes and 8 diagonals) find nearby surfaces.
  - Each surface plane mirrors the source. The bounce point is where the line from the image to the
    listener crosses the plane, and it counts only if both legs reach that same surface.
  - The 4 shortest paths become taps: a delay of (path − direct)/c, an amplitude of √(1 − α), a
    low-pass that falls with α, and an equal-power panner at the image position, which also gives
    the 1/path loss.
- **Late reverb**: 14 rays from the listener.
  - Escape fraction e is the share of rays with no hit within 50 m. Mean free path l is the mean hit
    distance, and ᾱ is the mean α of the hits.
  - Sabine with V/S = l/4 gives RT60 = 0.161·l / (4·ᾱ_eff), with ᾱ_eff = (1 − e)·ᾱ + e, clamped to
    0.1–1.8 s.
  - The wet level is (1 − e)²·√(1 − ᾱ), so open air (rays escape) has almost no reverb.
  - The impulse response is rendered by the HAA engine itself (a 4 ms noise burst into a `space`
    node at full wet). The size is calibrated so that the measured Schroeder T20 matches the RT60
    within 0.01 s at 0.25 to 1.7 s. A ConvolverNode plays it, with a cross-fade when the room
    changes.
- **Update rates**: occlusion and reflections are recomputed per voice every 0.2 s, and the room
  every 0.25 s. Panner and listener positions are set, never automated: automating them sends
  Chrome's panner down its per-sample path. Setting them, and leaving idle slots unconnected, brought
  16 voices with their 3D chains from 3.4× real time down to 0.61×.

## Voice budget (measured)

Measured in Node and in Chrome (OfflineAudioContext, 48 kHz), as a fraction of real time on the
audio thread:

| preset (stand-in)  | 1 voice | per voice with 3D chain |
|--------------------|---------|-------------------------|
| grain-impact       | 2.0–2.7 % | ≈ 3.3 %               |
| snow / footsteps   | 7–8 %     | ≈ 9 %                 |
| light-rain / rain  | 8–9 %     | ≈ 10 %                |
| engine-v8 (heavy)  | 20 %      | —                     |

- With the mix (reverb, limiter) and every slot idle, the base cost is ≈ 8 %.
- All voices of one context share one audio thread, whether they run in one worklet or in many. A
  pool of 16 short impacts uses ~0.6 of real time. One AudioWorkletNode per voice costs about the
  same CPU (16 players: 0.33 against 0.42 for the pool), but each one is a separate 2.1 MB wasm
  instance, against 0.18 MB per pooled voice. It also leaves no single place to steal voices.
- Loading a patch on the audio thread costs 0.2–0.6 ms, the first load up to 8 ms. Slot affinity
  avoids most loads.
- Rule of thumb: 12–16 short one-shots plus 2–3 loops per AudioContext. Presets that are cheap and
  short (under 0.5 s for impacts) are what make bursts of particles possible.
