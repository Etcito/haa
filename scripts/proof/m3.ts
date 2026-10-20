// Milestone 3 proof (module proof-m3, node): the built-in music rendered offline through a
// MixChain(BASE_PRESET) on the music bus input at DEFAULT_USER_VOLUMES, written to out/m3/*.wav
// (upbeat, calm, arc-upbeat, crossfade) and out/m3/stings/<id>.wav, then the aggregated safety
// CHECK and the report triplet in out/m3/. Cases: `render <name>` (one child process each).
//
//   node scripts/proof/m3.ts [--only <pattern>] [--full]
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AUDIO_RATE, BLOCK, BUS_INDEX, DEFAULT_USER_VOLUMES, MIX_INPUTS, SAFETY } from '../../src/contract.ts';
import type { FamilyDef, StereoRender } from '../../src/contract.ts';
import { measureStereo, safetyVerdict } from '../../src/analysis/measure.ts';
import type { Measure } from '../../src/analysis/measure.ts';
import type { Report } from '../../src/analysis/report.ts';
import { MUSIC_BED } from '../../src/content/musicBed.ts';
import { STING_FAMILIES } from '../../src/content/stings.ts';
import { hashSeed } from '../../src/dsp/rng.ts';
import { MixChain } from '../../src/mix/mixChain.ts';
import { BASE_PRESET } from '../../src/mix/presets/base.ts';
import { Arranger } from '../../src/music/arranger.ts';
import type { MusicState, NoteEvent } from '../../src/music/arranger.ts';
import { pcmHash } from '../../src/bank/pack.ts';
import { writeWav } from '../../src/node/writeWav.ts';
import { renderFamilyStrip, renderOffline } from '../../src/offline/render.ts';
import { registerBuiltins } from '../../src/registry/builtins.ts';
import { listStyles, styleIndex } from '../../src/registry/registry.ts';
import { writeReport } from './lib/out.ts';
import { check, checks, proofMain, runCases, selected } from './lib/proof.ts';
import type { ProofArgs, ProofCase } from './lib/proof.ts';

const SCRIPT = fileURLToPath(import.meta.url);
const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..', '..');
const OUT = join(ROOT, 'out', 'm3');

/** Steady style renders: length (quick / --full) and state. */
const STYLE_SECONDS = 20;
const STYLE_SECONDS_FULL = 60;
const STYLE_INTENSITY = 0.6;
/** Arc: intro → main (intensity ramp) → climax → resolveWin → post, in seconds. */
const ARC = { intro: 4, main: 12, climax: 8, resolveWin: 6, post: 4 } as const;
const ARC_INTENSITY_FROM = 0.3;
const ARC_INTENSITY_TO = 0.9;
/** Crossfade: upbeat for CROSSFADE_AT s, then calm over CROSSFADE_BARS bars, CROSSFADE_SECONDS total. */
const CROSSFADE_AT = 8;
const CROSSFADE_BARS = 2;
const CROSSFADE_SECONDS = 20;

/** Notes in scale: Arranger seed and length; sections at bar lines (intro, main, climax, then resolveWin → post). */
const SCALE_SEED = 1;
const SCALE_BARS = 64;
const SCALE_SECTIONS = { intro: 4, climaxAt: 44, climax: 8 } as const;
const SCALE_INTENSITY_FROM = 0.2;
const SCALE_INTENSITY_TO = 0.9;
/** Determinism: upbeat renders of DET_SECONDS at the same seed twice, and one at another seed. */
const DET_SECONDS = 4;
const DET_SEED = 7;
const DET_SEED_OTHER = 8;
/** Crossfade landing: the request moment of the crossfade case (step 8 of bar 6), and the bound. */
const LAND_BAR = 6;
const LAND_STEP = 8;
const LAND_MAX_BARS = 3;
/** 16th steps per bar of the Arranger. */
const BAR_STEPS = 16;

const LABELS = {
  safety: 'safety: music renders',
  scale: 'music: notes in scale',
  deterministic: 'music: deterministic',
  crossfade: 'music: crossfade lands',
} as const;

/** One written WAV of a case, with its measure and safetyVerdict failures. */
interface RenderEntry {
  group: 'music' | 'stings';
  id: string;
  /** The written WAV, relative to out/m3. */
  file: string;
  title: string;
  measure: Measure;
  failures: string[];
  note: string;
}

/** The @@RESULT of one case. */
interface CaseResult {
  id: string;
  entries: RenderEntry[];
  /** Crossfade only: the first frame of the block where the calm style was requested. */
  requestFrame?: number;
}

/** Index of a MUSIC_BED param (throws when absent, so a renamed param never goes unnoticed). */
function param(name: string): number {
  const i = MUSIC_BED.params.indexOf(name);
  if (i < 0) throw new Error(`no music param ${name}`);
  return i;
}

/** The style code of a registered style id (never a hard-coded number). */
function style(id: string): number {
  const i = styleIndex(id);
  if (i < 0) throw new Error(`style ${id} not registered`);
  return i;
}

/** A fresh MixChain(BASE_PRESET) at DEFAULT_USER_VOLUMES. */
function chain(): MixChain {
  const mix = new MixChain(AUDIO_RATE, BASE_PRESET);
  mix.setUser(DEFAULT_USER_VOLUMES.master, DEFAULT_USER_VOLUMES.music, DEFAULT_USER_VOLUMES.sfx);
  return mix;
}

/**
 * Mix inputs with only the music bus wired to (l, r); every other input is a silent pair.
 * The silent pairs are never written, so they stay zero.
 */
function musicInputs(l: Float32Array, r: Float32Array): (readonly [Float32Array, Float32Array])[] {
  const silent = [new Float32Array(BLOCK), new Float32Array(BLOCK)] as const;
  const inputs: (readonly [Float32Array, Float32Array])[] = [];
  for (let i = 0; i < MIX_INPUTS; i++) inputs.push(i === BUS_INDEX.music ? [l, r] : silent);
  return inputs;
}

/**
 * The music bed through the mix: one MusicSynth from MUSIC_BED.create, p = a copy of
 * MUSIC_BED.defaults mutated by script(t, p) before every block, its stereo block fed into the
 * music bus input of a MixChain(BASE_PRESET).
 */
function renderMusic(name: string, seconds: number, seed: number, script: (t: number, p: Float32Array) => void): StereoRender {
  const bed = MUSIC_BED.create(AUDIO_RATE, seed);
  const p = Float32Array.from(MUSIC_BED.defaults);
  const mix = chain();
  const bl = new Float32Array(BLOCK);
  const br = new Float32Array(BLOCK);
  const inputs = musicInputs(bl, br);
  return renderOffline((l, r, n, frame) => {
    script(frame / AUDIO_RATE, p);
    bl.fill(0);
    br.fill(0);
    bed.process(p, 0, bl, br, n);
    mix.process(inputs, l, r, n);
  }, seconds, AUDIO_RATE, name);
}

/** A stereo render fed through the mix chain on the music bus input. */
function mixRender(src: StereoRender, name: string): StereoRender {
  const mix = chain();
  const bl = new Float32Array(BLOCK);
  const br = new Float32Array(BLOCK);
  const inputs = musicInputs(bl, br);
  return renderOffline((l, r, n, frame) => {
    bl.set(src.left.subarray(frame, frame + n));
    br.set(src.right.subarray(frame, frame + n));
    mix.process(inputs, l, r, n);
  }, src.left.length / AUDIO_RATE, AUDIO_RATE, name);
}

/** Write one render and measure it. */
function entry(group: RenderEntry['group'], id: string, file: string, title: string, r: StereoRender, note: string): RenderEntry {
  writeWav(join(OUT, file), r);
  const measure = measureStereo(r.left, r.right, AUDIO_RATE);
  return { group, id, file, title, measure, failures: safetyVerdict(measure).failures, note };
}

/** The music seed of a render (the bed instance 0 seed of audio seed 1). */
const MUSIC_SEED = hashSeed(1, 'bed', MUSIC_BED.id, 0);

/** Case `render upbeat` / `render calm`: a steady render at intensity 0.6, phase 1. */
function styleCase(id: string, a: ProofArgs): CaseResult {
  const seconds = a.full ? STYLE_SECONDS_FULL : STYLE_SECONDS;
  const code = style(id);
  const [iStyle, iInt, iPhase] = [param('style'), param('intensity'), param('phase')];
  const r = renderMusic(id, seconds, MUSIC_SEED, (_t, p) => {
    p[iStyle] = code;
    p[iInt] = STYLE_INTENSITY;
    p[iPhase] = 1;
  });
  return {
    id: `render ${id}`,
    entries: [entry('music', id, `${id}.wav`, `music ${id}`, r,
      `style ${id} (code ${code}), intensity ${STYLE_INTENSITY}, phase 1, ${seconds} s`)],
  };
}

/** Case `render arc-upbeat`: intro → main (intensity ramp) → climax → resolveWin → post. */
function arcCase(): CaseResult {
  const code = style('upbeat');
  const [iStyle, iInt, iPhase, iClimax, iResolved, iWon] =
    [param('style'), param('intensity'), param('phase'), param('climax'), param('resolved'), param('won')];
  const tMain = ARC.intro;
  const tClimax = tMain + ARC.main;
  const tResolve = tClimax + ARC.climax;
  const seconds = tResolve + ARC.resolveWin + ARC.post;
  const r = renderMusic('arc-upbeat', seconds, MUSIC_SEED, (t, p) => {
    p[iStyle] = code;
    if (t < tMain) {
      p[iPhase] = 0;
      p[iInt] = ARC_INTENSITY_FROM;
    } else if (t < tClimax) {
      p[iPhase] = 1;
      p[iInt] = ARC_INTENSITY_FROM + (ARC_INTENSITY_TO - ARC_INTENSITY_FROM) * ((t - tMain) / ARC.main);
    } else if (t < tResolve) {
      p[iInt] = ARC_INTENSITY_TO;
      p[iClimax] = 1;
    } else {
      // resolveWin, then post (the arranger moves to post 2 bars after the resolve).
      p[iClimax] = 0;
      p[iPhase] = 2;
      p[iResolved] = 1;
      p[iWon] = 1;
    }
  });
  return {
    id: 'render arc-upbeat',
    entries: [entry('music', 'arc-upbeat', 'arc-upbeat.wav', 'music arc (upbeat)', r,
      `intro ${ARC.intro} s → main ${ARC.main} s (intensity ${ARC_INTENSITY_FROM}→${ARC_INTENSITY_TO}) → climax ${ARC.climax} s → resolveWin ${ARC.resolveWin} s → post ${ARC.post} s`)],
  };
}

/** Case `render crossfade`: upbeat, then style = calm with fadeBars 2 at CROSSFADE_AT s. */
function crossfadeCase(): CaseResult {
  const [up, calm] = [style('upbeat'), style('calm')];
  const [iStyle, iInt, iPhase, iFade] = [param('style'), param('intensity'), param('phase'), param('fadeBars')];
  let requestFrame = -1;
  const at = Math.round(CROSSFADE_AT * AUDIO_RATE);
  const r = renderMusic('crossfade', CROSSFADE_SECONDS, MUSIC_SEED, (t, p) => {
    p[iInt] = STYLE_INTENSITY;
    p[iPhase] = 1;
    const frame = Math.round(t * AUDIO_RATE);
    if (frame < at) {
      p[iStyle] = up;
    } else {
      if (requestFrame < 0) requestFrame = frame;
      p[iStyle] = calm;
      p[iFade] = CROSSFADE_BARS;
    }
  });
  return {
    id: 'render crossfade',
    requestFrame,
    entries: [entry('music', 'crossfade', 'crossfade.wav', 'music crossfade upbeat → calm', r,
      `upbeat for ${CROSSFADE_AT} s, then calm with fadeBars ${CROSSFADE_BARS} (requested at frame ${requestFrame}), ${CROSSFADE_SECONDS} s`)],
  };
}

/** Case `render stings`: each STING_FAMILIES strip through the mix on the music bus input. */
function stingsCase(): CaseResult {
  return {
    id: 'render stings',
    entries: STING_FAMILIES.map((def: FamilyDef) => {
      const strip = renderFamilyStrip(def, BASE_PRESET, { sampleRate: AUDIO_RATE });
      const r = mixRender(strip, def.id);
      return entry('stings', def.id, `stings/${def.id}.wav`, `sting ${def.id}`, r,
        `${def.variants} normalized variants (BASE_PRESET), through the mix on the music bus`);
    }),
  };
}

const CASES: ProofCase<CaseResult>[] = [
  { id: 'render upbeat', label: 'render upbeat', run: (a: ProofArgs) => styleCase('upbeat', a) },
  { id: 'render calm', label: 'render calm', run: (a: ProofArgs) => styleCase('calm', a) },
  { id: 'render arc-upbeat', label: 'render arc-upbeat', run: () => arcCase() },
  { id: 'render crossfade', label: 'render crossfade', run: () => crossfadeCase() },
  { id: 'render stings', label: 'render stings', run: () => stingsCase() },
];

/** The entry (with its case id) whose `key` is the largest (or smallest with `min`). */
function pick(rs: readonly CaseResult[], key: (m: Measure) => number, min = false): [string, number] {
  let at = '';
  let best = min ? Infinity : -Infinity;
  for (const r of rs) {
    for (const e of r.entries) {
      const v = key(e.measure);
      if (Number.isNaN(v)) return [`${r.id}: ${e.id}`, v];
      if (at === '' || (min ? v < best : v > best)) [at, best] = [`${r.id}: ${e.id}`, v];
    }
  }
  return [at, best];
}

/** A full MusicState with the steady defaults (phase 1, intensity 0.6) and overrides. */
function musicState(o: Partial<MusicState>): MusicState {
  return { style: 0, styleB: 0, styleMix: 0, intensity: STYLE_INTENSITY, phase: 1, climax: 0, resolved: 0, won: 0, sparkle: 0, tension: 0, ...o };
}

/** The state of bar `bar` of the notes-in-scale script (changes only at bar lines). */
function scaleState(bar: number): MusicState {
  const mainBars = SCALE_SECTIONS.climaxAt - SCALE_SECTIONS.intro;
  if (bar < SCALE_SECTIONS.intro) return musicState({ phase: 0, intensity: SCALE_INTENSITY_FROM });
  if (bar < SCALE_SECTIONS.climaxAt) {
    const k = (bar - SCALE_SECTIONS.intro) / (mainBars - 1);
    return musicState({ intensity: SCALE_INTENSITY_FROM + (SCALE_INTENSITY_TO - SCALE_INTENSITY_FROM) * k, sparkle: 1 });
  }
  if (bar < SCALE_SECTIONS.climaxAt + SCALE_SECTIONS.climax) return musicState({ intensity: SCALE_INTENSITY_TO, climax: 1, sparkle: 1 });
  // resolveWin; the arranger moves to post 2 bars later on its own.
  return musicState({ phase: 2, resolved: 1, won: 1 });
}

const PITCHED = new Set<NoteEvent['inst']>(['pad', 'bass', 'lead', 'pluck']);

/** Out-of-scale pitched notes and the pitched total of one style alone over SCALE_BARS bars. */
function scaleCount(styleCode: number): { out: number; total: number } {
  const st = listStyles()[styleCode];
  const arr = new Arranger(SCALE_SEED, [st]);
  const ev: NoteEvent[] = [];
  let out = 0;
  let total = 0;
  for (let bar = 0; bar < SCALE_BARS; bar++) {
    const s = scaleState(bar);
    arr.setState(s);
    for (let k = 0; k < BAR_STEPS; k++) {
      const n = arr.nextStep(ev);
      for (let i = 0; i < n; i++) {
        const e = ev[i];
        if (!PITCHED.has(e.inst)) continue;
        total++;
        const pc = (((e.midi - st.root - (s.climax ? 2 : 0)) % 12) + 12) % 12;
        if (!st.scale.includes(pc)) out++;
      }
    }
  }
  return { out, total };
}

/** pcmHash over left followed by right of a DET_SECONDS steady upbeat render at `seed`. */
function upbeatHash(seed: number): string {
  const code = style('upbeat');
  const [iStyle, iInt, iPhase] = [param('style'), param('intensity'), param('phase')];
  const r = renderMusic(`det-${seed}`, DET_SECONDS, seed, (_t, p) => {
    p[iStyle] = code;
    p[iInt] = STYLE_INTENSITY;
    p[iPhase] = 1;
  });
  const both = new Float32Array(r.left.length + r.right.length);
  both.set(r.left, 0);
  both.set(r.right, r.left.length);
  return pcmHash(both);
}

/**
 * Bars from a calm request at step LAND_STEP of bar LAND_BAR (upbeat before it, intensity 0.6,
 * phase 1) until Arranger.styleIndex equals calm; fractional bars count from the request step.
 */
function crossfadeLanding(): { bars: number; requestStep: number; landStep: number } {
  const [up, calm] = [style('upbeat'), style('calm')];
  const arr = new Arranger(MUSIC_SEED, listStyles());
  const ev: NoteEvent[] = [];
  const requestStep = LAND_BAR * BAR_STEPS + LAND_STEP;
  for (let k = 0; k < requestStep; k++) {
    if (k % BAR_STEPS === 0) arr.setState(musicState({ style: up }));
    arr.nextStep(ev);
  }
  arr.setState(musicState({ style: calm }));
  const limit = requestStep + 64 * BAR_STEPS;
  for (let k = requestStep; k < limit; k++) {
    arr.nextStep(ev);
    if (arr.styleIndex === calm) return { bars: (k - requestStep) / BAR_STEPS, requestStep, landStep: k };
  }
  return { bars: Infinity, requestStep, landStep: -1 };
}

proofMain('m3', async (a) => {
  registerBuiltins();
  const results = [...(await runCases(SCRIPT, CASES, a)).values()];
  for (const r of results) {
    for (const e of r.entries) {
      if (e.failures.length > 0) console.log(`  ${r.id} ${e.id}: ${e.failures.join(' ')}`);
    }
  }

  const report: Report = { proof: 'm3', entries: [], checks };
  for (const r of results) {
    for (const e of r.entries) {
      report.entries.push({ group: e.group, id: e.id, file: e.file, title: e.title, measure: e.measure, note: e.note });
    }
  }
  try {
    // 1. Every music render (steady styles, arc, crossfade, stings): one line per measure.
    if (results.length > 0) {
      const nonFinite = results.reduce((s, r) => s + r.entries.reduce((t, e) => t + e.measure.nonFinite, 0), 0);
      const [nfAt] = pick(results, (m) => m.nonFinite);
      check(LABELS.safety, nonFinite, '==', 0, 'non-finite samples', nfAt);
      const [dcAt, dc] = pick(results, (m) => Math.abs(m.dc));
      check(LABELS.safety, dc, '<', SAFETY.maxAbsDc, '|DC|', dcAt);
      const [peakAt, peak] = pick(results, (m) => m.samplePeakDb);
      check(LABELS.safety, peak, '<=', SAFETY.maxSamplePeakDb, 'dBFS sample peak', peakAt);
      const [rmsAt, rms] = pick(results, (m) => m.minWindowRmsDb, true);
      check(LABELS.safety, rms, '>=', SAFETY.minRmsDb, 'dBFS min 1 s window RMS', rmsAt);
    }

    // 2. Every registered style alone over 64 bars: pitched notes outside its scale.
    if (selected(LABELS.scale, a)) {
      let worst = '';
      let worstOut = -1;
      let out = 0;
      for (let i = 0; i < listStyles().length; i++) {
        const c = scaleCount(i);
        out += c.out;
        if (c.out > worstOut) [worst, worstOut] = [`worst ${listStyles()[i].id}: ${c.out} of ${c.total}`, c.out];
      }
      check(LABELS.scale, out, '==', 0, 'notes', worst);
    }

    // 3. Same seed, same pcmHash; another seed, another hash.
    if (selected(LABELS.deterministic, a)) {
      const [hA, hB, hC] = [upbeatHash(DET_SEED), upbeatHash(DET_SEED), upbeatHash(DET_SEED_OTHER)];
      const failures = Number(hA !== hB) + Number(hA === hC);
      check(LABELS.deterministic, failures, '==', 0, 'failures',
        `seed ${DET_SEED} ${hA} / ${hB}, seed ${DET_SEED_OTHER} ${hC}`);
    }

    // 4. Bars from a mid-bar calm request to the arranger's switch.
    if (selected(LABELS.crossfade, a)) {
      const l = crossfadeLanding();
      check(LABELS.crossfade, l.bars, '<=', LAND_MAX_BARS, 'bars',
        `request step ${l.requestStep} (bar ${LAND_BAR} step ${LAND_STEP}), calm at step ${l.landStep}`);
    }
  } finally {
    writeReport(OUT, report);
    console.log(`report ${relative(ROOT, join(OUT, 'index.html'))}`);
  }
});
