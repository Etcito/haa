// Bundle entry of worklet/haa-processors.js (scripts/build-worklet.ts). Custom-entry recipe: a
// game with its own beds or styles copies this file, adds its register*() calls before
// registerHaaProcessors(), and bundles it with esbuild (README).
import { registerBuiltins } from '../registry/builtins.ts'
import { registerHaaProcessors } from './processors.ts'

registerBuiltins()
registerHaaProcessors()
