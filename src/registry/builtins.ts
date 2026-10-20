// The built-in registration entry (module registry). DSP-safe: it registers the content-core
// arrays into the module registry, and the worklet entry calls it before the processors so the
// prebuilt bundle knows every built-in bed, family, mix and style.
//
// registerBuiltins() is idempotent: a builtin whose id is already registered for its kind is
// skipped, so a second call is a no-op and a call after clearRegistry() restores everything.
import { BUILTIN_BEDS, BUILTIN_FAMILIES, BUILTIN_MIXES, BUILTIN_STYLES } from '../content/index.ts'
import {
  getBed, getFamily, getMix, getStyle, registerBed, registerFamily, registerMix, registerStyle,
} from './registry.ts'

/** Register every built-in family, bed, mix preset and music style (idempotent). */
export function registerBuiltins(): void {
  for (const family of BUILTIN_FAMILIES) {
    if (getFamily(family.id) === undefined) registerFamily(family)
  }
  for (const bed of BUILTIN_BEDS) {
    if (getBed(bed.id) === undefined) registerBed(bed)
  }
  for (const mix of BUILTIN_MIXES) {
    if (getMix(mix.id) === undefined) registerMix(mix)
  }
  for (const style of BUILTIN_STYLES) {
    if (getStyle(style.id) === undefined) registerStyle(style)
  }
}
