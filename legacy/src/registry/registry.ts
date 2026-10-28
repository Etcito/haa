// Explicit content registries (module registry). DSP-safe: the state is module-level, so each
// realm (main thread, worklet, node process) has its own. It replaces the five origin-engine
// Vite glob imports (bank/families.ts, director/features.ts, host/catalog.ts twice,
// mix/presets.ts) with explicit define*/register* calls; no glob loading is ported.
//
// define* validates authored data and returns a NEW frozen definition (the caller's object is
// never mutated); register* validates through define* and rejects a duplicate id. List
// functions return arrays sorted by id; activeMix picks the highest priority, then the lowest id.
import {
  BUSES, FAMILY_MAX_SECONDS, FAMILY_VARIANTS_MAX, FAMILY_VARIANTS_MIN, VOICE_CATEGORIES,
} from '../contract.ts'
import type { BedDef, FamilyDef, FamilySpec, MixPreset, MusicStyle } from '../contract.ts'
import { hashSeed } from '../dsp/rng.ts'

/** Family id: lowercase dotted segments; a segment may continue in camelCase. */
const FAMILY_ID = /^[a-z][a-zA-Z0-9]*(?:\.[a-z][a-zA-Z0-9]*)*$/
/** Bed, style and preset id: lowercase words joined by dashes. */
const DASHED_ID = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/

const BUS_SET = new Set<string>(BUSES)
const CATEGORY_SET = new Set<string>(VOICE_CATEGORIES)

const families = new Map<string, FamilyDef>()
const beds = new Map<string, BedDef>()
const mixes = new Map<string, MixPreset>()
const styles = new Map<string, MusicStyle>()

/** Throw the shared authored-data error shape. */
function fail(kind: string, id: string, reason: string): never {
  throw new Error(`[haa] bad ${kind} ${id}: ${reason}`)
}

/** Best-effort id for an error message, even when the authored value is malformed. */
function idOf(value: unknown): string {
  if (value !== null && typeof value === 'object' && 'id' in value) {
    const id = (value as { id?: unknown }).id
    return typeof id === 'string' ? id : String(id)
  }
  return 'undefined'
}

/** Copy plain objects and arrays so a frozen definition owns its data. */
function cloneData<T>(value: T): T {
  if (Array.isArray(value)) return value.map((item) => cloneData(item)) as T
  if (value !== null && typeof value === 'object') {
    const source = value as Record<string, unknown>
    const copy: Record<string, unknown> = {}
    for (const key of Object.keys(source)) copy[key] = cloneData(source[key])
    return copy as T
  }
  return value
}

/** Freeze a cloned value recursively. */
function freezeDeep<T>(value: T): T {
  if (Array.isArray(value)) {
    for (const item of value) freezeDeep(item)
    return Object.freeze(value) as T
  }
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>
    for (const key of Object.keys(record)) freezeDeep(record[key])
    return Object.freeze(value) as T
  }
  return value
}

/** True when every number reachable in value is finite. */
function allNumbersFinite(value: unknown): boolean {
  if (typeof value === 'number') return Number.isFinite(value)
  if (Array.isArray(value)) return value.every(allNumbersFinite)
  if (value !== null && typeof value === 'object') {
    return Object.values(value as Record<string, unknown>).every(allNumbersFinite)
  }
  return true
}

/** Ascending id order (code-unit compare, independent of locale). */
function byId(a: { id: string }, b: { id: string }): number {
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
}

// --- Definitions -------------------------------------------------------------------------

/** Validate one family spec and return a new frozen definition with a seed filled in. */
export function defineFamily(spec: FamilySpec): FamilyDef {
  const id = idOf(spec)
  if (spec === null || typeof spec !== 'object') fail('family', id, 'definition must be an object')
  if (!FAMILY_ID.test(id)) fail('family', id, 'id must be lowercase dotted segments')
  if (!Number.isInteger(spec.variants) || spec.variants < FAMILY_VARIANTS_MIN || spec.variants > FAMILY_VARIANTS_MAX) {
    fail('family', id, `variants must be an integer in ${FAMILY_VARIANTS_MIN}..${FAMILY_VARIANTS_MAX}`)
  }
  if (!BUS_SET.has(spec.bus)) fail('family', id, `unknown bus ${String(spec.bus)}`)
  if (!CATEGORY_SET.has(spec.category)) fail('family', id, `unknown category ${String(spec.category)}`)
  if (!Number.isFinite(spec.maxSeconds) || spec.maxSeconds <= 0 || spec.maxSeconds > FAMILY_MAX_SECONDS) {
    fail('family', id, `maxSeconds must be finite in (0, ${FAMILY_MAX_SECONDS}]`)
  }
  if (typeof spec.render !== 'function') fail('family', id, 'render must be a function')
  if (spec.seed !== undefined && (typeof spec.seed !== 'number' || !Number.isFinite(spec.seed))) {
    fail('family', id, 'seed must be a finite number')
  }
  const def = cloneData(spec) as FamilyDef
  def.seed = spec.seed ?? hashSeed('family', id)
  return freezeDeep(def)
}

/** Validate one bed definition and return a new frozen copy (parameter arrays included). */
export function defineBed(spec: BedDef): BedDef {
  const id = idOf(spec)
  if (spec === null || typeof spec !== 'object') fail('bed', id, 'definition must be an object')
  if (!DASHED_ID.test(id)) fail('bed', id, 'id must be lowercase words joined by dashes')
  const params = spec.params
  if (!Array.isArray(params) || params.length === 0 || params.some((p) => typeof p !== 'string' || p.length === 0)) {
    fail('bed', id, 'params must be a non-empty list of names')
  }
  if (new Set(params).size !== params.length) fail('bed', id, 'params must be unique')
  const defaults = spec.defaults
  if (!Array.isArray(defaults) || defaults.length !== params.length) {
    fail('bed', id, 'defaults.length must equal params.length')
  }
  if (!defaults.every((v) => typeof v === 'number' && Number.isFinite(v))) fail('bed', id, 'defaults must be finite')
  if (spec.unsmoothed !== undefined) {
    if (!Array.isArray(spec.unsmoothed) || spec.unsmoothed.some((u) => !params.includes(u))) {
      fail('bed', id, 'unsmoothed must be a subset of params')
    }
  }
  if (typeof spec.create !== 'function') fail('bed', id, 'create must be a function')
  if (!BUS_SET.has(spec.bus)) fail('bed', id, `unknown bus ${String(spec.bus)}`)
  return freezeDeep(cloneData(spec))
}

/** Validate one mix preset and return a new frozen copy. */
export function defineMix(p: MixPreset): MixPreset {
  const id = idOf(p)
  if (p === null || typeof p !== 'object') fail('mix', id, 'definition must be an object')
  if (!DASHED_ID.test(id)) fail('mix', id, 'id must be lowercase words joined by dashes')
  if (typeof p.priority !== 'number' || !Number.isFinite(p.priority)) fail('mix', id, 'priority must be a finite number')
  for (const bus of BUSES) {
    if (p.buses === null || typeof p.buses !== 'object' || p.buses[bus] === null || typeof p.buses[bus] !== 'object') {
      fail('mix', id, `buses must contain every bus id (missing ${bus})`)
    }
  }
  const ceiling = p.master?.limiter?.ceilingDbtp
  if (typeof ceiling !== 'number' || !Number.isFinite(ceiling) || ceiling > -1.2) {
    fail('mix', id, 'limiter.ceilingDbtp must be finite and <= -1.2')
  }
  for (const category of VOICE_CATEGORIES) {
    if (p.loudness === null || typeof p.loudness !== 'object' || typeof p.loudness[category] !== 'number') {
      fail('mix', id, `loudness must contain every category (missing ${category})`)
    }
  }
  if (!allNumbersFinite(p)) fail('mix', id, 'every number must be finite')
  return freezeDeep(cloneData(p))
}

/** Validate one music style and return a new frozen copy. */
export function defineStyle(s: MusicStyle): MusicStyle {
  const id = idOf(s)
  if (s === null || typeof s !== 'object') fail('style', id, 'definition must be an object')
  if (!DASHED_ID.test(id)) fail('style', id, 'id must be lowercase words joined by dashes')
  const scale = s.scale
  if (!Array.isArray(scale) || scale.length === 0 || scale[0] !== 0) {
    fail('style', id, 'scale must be non-empty and start at 0')
  }
  for (let i = 0; i < scale.length; i++) {
    const degree = scale[i]
    if (!Number.isFinite(degree) || degree < 0 || degree > 11 || (i > 0 && degree <= scale[i - 1])) {
      fail('style', id, 'scale must be strictly ascending and within 0..11')
    }
  }
  if (!Number.isFinite(s.bpm) || s.bpm < 40 || s.bpm > 240) fail('style', id, 'bpm must be in 40..240')
  if (!Number.isFinite(s.swing) || s.swing < 0 || s.swing > 0.3) fail('style', id, 'swing must be in 0..0.3')
  let startsOnZero = false
  const progressions = Array.isArray(s.progressions) ? s.progressions : []
  for (const progression of progressions) {
    if (!Array.isArray(progression) || (progression.length !== 4 && progression.length !== 8)) {
      fail('style', id, 'each progression must have length 4 or 8')
    }
    for (const degree of progression) {
      if (!Number.isInteger(degree) || degree < 0 || degree >= scale.length) {
        fail('style', id, 'progression indices must be within the scale')
      }
    }
    if (progression[0] === 0) startsOnZero = true
  }
  if (!startsOnZero) fail('style', id, 'at least one progression must start on 0')
  return freezeDeep(cloneData(s))
}

// --- Registration ------------------------------------------------------------------------

/** Register a family; a duplicate id throws '[haa] duplicate family <id>'. */
export function registerFamily(def: FamilySpec): void {
  const frozen = defineFamily(def)
  if (families.has(frozen.id)) throw new Error(`[haa] duplicate family ${frozen.id}`)
  families.set(frozen.id, frozen)
}

/** Register several families in order; any invalid or duplicate entry throws. */
export function registerFamilies(defs: readonly FamilySpec[]): void {
  for (const def of defs) registerFamily(def)
}

/** Register a bed; a duplicate id throws '[haa] duplicate bed <id>'. */
export function registerBed(def: BedDef): void {
  const frozen = defineBed(def)
  if (beds.has(frozen.id)) throw new Error(`[haa] duplicate bed ${frozen.id}`)
  beds.set(frozen.id, frozen)
}

/** Register a mix preset; a duplicate id throws '[haa] duplicate mix <id>'. */
export function registerMix(p: MixPreset): void {
  const frozen = defineMix(p)
  if (mixes.has(frozen.id)) throw new Error(`[haa] duplicate mix ${frozen.id}`)
  mixes.set(frozen.id, frozen)
}

/** Register a music style; a duplicate id throws '[haa] duplicate style <id>'. */
export function registerStyle(s: MusicStyle): void {
  const frozen = defineStyle(s)
  if (styles.has(frozen.id)) throw new Error(`[haa] duplicate style ${frozen.id}`)
  styles.set(frozen.id, frozen)
}

// --- Lookups -----------------------------------------------------------------------------

/** The family with this id, or undefined. */
export function getFamily(id: string): FamilyDef | undefined {
  return families.get(id)
}

/** Every registered family, ascending id. */
export function listFamilies(): readonly FamilyDef[] {
  return [...families.values()].sort(byId)
}

/** The bed with this id, or undefined. */
export function getBed(id: string): BedDef | undefined {
  return beds.get(id)
}

/** Every registered bed, ascending id. */
export function listBeds(): readonly BedDef[] {
  return [...beds.values()].sort(byId)
}

/** The mix preset with this id, or undefined. */
export function getMix(id: string): MixPreset | undefined {
  return mixes.get(id)
}

/** Every registered mix preset, ascending id. */
export function listMixes(): readonly MixPreset[] {
  return [...mixes.values()].sort(byId)
}

/** The highest-priority preset (ties by lowest id); throws when none is registered. */
export function activeMix(): MixPreset {
  let best: MixPreset | undefined
  for (const p of mixes.values()) {
    if (best === undefined || p.priority > best.priority || (p.priority === best.priority && p.id < best.id)) best = p
  }
  if (best === undefined) throw new Error('[haa] no mix preset')
  return best
}

/** The style with this id, or undefined. */
export function getStyle(id: string): MusicStyle | undefined {
  return styles.get(id)
}

/** Every registered style, ascending id; its index is the music 'style' param code. */
export function listStyles(): readonly MusicStyle[] {
  return [...styles.values()].sort(byId)
}

/** Index of a style in listStyles(), or -1 when absent. */
export function styleIndex(id: string): number {
  return listStyles().findIndex((s) => s.id === id)
}

/** Family trim of p for id (dB), 0 when the preset has none. */
export function familyTrimDb(p: MixPreset, id: string): number {
  const value = p.familyTrimDb?.[id]
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/** Bed trim of p for id (dB), 0 when the preset has none. */
export function bedTrimDb(p: MixPreset, id: string): number {
  const value = p.bedTrimDb?.[id]
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/** Upper bound of the bank in seconds: the sum of variants x maxSeconds. */
export function bankSeconds(defs: readonly FamilyDef[] = listFamilies()): number {
  let total = 0
  for (const def of defs) total += def.variants * def.maxSeconds
  return total
}

/** Drop every registered definition (tests only). */
export function clearRegistry(): void {
  families.clear()
  beds.clear()
  mixes.clear()
  styles.clear()
}
