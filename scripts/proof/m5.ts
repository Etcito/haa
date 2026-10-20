// Milestone 5 proof (module proof-m5, node): the spatial model against the shared known-geometry
// table, the 1/N per-listener rule for N = 1..4, the Doppler approach and recede ratios, no import
// of three anywhere, and the committed Chrome trace (proofs/m5/trace.json) of the beacon sweep for
// the current build. Cases: `render demo`, then the review captures `render sweep` and
// `render flyby` (out/m5/sweep.wav, flyby.wav and the report triplet in out/m5/).
//
//   node scripts/proof/m5.ts [--only <pattern>] [--full]
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { measureStereo } from '../../src/analysis/measure.ts';
import type { Measure } from '../../src/analysis/measure.ts';
import type { Report } from '../../src/analysis/report.ts';
import { SP, newSpatialResult } from '../../src/contract.ts';
import type { Listener, SourcePos, Vec3 } from '../../src/contract.ts';
import { writeWav } from '../../src/node/writeWav.ts';
import { createOfflineAudio } from '../../src/offline/scene.ts';
import { createSpatial } from '../../src/spatial/model.ts';
import { distanceGain, dopplerRatio, spatializeOne } from '../../src/spatial/spatial.ts';
import type { OneListener } from '../../src/spatial/spatial.ts';
import { buildDemo } from '../build-demo.ts';
import type { BuildInfo } from '../build-demo.ts';
import { writeReport } from './lib/out.ts';
import { check, checkBool, checks, proofMain, runCases, selected } from './lib/proof.ts';
import type { ProofArgs, ProofCase } from './lib/proof.ts';

const SCRIPT = fileURLToPath(import.meta.url);
const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');
const OUT = join(ROOT, 'out', 'm5');
const TRACE = join(ROOT, 'proofs', 'm5', 'trace.json');

const LABELS = {
  geometry: 'geometry: known attenuation table',
  listeners: 'listeners: each of N contributes 1/N',
  doppler: 'doppler: approach and recede ratios',
  package: 'package: no three import',
  browser: 'browser: moving source pans left to right',
} as const;

/** Tolerances and thresholds of the shared Q7 bar. */
const TABLE_TOL = 1e-4;
const LISTENER_TOL = 1e-6;
const DOPPLER_TOL = 1e-4;
const PAN_DB = 3;
const MIN_TRACE_POINTS = 8;
/** Offline captures: the demo trajectory's 20 ms update period and 4 s length. */
const STEP_S = 0.02;
const CAPTURE_S = 4;

/** proofs/m5/trace.json (shared 'Browser run results'; no exported type yet, so declared here). */
interface TraceResult {
  version: 1; build: BuildInfo; ctxState: string; sampleRate: number; action: string;
  points: { t: number; x: number; leftDb: number; rightDb: number }[]; captures: { file: string; sha256: string }[];
}

/** One review capture written by a render case. */
interface Capture { id: string; file: string; title: string; measure: Measure }

const fmt = (v: number): string => String(Number(v.toPrecision(4)));
const readJson = <T>(file: string): T => JSON.parse(readFileSync(file, 'utf8')) as T;
const vec = (x: number, y: number, z: number): Vec3 => ({ x, y, z });
const listenerAt = (x: number, y: number, z: number): Listener => ({ pos: vec(x, y, z), quat: { x: 0, y: 0, z: 0, w: 1 } });
const newOne = (): OneListener => ({ gain: 0, pan: 0, lowpass: 0, send: 0, doppler: 0, dist: 0 });

/** One expected value of the known-geometry table: case id, field and the shared value. */
interface TableRow { id: string; got: number; want: number }

/**
 * Every case of the shared known-geometry table (1 listener at the origin, identity quat,
 * absorptionScale 1). Single-listener rows go through spatializeOne in float64 and the model's
 * gain; the 2-listener row goes through createSpatial (1/N included).
 */
function geometryTable(): TableRow[] {
  const env = { absorptionScale: 1 };
  const origin = listenerAt(0, 0, 0);
  const one = (src: SourcePos): OneListener => spatializeOne(origin, src, 1, 0, env, newOne());
  const model = createSpatial();
  const res = newSpatialResult();
  const modelGain = (src: SourcePos): number => model.evaluate([origin], src, null, res).sp[SP.GAIN];
  const rows: TableRow[] = [];
  const add = (id: string, got: number, want: number): void => { rows.push({ id, got, want }); };
  const cases: { id: string; src: SourcePos; gain: number; pan?: number; lowpass?: number; send?: number }[] = [
    { id: '(0,0,-2)', src: { pos: vec(0, 0, -2) }, gain: 1, pan: 0, lowpass: 19900, send: 0.0916667 },
    { id: '(8,0,0)', src: { pos: vec(8, 0, 0) }, gain: 0.5, pan: 1, lowpass: 19600, send: 0.1266667 },
    { id: '(0,0,40)', src: { pos: vec(0, 0, 40) }, gain: 0.1, pan: 0, lowpass: 6300 },
    { id: '(0,0,-68)', src: { pos: vec(0, 0, -68) }, gain: 0.0294118 },
    { id: '(0,0,-2) occluded', src: { pos: vec(0, 0, -2), occlusion: 1 }, gain: 0.3548134, lowpass: 995 },
  ];
  for (const c of cases) {
    const r = one(c.src);
    add(`${c.id} gain`, r.gain, c.gain);
    add(`${c.id} model gain`, modelGain(c.src), c.gain);
    if (c.pan !== undefined) add(`${c.id} pan`, r.pan, c.pan);
    if (c.lowpass !== undefined) add(`${c.id} lowpass`, r.lowpass, c.lowpass);
    if (c.send !== undefined) add(`${c.id} send`, r.send, c.send);
  }
  const two = model.evaluate([origin, listenerAt(100, 0, 0)], { pos: vec(0, 0, -8) }, null, res);
  add('2 listeners g0', two.sp[SP.GAIN], 0.25);
  add('2 listeners g1', two.sp[SP.GAIN + 1], 0);
  add('2 listeners pan0', two.sp[SP.PAN], 0);
  return rows;
}

/** For N = 1..4: the max error of Σ null-source gains vs 1 and of each equidistant gain vs distanceGain/N. */
function listenerErrors(): { n: number; err: number }[] {
  const model = createSpatial();
  const res = newSpatialResult();
  const src: SourcePos = { pos: vec(0, 0, -8) };
  const out: { n: number; err: number }[] = [];
  for (let n = 1; n <= 4; n++) {
    const ls = Array.from({ length: n }, () => listenerAt(0, 0, 0));
    model.evaluate(ls, null, null, res);
    let sum = 0;
    for (let l = 0; l < n; l++) sum += res.sp[SP.GAIN + l];
    let err = Math.abs(sum - 1) + Math.abs(res.sp[SP.COUNT] - n);
    model.evaluate(ls, src, null, res);
    for (let l = 0; l < n; l++) err = Math.max(err, Math.abs(res.sp[SP.GAIN + l] - distanceGain(8) / n));
    out.push({ n, err });
  }
  return out;
}

/** Errors of the Doppler approach and recede ratios (dopplerRatio and spatializeOne). */
function dopplerErrors(): { id: string; err: number }[] {
  const origin = listenerAt(0, 0, 0);
  const env = { absorptionScale: 1 };
  const rows: [string, number, number][] = [['approach', 34.3, 1.0540926], ['recede', -34.3, 0.9534626]];
  const out: { id: string; err: number }[] = [];
  for (const [id, vz, want] of rows) {
    const src: SourcePos = { pos: vec(0, 0, -100), vel: vec(0, 0, vz) };
    out.push({ id, err: Math.abs(dopplerRatio(origin, src) - want) });
    out.push({ id: `${id} spatializeOne`, err: Math.abs(spatializeOne(origin, src, 1, 0, env, newOne()).doppler - want) });
  }
  return out;
}

/** The module name, assembled so this file's own source never matches the import scan. */
const THREE = ['th', 'ree'].join('');
const THREE_IMPORT = new RegExp(
  `(?:\\bfrom\\s*|\\bimport\\s*\\(?\\s*|\\brequire\\s*\\(\\s*)['"]${THREE}(?:/[^'"]*)?['"]`, 'g');
const SCAN_DIRS = ['src', 'demo', 'scripts'];
const SKIP_DIRS = new Set(['node_modules']);
const GENERATED = new Set([join(ROOT, 'worklet', 'haa-processors.js')]);

/** Every .ts/.js file under dir (node_modules and the generated worklet bundle excluded). */
function sourceFiles(dir: string): string[] {
  if (!existsSync(dir)) return [];
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (!SKIP_DIRS.has(name)) out.push(...sourceFiles(path));
    } else if (/\.(?:ts|js)$/.test(name) && !GENERATED.has(path)) out.push(path);
  }
  return out;
}

/** Imports of three in src, demo and scripts, plus three entries in package.json (dev)dependencies. */
function threeImports(): { count: number; files: number; where: string[] } {
  const where: string[] = [];
  let count = 0;
  let files = 0;
  for (const dir of SCAN_DIRS) {
    for (const file of sourceFiles(join(ROOT, dir))) {
      files++;
      const n = readFileSync(file, 'utf8').match(THREE_IMPORT)?.length ?? 0;
      if (n > 0) { count += n; where.push(relative(ROOT, file)); }
    }
  }
  const pkg = readJson<{ dependencies?: Record<string, string>; devDependencies?: Record<string, string> }>(join(ROOT, 'package.json'));
  for (const key of ['dependencies', 'devDependencies'] as const) {
    for (const name of [THREE, `@types/${THREE}`]) {
      if (pkg[key] && Object.hasOwn(pkg[key], name)) { count++; where.push(`package.json ${key}.${name}`); }
    }
  }
  return { count, files, where };
}

/**
 * Render the beacon along the demo trajectory (demo/panels/spatial.ts) through
 * createOfflineAudio + createSpatial: setPos every 20 ms for 4 s, listener at the origin.
 */
function renderBeacon(id: string, title: string, from: Vec3, to: Vec3, vel: Vec3): Capture {
  const audio = createOfflineAudio({ spatial: createSpatial() });
  audio.setListeners([listenerAt(0, 0, 0)]);
  const pos = vec(from.x, from.y, from.z);
  const bed = audio.startBed('beacon', { pos, vel });
  if (!bed) throw new Error('bed beacon not registered');
  const steps = Math.round(CAPTURE_S / STEP_S);
  for (let k = 0; k < steps; k++) {
    const f = Math.min(1, (k * STEP_S) / CAPTURE_S);
    pos.x = from.x + (to.x - from.x) * f;
    pos.y = from.y + (to.y - from.y) * f;
    pos.z = from.z + (to.z - from.z) * f;
    bed.setPos(pos, vel);
    audio.advance(STEP_S);
  }
  const r = audio.take(id);
  const file = `${id}.wav`;
  writeWav(join(OUT, file), r);
  return { id, file, title, measure: measureStereo(r.left, r.right, r.sampleRate) };
}

const DEMO_CASE: ProofCase<BuildInfo> = { id: 'render-demo', label: 'render demo', run: () => buildDemo('out/demo') };
const CAPTURE_CASES: ProofCase<Capture>[] = [
  {
    id: 'render-sweep', label: 'render sweep',
    run: () => renderBeacon('sweep', 'beacon sweep x -20 → +20 at z -5', vec(-20, 0, -5), vec(20, 0, -5), vec(10, 0, 0)),
  },
  {
    id: 'render-flyby', label: 'render flyby',
    run: () => renderBeacon('flyby', 'beacon fly-by z -60 → +60 at x 3 (Doppler)', vec(3, 0, -60), vec(3, 0, 60), vec(0, 0, 30)),
  },
];

/** Mean of rightDb - leftDb over points[from, to). */
function meanBalance(points: TraceResult['points'], from: number, to: number): number {
  let s = 0;
  for (let i = from; i < to; i++) s += points[i].rightDb - points[i].leftDb;
  return s / (to - from);
}

proofMain('m5', async (a) => {
  if (a.caseId !== null) {
    // A child: run exactly the case it was spawned for (runCases prints its result and exits).
    if (a.caseId === DEMO_CASE.id) await runCases(SCRIPT, [DEMO_CASE], a);
    else await runCases(SCRIPT, CAPTURE_CASES, a);
    return;
  }

  const captures = [...(await runCases(SCRIPT, CAPTURE_CASES, a)).values()];
  try {
    if (selected(LABELS.geometry, a)) {
      const rows = geometryTable();
      const worst = rows.reduce((w, r) => (Math.abs(r.got - r.want) > Math.abs(w.got - w.want) ? r : w));
      check(LABELS.geometry, Math.abs(worst.got - worst.want), '<=', TABLE_TOL, '', `${worst.id}, ${rows.length} values`);
    }

    if (selected(LABELS.listeners, a)) {
      const errs = listenerErrors();
      const worst = errs.reduce((w, e) => (e.err > w.err ? e : w));
      check(LABELS.listeners, worst.err, '<=', LISTENER_TOL, '', `N = ${worst.n}`);
    }

    if (selected(LABELS.doppler, a)) {
      const errs = dopplerErrors();
      const worst = errs.reduce((w, e) => (e.err > w.err ? e : w));
      check(LABELS.doppler, worst.err, '<=', DOPPLER_TOL, '', worst.id);
    }

    if (selected(LABELS.package, a)) {
      const r = threeImports();
      check(LABELS.package, r.count, '==', 0, 'imports',
        r.where.length ? r.where.join(', ') : `${r.files} files + package.json`);
    }

    const browser = selected(LABELS.browser, a);
    // The check selects its demo case; 'render demo' alone only builds it.
    const build = browser || selected(DEMO_CASE.label, a)
      ? (await runCases(SCRIPT, [DEMO_CASE], { ...a, only: null })).get(DEMO_CASE.id) as BuildInfo
      : null;
    if (browser && build) {
      if (!existsSync(TRACE)) checkBool(LABELS.browser, false, 'proofs/m5/trace.json missing: run the browser task');
      const trace = readJson<TraceResult>(TRACE);
      const problems: string[] = [];
      if (!isDeepStrictEqual(trace.build, build)) problems.push('build differs from a fresh buildDemo');
      if (trace.ctxState !== 'running') problems.push(`ctxState ${trace.ctxState}`);
      const pts = trace.points ?? [];
      if (pts.length < MIN_TRACE_POINTS) problems.push(`${pts.length} points (>= ${MIN_TRACE_POINTS})`);
      let detail = problems.join('; ');
      if (pts.length >= 4) {
        const q = Math.floor(pts.length / 4);
        const first = meanBalance(pts, 0, q);
        const last = meanBalance(pts, pts.length - q, pts.length);
        if (!(first <= -PAN_DB)) problems.push('first quarter not left');
        if (!(last >= PAN_DB)) problems.push('last quarter not right');
        detail = `${problems.length ? problems.join('; ') + '; ' : ''}ctx ${trace.ctxState}, ${pts.length} points, ` +
          `R-L first quarter ${fmt(first)} dB (<= -${PAN_DB}), last quarter ${fmt(last)} dB (>= +${PAN_DB})`;
      }
      checkBool(LABELS.browser, problems.length === 0, detail);
    }
  } finally {
    if (captures.length > 0) {
      const report: Report = {
        proof: 'm5',
        entries: captures.map((c) => ({ group: 'spatial', id: c.id, file: c.file, title: c.title, measure: c.measure })),
        checks,
      };
      writeReport(OUT, report);
    }
  }
});
