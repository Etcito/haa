// Milestone 1 proof (module proof-m1, node): every built-in family rendered, measured and
// written to out/m1/families/<id>.wav, plus the wind bed accel sweep (out/m1/beds/wind.wav),
// then the 5 aggregated CHECKs (WAVs readable, safety x3, family loudness) and the report
// triplet in out/m1/. Cases: `family <id>` (one per BUILTIN_FAMILIES family) and `bed wind`.
//
//   node scripts/proof/m1.ts [--only <pattern>] [--full]
import { readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AUDIO_RATE, LOUDNESS_TOLERANCE_LU, SAFETY } from '../../src/contract.ts';
import type { FamilyDef } from '../../src/contract.ts';
import { measureMono, measureStereo, safetyVerdict } from '../../src/analysis/measure.ts';
import type { Measure } from '../../src/analysis/measure.ts';
import type { Report, ReportEntry } from '../../src/analysis/report.ts';
import { renderFamilyVariants } from '../../src/bank/render.ts';
import { BUILTIN_FAMILIES } from '../../src/content/index.ts';
import { WIND_BED } from '../../src/content/wind.ts';
import { hashSeed } from '../../src/dsp/rng.ts';
import { BASE_PRESET } from '../../src/mix/presets/base.ts';
import { writeWav } from '../../src/node/writeWav.ts';
import { renderBedDemo, renderFamilyStrip } from '../../src/offline/render.ts';
import { decodeWav } from '../../src/offline/wav.ts';
import { sha256File, writeReport } from './lib/out.ts';
import { check, checkBool, checks, proofMain, runCases } from './lib/proof.ts';
import type { ProofArgs, ProofCase } from './lib/proof.ts';

const SCRIPT = fileURLToPath(import.meta.url);
const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');
const OUT = join(ROOT, 'out', 'm1');

/** Wind sweep: speed 0 → WIND_TOP_MS m/s over the render, speed01 = speed / WIND_TOP_MS. */
const WIND_TOP_MS = 50;
const WIND_SECONDS = 8;
const WIND_SECONDS_FULL = 15;
/** Window of the bed safety verdicts (shared Q1: per 1 s window for renders of 2 s or more). */
const WINDOW_SECONDS = 1;

const LABELS = {
  wav: 'render: WAVs written and readable',
  finite: 'safety: no NaN/Inf and |DC| < 1e-3',
  peak: 'safety: sample peak <= -1 dBFS',
  silence: 'safety: no silent variant (RMS >= -60 dBFS)',
  loudness: 'loudness: family median within ±1 LU of category target',
} as const;

/** The worst safety values of one case over its units (variants, or 1 s windows of a bed). */
interface Worst {
  /** Units measured (variants or windows). */
  units: number;
  nonFinite: number;
  dc: number;
  dcUnit: number;
  samplePeakDb: number;
  peakUnit: number;
  minRmsDb: number;
  rmsUnit: number;
  /** safetyVerdict failures, as '<unit>:<failure>'. */
  failures: string[];
}

/** The @@RESULT of one case. */
interface CaseResult {
  group: 'families' | 'beds';
  id: string;
  /** The written WAV, relative to out/m1. */
  file: string;
  frames: number;
  channels: number;
  worst: Worst;
  /** The entry measure for the report. */
  measure: Measure;
  /** Families: the lower median of the normalized variant LUFS, and its category target. */
  medianLufs?: number;
  targetLufs?: number;
  note: string;
}

/** Fold the measures of the units of one case into its worst values (with safetyVerdict). */
function worstOf(ms: readonly Measure[]): Worst {
  const w: Worst = {
    units: ms.length, nonFinite: 0, dc: 0, dcUnit: 0, samplePeakDb: -Infinity, peakUnit: 0,
    minRmsDb: Infinity, rmsUnit: 0, failures: [],
  };
  ms.forEach((m, u) => {
    w.nonFinite += m.nonFinite;
    if (Math.abs(m.dc) > w.dc || Number.isNaN(m.dc)) [w.dc, w.dcUnit] = [Math.abs(m.dc), u];
    if (m.samplePeakDb > w.samplePeakDb) [w.samplePeakDb, w.peakUnit] = [m.samplePeakDb, u];
    if (!(m.minWindowRmsDb >= w.minRmsDb)) [w.minRmsDb, w.rmsUnit] = [m.minWindowRmsDb, u];
    for (const f of safetyVerdict(m).failures) w.failures.push(`${u}:${f}`);
  });
  return w;
}

/** The lower median of the finite values (-Infinity when none), as the bank rule. */
function lowerMedian(values: readonly number[]): number {
  const v = values.filter(Number.isFinite).sort((a, b) => a - b);
  return v.length === 0 ? -Infinity : v[Math.floor((v.length - 1) / 2)];
}

/**
 * A copy of def whose render replays the first output of each variant, so that the variants
 * are generated once per process although both renderFamilyVariants (measures) and
 * renderFamilyStrip (the WAV) render them. The replay is bit-identical to a new render.
 */
function replaying(def: FamilyDef): FamilyDef {
  const cache = new Map<number, Float32Array>();
  return {
    ...def,
    render(out, sr, rng, v) {
      const hit = cache.get(v);
      if (hit) {
        out.set(hit.subarray(0, Math.min(hit.length, out.length)));
        return hit.length;
      }
      const n = def.render(out, sr, rng, v);
      cache.set(v, out.slice(0, Math.max(0, Math.min(Math.floor(Number.isFinite(n) ? n : 0), out.length))));
      return n;
    },
  };
}

/** Case `family <id>`: normalized variants measured one by one, the strip written as WAV. */
function familyCase(def: FamilyDef): CaseResult {
  const rec = replaying(def);
  const variants = renderFamilyVariants(rec, AUDIO_RATE, BASE_PRESET);
  const ms = variants.map((x) => measureMono(x, AUDIO_RATE));
  const strip = renderFamilyStrip(rec, BASE_PRESET, { sampleRate: AUDIO_RATE });
  const file = `families/${def.id}.wav`;
  writeWav(join(OUT, file), strip);
  const worst = worstOf(ms);
  const medianLufs = lowerMedian(ms.map((m) => m.lufs));
  const targetLufs = BASE_PRESET.loudness[def.category];
  const stripMeasure = measureStereo(strip.left, strip.right, AUDIO_RATE);
  // The report entry shows the worst variant values; lufs is the checked median.
  const measure: Measure = {
    ...stripMeasure,
    nonFinite: worst.nonFinite,
    samplePeakDb: worst.samplePeakDb,
    truePeakDb: Math.max(...ms.map((m) => m.truePeakDb)),
    dc: worst.dc,
    rmsDb: Math.min(...ms.map((m) => m.rmsDb)),
    minWindowRmsDb: worst.minRmsDb,
    lufs: medianLufs,
    shortTermMaxLufs: Math.max(...ms.map((m) => m.shortTermMaxLufs)),
  };
  return {
    group: 'families', id: def.id, file, frames: strip.left.length, channels: 2, worst, measure,
    medianLufs, targetLufs,
    note: `${def.variants} variants, worst per variant; LUFS = median after normalization; bands of the strip`,
  };
}

/** Case `bed wind`: the accel sweep 0 → 50 m/s, measured whole and per 1 s window. */
function windCase(a: ProofArgs): CaseResult {
  const seconds = a.full ? WIND_SECONDS_FULL : WIND_SECONDS;
  const iSpeed = WIND_BED.params.indexOf('speed');
  const iSpeed01 = WIND_BED.params.indexOf('speed01');
  const r = renderBedDemo(WIND_BED, {
    seconds,
    seed: hashSeed(1, 'bed', WIND_BED.id, 0),
    sampleRate: AUDIO_RATE,
    name: 'wind',
    script: (t, p) => {
      const speed = WIND_TOP_MS * Math.min(1, t / seconds);
      p[iSpeed] = speed;
      p[iSpeed01] = speed / WIND_TOP_MS;
    },
  });
  const file = 'beds/wind.wav';
  writeWav(join(OUT, file), r);
  const win = Math.round(WINDOW_SECONDS * AUDIO_RATE);
  const windows: Measure[] = [];
  for (let at = 0; at + win <= r.left.length; at += win) {
    windows.push(measureStereo(r.left.subarray(at, at + win), r.right.subarray(at, at + win), AUDIO_RATE));
  }
  const worst = worstOf(windows);
  const levels = windows.map((m, i) => `${i}s ${m.rmsDb.toFixed(1)}/${m.samplePeakDb.toFixed(1)}`).join(', ');
  return {
    group: 'beds', id: 'wind', file, frames: r.left.length, channels: 2, worst,
    measure: measureStereo(r.left, r.right, AUDIO_RATE),
    note: `accel sweep speed 0→${WIND_TOP_MS} m/s over ${seconds} s; window RMS/peak dBFS: ${levels}`,
  };
}

const CASES: ProofCase<CaseResult>[] = [
  ...BUILTIN_FAMILIES.map((def) => ({ id: `family ${def.id}`, label: `family ${def.id}`, run: () => familyCase(def) })),
  { id: 'bed wind', label: 'bed wind', run: (a: ProofArgs) => windCase(a) },
];

/** The unit name of a worst value: 'v<i>' for a variant, '<i>s' for a bed window. */
function unit(r: CaseResult, u: number): string {
  return r.group === 'families' ? `v${u}` : `${u}s`;
}

/** The result whose `key` is the largest (or smallest with `min`), with its value. */
function pick(rs: readonly CaseResult[], key: (r: CaseResult) => number, min = false): [CaseResult, number] {
  let best = rs[0];
  let bv = key(best);
  for (const r of rs) {
    const v = key(r);
    if (Number.isNaN(v) || (min ? v < bv : v > bv)) [best, bv] = [r, v];
    if (Number.isNaN(v)) break;
  }
  return [best, bv];
}

proofMain('m1', async (a) => {
  // A stale media record never outlives a new run (children leave it to the parent).
  if (a.caseId === null) rmSync(join(OUT, 'media.json'), { force: true });
  const results = [...(await runCases(SCRIPT, CASES, a)).values()];
  if (results.length === 0) throw new Error('no case selected');
  for (const r of results) {
    if (r.worst.failures.length > 0) console.log(`  ${r.id}: ${r.worst.failures.join(' ')}`);
  }

  const report: Report = { proof: 'm1', entries: [], checks };
  for (const r of results) {
    const e: ReportEntry = { group: r.group, id: r.id, file: r.file, title: `${r.group === 'families' ? 'family' : 'bed'} ${r.id}`, measure: r.measure, note: r.note };
    if (r.targetLufs !== undefined) e.targetLufs = r.targetLufs;
    report.entries.push(e);
  }
  let passed = false;
  try {
    // 1. Every WAV decodes with the expected rate, channels and length.
    let readable = 0;
    let bad = '';
    for (const r of results) {
      try {
        const w = decodeWav(readFileSync(join(OUT, r.file)));
        if (w.sampleRate === AUDIO_RATE && w.channels.length === r.channels && w.channels[0].length === r.frames) readable++;
        else bad ||= r.id;
      } catch {
        bad ||= r.id;
      }
    }
    check(LABELS.wav, readable, '==', results.length, 'files', bad || undefined);

    // 2. No NaN/Inf; worst |DC| over every variant and bed window.
    const nonFinite = results.reduce((s, r) => s + r.worst.nonFinite, 0);
    const [dcCase, dc] = pick(results, (r) => r.worst.dc);
    const finiteOk = results.every((r) => r.worst.failures.every((f) => !f.endsWith(':nonFinite') && !f.endsWith(':dc')));
    checkBool(LABELS.finite, finiteOk && nonFinite === 0 && dc < SAFETY.maxAbsDc,
      `${nonFinite} non-finite samples, worst |DC| ${dc.toExponential(2)} [${dcCase.id} ${unit(dcCase, dcCase.worst.dcUnit)}] (< ${SAFETY.maxAbsDc})`);

    // 3. Worst sample peak.
    const [peakCase, peak] = pick(results, (r) => r.worst.samplePeakDb);
    check(LABELS.peak, peak, '<=', SAFETY.maxSamplePeakDb, 'dBFS', `${peakCase.id} ${unit(peakCase, peakCase.worst.peakUnit)}`);

    // 4. Quietest variant or bed window.
    const [rmsCase, rms] = pick(results, (r) => r.worst.minRmsDb, true);
    check(LABELS.silence, rms, '>=', SAFETY.minRmsDb, 'dBFS', `${rmsCase.id} ${unit(rmsCase, rmsCase.worst.rmsUnit)}`);

    // 5. Families only: the largest |median LUFS - target|.
    const fams = results.filter((r) => r.group === 'families');
    if (fams.length === 0) {
      checkBool(LABELS.loudness, true, 'no family selected');
    } else {
      const [lc, dev] = pick(fams, (r) => Math.abs((r.medianLufs as number) - (r.targetLufs as number)));
      check(LABELS.loudness, dev, '<=', LOUDNESS_TOLERANCE_LU, 'LU',
        `${lc.id}: median ${(lc.medianLufs as number).toFixed(2)} vs ${lc.targetLufs} LUFS`);
    }

    passed = true;
  } finally {
    writeReport(OUT, report);
    console.log(`report ${relative(ROOT, join(OUT, 'index.html'))}`);
  }

  // Media record for the owner's gate: written only by a passing full run, after the report,
  // over every WAV and the report triplet (repo-root-relative paths, sorted).
  if (passed && a.full && a.only === null) {
    const rel = [...results.map((r) => r.file), 'index.html', 'report.md', 'report.json'];
    const files = rel.map((f) => ({ path: `out/m1/${f}`, bytes: statSync(join(OUT, f)).size, sha256: sha256File(join(OUT, f)) }));
    files.sort((x, y) => (x.path < y.path ? -1 : x.path > y.path ? 1 : 0));
    writeFileSync(join(OUT, 'media.json'), `${JSON.stringify({ proof: 'm1', full: a.full, files }, null, 2)}\n`);
    console.log(`media ${relative(ROOT, join(OUT, 'media.json'))} (${files.length} files)`);
  }
});
