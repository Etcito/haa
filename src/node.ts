// The node-only public API of HAA (module repo): file output and, from M2, the MP3 bake.
// It does not re-export src/index.ts; import the browser-safe API from '@etcito/haa'.
// APPEND-ONLY per milestone: M2 appends bake and encodeMp3.

// M1
export { writeWav } from './node/writeWav.ts'

// M2
export { encodeMp3 } from './bake/mp3.ts'
export { bake } from './bake/bake.ts'
