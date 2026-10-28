// Listen page controller: patch selection, Play/Stop of both engines on one AudioContext,
// the A/B switch (A = Sona JS, B = HAA wasm), Match levels, the issue list, one control per
// published input and the traces panel (ARCHITECTURE listen module, §17).

import { createHaaSide } from './haa.ts';
import type { HaaSide } from './haa.ts';
import { createSonaSide } from './sona.ts';
import type { SonaSide } from './sona.ts';
import { createTracePanel } from './traces.ts';
import type { TracePanel } from './traces.ts';
import type { HaaInput, PatchIssue } from '../../packages/haa/src/index.ts';

/** Patch preselected in the list when the server offers it. */
const DEFAULT_PATCH = 'patches/examples/osc-curve.json';
/** A/B and Match levels gain ramps, in seconds. */
const RAMP_S = 0.005;
/** Match levels listening time per side, in seconds. */
const MATCH_S = 1;
/** Time left to the 5 ms ramp before the analyser is read, in milliseconds. */
const MATCH_SETTLE_MS = 30;
/** Interval between analyser reads while matching, in milliseconds. */
const MATCH_POLL_MS = 40;
/** RMS below this counts as silence and skips the trim. */
const RMS_FLOOR = 1e-6;
/** Steps of a continuous input slider. */
const SLIDER_STEPS = 1000;
const SONA_NA = 'n/a (HAA-only nodes)';

function byId<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`[listen] missing element #${id}`);
  return found as T;
}

const patchSelect = byId<HTMLSelectElement>('patch-select');
const fileInput = byId<HTMLInputElement>('file-input');
const playButton = byId<HTMLButtonElement>('play');
const stopButton = byId<HTMLButtonElement>('stop');
const abToggle = byId<HTMLButtonElement>('ab-toggle');
const abLabel = byId<HTMLSpanElement>('ab-label');
const matchButton = byId<HTMLButtonElement>('match');
const trimLabel = byId<HTMLSpanElement>('trim');
const sonaStatus = byId<HTMLSpanElement>('sona-status');
const issuesList = byId<HTMLUListElement>('issues');
const inputsRoot = byId<HTMLDivElement>('inputs');
const tracesRoot = byId<HTMLDivElement>('traces');
const statusLabel = byId<HTMLSpanElement>('status');

/** The loaded patch: raw bytes for HAA, the parsed JSON and the Sona-side normalisation. */
interface LoadedPatch {
  name: string;
  bytes: Uint8Array;
  json: unknown;
  graph: Record<string, unknown> | null;
  master: number | undefined;
}

/** Audio graph built once on the first Play: sides → gains → (trim) → analysers → destination. */
interface Mixer {
  ctx: AudioContext;
  gainA: GainNode;
  gainB: GainNode;
  trimB: GainNode;
  analyserA: AnalyserNode;
  analyserB: AnalyserNode;
}

let patch: LoadedPatch | null = null;
let mixer: Mixer | null = null;
let haa: HaaSide | null = null;
let sona: SonaSide | null = null;
let panel: TracePanel | null = null;
let unsubscribers: Array<() => void> = [];
/** Bumped by every Play, Stop and patch change, so late async work can tell it is stale. */
let session = 0;
let showB = false;
let matching = false;

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function setStatus(text: string): void {
  statusLabel.textContent = text;
}

function report(error: unknown): void {
  setStatus(message(error));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The Sona graph and master of a HAA patch, a Sona v2 project or a bare graph. */
function normalise(json: unknown): { graph: Record<string, unknown> | null; master: number | undefined } {
  if (!isRecord(json)) return { graph: null, master: undefined };
  const master = typeof json.master === 'number' ? json.master : undefined;
  if (typeof json.haa === 'number' || isRecord(json.graph)) {
    return { graph: isRecord(json.graph) ? json.graph : null, master };
  }
  if (Array.isArray(json.nodes) && 'duration' in json) return { graph: json, master: undefined };
  return { graph: null, master: undefined };
}

function setPatch(name: string, bytes: Uint8Array): void {
  stop();
  let json: unknown = null;
  try {
    json = JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    // The HAA loader reports `not-json`; the Sona side stays unavailable.
  }
  patch = { name, bytes, json, ...normalise(json) };
  if (mixer) mixer.trimB.gain.value = 1;
  showTrim(1);
  sonaStatus.textContent = patch.graph ? 'not started' : 'n/a (no graph)';
  issuesList.replaceChildren();
  inputsRoot.replaceChildren();
  setStatus(`loaded ${name}`);
}

async function fetchPatch(path: string): Promise<void> {
  const response = await fetch(`/${path}`);
  if (!response.ok) throw new Error(`[listen] ${path}: HTTP ${response.status}`);
  setPatch(path, new Uint8Array(await response.arrayBuffer()));
}

async function loadPatchList(): Promise<void> {
  const response = await fetch('/__patches');
  if (!response.ok) throw new Error(`[listen] /__patches: HTTP ${response.status}`);
  const list: unknown = await response.json();
  const paths = Array.isArray(list) ? list.filter((p): p is string => typeof p === 'string') : [];
  patchSelect.replaceChildren(
    ...paths.map((path) => {
      const option = document.createElement('option');
      option.value = path;
      option.textContent = path;
      return option;
    }),
  );
  const first = paths.includes(DEFAULT_PATCH) ? DEFAULT_PATCH : paths[0];
  if (first === undefined) {
    setStatus('no patch under patches/');
    return;
  }
  patchSelect.value = first;
  await fetchPatch(first);
}

function getMixer(): Mixer {
  if (mixer) return mixer;
  const ctx = new AudioContext({ latencyHint: 'interactive' });
  const gainA = ctx.createGain();
  const gainB = ctx.createGain();
  const trimB = ctx.createGain();
  const analyserA = ctx.createAnalyser();
  const analyserB = ctx.createAnalyser();
  analyserA.fftSize = 2048;
  analyserB.fftSize = 2048;
  gainA.connect(analyserA).connect(ctx.destination);
  gainB.connect(trimB).connect(analyserB).connect(ctx.destination);
  gainA.gain.value = 0;
  gainB.gain.value = 0;
  mixer = { ctx, gainA, gainB, trimB, analyserA, analyserB };
  return mixer;
}

/** Ramp `param` to `value` over 5 ms from now. */
function rampTo(ctx: AudioContext, param: AudioParam, value: number): void {
  const at = ctx.currentTime;
  param.cancelScheduledValues(at);
  param.setValueAtTime(param.value, at);
  param.linearRampToValueAtTime(value, at + RAMP_S);
}

/** Open one side alone: `b` true opens HAA, false opens Sona. */
function openSide(m: Mixer, b: boolean): void {
  rampTo(m.ctx, m.gainA.gain, b ? 0 : 1);
  rampTo(m.ctx, m.gainB.gain, b ? 1 : 0);
}

/** B stays on while the Sona side is unavailable. */
function effectiveB(): boolean {
  return showB || (haa !== null && sona === null);
}

function applyAb(): void {
  abLabel.textContent = effectiveB() ? 'B: HAA wasm' : 'A: Sona JS';
  if (mixer && !matching) openSide(mixer, effectiveB());
}

function toggleAb(): void {
  showB = !showB;
  applyAb();
}

function showTrim(gain: number): void {
  trimLabel.textContent = `${(20 * Math.log10(gain)).toFixed(1)} dB`;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Open one side alone for MATCH_S and return the RMS its analyser reads; null if stale. */
async function measure(m: Mixer, b: boolean, token: number): Promise<number | null> {
  const analyser = b ? m.analyserB : m.analyserA;
  const block = new Float32Array(analyser.fftSize);
  openSide(m, b);
  await sleep(MATCH_SETTLE_MS);
  let sum = 0;
  let count = 0;
  const end = performance.now() + MATCH_S * 1000;
  while (performance.now() < end) {
    if (token !== session) return null;
    analyser.getFloatTimeDomainData(block);
    for (let i = 0; i < block.length; i++) sum += block[i] * block[i];
    count += block.length;
    await sleep(MATCH_POLL_MS);
  }
  return count > 0 ? Math.sqrt(sum / count) : 0;
}

async function matchLevels(): Promise<void> {
  const m = mixer;
  if (!m || !haa || matching) return;
  if (!sona) {
    setStatus('match levels needs both sides (Sona is n/a)');
    return;
  }
  const token = session;
  matching = true;
  matchButton.disabled = true;
  try {
    const rmsA = await measure(m, false, token);
    const rmsB = rmsA === null ? null : await measure(m, true, token);
    if (rmsA === null || rmsB === null) return;
    // The B analyser reads after the trim: divide the current trim out.
    const trim = m.trimB.gain.value;
    const rawB = trim > 0 ? rmsB / trim : 0;
    if (rmsA < RMS_FLOOR || rawB < RMS_FLOOR) {
      setStatus('match levels skipped: a side is silent');
      return;
    }
    const next = rmsA / rawB;
    rampTo(m.ctx, m.trimB.gain, next);
    showTrim(next);
    setStatus('');
  } finally {
    matching = false;
    matchButton.disabled = false;
    if (token === session) applyAb();
  }
}

function showIssues(issues: PatchIssue[]): void {
  issuesList.replaceChildren(
    ...issues.map((issue) => {
      const item = document.createElement('li');
      item.className = `issue ${issue.severity}`;
      item.textContent = `${issue.severity} ${issue.code} ${issue.path}: ${issue.message}`;
      return item;
    }),
  );
}

function formatValue(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 100) return value.toFixed(0);
  if (abs >= 10) return value.toFixed(1);
  return value.toFixed(3);
}

/** A range slider over min..max, linear or logarithmic per `curve`. */
function continuousControl(input: HaaInput, value: HTMLSpanElement): HTMLInputElement {
  const { min, max } = input;
  const log = input.curve === 'log' && min > 0 && max > 0;
  const toValue = (p: number): number =>
    log ? Math.exp(Math.log(min) + p * (Math.log(max) - Math.log(min))) : min + p * (max - min);
  const toPosition = (v: number): number => {
    const clamped = Math.min(max, Math.max(min, v));
    const p = log
      ? (Math.log(clamped) - Math.log(min)) / (Math.log(max) - Math.log(min))
      : (clamped - min) / (max - min);
    return Number.isFinite(p) ? p : 0;
  };
  const unit = input.unit ? ` ${input.unit}` : '';
  const slider = document.createElement('input');
  slider.type = 'range';
  slider.min = '0';
  slider.max = String(SLIDER_STEPS);
  slider.step = '1';
  slider.value = String(Math.round(toPosition(input.default) * SLIDER_STEPS));
  value.textContent = `${formatValue(input.default)}${unit}`;
  slider.addEventListener('input', () => {
    const v = toValue(Number(slider.value) / SLIDER_STEPS);
    value.textContent = `${formatValue(v)}${unit}`;
    haa?.set(input.name, v);
    if (input.name === 'pedal') sona?.setPedal(v);
  });
  return slider;
}

function enumControl(input: HaaInput): HTMLSelectElement {
  const select = document.createElement('select');
  const values = input.values ?? [];
  select.replaceChildren(
    ...values.map((name) => {
      const option = document.createElement('option');
      option.value = name;
      option.textContent = name;
      return option;
    }),
  );
  const initial = values[input.default];
  if (initial !== undefined) select.value = initial;
  select.addEventListener('change', () => haa?.set(input.name, select.value));
  return select;
}

function triggerControl(input: HaaInput): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = `Trigger ${input.name}`;
  button.addEventListener('click', () => haa?.trigger(input.name));
  return button;
}

function showInputs(inputs: HaaInput[]): void {
  inputsRoot.replaceChildren(
    ...inputs.map((input) => {
      const row = document.createElement('label');
      row.className = 'field';
      const name = document.createElement('span');
      name.className = 'field-label';
      name.textContent = input.name;
      const value = document.createElement('span');
      value.className = 'value';
      const control =
        input.kind === 'enum' ? enumControl(input)
        : input.kind === 'trigger' ? triggerControl(input)
        : continuousControl(input, value);
      row.append(name, control, value);
      return row;
    }),
  );
}

/** Trace labels from the patch graph, in `info.nodes` plan order. */
function traceNodes(ids: string[]): Array<{ id: string; label: string }> {
  const nodes = patch?.graph && Array.isArray(patch.graph.nodes) ? patch.graph.nodes : [];
  return ids.map((id) => {
    const node: unknown = nodes.find((n: unknown) => isRecord(n) && n.id === id);
    const label = isRecord(node) && typeof node.label === 'string' ? node.label : '';
    return { id, label };
  });
}

/** Dispose both sides and clear the traces; the panel stays until the next Play. */
function stop(): void {
  session += 1;
  for (const off of unsubscribers) off();
  unsubscribers = [];
  haa?.dispose();
  sona?.dispose();
  if (sona) sonaStatus.textContent = 'stopped';
  haa = null;
  sona = null;
  panel?.clear();
}

async function play(): Promise<void> {
  const current = patch;
  if (!current) {
    setStatus('no patch loaded');
    return;
  }
  const m = getMixer();
  await m.ctx.resume();
  stop();
  const token = session;
  setStatus('');
  let haaError: unknown = null;
  let sonaError: unknown = null;
  const haaPromise = createHaaSide(m.ctx, current.bytes, m.gainB).catch((error: unknown) => {
    haaError = error;
    return null;
  });
  const sonaPromise = current.graph
    ? createSonaSide(m.ctx, current.graph, m.gainA, current.master).catch((error: unknown) => {
        sonaError = error;
        return null;
      })
    : Promise.resolve(null);
  const [h, s] = await Promise.all([haaPromise, sonaPromise]);
  if (token !== session || !h) {
    h?.dispose();
    s?.dispose();
    if (token === session) report(haaError ?? '[listen] HAA side unavailable');
    return;
  }
  haa = h;
  sona = s;
  sonaStatus.textContent = s ? 'playing'
    : sonaError ? message(sonaError)
    : current.graph ? SONA_NA
    : 'n/a (no graph)';
  if (sonaError) report(sonaError);
  applyAb();
  showIssues(h.issues);
  showInputs(h.info.inputs);
  panel?.dispose();
  const traces = createTracePanel(tracesRoot, traceNodes(h.info.nodes));
  panel = traces;
  unsubscribers.push(h.onTrace((t) => traces.pushHaa(t.seconds, t.values)));
  if (s) unsubscribers.push(s.onTrace((seconds, values) => traces.pushSona(seconds, values)));
  if (!h.playable) setStatus('HAA patch not playable: see issues');
}

function isFormField(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement
    || target instanceof HTMLSelectElement
    || target instanceof HTMLTextAreaElement;
}

patchSelect.addEventListener('change', () => {
  fetchPatch(patchSelect.value).catch(report);
});

fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  if (!file) return;
  file.arrayBuffer()
    .then((buffer) => setPatch(file.name, new Uint8Array(buffer)))
    .catch(report);
});

playButton.addEventListener('click', () => {
  play().catch(report);
});

stopButton.addEventListener('click', () => {
  try {
    stop();
  } catch (error) {
    report(error);
  }
});

abToggle.addEventListener('click', toggleAb);

document.addEventListener('keydown', (event) => {
  if (event.key !== 'a' || event.repeat || event.ctrlKey || event.metaKey || event.altKey) return;
  if (isFormField(event.target)) return;
  event.preventDefault();
  toggleAb();
});

matchButton.addEventListener('click', () => {
  matchLevels().catch(report);
});

loadPatchList().catch(report);
