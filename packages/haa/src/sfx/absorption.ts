// Absorption per octave band of the sound materials (docs/physics-sound.md, "Absorption per band").
//
// Bands: 125, 250, 500, 1000, 2000, 4000 Hz (the published tables stop there; 8 kHz takes the 4 kHz
// value, air absorption dominates it in any room larger than a cupboard). Random-incidence Sabine α
// from the absorption table of sengpielaudio.com's "RT60 calculator"
// (https://sengpielaudio.com/calculator-RT60Coeff.htm, the usual architectural-acoustics values):
//   concrete  "Floor: concrete or tile"       0.01  0.01  0.015 0.02  0.02  0.02
//   wood      "Wall: Wood: 3/8'' plywood panel" 0.28  0.22  0.17  0.09  0.10  0.11
//   tile      "Wall: Marble/Tile"             0.01  0.01  0.01  0.01  0.02  0.02
//   ceramic   as tile
//   glass     "Wall: Glass: window"           0.35  0.25  0.18  0.12  0.07  0.04
//   stone     "Wall: Brick: unglazed"         0.03  0.03  0.03  0.04  0.05  0.07
// No octave-band table was found for sheet metal, snow, soil or plastic: they stay flat.
//
// A material keeps the mid-band α its level gives it (`absorption`, the 500 Hz-1 kHz mean, on which
// the rooms' reverb times and levels were checked against Sabine and Barron) and takes its table
// row's spectrum: α(band) = absorption x row(band) / row(mid). A material can also give its own
// `absorptionBands`.
import type { SoundMaterial, SoundMaterialKind } from './contract.ts';

export const ABSORPTION_BANDS_HZ = [125, 250, 500, 1000, 2000, 4000] as const;

export const ABSORPTION_TABLE: Partial<Record<SoundMaterialKind, readonly number[]>> = {
  concrete: [0.01, 0.01, 0.015, 0.02, 0.02, 0.02],
  wood: [0.28, 0.22, 0.17, 0.09, 0.1, 0.11],
  tile: [0.01, 0.01, 0.01, 0.01, 0.02, 0.02],
  ceramic: [0.01, 0.01, 0.01, 0.01, 0.02, 0.02],
  glass: [0.35, 0.25, 0.18, 0.12, 0.07, 0.04],
  stone: [0.03, 0.03, 0.03, 0.04, 0.05, 0.07],
};

const clamp01 = (x: number): number => Math.min(1, Math.max(0, x));

/** The six octave bands of a material of mid-band α `absorption` and kind `kind` (flat without a table row). */
export function absorptionBands(absorption: number, kind?: SoundMaterialKind, own?: readonly number[]): number[] {
  if (own && own.length >= 6) return own.slice(0, 6).map(clamp01);
  const row = kind ? ABSORPTION_TABLE[kind] : undefined;
  if (!row) return ABSORPTION_BANDS_HZ.map(() => clamp01(absorption));
  const mid = (row[2] + row[3]) / 2;
  return row.map((x) => clamp01((absorption * x) / mid));
}

/** α of the three bands the reverb uses: low (125-250 Hz), mid (500 Hz-1 kHz), high (2-4 kHz). */
export function absorptionTriple(a: number | readonly number[]): [number, number, number] {
  if (typeof a === 'number') return [a, a, a];
  return [(a[0] + a[1]) / 2, (a[2] + a[3]) / 2, (a[4] + a[5]) / 2];
}

/** What a geometry record carries for a material: its bands (or its α alone). */
export function surfaceAbsorption(m: Pick<SoundMaterial, 'absorption' | 'absorptionBands'> | null | undefined, fallback = 0.1): number | readonly number[] {
  return m ? m.absorptionBands ?? m.absorption : fallback;
}
