// The Royal Fiesta sound pack for HAA: one-shot families, the crowd bed, the crowd model and
// the 'fiesta' mix preset. It is registered explicitly, never built into @etcito/haa:
//
//   import { registerRoyalFiesta } from './src/index.ts'
//   registerRoyalFiesta()            // idempotent: safe to call twice
//   createOfflineAudio({ mix: 'fiesta', ... })
//
// Everything it imports comes from the public '@etcito/haa' API.
import { getBed, getFamily, getMix, registerBed, registerFamily, registerMix } from '@etcito/haa'
import { CROWD_BED, CROWD_FAMILIES } from './crowd.ts'
import { FIESTA_FAMILIES } from './families.ts'
import { FIESTA_PRESET } from './mix.ts'

export { CROWD_BED, CROWD_FAMILIES } from './crowd.ts'
export { FIESTA_FAMILIES } from './families.ts'
export { FIESTA_PRESET } from './mix.ts'
export { CROWD_PARAMS, CrowdModel, renderCrowdShot } from './crowdModel.ts'

/** Every family of the pack, in registration order. */
export const ROYAL_FIESTA_FAMILIES = [...FIESTA_FAMILIES, ...CROWD_FAMILIES] as const

/** Registers the pack's families, the 'crowd' bed and the 'fiesta' mix. Skips what is already registered. */
export function registerRoyalFiesta(): void {
  for (const family of ROYAL_FIESTA_FAMILIES) {
    if (getFamily(family.id) === undefined) registerFamily(family)
  }
  if (getBed(CROWD_BED.id) === undefined) registerBed(CROWD_BED)
  if (getMix(FIESTA_PRESET.id) === undefined) registerMix(FIESTA_PRESET)
}
