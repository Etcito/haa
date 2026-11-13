// Quickstart: typechecked usage of the HAA browser runtime and the offline scene (never executed).
import { createAudio, createOfflineAudio } from '../src/index.ts'

/** Starts HAA on a fresh AudioContext (call it from a user gesture). */
export async function quickstart(): Promise<void> {
  const audio = await createAudio(new AudioContext(), {
    manifestUrl: './audio/manifest.json',   // written by the bake (npm run bake / build:demo)
    processorUrl: './haa-processors.js',    // the prebuilt worklet/haa-processors.js
  })
  await audio.ready()   // every family loaded or fallback-rendered; never rejects

  audio.play('impact.wood.small', { gain: 0.8 })

  const wind = audio.startBed('wind')
  wind?.setParam('speed', 20)   // m/s

  audio.music.play('upbeat')
  audio.music.crossfade('calm', 2)   // bar-quantized, over 2 bars

  wind?.stop(1)
  await audio.close()
}

/** The same controls rendered offline (node or browser), e.g. for a test or a WAV. */
export function renderOffline(): Float32Array {
  const scene = createOfflineAudio({ seed: 1 })
  scene.play('impact.wood.small', { gain: 0.8 })
  scene.advance(1)
  return scene.take('wood').left
}
