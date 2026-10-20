// Milestone 6 proof (module fiesta-race, node): every Royal Fiesta and crowd family rendered
// with FIESTA_PRESET, measured and written to out/m6/families/<id>.wav, plus the crowd bed demo
// (out/m6/beds/crowd.wav), the scripted 60 s race (out/m6/fiesta-race.wav and its event log
// out/m6/fiesta-race.events.json), then the aggregated CHECKs and the report triplet in out/m6/
// (the race as 6 entries of 10 s, each with its octave bands and its events).
// Cases: `family <id>` (one per FIESTA_FAMILIES and CROWD_FAMILIES family), `bed crowd`,
// `render fiesta-race` (1 listener) and, with --full, `render fiesta-race 2 listeners`.
//
//   node --conditions=haa-source examples/royal-fiesta-pack/scripts/proof-m6.ts [--only <pattern>] [--full]
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderBedDemo, renderFamilyStrip, renderFamilyVariants, hashSeed, measureMono, measureStereo, safetyVerdict, AUDIO_RATE, LOUDNESS_TOLERANCE_LU, SAFETY, type FamilyDef, type Measure, type Report, type ReportEntry } from '@etcito/haa';
import { writeWav } from '@etcito/haa/node';
import { CROWD_BED, CROWD_FAMILIES, FIESTA_FAMILIES, FIESTA_PRESET } from '../src/index.ts';
import { FIESTA_RACE } from './raceScript.ts';
import type { RaceEvent } from './raceScript.ts';
import { renderFiestaRace } from './fiesta-race.ts';
import { writeReport } from '../../../scripts/proof/lib/out.ts';
import { ProofFail, check, checkBool, checks, proofMain, runCases } from '../../../scripts/proof/lib/proof.ts';
import type { ProofCase } from '../../../scripts/proof/lib/proof.ts';

const SCRIPT = fileURLToPath(import.meta.url);
const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const OUT = join(ROOT, 'out', 'm6');

/** Crowd bed demo: excitement 0 → 1 over CROWD_SECONDS, swell 1 in [SWELL_FROM, SWELL_TO). */
const CROWD_SECONDS = 12;
const SWELL_FROM = 8.0;
const SWELL_TO = 9.5;
/** Window of the bed safety verdicts (shared Q1: per 1 s window for renders of 2 s or more). */
const WINDOW_SECONDS = 1;

const LABELS = {
  safety: 'fiesta families: safety',
  loudness: 'fiesta families: loudness within ±1 LU of category target',
  raceSafety: 'race: safety',
  raceLoudness: 'race: integrated loudness',
  racePlayed: 'race: every scripted sound played',
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
  /** The written WAV, relative to out/m6. */
  file: string;
  frames: number;
  worst: Worst;
  /** The entry measure for the report. */
  measure: Measure;
  /** Families: the lower median of the normalized variant LUFS, and its category target. */
  medianLufs?: number;
  targetLufs?: number;
  note: string;
}

/** Race report windows (s) and the control rate of scripts/fiesta-race.ts (event log field). */
const RACE_WINDOW_SECONDS = 10;
const RACE_CONTROL_HZ = 60;

/** One 10 s window of a race render, as a report entry. */
interface RaceWindow {
  id: string;
  measure: Measure;
  note: string;
}

/** The @@RESULT of a race render case. */
interface RaceResult {
  group: 'race';
  id: string;
  listeners: 1 | 2;
  file: string;
  frames: number;
  /** Worst safety values over the 1 s windows. */
  worst: Worst;
  /** The whole render. */
  measure: Measure;
  stats: { played: number; dropped: number; stolen: number };
  scheduled: number;
  /** The 10 s report windows (1 listener only). */
  windows: RaceWindow[];
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

/** Case `family <id>`: normalized variants (FIESTA_PRESET) measured one by one, the strip written as WAV. */
function familyCase(def: FamilyDef): CaseResult {
  const rec = replaying(def);
  const variants = renderFamilyVariants(rec, AUDIO_RATE, FIESTA_PRESET);
  const ms = variants.map((x) => measureMono(x, AUDIO_RATE));
  const strip = renderFamilyStrip(rec, FIESTA_PRESET, { sampleRate: AUDIO_RATE });
  const file = `families/${def.id}.wav`;
  writeWav(join(OUT, file), strip);
  const worst = worstOf(ms);
  const medianLufs = lowerMedian(ms.map((m) => m.lufs));
  const targetLufs = FIESTA_PRESET.loudness[def.category];
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
    group: 'families', id: def.id, file, frames: strip.left.length, worst, measure, medianLufs, targetLufs,
    note: `${def.variants} variants (${def.category}), worst per variant; LUFS = median after normalization; bands of the strip`,
  };
}

/** Case `bed crowd`: excitement 0 → 1 over 12 s with a swell, measured whole and per 1 s window. */
function crowdCase(): CaseResult {
  const iExcitement = CROWD_BED.params.indexOf('excitement');
  const iSwell = CROWD_BED.params.indexOf('swell');
  const r = renderBedDemo(CROWD_BED, {
    seconds: CROWD_SECONDS,
    seed: hashSeed(1, 'bed', CROWD_BED.id, 0),
    sampleRate: AUDIO_RATE,
    name: 'crowd',
    script: (t, p) => {
      p[iExcitement] = Math.min(1, t / CROWD_SECONDS);
      p[iSwell] = t >= SWELL_FROM && t < SWELL_TO ? 1 : 0;
    },
  });
  const file = 'beds/crowd.wav';
  writeWav(join(OUT, file), r);
  const win = Math.round(WINDOW_SECONDS * AUDIO_RATE);
  const windows: Measure[] = [];
  for (let at = 0; at + win <= r.left.length; at += win) {
    windows.push(measureStereo(r.left.subarray(at, at + win), r.right.subarray(at, at + win), AUDIO_RATE));
  }
  const worst = worstOf(windows);
  const levels = windows.map((m, i) => `${i}s ${m.rmsDb.toFixed(1)}/${m.samplePeakDb.toFixed(1)}`).join(', ');
  return {
    group: 'beds', id: 'crowd', file, frames: r.left.length, worst,
    measure: measureStereo(r.left, r.right, AUDIO_RATE),
    note: `excitement 0→1 over ${CROWD_SECONDS} s, swell ${SWELL_FROM}–${SWELL_TO} s, size/level default; window RMS/peak dBFS: ${levels}`,
  };
}

/** Measures of consecutive windows of `seconds` (a shorter tail is dropped). */
function windowsOf(l: Float32Array, r: Float32Array, seconds: number): Measure[] {
  const win = Math.round(seconds * AUDIO_RATE);
  const out: Measure[] = [];
  for (let at = 0; at + win <= l.length; at += win) {
    out.push(measureStereo(l.subarray(at, at + win), r.subarray(at, at + win), AUDIO_RATE));
  }
  return out;
}

/** One event in the CLI log's fixed key order, without absent keys (as scripts/fiesta-race.ts). */
function logEvent(e: RaceEvent): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const key of ['t', 'kind', 'family', 'player', 'gain', 'duck', 'strength', 'param', 'value', 'style'] as const) {
    if (e[key] !== undefined) out[key] = e[key];
  }
  return out;
}

/** An event as 't kind id p<player>' for the window notes. */
function eventText(e: RaceEvent): string {
  const id = e.family ?? e.duck ?? e.style ?? e.param ?? '';
  return `${e.t.toFixed(2)} ${e.kind} ${id}${e.player === undefined ? '' : ` p${e.player}`}`;
}

/** Case `render fiesta-race[ 2 listeners]`: the 60 s race, measured whole, per 1 s and per 10 s. */
function raceCase(listeners: 1 | 2): RaceResult {
  const { render, stats, scheduled } = renderFiestaRace(listeners);
  const file = listeners === 2 ? 'fiesta-race-2l.wav' : 'fiesta-race.wav';
  writeWav(join(OUT, file), render);
  if (listeners === 1) {
    const doc = {
      name: 'fiesta-race',
      listeners,
      seconds: FIESTA_RACE.seconds,
      players: FIESTA_RACE.players,
      seed: FIESTA_RACE.seed,
      controlHz: RACE_CONTROL_HZ,
      scheduled,
      stats: { played: stats.played, dropped: stats.dropped, stolen: stats.stolen },
      events: FIESTA_RACE.events.map(logEvent),
    };
    mkdirSync(OUT, { recursive: true });
    writeFileSync(join(OUT, 'fiesta-race.events.json'), `${JSON.stringify(doc, null, 2)}\n`);
  }
  const worst = worstOf(windowsOf(render.left, render.right, WINDOW_SECONDS));
  const windows = listeners !== 1 ? [] : windowsOf(render.left, render.right, RACE_WINDOW_SECONDS).map((measure, w) => {
    const from = w * RACE_WINDOW_SECONDS;
    const to = from + RACE_WINDOW_SECONDS;
    const evs = FIESTA_RACE.events.filter((e) => e.t >= from && e.t < to).map(eventText);
    return { id: `fiesta-race ${from}-${to} s`, measure, note: `events: ${evs.length === 0 ? 'none' : evs.join(', ')}` };
  });
  return {
    group: 'race', id: listeners === 2 ? 'fiesta-race 2 listeners' : 'fiesta-race', listeners, file,
    frames: render.left.length, worst, measure: measureStereo(render.left, render.right, AUDIO_RATE),
    stats: { played: stats.played, dropped: stats.dropped, stolen: stats.stolen }, scheduled, windows,
  };
}

const FAMILIES: readonly FamilyDef[] = [...FIESTA_FAMILIES, ...CROWD_FAMILIES];

const FULL = process.argv.includes('--full');

const CASES: ProofCase<CaseResult | RaceResult>[] = [
  ...FAMILIES.map((def) => ({ id: `family ${def.id}`, label: `family ${def.id}`, run: () => familyCase(def) })),
  { id: 'bed crowd', label: 'bed crowd', run: () => crowdCase() },
  { id: 'render fiesta-race', label: 'render fiesta-race', run: () => raceCase(1) },
  ...(FULL ? [{ id: 'render fiesta-race 2 listeners', label: 'render fiesta-race 2 listeners', run: () => raceCase(2) }] : []),
];

/** The unit name of a worst value: 'v<i>' for a variant, '<i>s' for a bed or race window. */
function unit(r: CaseResult | RaceResult, u: number): string {
  return r.group === 'families' ? `v${u}` : `${u}s`;
}

/** The result whose `key` is the largest (or smallest with `min`), with its value. */
function pick<T>(rs: readonly T[], key: (r: T) => number, min = false): [T, number] {
  let best = rs[0];
  let bv = key(best);
  for (const r of rs) {
    const v = key(r);
    if (Number.isNaN(v) || (min ? v < bv : v > bv)) [best, bv] = [r, v];
    if (Number.isNaN(v)) break;
  }
  return [best, bv];
}

proofMain('m6', async (a) => {
  const all = [...(await runCases(SCRIPT, CASES, a)).values()];
  if (all.length === 0) throw new Error('no case selected');
  const results = all.filter((r): r is CaseResult => r.group !== 'race');
  const races = all.filter((r): r is RaceResult => r.group === 'race');
  for (const r of all) {
    if (r.worst.failures.length > 0) console.log(`  ${r.id}: ${r.worst.failures.join(' ')}`);
  }

  const report: Report = { proof: 'm6', entries: [], checks };
  for (const r of results) {
    const e: ReportEntry = { group: r.group, id: r.id, file: r.file, title: `${r.group === 'families' ? 'family' : 'bed'} ${r.id}`, measure: r.measure, note: r.note };
    if (r.targetLufs !== undefined) e.targetLufs = r.targetLufs;
    report.entries.push(e);
  }
  for (const r of races) {
    if (r.windows.length > 0) {
      for (const w of r.windows) {
        report.entries.push({ group: 'race windows', id: w.id, file: r.file, title: `race ${w.id}`, measure: w.measure, note: w.note });
      }
    } else {
      report.entries.push({
        group: 'race', id: r.id, file: r.file, title: `render ${r.id}`, measure: r.measure, targetLufs: FIESTA_PRESET.sceneLufs,
        note: `${r.listeners} listeners; played ${r.stats.played}/${r.scheduled}, dropped ${r.stats.dropped}, stolen ${r.stats.stolen}`,
      });
    }
  }
  // Every selected check prints before the run exits 1: the first ProofFail is kept and
  // rethrown after the last check, so one failing check does not hide the others.
  let firstFail: unknown = null;
  const run = (body: () => void): void => {
    try {
      body();
    } catch (e) {
      if (!(e instanceof ProofFail)) throw e;
      firstFail ??= e;
    }
  };
  try {
    const famsAndBed = results.filter((r) => r.group === 'families' || r.id === 'crowd');
    const fams = results.filter((r) => r.group === 'families');

    // 1. Safety over every family variant and crowd bed window: NaN/Inf, |DC|, peak, silence.
    if (famsAndBed.length > 0) run(() => {
      const nonFinite = famsAndBed.reduce((s, r) => s + r.worst.nonFinite, 0);
      const [dcCase, dc] = pick(famsAndBed, (r) => r.worst.dc);
      const [peakCase, peak] = pick(famsAndBed, (r) => r.worst.samplePeakDb);
      const [rmsCase, rms] = pick(famsAndBed, (r) => r.worst.minRmsDb, true);
      const ok = famsAndBed.every((r) => r.worst.failures.length === 0) && nonFinite === 0 &&
        dc < SAFETY.maxAbsDc && peak <= SAFETY.maxSamplePeakDb && rms >= SAFETY.minRmsDb;
      checkBool(LABELS.safety, ok,
        `${nonFinite} non-finite samples; worst |DC| ${dc.toExponential(2)} [${dcCase.id} ${unit(dcCase, dcCase.worst.dcUnit)}] (< ${SAFETY.maxAbsDc}); ` +
        `worst sample peak ${peak.toFixed(2)} dBFS [${peakCase.id} ${unit(peakCase, peakCase.worst.peakUnit)}] (<= ${SAFETY.maxSamplePeakDb}); ` +
        `min RMS ${rms.toFixed(2)} dBFS [${rmsCase.id} ${unit(rmsCase, rmsCase.worst.rmsUnit)}] (>= ${SAFETY.minRmsDb})`);
    });

    // 2. Families only: the largest |lower-median LUFS - FIESTA_PRESET.loudness[category]|.
    if (fams.length > 0) run(() => {
      const [lc, dev] = pick(fams, (r) => Math.abs((r.medianLufs as number) - (r.targetLufs as number)));
      check(LABELS.loudness, dev, '<=', LOUDNESS_TOLERANCE_LU, 'LU',
        `${lc.id}: median ${(lc.medianLufs as number).toFixed(2)} vs ${lc.targetLufs} LUFS`);
    });

    // 3. Race safety over every race render and its 1 s windows: NaN/Inf, |DC|, peak, no silent second.
    if (races.length > 0) run(() => {
      const nonFinite = races.reduce((s, r) => s + r.worst.nonFinite, 0);
      const [dcCase, dc] = pick(races, (r) => r.worst.dc);
      const [peakCase, peak] = pick(races, (r) => r.worst.samplePeakDb);
      const [rmsCase, rms] = pick(races, (r) => r.worst.minRmsDb, true);
      const ok = races.every((r) => r.worst.failures.length === 0) && nonFinite === 0 &&
        dc < SAFETY.maxAbsDc && peak <= SAFETY.maxSamplePeakDb && rms >= SAFETY.minRmsDb;
      checkBool(LABELS.raceSafety, ok,
        `${nonFinite} non-finite samples; worst |DC| ${dc.toExponential(2)} [${dcCase.id} ${unit(dcCase, dcCase.worst.dcUnit)}] (< ${SAFETY.maxAbsDc}); ` +
        `worst sample peak ${peak.toFixed(2)} dBFS [${peakCase.id} ${unit(peakCase, peakCase.worst.peakUnit)}] (<= ${SAFETY.maxSamplePeakDb}); ` +
        `min 1 s window RMS ${rms.toFixed(2)} dBFS [${rmsCase.id} ${unit(rmsCase, rmsCase.worst.rmsUnit)}] (>= ${SAFETY.minRmsDb})`);
    });

    // 4. Race integrated loudness: the largest |LUFS - FIESTA_PRESET.sceneLufs| over the race renders.
    if (races.length > 0) run(() => {
      const [lc, dev] = pick(races, (r) => Math.abs(r.measure.lufs - FIESTA_PRESET.sceneLufs));
      check(LABELS.raceLoudness, dev, '<=', LOUDNESS_TOLERANCE_LU, 'LU',
        `${lc.id}: ${lc.measure.lufs.toFixed(2)} vs ${FIESTA_PRESET.sceneLufs} LUFS`);
    });

    // 5. Every scripted play played: dropped 0 and played == scheduled, per race render.
    if (races.length > 0) run(() => {
      const ok = races.every((r) => r.stats.dropped === 0 && r.stats.played === r.scheduled);
      checkBool(LABELS.racePlayed, ok, races.map((r) =>
        `${r.id}: played ${r.stats.played} / scheduled ${r.scheduled}, dropped ${r.stats.dropped}, stolen ${r.stats.stolen}`).join('; '));
    });

    if (firstFail !== null) throw firstFail;
  } finally {
    writeReport(OUT, report);
    console.log(`report ${relative(ROOT, join(OUT, 'index.html'))}`);
  }
});
