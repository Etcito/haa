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
| `density`  | impact rate of a swarm ÷ 200 per second (cluster presets only, see Swarms)  |
| `shape`    | the ringing object's resonator shape: index in `SHAPES` ÷ 6 (see Resonator body) |
| `dim`      | the ringing object's largest extent, log-scaled from 1 cm (0) to 10 m (1)   |
| `thickness`| its wall or plate thickness, log-scaled from 0.5 mm (0) to 0.5 m (1)        |
| `flux`     | a face under a distributed source: max(0, −n·d) × exposure (rain, see Rain on faces) |
| `area`     | that face's exposed area, log-scaled from 0.01 m² (0) to 10 000 m² (1)       |

For `slide` and `roll`, `energy` is the pressing power (normal force × speed, in watts) on the same
log axis.

### Resonator body (hand-chosen shape)

Material and shape are separate: the material says what the object is made of (its presets,
`hardness`, `absorption`), the body says what it is shaped like. The shape is chosen by hand on the
object, never computed from the mesh.

```ts
import { SHAPES, withBody } from '@etcito/haa/sfx';
// SHAPES = ['block', 'plate', 'sheet', 'bar', 'box', 'shell', 'bottle']
const crate = withBody(wood, { shape: 'box', dim: 0.6, thickness: 0.02 });      // hollow wooden box
const slab = withBody(concrete, { shape: 'block', dim: 2, thickness: 0.3 });   // thick concrete slab
const roof = withBody(metal, { shape: 'sheet', dim: 4, thickness: 0.001 });    // thin metal sheet
// materialOf(collider) returns the object's material: crate for the crate's collider, and so on.
```

- `SoundBody = { shape, dim, thickness? }` (metres). `withBody(material, body)` returns that
  object's material: same presets, its own body. `defineMaterial({ ..., body })` sets a default body.
- Every preset that rings an object receives that object's body: the impact presets of both
  sides each get their own object's `shape`, `dim`, `thickness` (and the other side's `hardness`),
  a slide the surface's, a roll the roller's, a step the ground's, a break the breaking object's,
  rain on a face that face's. A slot's forced `inputs` still win.
- In the graph: declare `shape` with `min: 0, max: 6` (linear) to receive the index, then select a
  mode table by it (block, plate, sheet, bar, box, shell, bottle, in that order); `dim` sets the base
  frequency (bigger is lower), `thickness` the damping and how bending-dominated the modes are (a
  thin sheet rings long and dense, a block is dead after a few ms). Without a body the defaults are
  `shape` 0 (block), `dim` 0.4 (≈ 16 cm), `thickness` 0.5 (≈ 1.6 cm).
- Ownership: the declaration and the plumbing are here (contract.ts); the nodes and presets that
  consume `shape` / `dim` / `thickness` are the presets' work.

- A one-shot preset (`impact`, `step`, `break`) is a timeline graph. It is restarted for each event
  and stopped after one pass, even when the patch loops. Its inputs are set before the restart, and
  the engine's `restart()` places continuous inputs on their target values without gliding.
- A continuous preset (`slide`, `roll`, and loops started with `loop()`) runs until it is stopped
  and gets new inputs every frame. A timeline graph without `loop` is restarted when it ends.
- A preset that declares an input with another range receives the 0..1 value mapped onto
  `min..max` (log when `curve` is `log`). A one-shot (or a game loop) that does not read `energy`
  is scaled by level instead: 10^(−2·(1 − energy)), so 0 dB at 1 and −40 dB at 0.
- A `slide`/`roll` voice is never scaled by `energy`: its preset's `speed` input is its loudness law.
  The two loops are calibrated against the impacts: at 2-3 m/s a rolling 2 kg ball and a sliding
  crate sit about 10-15 dB under their own impacts (10 ms RMS at the same distance), at rest they are
  silent. (Scaling them by the pressing power as well put a rolling ball 40-60 dB under its impacts.)
- Reserved input `_dist`: a preset that declares it receives the distance in metres. Direction is
  handled by an HRTF panner, so `_pan` and `_doppler` are not sent.

### How the presets consume the body (`modal` node)

The `modal` node's `shape` enum has the `SHAPES` order (block, plate, sheet, bar, box, shell, bottle)
and is modulatable, so the `shape` input (declared 0..6) wires straight into it. A preset computes the
lowest mode as 1800 Hz · k_shape · thickness / dim² (bending of a plate or a bar of wood; k is 1 for a
plate, a sheet, a box or a block, 1.2 for a bar, 0.35 for a shell or a bottle), the node's `material`
scales it (steel 1.6×, glass 1.5×, ...), and `thickness` sets the ring time (a 0.5 mm sheet rings ~6×
longer than the default 1.6 cm, a 0.5 m slab ~10× shorter). `box` and `bottle` add their air cavity.
Consumed by `solid-impact`, `wood-crate-impact`, `metal-impact`, `glass-clink`, `footstep-wood`,
`footstep-metal`, `slide-scrape` (the surface) and `roll-rumble` (the floor). The material itself is
the preset; the generic ones (`solid-impact`, `slide-scrape`, `roll-rumble`, `rain-surface`) also
declare an enum input `material` (`wood`, `metal`, `glass`, `stone`, `plastic`, `ceramic`, `concrete`,
`tile`, `snow`; an index) that a slot can force through its `inputs`.

The runtime sends it from the ringing object's material: `defineMaterial({ name: 'crate', material:
'wood', ... })` (`SOUND_MATERIALS`, `SoundMaterialKind`). In the contract the value is index / 8
(`materialInput('stone')`, which is also what a slot's forced `inputs.material` takes); an enum input
receives the index (`toDeclared` rounds index / 8 back). A material without `material` sends nothing
and the preset keeps its own default. Rain on a face sends its face's material too.

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

sfx.play('explosion', point, { energy: 1, size: 1 }, { extent: 2 });  // a one-shot from game code
const rain = sfx.loop('rain-surface', above, { energy: 0.6 }, { extent: 25 }); // a large continuous voice
sfx.handle({ kind: 'step', position, striker: boot, surface: snow,   // a footstep, 3 cm above the ground
             inputs: { energy: 0.5, depth: 0.6, speed: 0.4, size: 0.55 } });
sfx.stats().voices;  // per voice: visible share, excess path, direct loss, diffuse level, RT60, pan
```

- `createSfxEngine({ acoustics: { traceRays, bounces, reverbStride, traceEvery, voiceEvery } })`
  sets the ray budget (defaults 256, 6, 2, 0.1 s, 0.15 s).
- `extent` (m) on `play`/`loop` options or on an event: the radius of a large source (rain, an
  explosion). It spreads around the listener up close and is probed over its size for occlusion.

- **Material slots**: `impact`, `slide`, `roll`, `step`, `break`. A slot is a preset name, or
  `{ preset, gain, inputs, cluster }`, where `inputs` forces fixed values (for example a tin can is
  `size: 0.1`) and `cluster` names the swarm preset of debris (see Swarms). `hardness` is 0..1 and
  `absorption` is the surface's Sabine α.
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
  - Impacts below 0.35 m/s normal speed are dropped. Each pair waits 60 ms and each moving collider
    20 ms between impacts; a fixed collider (the ground) does not wait, or a swarm of pebbles
    landing on it made one impact per 20 ms. While in contact, a new impact needs an impulse spike of 4× the running mean.
  - A game that polls contacts, or the Rust engine's `ege.contact` events, can feed
    `contactStarted` and `contactStopped` instead of the EventQueue.
  - `surfaceBelow(p)` returns the material under a foot.
- **Voices**: one AudioWorkletNode (`haa-sfx-pool`, in `@etcito/haa/worklet`) runs one wasm
  instance with N slots (16 by default), one stereo output per slot.
  - A slot keeps its last patch, and a new voice goes to a slot that already holds its preset.
  - Admission is a CPU budget: each preset's render cost is measured once on the main thread, the
    3D chain adds 1.8 % (see Acoustic model), and the default budget is 60 % of the audio thread.
  - When the budget or the voices are full, the voice with the lowest priority is stolen. Priority is
    energy ÷ (1 + d/8), +0.25 for a continuous voice. A one-shot estimated below −60 dB is not
    started.
  - A stolen voice fades out over 60 ms in its own slot while the new one starts in a spare: 2 of
    the slots are kept for that (16 slots, 14 voices at once). Only when every slot is busy, fading
    ones included, is the oldest fading voice cut. A continuous voice (slide, roll, a loop, a swarm's
    cluster voice) fades in over 50 ms; one-shots start at once (their attack is the sound).
  - A slide/roll voice is started only when it would be heard: each looping `speed` preset's RMS is
    measured once at 13 speeds, and a contact estimated under −70 dB at the listener (that level at
    its speed, 1/r beyond 2 m) takes no voice. A running one that stays 6 dB under that for 0.25 s is
    stopped. A bottle rocking to rest no longer restarts silent `roll-rumble` voices.
  - Deferred stops and releases run on the audio clock (each `update`/`handle`), with a timer as the
    fallback, so an OfflineAudioContext render frees its voices on time.

## Swarms

One voice per tiny impact does not scale: 40 pebbles or the 14 shards of a bottle exhausted the 16
slots. An impact whose striker's slot has a `cluster` preset goes through `SwarmTracker`
(`src/sfx/swarm.ts`):

- A swarm is a region (4 m) of impacts sharing a cluster preset. In each 0.3 s window its first 3
  impacts keep their own voices, and up to 2 more that are louder (by 0.05 energy) than all of them,
  so the pattern still reads as discrete hits.
- Every other impact feeds ONE continuous voice of the cluster preset at the swarm's centre:
  `density` is the impact rate (a leaky count, τ = 0.08 s, ÷ 200), `energy` the power mean (on
  the 48 dB axis of the impact presets) and `size`, `hardness` the means of the last impacts, with
  the same τ: the grains then carry on average exactly the summed power of the impacts. The voice
  fades in over 50 ms, and fades out over 0.2 s from 0.15 s after the last impact: the cluster is
  never switched on or off in one step. The 0.3 s window slides with every impact (no window edge).
- A cluster preset is a `crackle` cloud of `ring` events, each one a small version of the single
  impact (same pitch range, contact and energy-to-level map), whose rate is `density` × 200, so its
  summed level follows the impacts it stands for (the energy of one event within ±2 dB of one
  impact). `particle-cluster` (pebbles, debris) and `glass-cluster` (shards) are in `patches/sfx/`.
- Impact presets for the materials: `wood-crate-impact`, `metal-impact` (a struck steel plate),
  `particle-impact`, `glass-clink`, `glass-shards`; `metal-ricochet` is a bullet glancing off metal
  (ping and tumbling whine), not a plate hit.
- `createSfxEngine({ swarm: false })` gives every impact its own voice; `swarm: { individual, ... }`
  tunes the grouping. `stats().clustered` counts the impacts absorbed.

## Rain on faces (distributed sources from the geometry)

Rain is not one loop around the listener: every exposed face of the level plays its own rain,
driven by its geometry, its material and its hand-chosen body (`src/sfx/faces.ts`, node
`rainOnFace` in haa-core, preset `patches/sfx/rain-on-face.json`).

```ts
import { withBody } from '@etcito/haa/sfx';
const faces: SoundFace[] = [{
  center, normal,           // outward normal
  u, v,                     // half extents in its plane: the face spans center ± u ± v
  material: withBody(tile, { shape: 'plate', dim: 0.3, thickness: 0.012 }),
  thickness: 0.2,           // the solid behind it (a roof slab): heard from under it, through it
  label: 'hall roof',
}, /* ... every face of the level: roofs, plates, floors, the ground, walls */];
const rain = sfx.faceSource(faces, { preset: 'rain-on-face', direction: { x: 0, y: -1, z: 0 }, intensity: 0.6 });
rain.set({ direction: windTilted });   // a wall starts to receive rain
rain.setFaces(faces);                  // a roof changed material
rain.stop(0.6);
```

- Per face: `flux` = max(0, −n·d) (a vertical wall under vertical rain gets 0, a roof facing
  wind-driven rain the most). Exposure: a grid of cells (2 m, at most 40 × 40 per face), each casting
  one ray toward where the rain comes from; the exposed area is the cells whose ray escapes (the
  ground under a roof or under the snow patch is dry). Recomputed when the direction changes.
- The emitter of a face sits at its nearest exposed part (the nearest exposed cell, refined by the
  exposed cells around it), with `extent` = √(area/π) (at most 20 m) and `nearest`: standing on the
  wet ground the rain is all around, a roof 10 m away comes from its edge. Under a face (behind it
  and within its footprint) it radiates from the solid's other surface through a loss by body
  (`FAR_SIDE_LOSS_DB`: a sheet 3/5/8 dB at 250 Hz/1 kHz/4 kHz, a plate 10/16/24, a block 28/38/50),
  ramped over the last metre of the footprint: inside the hall the rain is on the roof above you,
  coloured by the room's reverb.
- The preset receives `energy` (intensity), `flux`, `area`, the face material's `hardness` and the
  body's `shape`, `dim`, `thickness`. The 6 faces loudest at the listener play (estimated from
  intensity, flux, area, hardness, far-side loss and distance); a playing face leaves only for one
  3 dB louder, and faces fade in and out over 0.6 s.
- `rain-on-face` sits about 35 dB under an energy-1 impact's peak at the listener (output gain 0.04):
  at gain 1 rain on open ground measured −14 dBFS RMS, above a 160 J crate impact (−29 dBFS RMS over
  100 ms) and 35 dB above footsteps.
- `rainOnFace` (Rust): drops are Poisson at 600·intensity^1.3·flux·min(area, 25 m²) per second
  (more area beyond 25 m² only adds density, the far parts are the chain's distance; above 4000
  drops/s each drop carries several). A drop is a short noise splash (bigger drops longer and
  louder, harder surfaces shorter and brighter: a low-pass from 1.2 kHz for snow to 7 kHz for
  metal, 1200 + 5800·hardness²) driving a 6-mode bank of the face's shape: base frequency from the
  shape, the local size (a block answers with 0.3 m of its surface, a plate with 1 m, a sheet with
  1 m) and the stiffness (0.35 + 1.3·hardness²), ring time from the shape, the thickness and the
  hardness. Faces over 2 m² alternate two banks ±7 % apart. Snow and earth are nearly silent (0.08
  of the level).
- Checked against CC0 freesound recordings of rain on metal roofs (509116, 509113, a car roof 344460):
  their spectral centroid is ~750 Hz (540-1100 Hz over 0.5 s windows), most energy under 2 kHz: a
  metal roof drums, it does not ping. The sheet's modes were 1.6 kHz and up (centroid 2 kHz); they now
  start at ~0.8 × 640 Hz with a wider spread and more of the drop is splash (centroid ~570 Hz, the
  long-term spectrum 6.8 dB from the recordings' mean instead of 13.1). Tiles (refs 333509, 669487:
  centroid ~1.4 kHz) and concrete (street rain 93033, 470484, 470485, 527498: ~1.3 kHz) were already
  within the recordings' spread; no recording of rain on a wooden roof was found, wood stays as it was.
- The same mechanism serves any source spread over the level's faces (hail, sand blown by the wind,
  sparks): a preset that reads `flux` and `area`.

Measured in the studio before that check (offline render, 2.9 s of rain at intensity 0.6, per-ear
energy and spectral centroid), listener in the hall's centre with each roof: tiles 1.24 kHz, metal sheet 2.08 kHz (highs
7 dB above the tiles), wood boards 1.01 kHz, concrete slab 0.54 kHz and 20 dB quieter. Under the open
shelters: tiles 1.12 kHz, wood 0.98 kHz. In the wooden shed under its metal roof 2.19 kHz. Walking
from outside in under the hall's roof at 2 m/s, the largest change is 2.1 dB per 100 ms.

## Acoustic model

Everything comes from Rapier ray casts against the fixed colliders (`src/sfx/raytrace.ts` for the
tracing, `src/sfx/acoustics.ts` for the image sources, `src/sfx/spatial3d.ts` for the Web Audio
chain). Each hit carries the material's α. Values are computed from the geometry at a steady rate
and glided every frame, so nothing switches in one step.

### Why the former model did not hear the room (2026-11-14)

The owner: the reverb did not seem to come from the level, standing in front of the open door
inside the room sounded like the middle of the room. Causes, measured on the studio's hall:

- The room was 14 rays from the listener, Sabine from their mean hit distance, the level from a
  cube of that mean free path, then a hard cap (+6 dB) and 7 RT60 steps. Inside any closed room
  the level hit the cap and the rays near the door barely changed: 1 m inside the door came out
  2.7 dB WETTER than the centre (late energy of a click +8.0 against +5.3 dB).
- It ignored the source: one reverb for every voice, from the listener alone. A click in the hall
  heard 4 m outside its door got −33 dB of tail and a 0.37 s decay, the hall's 2 s were gone.
- Its 4 early reflections were mono taps through equal-power panners, the strongest of them, for a
  footstep, the floor right under the source: an echo 0.2 ms after the direct sound, a comb filter.
  In the shed the first 50 ms of a footstep came out +12.5 dB over the dry step, brighter (spectral
  centroid 2.8 → 3.6 kHz, −6 dB in the bass, +2 dB in the highs) with a 4.6 dB ripple across the
  spectrum; a snow step +11.6 dB with +4 dB in the highs. That is the owner's "a nail that will not
  go into concrete" and "a wet cloth slapped on a wall": the room turned the step into another object.
- Occlusion was 3 rays (0, 1/3, 2/3 or 1 blocked), the IR swapped between steps, values were set
  every 0.2-0.25 s: a source sliding behind the free wall and a listener walking out of the hall
  both stepped by 5.5 dB within 100 ms.

### Listener trace

`traceListener`: 256 rays from the listener (Fibonacci sphere), each followed over 6 bounces,
specular with 35 % scattering, deterministic (the same place gives the same paths). It gives the
listener's space (`stats().room`): share of rays that hit nothing, share that leave within the 6
bounces (openings, sky), mean free path, energy kept per bounce (absorption and leaks together),
RT60 = 60·l / (c·dB lost per bounce), clamped to 0.1-8.5 s, and a volume estimate. It keeps every
bounce point, and the points the listener sees along its rays (for the direct path search).
Recomputed every 0.1 s (≈ 1500 casts, 3-4 ms; a Rapier cast costs 2-2.7 µs from JS).

### Each voice's diffuse field (source AND listener)

`voiceReverb`, every 0.15 s per voice (staggered): next-event estimation from every 2nd traced
ray's bounce points to the source. A bounce point that sees the source adds the energy a Lambert
surface returns along that ray: (4π/N)·T·(1 − α)·cos θ / (π·r²) × (2 m)², relative to the direct
sound at 2 m, T the energy left after the earlier bounces. First bounces add only their scattered
share (35 %): the specular part is the image-source taps' job. Energy arriving less than 5 ms after
the direct sound is part of it (the ground right under a footstep). Beyond the 6 traced bounces the
last orders carry on at the measured decay.

- Level: the sum. The listener's room, the source's room and the coupling through openings come
  out of the same sum: a click in the hall heard from outside reaches only the rays that enter the
  door, so its tail is the hall's, weaker, and from the door's side.
- Reverb time: from the paths that carry this source (their absorption and how often they leave),
  bounded by the centre time of the arrivals (an exponential that starts at t0 has its centre at
  t0 + RT60/13.8), which keeps a facade outdoors from ringing like a room.
- Direction: the energy-weighted arrival direction; its length is how directional the field is.
- Pre-delay: the time by which 15 % of the traced energy has arrived.

Mix: the level is shifted by the reverb level, `setReverbLevel(dB)` (`createSfxEngine({
reverbLevelDb })`, −30..+6 dB; **0 dB is the physical estimate**, the default −7 dB), then capped at
+12 dB re the direct sound at 2 m. A bare room is 10 dB wetter than a footstep's direct sound at
the feet (critical distance under half a metre): true, and a wash. A constant shift keeps every
contrast between places. The studio's Reverb slider is this value. (`setReverbAmount` still scales
the bank's output and lengthens RT60 above 1.)

### Reverb bank

11 convolvers, always running, with impulse responses at RT60 0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0,
2.8, 4.0, 5.6 and 8.0 s, √2 apart, each at unit energy and calibrated on its late decay (Schroeder
T20 from 50 ms: the `space` tail bends, its late slope read 20 % longer than its T20 from the onset).
Up to 1.6 s they are rendered by the HAA engine (a burst into the `space` node at full wet). A longer
one keeps the render's first 0.3 s (the onset and colour of the room), slowed to the target, and
continues with a decorrelated noise tail low-passed at 6 kHz, decaying exactly at the target,
level-matched over 0.25-0.3 s and cross-faded in by 0.35 s. Stretching the whole render amplified
its floor (an 8 s target measured 6.8 s), and the engine renders this patch cleanly only up to about
5 s (longer timelines came out NaN after 1.9 s; samples that are not finite are zeroed). Measured:
2.00, 2.75, 4.02, 5.60, 8.01 s. A voice sends to the two neighbours of its RT60 (sine law, equal
power): a voice walking from the shed into the hall glides from one IR to the next. Two decays a
factor 2 apart did not blend (a 0.86 s shed measured 1.37 s). Start-up: 1.55 s on the main thread
for the 11 responses (the long noise tails), once, when the engine is created.

### Voice chain

```
slot -> level -> propagation delay (d/c: arrival time, Doppler when it moves) -> air shelf -> trunk
trunk -> direct gain + 2 diffraction shelves -> HRTF panner at the apparent position -> dry
                                             -> unpanned stereo (a large source up close)  -> dry
trunk -> low-pass (taps' absorption) -> 4 taps: delay -> gain -> HRTF panner at the image -> dry
trunk -> send -> pre-delay -> low-pass (absorbent rooms are darker) -> stereo pan -> 2 convolvers
```

- Distance: 1/r beyond 2 m (the panner); under 2 m the level keeps rising as 2/d down to 1 m
  (+6 dB, near field).
- Air absorption: a high shelf at 4 kHz, −0.3 dB per metre (exaggerated: −6 dB at 20 m), capped at
  −24 dB.
- Large sources (`extent`): the distance is taken to the near side (d − 0.8·extent), and the voice
  crossfades from the HRTF panner to its own unpanned stereo as (extent/d)²: rain all around.
- The diffuse field's left/right: half the lateral component of its direction pans the send (a door
  on the left: the right ear 5-8 dB lower, not silent).
- Parameters are written only when they moved (0.03 dB, 0.5 % in frequency, 20 µs of delay), as
  25 ms linear ramps that end: a never-ending setTarget on every parameter every frame keeps
  Chrome's biquads and delays on their per-sample paths.
- The bank's convolvers are connected only while a voice sends to them, and for their tail after:
  Chrome's ConvolverNode never sleeps on silence (a 10 s response measured 14 % of the audio thread
  with nothing in it, the 11 of the bank about half of it). 14 pink-noise loops: 0.92 → 0.47 × real
  time offline. The engine now budgets 3 % per chain (`CHAIN_COST`).
- Cost (16 pink-noise loops, chains and bank, no ray cast, offline): 1.45 × real time with 6 taps of
  their own filters, two air filters, a proximity shelf and setTarget every frame; 1.29 with the
  writes above; 0.75 with 4 taps sharing one filter and one air shelf (the former chain: 0.61); with
  the bank extended to 4 s and 14 voices (2 spare slots), 0.92. The engine budgets 1.8 % of the audio
  thread per chain.

### Direct path: gradual occlusion and diffraction

`directPath`, with the voice's geometry:

- Visible share: 15 rays from the listener to a disk around the source (radius = extent, at least
  0.3 m: the centre, 6 at half radius, 8 at full radius). A probe point inside the floor or a wall
  the source lies against does not count (a footstep is not half hidden by its own floor). The last
  0.2 m of each ray is skipped.
- Hidden: the shortest path around the obstacles. One bend: in 5 planes across the straight line
  (at 10-90 % of it), for 12 directions each, the smallest offset whose point both ends see, by
  bisection; the offset of a direction varies continuously as the source moves, so the excess path
  glides. Fallbacks: points the listener sees along its traced rays, then two bends through points
  the listener and the source see (a source deep in a room, through its door).
- Loss: Kurze-Anderson edge diffraction from the Fresnel number of the excess path at 250 Hz,
  1 kHz and 4 kHz, scaled by 0.65 (`DIFFRACTION_SCALE`): 5 dB at the shadow boundary, more and more
  in the highs as the path bends further, capped at 45 dB; no path at all is the wall's
  transmission (28/38/50 dB). The visible share plays at full level, the rest with that loss
  (energy blend). Broadband gain from the 250 Hz figure, the two high shelves (600 Hz, 2.5 kHz) from
  the differences. `setOcclusionStrength(s)` (0..2) scales every dB figure.
- Where it comes from: the panner sits at the bend (the edge or the door), at the length of the
  path, blended with the true direction by the visible share.

### Early reflections

First-order image sources from 64 rays from the source. A tap counts only if both legs reach the
same surface; four more bounce points 0.35 m around it give it a weight (the valid share), so an
echo fades as its bounce point slides off a wall's edge or into a door. Echoes under 1.5 ms after
the direct sound are dropped (they merge with it: the footstep's floor). Gain √((1 − α)(1 − 0.35)),
the 1/path loss from the panner at the image. A tap keeps its plane while it reflects; a new plane
takes a free tap from silence.

### Smoothing

Every value glides per frame toward the last geometry: direct path 0.12 s, the bend point 0.15 s,
diffuse level and direction 0.2 s, reverb time 0.35 s, taps 0.1 s.

### Measured (Chrome, OfflineAudioContext replay through the scene's ray cast)

The studio's 3D scene (haa-studio, `src/ui/panels/scene/`) is the test bed: a first-person player
(WASD / ZQSD / arrows, mouse look on click, Shift to run) is the listener, a Rapier capsule that
walks through the doors; its footsteps play the ground's `step` slot at the foot (3 cm above the
surface, about 1.6 m below the ears), stride and energy from its speed (`footstep-wood` and
`footstep-metal` are used as soon as patches/sfx has them). A concrete hall (12 x 8 x 5 m, a 2 m
door, its roof material chosen in the panel: tiles, metal sheet, wood boards, concrete slab) and a
wooden shed under a metal sheet roof (4 x 4 x 2.6 m, wooden floor) give two spaces to walk between,
and two open shelters side by side (tiles, wood boards) compare roofs in the rain. Every object
declares its body by hand. The panel's Rain tilt tilts the rain toward −x.
Two large concrete buildings, 30 x 20 m and 10 m high with a 3 m door, one empty and one with 40
fixed crates and blocks (0.7-2.4 m wide, up to 3 m high, an aisle from the door), compare size and
clutter. The objects thrown are held in the hand (number keys 1-6: crate, bottle, ball, a handful
of pebbles, a gun whose rounds ricochet, a grenade that explodes after 2.5 s), shown at the bottom
right of the view; a click in the view throws along the view (hold to throw up to twice as hard),
and the panel's item buttons select and throw once, so they work without the pointer lock.
Concrete is α 0.05 in the scene (0.02 made every concrete room ring past the bank).
Pointer lock could not be granted to an automated Chrome window (`WrongDocumentError`, the window
is not the focused one); drag-to-look works there, and Esc stops the walk.

A 1 ms click, energy per ear in windows from its arrival, dB relative to the dry click's direct
sound at 2 m. Studio hall (12 x 8 x 5 m concrete, 2 m door), the listener's own click 1.5 m ahead
on the floor; both builds on the same geometry.

(Measured with the scene's concrete at α 0.02; at 0.05, its value now, the hall centre gives T20 3.59 s.)

| listener            | before: late (50 ms-2 s), T20 | now (reverb level −7 dB): late, T20 |
|---------------------|-------------------------------|-------------------------------------|
| hall centre         | +5.3 dB, 2.07 s               | +6.9 dB, 3.99 s                     |
| 1 m inside the door | +8.0 dB, 2.07 s               | +0.6 dB, 3.21 s                     |
| in the doorway      | −4.8 dB, 0.76 s               | −8.7 dB, 1.60 s                     |
| 4 m outside         | −24.1 dB, 0.14 s              | −20.6 dB, 0.48 s                    |
| shed centre         | +5.5 dB, 0.76 s               | +6.1 dB, 1.06 s                     |
| shed door           | −4.5 dB, 0.33 s               | −9.7 dB, 0.92 s                     |

A click at the hall's centre heard from those places: now +9.8 / +10.3 / +6.5 (L +8.1, R +4.8) /
−4.0 dB (L −0.7, R −7.2: through the door, on the left), T20 4.0 s everywhere; before +5.2 / +8.0 /
−4.9 / −33.0 dB, T20 0.37 s outside.

A footstep (`footstep-concrete`, at the feet) against the same step dry (no ray, no reverb), first
50 ms: shed +5.5 dB (before +12.5), same three band levels (before −6 dB bass, +2 dB highs),
spectral ripple 1.7 dB (before 4.6); snow step in the shed +3.7 dB, bands unchanged, ripple 1.6
(before +11.6 dB, +4 dB highs, ripple 3.8).

Size and clutter (own click 1.5 m ahead along the aisle, 11 s renders, concrete α 0.05):

| place                      | late (50 ms-2 s) | early (3-50 ms) | T20    | EDT    | slap (peak / median, 40-300 ms) |
|----------------------------|------------------|-----------------|--------|--------|---------------------------------|
| wooden shed, centre        | +6.1 dB          | +7.7 dB         | 1.07 s | 0.85 s | 12.6 dB                         |
| hall 12 x 8 x 5, centre    | +5.8 dB          | +0.5 dB         | 3.59 s | 2.54 s | 9.2 dB                          |
| big empty, centre          | −1.9 dB          | −16.6 dB        | 8.03 s | 6.44 s | 10.7 dB                         |
| big empty, 3 m from door   | −1.1 dB          | −17.9 dB        | 8.03 s | 6.37 s | 11.0 dB                         |
| big with cubes, centre     | −0.5 dB          | −8.1 dB         | 7.45 s | 5.28 s | 9.7 dB                          |
| big with cubes, near door  | +0.4 dB          | −6.1 dB         | 6.74 s | 4.77 s | 8.7 dB                          |

The cubes give 8-12 dB more early energy (denser reflections), an EDT 18-25 % shorter, a decay
7-16 % shorter and less distinct slaps; the empty building's first reflections arrive 50-70 ms
after the direct sound (pre-delay), the cluttered one's 25-35 ms.

Steadiness (pink noise loop, 100 ms RMS): moving at 1.5 m/s from the open behind the free wall,
largest step 2.2 dB per 100 ms (before 5.5); the listener walking at 1.4 m/s from the hall centre
out through the door, 1.2 dB (before 5.5).

## Voice budget (measured)

Measured in Node and in Chrome (OfflineAudioContext, 48 kHz), as a fraction of real time on the
audio thread:

| preset (stand-in)  | 1 voice | per voice with the former 3D chain |
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
