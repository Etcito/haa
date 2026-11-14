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

For `slide` and `roll`, `energy` is the pressing power (normal force × speed, in watts) on the same
log axis.

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
  - When the budget or the slots are full, the voice with the lowest priority is stolen. Priority is
    energy ÷ (1 + d/8), +0.25 for a continuous voice. A one-shot estimated below −60 dB is not
    started.
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
  stops 0.15 s after the last impact.
- A cluster preset is a `grain` cloud whose grain is the single impact (same tone, decay and
  energy-to-level map) and whose rate is `density` × 200, so its summed level follows the impacts it
  stands for (one grain within ±2 dB of one impact). `particle-cluster` (pebbles, debris) and
  `glass-cluster` (shards) are in `patches/sfx/`.
- `createSfxEngine({ swarm: false })` gives every impact its own voice; `swarm: { individual, ... }`
  tunes the grouping. `stats().clustered` counts the impacts absorbed.

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
RT60 = 60·l / (c·dB lost per bounce), clamped to 0.1-3 s, and a volume estimate. It keeps every
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

Mix: the level is lowered by 5 dB (`WET_OFFSET_DB`) and above 0 dB grows at half the rate
(`WET_RATIO`), then capped at +6 dB. A bare room is 10 dB wetter than a footstep's direct sound at
the feet (critical distance under half a metre): true, and a wash. The offset keeps the contrasts
between places; the knee only tames the wettest rooms.

### Reverb bank

7 convolvers, always running, with impulse responses rendered once by the HAA engine (a burst into
the `space` node at full wet) at RT60 0.25, 0.35, 0.5, 0.7, 1.0, 1.4 and 2.0 s, √2 apart, each at
unit energy and calibrated on its late decay (Schroeder T20 from 50 ms: the `space` tail bends, its
late slope read 20 % longer than its T20 from the onset); the 2 s one is a 1.6 s render slowed by an
exponential gain, also calibrated. A voice sends to the two neighbours of its RT60 (sine law, equal
power): a voice walking from the shed into the hall glides from one IR to the next. Two decays a
factor 2 apart did not blend (a 0.86 s shed measured 1.37 s). Start-up: 200 ms for the 7 renders.

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
- Cost (16 pink-noise loops, chains and bank, no ray cast, offline): 1.45 × real time with 6 taps of
  their own filters, two air filters, a proximity shelf and setTarget every frame; 1.29 with the
  writes above; 0.75 with 4 taps sharing one filter and one air shelf (the former chain: 0.61). The
  engine budgets 1.8 % of the audio thread per chain.

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
surface, about 1.6 m below the ears), stride and energy from its speed. A concrete hall (12 x 8 x
5 m, a 2 m door) and a wooden shed (4 x 4 x 2.6 m, wooden floor) give two spaces to walk between.

A 1 ms click, energy per ear in windows from its arrival, dB relative to the dry click's direct
sound at 2 m. Studio hall (12 x 8 x 5 m concrete, 2 m door), the listener's own click 1.5 m ahead
on the floor; both builds on the same geometry.

| listener            | before: late (50 ms-2 s), T20 | now: late, T20, voice RT60 |
|---------------------|-------------------------------|----------------------------|
| hall centre         | +5.3 dB, 2.07 s               | +6.4 dB, 2.05 s, 3 s (bank max 2 s) |
| 1 m inside the door | +8.0 dB, 2.07 s               | +0.8 dB, 2.06 s            |
| in the doorway      | −4.8 dB, 0.76 s               | −6.0 dB, 1.85 s            |
| 4 m outside         | −24.1 dB, 0.14 s              | −18.7 dB, 0.47 s           |
| wooden shed centre  | +5.5 dB, 0.76 s               | +3.5 dB, 1.01 s (RT60 0.86 s) |
| shed door           | −4.5 dB, 0.33 s               | −8.8 dB, 0.74 s            |

A click at the hall's centre heard from those places: now +8.1 / +8.6 / +6.4 (L +8.1, R +4.6) /
−1.6 dB (L +1.7, R −4.9: through the door, on the left), T20 2.06 s everywhere; before +5.2 / +8.0 /
−4.9 / −33.0 dB, T20 0.37 s outside.

A footstep (`footstep-concrete`, at the feet) against the same step dry (no ray, no reverb), first
50 ms: shed +5.5 dB (before +12.5), same three band levels (before −6 dB bass, +2 dB highs),
spectral ripple 1.7 dB (before 4.6); snow step in the shed +3.7 dB, bands unchanged, ripple 1.6
(before +11.6 dB, +4 dB highs, ripple 3.8).

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
