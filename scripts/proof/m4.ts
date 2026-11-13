// Milestone 4 proof (module proof-m4, node): the committed worklet bundle is fresh, the demo
// builds into out/demo, every core demo action sounds through createOfflineAudio, the loopback
// server serves the build with the shared MIME types and refuses traversal, and the committed
// Chrome levels (proofs/m4/levels.json) match the fresh build. Cases: `render demo`, then
// `family <id>`, `bed wind`, `music upbeat`, `music calm` (grouped in at most NMC_CPUS children).
//
//   node scripts/proof/m4.ts [--only <pattern>] [--full]
import { existsSync, readFileSync } from 'node:fs';
import { request } from 'node:http';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import type { BankManifest } from '../../src/contract.ts';
import { rmsDb, samplePeakDb } from '../../src/dsp/meter.ts';
import { createOfflineAudio } from '../../src/offline/scene.ts';
import { registerBuiltins } from '../../src/registry/builtins.ts';
import { listFamilies } from '../../src/registry/registry.ts';
import { PANEL } from '../../demo/panels/core.ts';
import type { DemoAction } from '../../demo/panel.ts';
import { buildDemo } from '../build-demo.ts';
import type { BuildInfo } from '../build-demo.ts';
import { buildWorkletText } from '../build-worklet.ts';
import { startServer } from '../serve.ts';
import { checkBool, proofMain, runCases, selected } from './lib/proof.ts';
import type { ProofArgs, ProofCase } from './lib/proof.ts';

const SCRIPT = fileURLToPath(import.meta.url);
const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');
const DEMO = join(ROOT, 'out', 'demo');
const LEVELS = join(ROOT, 'proofs', 'm4', 'levels.json');

const LABELS = {
  worklet: 'worklet: committed bundle is fresh',
  demo: 'demo: builds',
  offline: 'offline: every demo action sounds',
  server: 'server: serves the demo with correct types',
  browser: 'browser: committed Chrome levels',
} as const;

/** Thresholds of the shared Q6 bar (offline and in Chrome). */
const MIN_MAX_RMS_DB = -40;
const MAX_PEAK_DB = -1;
const MAX_BASELINE_DB = -70;
/** Level meter window and the wind drive step (the action's setInterval period). */
const RMS_WINDOW_S = 0.05;
const WIND_STEP_S = 0.05;
const WIND_TOP_MS = 40;
const WIND_SPEED01_MS = 50;

const DEMO_FILES = ['index.html', 'main.js', 'haa-processors.js', 'nmc.gpu.json', 'build.json', 'audio/manifest.json'];
/** The shared MIME table of scripts/serve.ts, per served path. */
const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.mp3': 'audio/mpeg' } as const;

/** Prefix of the grouped offline case ids: 'offline:<action id>,<action id>,...'. */
const GROUP = 'offline:';

/** proofs/m4/levels.json (shared 'Browser run results'; no exported type yet, so declared here). */
interface LevelsResult {
  version: 1; build: BuildInfo; ctxState: string; sampleRate: number; userAgent: string;
  actions: { id: string; baselineDb: number; maxRmsDb: number; peakDb: number }[]; captures: { file: string; sha256: string }[];
}

/** Level of one action rendered offline. */
interface ActionLevel { id: string; maxRmsDb: number; peakDb: number }

/** The --only label of a core action id ('family:x' → 'family x'). */
const actionLabel = (id: string): string => id.replace(':', ' ');
const fmt = (v: number): string => String(Number(v.toPrecision(4)));

function coreActions(): DemoAction[] {
  registerBuiltins();
  return PANEL.actions();
}

/** Max 50 ms RMS of the mono mix (the analyser's (L+R)/2 downmix) and the stereo sample peak. */
function levelOf(id: string, left: Float32Array, right: Float32Array, sr: number): ActionLevel {
  const mono = new Float32Array(left.length);
  for (let i = 0; i < mono.length; i++) mono[i] = 0.5 * (left[i] + right[i]);
  const win = Math.round(RMS_WINDOW_S * sr);
  let maxRmsDb = -200;
  for (let i = 0; i + win <= mono.length; i += win) maxRmsDb = Math.max(maxRmsDb, rmsDb(mono, i, i + win));
  return { id, maxRmsDb, peakDb: samplePeakDb(left, right) };
}

/** Run one action on a fresh offline scene for its seconds, then stop it. */
function runAction(action: DemoAction): ActionLevel {
  const audio = createOfflineAudio();
  let stop: (() => void) | void;
  if (action.id === 'bed:wind') {
    // The action's 50 ms setInterval cannot run offline: drive the same ramp in 50 ms steps.
    const bed = audio.startBed('wind');
    if (!bed) throw new Error('bed wind not registered');
    for (let t = 0; t < action.seconds - 1e-9; t += WIND_STEP_S) {
      const s = Math.min(WIND_TOP_MS, (t / 3) * WIND_TOP_MS);
      bed.setParam('speed', s);
      bed.setParam('speed01', s / WIND_SPEED01_MS);
      audio.advance(Math.min(WIND_STEP_S, action.seconds - t));
    }
    stop = () => bed.stop();
  } else {
    stop = action.run(audio);
    audio.advance(action.seconds);
  }
  if (stop) stop();
  const r = audio.take(action.id);
  return levelOf(action.id, r.left, r.right, r.sampleRate);
}

/** The offline cases: the selected actions split over at most NMC_CPUS children. */
function offlineCases(actions: readonly DemoAction[], a: ProofArgs): ProofCase<ActionLevel[]>[] {
  const all = selected(LABELS.offline, a);
  const picked = actions.filter((x) => all || selected(actionLabel(x.id), a));
  const n = Math.min(picked.length, Math.max(1, Math.floor(Number(process.env.NMC_CPUS) || 1)));
  const groups = Array.from({ length: n }, (_, k) => picked.filter((_, i) => i % n === k));
  return groups.map((g) => groupCase(g.map((x) => x.id), actions));
}

function groupCase(ids: readonly string[], actions: readonly DemoAction[]): ProofCase<ActionLevel[]> {
  return {
    id: GROUP + ids.join(','),
    label: ids.map(actionLabel).join(' + '),
    run: () => ids.map((id) => {
      const action = actions.find((x) => x.id === id);
      if (!action) throw new Error(`unknown action ${id}`);
      return runAction(action);
    }),
  };
}

/** GET a path verbatim (no URL normalization) and resolve its status, type and body. */
function get(port: number, path: string): Promise<{ status: number; type: string; body: Buffer }> {
  return new Promise((resolve, reject) => {
    const req = request({ host: '127.0.0.1', port, path, method: 'GET' }, (res) => {
      const chunks: Buffer[] = [];
      res.on('data', (c: Buffer) => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode ?? 0, type: String(res.headers['content-type'] ?? ''), body: Buffer.concat(chunks) }));
      res.on('error', reject);
    });
    req.on('error', reject);
    req.end();
  });
}

const readJson = <T>(file: string): T => JSON.parse(readFileSync(file, 'utf8')) as T;

const DEMO_CASE: ProofCase<BuildInfo> = { id: 'render-demo', label: 'render demo', run: () => buildDemo('out/demo') };

proofMain('m4', async (a) => {
  const actions = coreActions();
  if (a.caseId !== null) {
    // A child: rebuild exactly the case it was spawned for (runCases prints its result and exits).
    const ids = a.caseId.startsWith(GROUP) ? a.caseId.slice(GROUP.length).split(',') : null;
    if (ids) await runCases(SCRIPT, [groupCase(ids, actions)], a);
    else await runCases(SCRIPT, [DEMO_CASE], a);
    return;
  }
  // The cases are selected here (a check label selects all of its cases), so runCases runs them all.
  const every: ProofArgs = { ...a, only: null };

  if (selected(LABELS.worklet, a)) {
    const fresh = (await buildWorkletText()) === readFileSync(join(ROOT, 'worklet/haa-processors.js'), 'utf8');
    checkBool(LABELS.worklet, fresh, fresh ? 'worklet/haa-processors.js equals a fresh build' : 'worklet/haa-processors.js is stale; run node scripts/build-worklet.ts');
  }

  if (selected(LABELS.demo, a) || selected(DEMO_CASE.label, a)) {
    await runCases(SCRIPT, [DEMO_CASE], every);
    const missing = DEMO_FILES.filter((f) => !existsSync(join(DEMO, f)));
    const families = missing.length ? -1 : readJson<BankManifest>(join(DEMO, 'audio/manifest.json')).families.length;
    const want = listFamilies().length;
    checkBool(LABELS.demo, missing.length === 0 && families === want,
      missing.length ? `missing ${missing.join(', ')}` : `${DEMO_FILES.length} files, manifest families ${families} (== ${want})`);
  }

  const cases = offlineCases(actions, a);
  if (cases.length > 0) {
    const levels = [...(await runCases(SCRIPT, cases, every)).values()].flat();
    const quiet = levels.reduce((w, x) => (x.maxRmsDb < w.maxRmsDb ? x : w));
    const loud = levels.reduce((w, x) => (x.peakDb > w.peakDb ? x : w));
    checkBool(LABELS.offline, quiet.maxRmsDb >= MIN_MAX_RMS_DB && loud.peakDb <= MAX_PEAK_DB,
      `${levels.length} actions, worst max RMS ${fmt(quiet.maxRmsDb)} dBFS [${quiet.id}] (>= ${MIN_MAX_RMS_DB}), ` +
      `worst peak ${fmt(loud.peakDb)} dBFS [${loud.id}] (<= ${MAX_PEAK_DB})`);
  }

  if (selected(LABELS.server, a)) {
    const manifest = readJson<BankManifest>(join(DEMO, 'audio/manifest.json'));
    const mp3 = manifest.families[0]?.file;
    const paths: [string, string][] = [
      ['/', MIME['.html']], ['/main.js', MIME['.js']], ['/haa-processors.js', MIME['.js']],
      ['/audio/manifest.json', MIME['.json']], [`/audio/${mp3}`, MIME['.mp3']],
    ];
    const server = await startServer(DEMO, 0);
    const bad: string[] = [];
    let traversal = 0;
    try {
      for (const [path, type] of paths) {
        const r = await get(server.port, path);
        if (!mp3 || r.status !== 200 || r.type !== type) bad.push(`${path} ${r.status} ${r.type}`);
      }
      const r = await get(server.port, '/../package.json');
      traversal = r.status;
      const pkg = readFileSync(join(ROOT, 'package.json'));
      if ((r.status !== 403 && r.status !== 404) || r.body.equals(pkg)) bad.push(`/../package.json ${r.status}`);
    } finally {
      await server.close();
    }
    checkBool(LABELS.server, bad.length === 0,
      bad.length ? `wrong: ${bad.join('; ')}` : `${paths.length} paths 200 with their MIME types, /../package.json ${traversal}`);
  }

  if (selected(LABELS.browser, a)) {
    if (!existsSync(LEVELS)) checkBool(LABELS.browser, false, 'proofs/m4/levels.json missing: run the browser task');
    const res = readJson<LevelsResult>(LEVELS);
    const buildFile = join(DEMO, 'build.json');
    if (!existsSync(buildFile)) checkBool(LABELS.browser, false, 'out/demo/build.json missing: run the demo check');
    const build = readJson<BuildInfo>(buildFile);
    const ids = res.actions.map((x) => x.id);
    const want = actions.map((x) => x.id);
    const problems: string[] = [];
    if (!isDeepStrictEqual(res.build, build)) problems.push('build differs from out/demo/build.json');
    if (res.ctxState !== 'running') problems.push(`ctxState ${res.ctxState}`);
    if (!isDeepStrictEqual(ids, want)) problems.push(`action ids differ from the ${want.length} core actions`);
    if (res.actions.length === 0) problems.push('no actions');
    const worst = (key: 'maxRmsDb' | 'baselineDb' | 'peakDb', low: boolean) =>
      res.actions.reduce((w, x) => ((low ? x[key] < w[key] : x[key] > w[key]) ? x : w), res.actions[0]);
    let detail = problems.join('; ');
    if (res.actions.length > 0) {
      const quiet = worst('maxRmsDb', true);
      const noisy = worst('baselineDb', false);
      const loud = worst('peakDb', false);
      if (!(quiet.maxRmsDb >= MIN_MAX_RMS_DB)) problems.push('max RMS too low');
      if (!(noisy.baselineDb <= MAX_BASELINE_DB)) problems.push('baseline too high');
      if (!(loud.peakDb <= MAX_PEAK_DB)) problems.push('peak too high');
      detail = `${problems.length ? problems.join('; ') + '; ' : ''}ctx ${res.ctxState}, ${ids.length} actions, ` +
        `worst max RMS ${fmt(quiet.maxRmsDb)} dBFS [${quiet.id}] (>= ${MIN_MAX_RMS_DB}), ` +
        `worst baseline ${fmt(noisy.baselineDb)} dBFS [${noisy.id}] (<= ${MAX_BASELINE_DB}), ` +
        `worst peak ${fmt(loud.peakDb)} dBFS [${loud.id}] (<= ${MAX_PEAK_DB})`;
    }
    checkBool(LABELS.browser, problems.length === 0, detail);
  }
});
