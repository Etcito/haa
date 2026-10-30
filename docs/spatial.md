# Spatial scene

`createSpatialScene(ctx, output?)` places players in a world. It applies distance falloff,
equal-power pan and doppler, and feeds the result to the patch's reserved inputs. It is exported
from `@etcito/haa` (`packages/haa/src/spatial.ts`).

## Routing

- `scene.output` is a GainNode connected to `output ?? ctx.destination`.
- `addEmitter(id, player, falloff?)` disconnects `player.node` and routes it as
  `player.node -> GainNode -> StereoPannerNode -> scene.output`. Adding an id that already exists
  disposes the old emitter first.
- `update()` (once per frame) recomputes every emitter from the listener and the emitter's
  position and velocity. It writes `setGain(g) × emitterGain(dist)` to the emitter gain and the pan
  to the StereoPanner.
- `emitter.dispose()` disconnects that emitter's nodes. `scene.dispose()` disconnects every emitter
  and the output. The players themselves are not disposed: the caller owns them.

## Distance falloff

`Falloff { ref, max, rolloff }`, with `DEFAULT_FALLOFF = { ref: 2, max: 60, rolloff: 1 }` (metres).

```
emitterGain(d) = 1                                  d <= ref
               = ref / (ref + rolloff × (d − ref))  ref < d <= max
               = 0                                  d > max
```

With the defaults: 1 at 1 m, 0.5 at 4 m, about 0.034 at 60 m, and 0 at 61 m. `d` is the 3D distance.

## Pan

- The listener is `{ x, y, z, fx, fz }`. `(fx, fz)` is its horizontal facing and does not need to
  be unit length.
- The listener's right vector is `(fz, −fx)` in XZ. **Convention:** facing +Z, +X is on the right.
- `pan = dot(normalise(emitter − listener) in XZ, right)`, clamped to −1..1. It is 0 when the
  emitter is straight above or below the listener, or when the facing is zero.
- The StereoPanner uses the pan as-is (equal power). `equalPowerPan(pan)` exposes the same law:
  `left = cos((pan + 1)π/4)` and `right = sin((pan + 1)π/4)`.

## Doppler

- `SPEED_OF_SOUND = 343` m/s. `v` is the emitter's speed toward the listener: the component of its
  velocity (`setVelocity`) along the emitter-to-listener direction. The listener is treated as
  still.
- `dopplerRatio(v) = c / (c − v)`, clamped to 0.5..2. A source that is approaching gives a ratio
  above 1.
- The scene does not resample the audio. The patch applies the ratio through `_doppler`.

## Reserved inputs

On every `update`, the scene calls `player.set(name, value)` for these inputs. It only does so
for the names that `player.info.inputs` declares, and only when the value changed since the last
send:

| input      | value                         | engine default |
|------------|-------------------------------|----------------|
| `_dist`    | 3D distance in metres         | 0              |
| `_pan`     | pan, −1..1                    | 0              |
| `_doppler` | doppler ratio, 0.5..2         | 1              |

A patch that does not declare them gets no messages. Its spatial cues come only from the gain and
the StereoPanner.
