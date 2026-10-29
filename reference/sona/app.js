import { t, tr, tx, html as ui, locale, getLanguage, setLanguage, onLanguageChange, registerStaticTranslations, applyStaticTranslations, translateError } from './i18n.js';
import { NODE_TYPES, PRESETS, SOUND_REVISION, makeNode, clonePreset } from './catalog.js';
import { AudioEngine } from './audio.js';
import { validateGraph } from './dsp.js';
import { ComparisonPanel } from './comparison-ui.js';
import { ModulationPreviews } from './modulation-preview.js';
import { renderModulationPreview } from './modulation-preview-view.js';

const $ = (id) => document.getElementById(id);
const escapeHTML = (value) => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const clamp = (v, min, max) => Math.max(min, Math.min(max, v));
const categoryName = { source: 'Générateur', control: 'Modulation', effect: 'Effet', output: 'Destination' };
const symbols = { engine:'≋', wind:'〰', droplet:'◒', waves:'∿', noise:'▥', curve:'⌁', activity:'∿', envelope:'⋀', filter:'⊿', drive:'ϟ', volume:'◖', delay:'⋮', reverb:'≋', speaker:'◖', random:'∾', resonator:'◎', formant:'⋒', equalizer:'≡', compressor:'⇥', space:'⤢', pulse:'⋀', math:'×', comb:'⋮' };
const STORAGE_KEY = 'sona-project-v1';
const presetRevision = id => PRESETS.find(preset => preset.id === id)?.revision ?? SOUND_REVISION;
const builtInLabels = new Map();
for (const preset of PRESETS) for (const node of preset.graph.nodes) {
  const key = node.id + ':' + node.type;
  builtInLabels.set(key, [...(builtInLabels.get(key) || []), node.label]);
}
// Render known defaults in the selected language; authored labels stay verbatim.
function nodeText(node) {
  if (!node) return '';
  if (node.labelCustom) return node.label;
  const candidates = [...(builtInLabels.get(node.id + ':' + node.type) || []), NODE_TYPES[node.type]?.name].filter(Boolean);
  const source = candidates.find(value => [value, tr(value, 'en'), tr(value, 'fr')].includes(node.label));
  return source ? tr(source) : node.label;
}
function projectText() {
  if (titleCustom) return title;
  if (title === 'Mon exploration sonore' || title === 'My sound exploration') return tr('Mon exploration sonore');
  const preset = PRESETS.find(p => p.id === presetId);
  if (preset) {
    const variants = ['en', 'fr'].map(code => tr(preset.name, code) + ' · ' + tr(preset.subtitle, code).toLocaleLowerCase(code));
    variants.push(preset.name + ' · ' + preset.subtitle.toLocaleLowerCase('fr'));
    if (variants.includes(title)) return tr(preset.name) + ' · ' + tr(preset.subtitle).toLocaleLowerCase(locale());
  }
  return title;
}
let graph = clonePreset('v12');
let presetId = 'v12';
let title = PRESETS[0].name + ' · ' + PRESETS[0].subtitle.toLocaleLowerCase(locale());
let titleCustom = false;
let recipeRevision = presetRevision(presetId);
let selectedNode = graph.nodes.find(n => NODE_TYPES[n.type].category === 'source')?.id || null;
let selectedEdge = null;
let selectedCurve = graph.nodes.find(n => n.type === 'curve')?.id || null;
let master = .65;
let view = { x: 30, y: 30, scale: .7 };
let pendingCable = null;
let gesture = null;
let cursor = { x: 0, y: 0 };
let history = [];
let future = [];
let saveTimer;
let audioUpdateTimer;
let toastTimer;
let toastSource = '';
let saveStatus = 'Sauvegarde locale';
let exporting = false;
let scopeMode = 'wave';
let currentTime = 0;
let lastScopeAt = 0;
let loadingAudio = false;
let parameterGesture = false;
let performanceGesture = false;
let startDriveSession = null;
let startDriveSerial = 0;
let curvePoint = null;
const nodeElements = new Map();
let modulationPaintQueued = false;
const modulationPaintIds = new Set();
const modulationPreviews = new ModulationPreviews({onChange: ids => {
  for (const id of ids) modulationPaintIds.add(id);
  if (modulationPaintQueued || !modulationPaintIds.size) return;
  modulationPaintQueued = true;
  requestAnimationFrame(() => {
    const affected = new Set(modulationPaintIds);
    modulationPaintIds.clear(); modulationPaintQueued = false;
    paintModulationPreviews(affected);
  });
}});

function uniqueId(prefix = 'n') {
  console.info('[Sona] Nouvel identifiant');
  const values = new Uint32Array(2);
  if (globalThis.crypto?.getRandomValues) crypto.getRandomValues(values);
  else { values[0] = Date.now() >>> 0; values[1] = Math.floor(Math.random() * 4294967296); }
  return prefix + '-' + Array.from(values, value => value.toString(36)).join('-');
}

const audio = new AudioEngine({
  onModulation: packet => modulationPreviews.accept(packet),
  onModulationReset: state => modulationPreviews.resetAudio(state),
  onTime: (seconds) => {
    currentTime = Number(seconds) || 0;
    updateTransport();
    movePlayhead();
  },
  onEnded: () => {
    currentTime = 0;
    if (startDriveSession) {
      const session = startDriveSession;
      cancelStartDrive();
      if (session.graph === graph && session.presetId === presetId && performanceBinding()) {
        console.info('[Sona] Engine start complete; handing over to the live pedal');
        renderPerformance();
        startAudio();
        return;
      }
    }
    updateTransport();
    movePlayhead();
  },
  onError: (error) => {
    cancelStartDrive();
    showToast(error?.message || String(error), true);
    updateTransport();
  },
});

const comparison = new ComparisonPanel({
  audioEngine: audio,
  getProject: () => ({graph, presetId, master}),
  beforePlay: () => {
    cancelStartDrive();
    clearTimeout(audioUpdateTimer);
    audio.stop(); currentTime = 0;
    updateTransport(); movePlayhead();
  },
  onError: message => showToast(message, true),
});

function formatValue(value, param) {
  if (param?.options) return tr(param.options.find(option => option.value === value)?.label || value);
  const v = Number(value);
  if (!Number.isFinite(v)) return '—';
  let text;
  if (param?.unit === 'Hz' && v >= 1000) text = (v / 1000).toLocaleString(locale(), { maximumFractionDigits: 2 }) + ' kHz';
  else if (param?.unit === 'tr/min') text = Math.round(v).toLocaleString(locale()) + tx(' tr/min');
  else text = v.toLocaleString(locale(), { maximumFractionDigits: param?.step < .0001 ? 6 : param?.step < .001 ? 4 : param?.step < .01 ? 3 : param?.step < 1 ? 2 : 0 }) + (param?.unit ? ' ' + tr(param.unit) : '');
  return text;
}

function showToast(message, isError = false) {
  console.info('[Sona] Notification', message);
  toastSource = message;
  $('toast').textContent = typeof message === 'function' ? message() : translateError(message);
  $('toast').classList.toggle('error', isError);
  $('toast').hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $('toast').hidden = true; }, 3800);
}

function snapshot() {
  return JSON.stringify({ graph, presetId, title, titleCustom, selectedNode, selectedCurve, recipeRevision });
}

function remember() {
  console.info('[Sona] État conservé');
  const state = snapshot();
  if (history.at(-1) !== state) history.push(state);
  if (history.length > 50) history.shift();
  future = [];
  updateHistoryButtons();
}

function updateHistoryButtons() {
  $('undoButton').disabled = !history.length;
  $('redoButton').disabled = !future.length;
}

function restoreSnapshot(state) {
  console.info('[Sona] Restauration');
  if (startDriveSession) stopAudio();
  const data = JSON.parse(state);
  ({ graph, presetId, title, selectedNode, selectedCurve } = data);
  titleCustom = data.titleCustom !== false;
  recipeRevision = data.recipeRevision ?? presetRevision(presetId);
  selectedEdge = null;
  renderAll();
  commit();
}

function undo() {
  console.info('[Sona] Annuler');
  if (!history.length) return;
  future.push(snapshot());
  restoreSnapshot(history.pop());
  updateHistoryButtons();
}

function redo() {
  console.info('[Sona] Rétablir');
  if (!future.length) return;
  history.push(snapshot());
  restoreSnapshot(future.pop());
  updateHistoryButtons();
}

function commit({ audioChanged = true, immediateAudio = false } = {}) {
  if (audioChanged) modulationPreviews.setGraph(graph);
  comparison.graphChanged(audioChanged);
  saveStatus = 'Enregistrement…';
  $('saveState').textContent = tr(saveStatus);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveLocal, 350);
  if (audioChanged && audio.isPlaying && !startDriveSession) {
    clearTimeout(audioUpdateTimer);
    if (immediateAudio) {
      try { audio.setGraph(graph); } catch (error) { showToast(error.message, true); }
    } else audioUpdateTimer = setTimeout(() => {
      if (startDriveSession) return;
      try { audio.setGraph(graph); } catch (error) { showToast(error.message, true); }
    }, 35);
  }
  renderPerformance();
  $('nodeCount').textContent = t('nodeCount', {nodes: graph.nodes.length, connections: graph.connections.length});
}

function saveLocal() {
  console.info('[Sona] Sauvegarde locale');
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({version:2, recipeRevision, graph, presetId, title, titleCustom, master}));
    saveStatus = 'Sauvegardé ici';
    $('saveState').textContent = tr(saveStatus);
  } catch {
    saveStatus = 'Session uniquement';
    $('saveState').textContent = tr(saveStatus);
  }
}

function safeProject(data) {
  console.info('[Sona] Vérification du projet');
  if (!data || !data.graph || !Array.isArray(data.graph.nodes) || !Array.isArray(data.graph.connections)) throw new Error(tx('Ce fichier ne contient pas de projet Sona.'));
  const g = data.graph;
  if (g.nodes.length > 40 || g.connections.length > 160) throw new Error(tx('Limite : 40 nœuds et 160 liaisons par projet.'));
  if (!Number.isFinite(g.duration) || g.duration < .5 || g.duration > 30) throw new Error(tx('La durée doit être comprise entre 0,5 et 30 secondes.'));
  const ids = new Set();
  const nodes = g.nodes.map((node, index) => {
    const schema = NODE_TYPES[node.type];
    if (!schema || typeof node.id !== 'string' || !node.id || node.id.length > 80 || ids.has(node.id)) throw new Error(tx('Nœud inconnu ou identifiant invalide.'));
    ids.add(node.id);
    const params = {};
    schema.params.forEach(p => {
      const value = node.params?.[p.key] ?? p.default;
      if (p.options) {
        if (!p.options.some(option => option.value === value)) throw new Error(tx('Valeur de paramètre invalide.'));
      } else if (!Number.isFinite(value) || value < p.min || value > p.max) throw new Error(tx('Paramètre hors limites : ') + tr(p.label));
      params[p.key] = value;
    });
    const clean = {
      id: node.id, type: node.type, label: String(node.label || schema.name).slice(0, 60), labelCustom: node.labelCustom === true, params, compact: node.compact === true, muted: node.muted === true, bypass: node.bypass === true,
      x: Number.isFinite(node.x) ? clamp(node.x, -5000, 10000) : 100 + index * 40,
      y: Number.isFinite(node.y) ? clamp(node.y, -5000, 10000) : 100 + index * 30,
    };
    if (node.type === 'curve') {
      if (!Array.isArray(node.curve) || node.curve.length < 2 || node.curve.length > 64) throw new Error(tx('La courbe doit contenir de 2 à 64 points.'));
      if (node.curve.some(p => !Array.isArray(p) || p.length !== 2 || p.some(v => !Number.isFinite(v) || v < 0 || v > 1))) throw new Error(tx('Points de courbe invalides.'));
      clean.curve = node.curve.map(p => [...p]).sort((a,b) => a[0]-b[0]);
      if (clean.curve[0][0] !== 0 || clean.curve.at(-1)[0] !== 1) throw new Error(tx('La courbe doit commencer à 0 et se terminer à 1.'));
    }
    return clean;
  });
  const byId = new Map(nodes.map(n => [n.id,n]));
  const edgeIds = new Set();
  const connections = g.connections.map((c, i) => {
    const from = byId.get(c.from), to = byId.get(c.to);
    if (!from || !to || from.id === to.id || from.type === 'output') throw new Error(tx('Connexion invalide.'));
    const isControl = NODE_TYPES[from.type].category === 'control';
    const param = NODE_TYPES[to.type].params.find(p => p.key === c.target);
    if (c.target === 'audio' ? isControl || !NODE_TYPES[to.type].inputs : !isControl || !param?.modulatable) throw new Error(tx('Les types des ports sont incompatibles.'));
    if (!Number.isFinite(c.amount ?? 1) || Math.abs(c.amount ?? 1) > 100000) throw new Error(tx('Intensité de modulation invalide.'));
    const id = typeof c.id === 'string' && c.id.length < 80 && !edgeIds.has(c.id) ? c.id : 'import-c-' + i;
    edgeIds.add(id);
    return {id, from:c.from, to:c.to, target:c.target, amount:c.amount ?? 1};
  });
  const cleanGraph = {nodes,connections,duration:g.duration,loop:!!g.loop};
  const performance = cleanPerformance(g.performance, cleanGraph);
  if (performance) cleanGraph.performance = performance;
  validateGraph(cleanGraph);
  return {graph:cleanGraph,presetId:PRESETS.some(p=>p.id === data.presetId) ? data.presetId : '',title:String(data.title || tx('Mon exploration sonore')).slice(0,80),titleCustom:data.titleCustom !== false,master:Number.isFinite(data.master) ? clamp(data.master,0,1) : .65,recipeRevision:Number.isInteger(data.recipeRevision) ? clamp(data.recipeRevision,1,SOUND_REVISION) : 1};
}

// Performance controls describe ordinary graph parameters. The audio graph,
// saved project and WAV renderer all receive the same parameter and edge values.
function performanceBinding(targetGraph = graph, metadata = targetGraph.performance) {
  if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) return null;
  const node = targetGraph.nodes.find(item => item.id === metadata.node);
  const schema = node && NODE_TYPES[node.type];
  const param = schema?.params.find(item => item.key === metadata.parameter);
  if (schema?.category !== 'control' || !param?.modulatable || param.options || param.min > 0 || param.max < 1) return null;
  const curve = targetGraph.nodes.find(item => item.id === metadata.curveNode && item.type === 'curve') || null;
  const edge = curve && targetGraph.connections.find(item => item.id === metadata.edge && item.from === curve.id && item.to === node.id && item.target === param.key) || null;
  return { metadata, node, param, curve, edge, mode: metadata.mode === 'live' ? 'live' : 'curve' };
}

function cleanPerformance(metadata, targetGraph) {
  const binding = performanceBinding(targetGraph, metadata);
  if (!binding) return null;
  const { node, param, curve, edge, mode } = binding;
  const clean = { node: node.id, parameter: param.key, mode, manualValue: clamp(Number.isFinite(metadata.manualValue) ? metadata.manualValue : node.params[param.key], 0, 1) };
  if (curve) clean.curveNode = curve.id;
  if (edge) clean.edge = edge.id;
  if (typeof metadata.label === 'string') clean.label = metadata.label.slice(0, 60);
  return clean;
}

function performanceCommand(binding = performanceBinding()) {
  if (!binding) return 0;
  const { node, param, curve, edge, mode } = binding;
  let value = Number(node.params[param.key]) || 0;
  if (mode === 'curve' && curve && edge && !curve.muted) value += curveAt(curve.curve, clamp(currentTime / graph.duration, 0, 1)) * curve.params.amount * edge.amount;
  return clamp(value, 0, 1);
}

function updatePerformanceValue() {
  const binding = performanceBinding();
  if (!binding) return;
  const value = Math.round(performanceCommand(binding) * 100);
  $('performanceValue').value = value;
  $('performanceReadout').textContent = value.toLocaleString(locale()) + ' %';
  $('performanceValue').setAttribute('aria-valuetext', value.toLocaleString(locale()) + ' %');
}

function renderPerformance() {
  const binding = performanceBinding();
  $('performancePanel').hidden = !binding;
  if (!binding) return;
  const { metadata, node, curve, edge, mode } = binding;
  const starting = !!startDriveSession;
  const label = tr(metadata.label || 'Pedal');
  $('performancePanel').setAttribute('aria-label', tr('Pedal control'));
  $('performanceLabel').textContent = label;
  $('performanceValue').setAttribute('aria-label', label);
  $('performanceValue').title = tr('Press to accelerate; release to settle at idle.');
  $('performanceValue').disabled = mode !== 'live' || !!node.muted;
  $('performanceLive').textContent = tr('Live');
  $('performanceCurve').textContent = tr('Curve');
  $('performanceLive').setAttribute('aria-pressed', String(mode === 'live'));
  $('performanceCurve').setAttribute('aria-pressed', String(mode === 'curve'));
  $('performanceLive').disabled = starting;
  $('performanceCurve').disabled = starting || !curve || !edge;
  $('performanceCurve').title = curve && edge ? tr('Use the single pedal curve.') : tr('Reconnect the pedal curve to use Curve mode.');
  $('editPerformanceCurve').textContent = tr('Edit pedal curve ↗');
  $('editPerformanceCurve').disabled = !curve;
  const startButton = $('startAndDrive');
  startButton.hidden = presetId !== 'v12' || !PRESETS.some(preset => preset.id === 'engine-start');
  startButton.disabled = starting || loadingAudio || !!node.muted;
  startButton.textContent = tr(starting ? 'Starting…' : 'Start & drive');
  startButton.title = tr('Play the separate start once, then drive. WAV exports the selected graph.');
  $('performanceHint').textContent = tr(node.muted
    ? 'The pedal module is muted. Enable it in the graph.'
    : starting ? 'While the engine starts, set the pedal for the drive.'
    : mode === 'live' ? 'Live holds the pedal at its current position for WAV export. Use Curve for an evolving gesture.'
    : !curve || !edge ? 'The pedal curve is disconnected. Switch to Live or reconnect it in the graph.'
    : 'The pedal curve drives the engine. Switch to Live to take over.');
  updatePerformanceValue();
}

function syncPerformanceParameter(binding) {
  updateNodeValue(binding.node, binding.param.key);
  if (selectedNode === binding.node.id || selectedEdge === binding.edge?.id) renderInspector();
}

function setPerformanceMode(mode, { takeOver = true } = {}) {
  const binding = performanceBinding();
  if (!binding || startDriveSession || !['live','curve'].includes(mode)) return;
  if (mode === 'curve' && (!binding.curve || !binding.edge)) return;
  const { metadata, node, param, edge } = binding;
  if (mode === binding.mode && (mode === 'live' ? !edge || edge.amount === 0 : edge.amount === 1 && node.params[param.key] === 0)) return;
  const value = takeOver && audio.isPlaying && binding.mode === 'curve'
    ? performanceCommand(binding)
    : clamp(Number.isFinite(metadata.manualValue) ? metadata.manualValue : node.params[param.key], 0, 1);
  remember();
  metadata.mode = mode;
  if (mode === 'live') {
    metadata.manualValue = value;
    node.params[param.key] = value;
    if (edge) edge.amount = 0;
  } else {
    metadata.manualValue = clamp(node.params[param.key], 0, 1);
    node.params[param.key] = 0;
    edge.amount = 1;
  }
  performanceGesture = false;
  syncPerformanceParameter(binding);
  commit({ immediateAudio: true });
}

function setPerformanceValue(value) {
  const binding = performanceBinding();
  if (!binding || binding.mode !== 'live' || binding.node.muted || !Number.isFinite(value)) return;
  value = clamp(value, 0, 1);
  if (binding.node.params[binding.param.key] === value && (!binding.edge || binding.edge.amount === 0)) return;
  if (!performanceGesture) { remember(); performanceGesture = true; }
  binding.metadata.manualValue = value;
  binding.node.params[binding.param.key] = value;
  if (binding.edge) binding.edge.amount = 0;
  syncPerformanceParameter(binding);
  commit({ immediateAudio: true });
}

function editPerformanceCurve() {
  const curve = performanceBinding()?.curve;
  if (!curve) return;
  selectNode(curve.id);
  focusNode(curve.id);
  $('curveEditor').querySelector('[data-curve-index]')?.focus?.({ preventScroll: true });
}

function cancelStartDrive() {
  if (!startDriveSession) return;
  startDriveSession = null;
  startDriveSerial++;
  loadingAudio = false;
  $('playButton').disabled = false;
  renderPerformance();
}

async function startAndDrive() {
  const binding = performanceBinding();
  if (!binding || binding.node.muted || loadingAudio || presetId !== 'v12' || !PRESETS.some(preset => preset.id === 'engine-start')) return;
  stopAudio();
  setPerformanceMode('live', { takeOver: false });
  const session = { graph, presetId, serial: ++startDriveSerial };
  startDriveSession = session;
  loadingAudio = true;
  $('playButton').disabled = true;
  renderPerformance();
  updateTransport();
  try {
    const starter = clonePreset('engine-start');
    starter.loop = false;
    const started = await audio.start(starter);
    if (startDriveSession !== session || startDriveSerial !== session.serial) return;
    if (started === false) { cancelStartDrive(); return; }
    audio.setMaster(master);
  } catch (error) {
    // AudioEngine reports its own errors through onError. Only report here if
    // the failure happened before that callback (for example an invalid preset).
    if (startDriveSession === session) { cancelStartDrive(); showToast(error.message || tx('Le moteur audio ne peut pas démarrer.'), true); }
  } finally {
    if (startDriveSession === session && startDriveSerial === session.serial) {
      loadingAudio = false;
      $('playButton').disabled = false;
      renderPerformance();
      updateTransport();
    }
  }
}

function loadLocal() {
  console.info('[Sona] Ouverture de la session');
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      const data = safeProject(JSON.parse(saved));
      ({graph,presetId,title,titleCustom,master,recipeRevision} = data);
      selectedNode = graph.nodes.find(n=>NODE_TYPES[n.type].category === 'source')?.id || graph.nodes[0]?.id || null;
      selectedCurve = graph.nodes.find(n=>n.type === 'curve')?.id || null;
    }
  } catch (error) { console.warn('[Sona] Session ignorée', error.message); }
}

function renderPresets() {
  console.info('[Sona] Affichage des presets');
  $('presetList').innerHTML = PRESETS.map(p => ui`<button type="button" class="preset-button ${p.id === presetId ? 'active' : ''}" data-preset="${p.id}" aria-pressed="${p.id === presetId}"><span class="preset-icon" style="color:${p.color}">${symbols[p.icon] || '∿'}</span><span><span class="preset-name">${escapeHTML(tr(p.name))}</span><span class="preset-subtitle">${escapeHTML(tr(p.subtitle))}</span></span><span class="preset-arrow">↗</span></button>`).join('');
  $('presetPicker').innerHTML = ui`<option value="" disabled ${!presetId ? 'selected' : ''}>${t('choosePreset')}</option>` + PRESETS.map(p => ui`<option value="${p.id}" ${p.id === presetId ? 'selected' : ''}>${escapeHTML(tr(p.name))} · ${escapeHTML(tr(p.subtitle))}</option>`).join('');
  $('presetPicker').setAttribute('aria-label', t('choosePreset'));
  $('presetDescription').textContent = t('moduleCount', {count: Object.values(NODE_TYPES).filter(schema => !schema.legacy).length});
  $('presetUpdate').hidden = !presetId || recipeRevision >= presetRevision(presetId);
  const updateText = $('presetUpdate').querySelector('span');
  if (updateText) updateText.textContent = tr(presetId === 'v12'
    ? 'Updated V12: load changes the pressure, intake and exhaust character. One pedal still controls the engine. Load the updated graph.'
    : presetId === 'engine-start'
      ? 'Updated start: continuous cranking, compression pulses and a progressive catch into idle. Load the updated graph.'
    : presetId === 'light-rain'
      ? 'Updated rain: distinct, irregular droplets over a softer background. Load the rebuilt graph.'
    : ['snow','footsteps-st2','gravel'].includes(presetId)
      ? 'Updated footstep: one gesture curve drives the contact, release and weight responses. Load the updated graph to use it.'
      : 'Updated presets use simple, reusable modules. Your saved project is kept until you load a preset.');
}

function renderLibrary() {
  const query = $('nodeSearch').value.trim().toLocaleLowerCase(locale());
  $('nodeLibrary').innerHTML = ['source','control','effect','output'].map(category => {
    const types = Object.entries(NODE_TYPES).filter(([type,s]) => !s.legacy && s.category === category && [s.name,s.description,tr(s.name,'en'),tr(s.name,'fr'),tr(s.description,'en'),tr(s.description,'fr')].join(' ').toLocaleLowerCase(locale()).includes(query));
    if (!types.length) return '';
    return ui`<section class="library-group"><h3>${{source:tx('Générateurs'),control:tx('Modulations'),effect:tx('Effets'),output:tx('Sortie')}[category]}</h3>${types.map(([type,s]) => ui`<button type="button" class="library-item" data-add="${type}" title="Ajouter : ${escapeHTML(tr(s.description))}"><span class="lib-icon" style="color:${s.color}">${symbols[s.icon] || '∿'}</span><span><span class="lib-title">${escapeHTML(tr(s.name))}</span><span class="lib-desc">${{pulse:tx('Fréquence, forme, irrégularité'),math:tr('Add · multiply · min/max · smoothstep'),comb:tx('Délai, retour, absorption'),engine:tx('12 combustions séparées'),reed:tx('Pression & instabilités'),bubble:tx('Impact & microbulles'),oscillator:tx('4 formes d’onde'),grain:tr('Density, pitch & micro-events'),noise:tr('5 colours · airflow & detail'),curve:tx('Évolution dessinée'),response:tr('One gesture · several envelopes'),motion:tr('Drive · inertia · stages'),lfo:tx('Variation cyclique'),envelope:tx('Attaque & relâchement'),random:tx('Dérive sans répétition'),resonator:tx('Cavités & matériaux'),formant:tx('Résonances mobiles'),equalizer:tx('Graves, corps, détail'),compressor:tx('Dynamique & présence'),space:tx('Réflexions en stéréo'),filter:tx('Couleur du son'),drive:tx('Grain & caractère'),gain:tx('Niveau & mélange'),delay:tx('Répétitions'),reverb:tx('Espace & profondeur'),output:tx('Destination audio')}[type]}</span></span><span class="lib-add">+</span></button>`).join('')}</section>`;
  }).join('') || tx('<p class="library-empty">Aucune brique trouvée.</p>');
}

function miniPreview(node) {
  const schema = NODE_TYPES[node.type];
  if (schema.category === 'control') return renderModulationPreview(node, modulationPreviews.frame(node.id), {name: nodeText(node)});
  const points = [];
  for (let i=0; i<=64; i++) {
    const x=i/64;
    let y;
    if (node.type === 'pulse') y=.18+.7*Math.exp(-((x*6)%1)*15);
    else if (node.type === 'comb') y=.5+Math.sin(x*100)*Math.exp(-x*2)*.35;
    else if (node.type === 'noise') y=.5+Math.sin(i*12.9898)*Math.cos(i*7.31)*.4;
    else if (node.type === 'bubble') y=.5+(Math.sin(x*150)+Math.sin(x*211)*.2)*Math.exp(-x*7)*.4;
    else if (node.type === 'resonator') y=.5+(Math.sin(x*63)+.25*Math.sin(x*107))*Math.exp(-x*3)*.3;
    else if (node.type === 'formant') y=.15+.6*Math.exp(-Math.pow((x-.2)/.09,2))+.42*Math.exp(-Math.pow((x-.54)/.1,2))+.25*Math.exp(-Math.pow((x-.85)/.08,2));
    else if (node.type === 'compressor') y=x<.45?x:.45+(x-.45)*.3;
    else if (node.type === 'space') y=.5+Math.sin(x*201)*Math.cos(x*57)*Math.exp(-x*2.8)*.4;
    else if (node.type === 'filter') y=.83/(1+Math.exp((x-.55)*14))+.08;
    else y=.5+(.3*Math.sin(x*58)+.1*Math.sin(x*116))*Math.sin(.3+x*2.7);
    points.push(ui`${i*3},${31-y*27}`);
  }
  return ui`<svg class="node-preview" viewBox="0 0 192 36" aria-hidden="true"><path d="M0 18H192" stroke="${schema.color}" opacity=".12"/><polyline points="${points.join(' ')}" fill="none" stroke="${schema.color}" stroke-width="1.4" opacity=".9"/></svg>`;
}

// Updated from bounded preview/playback callbacks. Replacing only the signal
// drawing preserves selection, keyboard focus, ports and active edit gestures.
function paintModulationPreviews(ids) {
  const affected = new Set(ids);
  for (const id of affected) {
    const node = graph.nodes.find(item => item.id === id);
    if (!node || NODE_TYPES[node.type].category !== 'control') continue;
    const preview = nodeElements.get(id)?.querySelector('.modulation-preview');
    if (preview) preview.outerHTML = miniPreview(node);
  }
  if (!affected.has(selectedNode)) return;
  const node = graph.nodes.find(item => item.id === selectedNode);
  const expanded = $('modulationInspectorPreview');
  if (expanded && node && NODE_TYPES[node.type].category === 'control') {
    expanded.innerHTML = renderModulationPreview(node, modulationPreviews.frame(node.id), {name: nodeText(node), expanded: true});
  }
}

function curveAt(points, x) {
  if (!points?.length) return 0;
  for (let i=1; i<points.length; i++) {
    if (x<=points[i][0]) {
      const [a,b] = [points[i-1],points[i]];
      const t=clamp((x-a[0])/(b[0]-a[0]||1),0,1);
      return a[1]+(b[1]-a[1])*t;
    }
  }
  return points.at(-1)[1];
}

function renderNodes() {
  console.info('[Sona] Affichage du graphe');
  modulationPreviews.setGraph(graph);
  nodeElements.clear();
  $('world').innerHTML = graph.nodes.map(node => {
    const s=NODE_TYPES[node.type];
    const control=s.category === 'control';
    const availableParams = node.type === 'math' ? s.params.filter(p=>parameterEnabled(node,p.key)||graph.connections.some(c=>c.to===node.id&&c.target===p.key)) : s.params;
    const visibleParams = node.type === 'math' ? availableParams : node.compact ? availableParams.filter((p,i)=>i<2||graph.connections.some(c=>c.to===node.id&&c.target===p.key)) : availableParams;
    return ui`<article class="node ${node.muted||node.bypass?'node-disabled':''} ${node.id === selectedNode ? 'selected':''}" data-node="${escapeHTML(node.id)}" style="left:${node.x}px;top:${node.y}px;--node-color:${s.color}" aria-label="${escapeHTML(nodeText(node))}"><header class="node-header"><span class="node-icon">${symbols[s.icon] || '∿'}</span><span class="node-title">${escapeHTML(nodeText(node))}</span><button class="node-expand" data-expand="${escapeHTML(node.id)}" aria-label="${node.compact?tx('Afficher tous les ports de'):tx('Réduire')} ${escapeHTML(nodeText(node))}" title="${node.compact?tx('Afficher tous les paramètres'):tx('Réduire le nœud')}">${node.compact?'⌄':'⌃'}</button><button class="node-delete" data-delete="${escapeHTML(node.id)}" aria-label="Supprimer ${escapeHTML(nodeText(node))}" title="Supprimer">×</button></header><div class="node-body"><div class="node-subtitle">${escapeHTML(tr(s.name))}</div>${miniPreview(node)}${visibleParams.map(p => ui`<div class="node-param" data-param-row="${p.key}">${p.modulatable ? ui`<button ${parameterEnabled(node,p.key)?'':'disabled'} class="port port-in control-port ${graph.connections.some(c=>c.to===node.id && c.target===p.key)?'connected':''}" data-input="${escapeHTML(node.id)}" data-target="${p.key}" aria-label="${escapeHTML(t('modulate', {parameter:tr(p.label),name:nodeText(node)}))}" title="${parameterEnabled(node,p.key)?tr('Entrée de modulation :')+' '+tr(p.label):tr(parameterAvailability(node,p.key))}"></button>`:''}<span>${escapeHTML(tr(p.label))}</span><span class="node-param-value ${graph.connections.some(c=>c.to===node.id && c.target===p.key)?'param-connected':''}" data-value="${p.key}">${escapeHTML(formatValue(node.params[p.key],p))}</span></div>`).join('')}</div><footer class="node-footer"><span>${s.inputs?ui`<button class="port port-in audio-port" data-input="${escapeHTML(node.id)}" data-target="audio" aria-label="${escapeHTML(t('inputOf', {name:nodeText(node)}))}" title="${node.type === 'output' ? t('outputPort') : tr('Entrée audio')}"></button>Entrée audio`:control?tx('Contrôle temporel'):tx('Source audio')}</span><span>${node.type!=='output'?ui`${control?'Modulation':'Signal'}<button class="port port-out ${control?'control-port':'audio-port'}" data-output="${escapeHTML(node.id)}" aria-label="${escapeHTML(t('outputOf', {kind:control?tr('Modulation'):'audio',name:nodeText(node)}))}" title="Relier cette sortie"></button>`:tx('Écoute ◖')}</span></footer></article>`;
  }).join('');
  document.querySelectorAll('.node').forEach(el=>nodeElements.set(el.dataset.node,el));
  updateView();
}

function toggleNodePorts(id) {
  console.info('[Sona] Affichage des ports', id);
  const node=graph.nodes.find(n=>n.id===id);
  if(!node)return;
  node.compact=!node.compact;
  renderNodes();
  commit({audioChanged:false});
}

function toggleNodeEnabled() {
  console.info('[Sona] Activation de la brique');
  const node=graph.nodes.find(n=>n.id===selectedNode);
  if(!node)return;
  remember();
  if(NODE_TYPES[node.type].category==='effect') node.bypass=!node.bypass;
  else node.muted=!node.muted;
  renderNodes();renderInspector();commit();
}

function updateNodeValue(node, key) {
  const element=nodeElements.get(node.id)?.querySelector(ui`[data-value="${key}"]`);
  const param=NODE_TYPES[node.type].params.find(p=>p.key===key);
  if(element) element.textContent=formatValue(node.params[key],param);
}

function portPosition(id, target) {
  const el = nodeElements.get(id)?.querySelector(target ? ui`[data-input][data-target="${target}"]` : '[data-output]');
  if (!el) return null;
  const rect=el.getBoundingClientRect(), canvas=$('graphViewport').getBoundingClientRect();
  return { x:rect.left+rect.width/2-canvas.left,y:rect.top+rect.height/2-canvas.top };
}

function cablePath(a,b) {
  const bend=Math.max(45,Math.abs(b.x-a.x)*.48);
  return ui`M${a.x},${a.y} C${a.x+bend},${a.y} ${b.x-bend},${b.y} ${b.x},${b.y}`;
}

function renderEdges() {
  $('edges').innerHTML = graph.connections.map(c=>{
    const a=portPosition(c.from),b=portPosition(c.to,c.target);
    if (!a || !b) return '';
    const d=cablePath(a,b);
    const target=graph.nodes.find(n=>n.id===c.to);
    const parameter=NODE_TYPES[target?.type]?.params.find(p=>p.key===c.target);
    return ui`<g class="edge ${c.target!=='audio'?'control':''} ${c.id===selectedEdge?'selected':''}" data-edge="${escapeHTML(c.id)}"><path class="edge-line" d="${d}"/><path class="edge-hit" d="${d}" tabindex="0" role="button" aria-label="${c.target==='audio'?tx('Liaison audio'):tr('Modulation')+' '+escapeHTML(tr(parameter?.label||c.target))}"/></g>`;
  }).join('');
  if(pendingCable) {
    const a=portPosition(pendingCable.from);
    if(a) $('edges').innerHTML += ui`<path class="pending-edge ${pendingCable.kind==='control'?'control':''}" d="${cablePath(a,cursor)}"/>`;
  }
}

function updateView() {
  $('world').style.transform=ui`translate(${view.x}px,${view.y}px) scale(${view.scale})`;
  $('graphViewport').style.backgroundSize=ui`${24*view.scale}px ${24*view.scale}px`;
  $('graphViewport').style.backgroundPosition=ui`${view.x}px ${view.y}px`;
  $('zoomValue').textContent=Math.round(view.scale*100)+' %';
  renderEdges();
}

function fitGraph() {
  console.info('[Sona] Cadrage du graphe');
  if(!graph.nodes.length) { view={x:40,y:40,scale:1}; updateView(); return; }
  const width=$('graphViewport').clientWidth,height=$('graphViewport').clientHeight;
  const minX=Math.min(...graph.nodes.map(n=>n.x)),minY=Math.min(...graph.nodes.map(n=>n.y));
  const maxX=Math.max(...graph.nodes.map(n=>n.x+220)),maxY=Math.max(...graph.nodes.map(n=>n.y+(nodeElements.get(n.id)?.offsetHeight||220)));
  const scale=clamp(Math.min((width-90)/(maxX-minX),(height-95)/(maxY-minY)),.12,1.1);
  view={scale,x:(width-(maxX-minX)*scale)/2-minX*scale,y:(height-(maxY-minY)*scale)/2-minY*scale};
  updateView();
}

function zoom(factor, cx=$('graphViewport').clientWidth/2, cy=$('graphViewport').clientHeight/2) {
  const scale=clamp(view.scale*factor,.12,1.8);
  view.x=cx-(cx-view.x)*scale/view.scale;
  view.y=cy-(cy-view.y)*scale/view.scale;
  view.scale=scale;
  updateView();
}

function focusNode(id) {
  const node = graph.nodes.find(n => n.id === id), element = nodeElements.get(id);
  if (!node || !element) return;
  selectNode(id);
  const viewport = $('graphViewport');
  view = { scale: .95, x: viewport.clientWidth / 2 - (node.x + element.offsetWidth / 2) * .95, y: viewport.clientHeight / 2 - (node.y + element.offsetHeight / 2) * .95 };
  updateView();
}

function selectNode(id) {
  console.info('[Sona] Sélection du nœud', id);
  selectedNode=id;
  selectedEdge=null;
  nodeElements.forEach((el,key)=>el.classList.toggle('selected',key===id));
  const node=graph.nodes.find(n=>n.id===id);
  if(node?.type==='curve') selectedCurve=id;
  renderInspector();
  renderCurveSelect();
  renderCurve();
  renderEdges();
}

function selectEdge(id) {
  console.info('[Sona] Sélection de la liaison', id);
  selectedEdge=id;
  selectedNode=null;
  nodeElements.forEach(el=>el.classList.remove('selected'));
  renderInspector();
  renderEdges();
}

function parameterEnabled(node, key) {
  if (node.type === 'math') {
    if (key === 'b') return node.params.operation !== 'smoothstep';
    if (key === 'edge0' || key === 'edge1') return node.params.operation === 'smoothstep';
  }
  if (node.type === 'response' && node.params.mode === 'follow' && key === 'sensitivity') return false;
  if (node.type === 'grain') {
    if (key === 'resonance') return node.params.wave === 'filtered-noise';
    if (['frequency','spread','glide'].includes(key)) return node.params.wave !== 'noise';
  }
  return true;
}

function parameterAvailability(node, key) {
  if (node.type === 'math') return key === 'b' ? 'Smoothstep uses Value A and the two edges. Value B is retained for other operations.' : 'Only used for Smoothstep.';
  if (node.type === 'response') return 'Only used for rising or falling motion.';
  return key === 'resonance' ? 'Only used for filtered-noise grains.' : 'Used for sine and filtered-noise grains.';
}

function parameterControl(node,p) {
  const value=node.params[p.key];
  const enabled=parameterEnabled(node,p.key);
  const disabled=enabled?'':'disabled';
  const incoming=graph.connections.filter(c=>c.to===node.id&&c.target===p.key);
  const input=p.options ? ui`<select ${disabled} class="parameter-select" data-change-param="${p.key}" aria-label="${escapeHTML(tr(p.label))}">${p.options.map(o=>ui`<option value="${o.value}" ${o.value===value?'selected':''}>${escapeHTML(tr(o.label))}</option>`).join('')}</select>` : ui`<input ${disabled} class="parameter-slider" data-change-param="${p.key}" type="range" ${p.scale==='log'?'min="0" max="1000" step="1"':ui`min="${p.min}" max="${p.max}" step="${p.step}"`} value="${p.scale==='log'?Math.round(Math.log(value/p.min)/Math.log(p.max/p.min)*1000):value}" aria-label="${escapeHTML(tr(p.label))}"><div class="parameter-limits"><span>${formatValue(p.min,p)}</span><input ${disabled} class="parameter-number" data-number-param="${p.key}" type="number" min="${p.min}" max="${p.max}" step="${p.step}" value="${value}" aria-label="Valeur exacte : ${escapeHTML(tr(p.label))}"><span>${formatValue(p.max,p)}</span></div>`;
  return ui`<div class="parameter ${enabled?'':'parameter-inactive'}"><div class="parameter-heading"><label>${escapeHTML(tr(p.label))}${incoming.length ? ' <small class="parameter-base">base</small>' : ''}</label><output data-param-output="${p.key}">${escapeHTML(formatValue(value,p))}</output></div>${input}${!enabled?ui`<p class="parameter-availability">${tr(parameterAvailability(node,p.key))}</p>`:''}${incoming.map(c=>ui`<button class="connection-pill" data-select-edge="${escapeHTML(c.id)}">⌁ ${escapeHTML(nodeText(graph.nodes.find(n=>n.id===c.from))||tr('Modulation'))} <span>× ${formatValue(c.amount,{step:.000001})}</span></button>`).join('')}</div>`;
}

function mathInspectorHelp(node) {
  console.info('[Sona] Showing modulation math guidance', node.params.operation);
  const message=node.params.operation==='smoothstep'
    ? 'Connect modulation to Value A with its base at 0. Start edge maps to 0 and End edge to 1, with a smooth transition between them. Reverse the edges for a falling transition; equal edges produce a hard step. For smoothing over time, use Control response.'
    : 'For two modulation signals, set the Value A and Value B bases to 0, then connect the purple ports. Leave an input unconnected to use its base as a constant. Multiple cables into one input are added together.';
  return ui`<div class="inspector-note">${escapeHTML(tr(message))}</div>`;
}

function renderInspector() {
  console.info('[Sona] Inspecteur');
  if(selectedEdge) {
    const c=graph.connections.find(e=>e.id===selectedEdge);
    if(!c) {selectedEdge=null; return renderInspector();}
    const from=graph.nodes.find(n=>n.id===c.from),to=graph.nodes.find(n=>n.id===c.to);
    const p=NODE_TYPES[to.type].params.find(p=>p.key===c.target);
    const range=p ? Math.max(Math.abs(p.max-p.min),1) : 2;
    $('inspector').innerHTML=ui`<div class="inspector-header"><span class="inspector-category">${p?'MODULATION':tx('LIAISON AUDIO')}</span><h2 class="inspector-title">${p?tx('Faire évoluer le son'):tx('Le chemin du signal')}</h2><p class="inspector-description">${p?tx('Le signal de contrôle fait varier la valeur de base. Une intensité négative inverse son effet.'):to.type==='output'?t('outputPort'):tx('Les signaux qui arrivent sur la même entrée sont additionnés.')}</p></div><div class="inspector-content"><div class="connection-route"><span>${escapeHTML(nodeText(from))}</span><span class="route-arrow">↓</span><span>${escapeHTML(nodeText(to))}${p?' · '+escapeHTML(tr(p.label)):''}</span></div><div class="parameter"><div class="parameter-heading"><label>${p?tx('Intensité'):tx('Niveau de liaison')}</label><output id="edgeAmountValue">${formatValue(c.amount,p)}</output></div><input id="edgeAmount" class="parameter-slider" type="range" min="${-range}" max="${range}" step="${p?.step||.01}" value="${c.amount}" aria-label="Intensité de la liaison"><input id="edgeAmountNumber" class="parameter-number" type="number" min="-100000" max="100000" step="${p?.step||.01}" value="${c.amount}" aria-label="Intensité exacte de la liaison"></div>${p?ui`<div class="modulation-formula">${escapeHTML(tr(p.label))} = valeur de base<br>+ modulation × intensité</div>`:''}<button class="danger-button" id="deleteEdge">Déconnecter la liaison</button></div>`;
    return;
  }
  const node=graph.nodes.find(n=>n.id===selectedNode);
  if(!node) {
    $('inspector').innerHTML=tx('<div class="empty-inspector"><span class="empty-icon">⌁</span><h2>Chaque son commence par une idée.</h2><p>Sélectionnez une brique pour régler ses paramètres, ou une liaison pour ajuster sa modulation.</p><span class="port-help"><i></i> Audio <i class="purple"></i> Modulation</span></div>');
    return;
  }
  const s=NODE_TYPES[node.type];
  const parameters=node.type==='math' ? [...s.params].sort((a,b)=>a.key==='operation'?-1:b.key==='operation'?1:0) : s.params;
  $('inspector').innerHTML=ui`<div class="inspector-header" style="--node-color:${s.color}"><span class="inspector-category">${tr(categoryName[s.category])}</span><h2 class="inspector-title">${escapeHTML(nodeText(node))}</h2><p class="inspector-description">${escapeHTML(tr(s.description))}</p></div><div class="inspector-content"><button id="toggleNodeEnabled" type="button" class="node-state-toggle ${node.muted||node.bypass?'inactive':''}" aria-pressed="${!node.muted&&!node.bypass}"><span>${node.muted||node.bypass?'○':'●'}</span>${s.category==='effect'?(node.bypass?tx('Effet contourné · réactiver'):tx('Effet actif · contourner')):(node.muted?tx('Brique muette · réactiver'):tx('Brique active · rendre muette'))}</button>${s.category==='control'?`<div id="modulationInspectorPreview">${renderModulationPreview(node,modulationPreviews.frame(node.id),{name:nodeText(node),expanded:true})}</div>`:''}${node.type==='math'?mathInspectorHelp(node):''}${parameters.map(p=>parameterControl(node,p)).join('')}${node.type==='curve'?tx('<div class="inspector-note">↙ Modifiez les points dans le panneau Automation.</div>'):''}<div class="inspector-divider"></div><label class="node-name-label" for="nodeLabel">Nom de la brique</label><input id="nodeLabel" class="node-name-input" maxlength="60" value="${escapeHTML(nodeText(node))}"><div class="node-actions"><button class="secondary-button" id="duplicateNode">Dupliquer</button><button class="danger-button" id="deleteNode">Supprimer</button></div></div>`;
}

function renderCurveSelect() {
  const curves=graph.nodes.filter(n=>n.type==='curve');
  if(!curves.some(n=>n.id===selectedCurve)) selectedCurve=curves[0]?.id||null;
  $('curveSelect').innerHTML=curves.length?curves.map(n=>ui`<option value="${escapeHTML(n.id)}" ${n.id===selectedCurve?'selected':''}>${escapeHTML(nodeText(n))}</option>`).join(''):tx('<option value="">Aucune courbe</option>');
  $('curveSelect').disabled=!curves.length;
  document.querySelectorAll('.curve-preset').forEach(b=>b.disabled=!curves.length);
  $('curveHint').textContent=curves.length?tx('Glisser un point · double-clic : ajouter · clic droit : retirer'):tx('Ajoutez une Courbe temporelle depuis la bibliothèque pour dessiner une modulation.');
}

function curveBounds() {
  const width=$('curveEditor').clientWidth||700,height=$('curveEditor').clientHeight||130;
  return {width,height,left:38,right:width-20,top:12,bottom:height-25};
}

function renderCurve() {
  const node=graph.nodes.find(n=>n.id===selectedCurve);
  const b=curveBounds();
  const point=([x,y])=>[b.left+x*(b.right-b.left),b.bottom-y*(b.bottom-b.top)];
  $('curveEditor').setAttribute('viewBox',ui`0 0 ${b.width} ${b.height}`);
  let html='';
  for(let i=0;i<=4;i++) {
    const x=b.left+(b.right-b.left)*i/4;
    html+=ui`<path class="curve-grid" d="M${x},${b.top}V${b.bottom}"/><text class="curve-label" x="${x}" y="${b.height-6}" text-anchor="${i===4?'end':i===0?'start':'middle'}">${(graph.duration*i/4).toLocaleString(locale(),{maximumFractionDigits:1})} s</text>`;
  }
  [0,.5,1].forEach(y=>{
    const py=point([0,y])[1];
    html+=ui`<path class="curve-grid" d="M${b.left},${py}H${b.right}"/><text class="curve-label" x="${b.left-10}" y="${py+4}" text-anchor="end">${Math.round(y*100)} %</text>`;
  });
  if(node) {
    const coords=node.curve.map(point),d='M'+coords.map(p=>p.join(',')).join(' L');
    html+=ui`<path class="curve-fill" d="${d} L${b.right},${b.bottom} L${b.left},${b.bottom}Z"/><path class="curve-line" d="${d}"/>`;
    html+=coords.map(([x,y],i)=>ui`<g class="curve-point" data-curve-index="${i}" tabindex="0" role="slider" aria-label="${t('point', {number:i+1,position:Math.round(node.curve[i][0]*100)})}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(node.curve[i][1]*100)}"><circle class="point-hit" cx="${x}" cy="${y}" r="11"/><circle class="point-dot" cx="${x}" cy="${y}" r="4"/></g>`).join('');
  } else html+=ui`<text class="curve-empty-label" x="${(b.width)/2}" y="${b.height/2}" text-anchor="middle">Les modulations donnent vie aux paramètres</text>`;
  html+=ui`<path id="playhead" class="curve-playhead" d="M${b.left},${b.top}V${b.bottom}"/>`;
  $('curveEditor').innerHTML=html;
  movePlayhead();
}

function movePlayhead() {
  const b=curveBounds(),x=b.left+clamp((startDriveSession ? 0 : currentTime)/graph.duration,0,1)*(b.right-b.left);
  const head=$('playhead');
  if(head) {head.setAttribute('d',ui`M${x},${b.top}V${b.bottom}`);head.style.opacity=audio.isPlaying?'1':'.25';}
}

function renderAll() {
  console.info('[Sona] Mise à jour de l’atelier');
  $('projectTitle').textContent=projectText();
  $('durationInput').value=graph.duration;
  $('masterVolume').value=Math.round(master*100);
  $('volumeValue').textContent=Math.round(master*100)+' %';
  renderPresets();
  renderNodes();
  renderInspector();
  renderCurveSelect();
  renderCurve();
  renderPerformance();
  updateHistoryButtons();
  updateTransport();
  comparison.graphChanged(false);
  $('nodeCount').textContent=t('nodeCount', {nodes: graph.nodes.length, connections: graph.connections.length});
}

async function loadPreset(id) {
  console.info('[Sona] Choix du preset',id);
  const wasPlaying=audio.isPlaying;
  cancelStartDrive();
  clearTimeout(audioUpdateTimer);
  audio.stop();
  remember();
  graph=clonePreset(id);
  recipeRevision=presetRevision(id);
  presetId=id;
  const preset=PRESETS.find(p=>p.id===id);
  title=preset.name+' · '+preset.subtitle.toLocaleLowerCase(locale());
  titleCustom=false;
  selectedNode=graph.nodes.find(n=>NODE_TYPES[n.type].category==='source')?.id||null;
  selectedEdge=null;
  selectedCurve=graph.nodes.find(n=>n.type==='curve')?.id||null;
  currentTime=0;
  pendingCable=null;
  renderAll();
  requestAnimationFrame(()=>$('presetList').querySelector('[aria-pressed="true"]')?.scrollIntoView({block:'nearest',inline:'nearest'}));
  fitGraph();
  commit();
  if(wasPlaying) await startAudio();
}

function addNode(type) {
  console.info('[Sona] Ajouter',type);
  if(graph.nodes.length>=40) return showToast(tx('Ce projet contient déjà 40 nœuds.'),true);
  remember();
  const width=$('graphViewport').clientWidth,height=$('graphViewport').clientHeight;
  const originX=(width*.4-view.x)/view.scale,originY=(height*.3-view.y)/view.scale;
  let x=originX,y=originY;
  for (let attempt=0;attempt<120;attempt++) {
    x=originX+(attempt%3)*265; y=originY+Math.floor(attempt/3)*280;
    const overlaps=graph.nodes.some(existing=>x<existing.x+245&&x+245>existing.x&&y<existing.y+(nodeElements.get(existing.id)?.offsetHeight||220)+25&&y+245>existing.y);
    if(!overlaps)break;
  }
  const node=makeNode(type,uniqueId(),Math.round(x),Math.round(y));
  node.labelCustom = false;
  graph.nodes.push(node);
  selectedNode=node.id;
  selectedEdge=null;
  if(type==='curve') selectedCurve=node.id;
  renderNodes();
  renderInspector();
  renderCurveSelect();
  renderCurve();
  if(x*view.scale+view.x<0||(x+220)*view.scale+view.x>width||y*view.scale+view.y<0||(y+245)*view.scale+view.y>height)fitGraph();
  commit();
  showToast(()=>t('moduleAdded', {name:nodeText(node)}));
}

function deleteNode(id=selectedNode) {
  console.info('[Sona] Supprimer',id);
  if(!graph.nodes.some(n=>n.id===id)) return;
  if(startDriveSession && id === graph.performance?.node) stopAudio();
  remember();
  graph.nodes=graph.nodes.filter(n=>n.id!==id);
  graph.connections=graph.connections.filter(c=>c.from!==id&&c.to!==id);
  if(selectedNode===id) selectedNode=null;
  pendingCable=null;
  selectedEdge=null;
  renderAll();
  commit();
}

function duplicateNode() {
  console.info('[Sona] Dupliquer');
  const original=graph.nodes.find(n=>n.id===selectedNode);
  if(!original||graph.nodes.length>=40) return;
  remember();
  const copy=structuredClone(original);
  copy.id=uniqueId();
  copy.x+=50;copy.y+=60;copy.label=nodeText(original)+tx(' · copie');copy.labelCustom=true;
  graph.nodes.push(copy);
  selectedNode=copy.id;
  renderAll();commit();
}

function createsCycle(from,to) {
  const visited=new Set(),stack=[to];
  while(stack.length) {
    const id=stack.pop();
    if(id===from) return true;
    if(visited.has(id)) continue;
    visited.add(id);
    graph.connections.filter(c=>c.from===id).forEach(c=>stack.push(c.to));
  }
  return false;
}

function connect(to,target) {
  console.info('[Sona] Connexion',to,target);
  if(!pendingCable) return;
  const {from,kind}=pendingCable;
  pendingCable=null;
  document.body.classList.remove('connecting');
  const dest=graph.nodes.find(n=>n.id===to);
  if(!dest) return renderEdges();
  if(target!=='audio'&&!parameterEnabled(dest,target)){renderEdges();return showToast(tr(parameterAvailability(dest,target)));}
  if((kind==='control')!==(target!=='audio')) {renderEdges();return showToast(tx('Reliez audio à audio, ou modulation à un paramètre violet.'),true);}
  if(from===to||createsCycle(from,to)) {renderEdges();return showToast(tx('Cette liaison créerait une boucle. Utilisez la brique Écho pour la réinjection.'),true);}
  const exclusiveOutput = target === 'audio' && dest.type === 'output';
  const existing = graph.connections.find(c=>c.from===from&&c.to===to&&c.target===target);
  const previousInputs = exclusiveOutput ? graph.connections.filter(c=>c.to===to&&c.target==='audio') : [];
  if(existing && (!exclusiveOutput || previousInputs.length === 1)) {renderEdges();return showToast(tx('Cette liaison existe déjà.'));}
  remember();
  const p=NODE_TYPES[dest.type].params.find(p=>p.key===target);
  let amount=1;
  if(p) amount={input:1,rpm:3000,frequency:200,cutoff:3000,rate:1,gain:1,resonance:1}[p.key]??Math.min(1,(p.max-p.min)*.5);
  if(exclusiveOutput) graph.connections=graph.connections.filter(c=>!(c.to===to&&c.target==='audio'));
  const c=existing || {id:uniqueId('c'),from,to,target,amount};
  graph.connections.push(c);
  if(graph.performance?.curveNode === from && graph.performance.node === to && graph.performance.parameter === target) {
    graph.performance.edge = c.id;
    if(graph.performance.mode === 'live') c.amount = 0;
  }
  selectedEdge=c.id;
  selectedNode=null;
  renderNodes();renderInspector();commit();
  showToast(p?tx('Modulation reliée · ajustez son intensité à droite.'):exclusiveOutput&&previousInputs.length?tx('Connexion de sortie remplacée.'):tx('Liaison audio créée.'));
}

function deleteEdge() {
  console.info('[Sona] Déconnecter');
  if(!selectedEdge) return;
  remember();
  graph.connections=graph.connections.filter(c=>c.id!==selectedEdge);
  selectedEdge=null;
  renderNodes();renderInspector();commit();
}

async function startAudio() {
  console.info('[Sona] Lecture');
  if(loadingAudio) return;
  comparison.stop();
  loadingAudio=true;
  $('playButton').disabled=true;
  renderPerformance();
  try {
    validateGraph(graph);
    if(!graph.nodes.some(n=>n.type==='output')) throw new Error(tx('Ajoutez une Sortie audio et reliez-y votre signal.'));
    if(!graph.connections.some(c=>graph.nodes.find(n=>n.id===c.to)?.type==='output'&&c.target==='audio')) throw new Error(tx('Reliez une source à la Sortie audio.'));
    await audio.start(graph);
    audio.setMaster(master);
  } catch(error) { showToast(error.message||tx('Le moteur audio ne peut pas démarrer.'),true); }
  finally {loadingAudio=false;$('playButton').disabled=false;renderPerformance();updateTransport();}
}

async function toggleAudio() {
  console.info('[Sona] Lecture / arrêt');
  if(audio.isPlaying || startDriveSession) stopAudio(); else await startAudio();
}

function stopAudio() {
  console.info('[Sona] Arrêt');
  cancelStartDrive();
  clearTimeout(audioUpdateTimer);
  audio.stop();
  comparison.stop();
  currentTime=0;
  updateTransport();movePlayhead();
}

function updateTransport() {
  const playing=audio.isPlaying;
  $('playButton').innerHTML=playing?tx('<span aria-hidden="true">Ⅱ</span> Arrêter'):tx('<span aria-hidden="true">▶</span> Écouter');
  $('playButton').setAttribute('aria-pressed',String(playing));
  $('playButton').setAttribute('aria-label',playing?tx('Arrêter la lecture'):tx('Écouter le son'));
  $('loopButton').classList.toggle('active',graph.loop);
  $('loopButton').setAttribute('aria-pressed',String(graph.loop));
  const minutes=Math.floor(currentTime/60),seconds=Math.floor(currentTime%60),milliseconds=Math.floor((currentTime%1)*1000);
  $('timeDisplay').textContent=ui`${minutes}:${String(seconds).padStart(2,'0')}${getLanguage()==='fr'?',':'.'}${String(milliseconds).padStart(3,'0')}`;
  $('audioState').textContent=startDriveSession?tr('STARTING'):playing?tx('EN DIRECT'):tx('EN VEILLE');
  $('audioState').classList.toggle('live',playing);
  $('statusText').textContent=startDriveSession?tr('Starting engine…'):playing?tx('Synthèse en cours'):tx('Prêt à jouer');
  document.documentElement.dataset.audioState=playing?'playing':'stopped';
  updatePerformanceValue();
}

function downloadBlob(blob,name) {
  console.info('[Sona] Téléchargement',name);
  const url=URL.createObjectURL(blob),a=document.createElement('a');
  a.href=url;a.download=name;document.body.append(a);a.click();a.remove();
  setTimeout(()=>URL.revokeObjectURL(url),10000);
}

function saveProject() {
  console.info('[Sona] Export du projet');
  const data={version:2,recipeRevision,title,titleCustom,presetId,graph,master};
  downloadBlob(new Blob([JSON.stringify(data,null,2)],{type:'application/json'}),'sona-'+(presetId||'projet')+'.json');
  $('projectMenu').hidden=true;
  showToast(tx('Projet exporté avec ses nœuds, ses réglages et ses courbes.'));
}

async function importProject(file) {
  console.info('[Sona] Import du projet');
  if(!file) return;
  if(file.size>1000000) return showToast(tx('Ce fichier est trop volumineux (1 Mo maximum).'),true);
  try {
    const data=safeProject(JSON.parse(await file.text()));
    stopAudio();remember();
    ({graph,presetId,title,titleCustom,master,recipeRevision}=data);
    audio.setMaster(master);
    selectedNode=graph.nodes[0]?.id||null;
    selectedEdge=null;selectedCurve=null;
    renderAll();fitGraph();commit();
    showToast(tx('Projet ouvert.'));
  } catch(error) {showToast(error.message||tx('Impossible d’ouvrir ce projet.'),true);}
  $('fileInput').value='';
}

async function exportWav() {
  console.info('[Sona] Export WAV');
  exporting=true;
  $('exportButton').disabled=true;
  $('exportButton').textContent=tx('Rendu en cours…');
  try {
    const blob=await audio.renderWav(graph,{duration:graph.duration,master});
    downloadBlob(blob,'sona-'+(presetId||'creation')+'.wav');
    const seconds=graph.duration;
    showToast(()=>t('exportDone', {seconds:seconds.toLocaleString(locale())}));
  } catch(error) {showToast(error.message||tx('L’export audio a échoué.'),true);}
  finally {exporting=false;$('exportButton').disabled=false;$('exportButton').textContent=tx('↓ Exporter WAV');}
}

function newProject() {
  console.info('[Sona] Nouveau projet');
  stopAudio();remember();
  graph={nodes:[makeNode('output',uniqueId('output'),650,180)],connections:[],duration:8,loop:true};
  recipeRevision=SOUND_REVISION;
  presetId='';title=tx('Mon exploration sonore');titleCustom=false;selectedNode=graph.nodes[0].id;selectedEdge=null;selectedCurve=null;
  $('projectMenu').hidden=true;
  renderAll();fitGraph();commit();
  showToast(tx('Graphe prêt · ajoutez un générateur à gauche.'));
}

function updateParameter(input) {
  console.info('[Sona] Updating a node parameter', input.dataset.changeParam||input.dataset.numberParam);
  const node=graph.nodes.find(n=>n.id===selectedNode);
  if(!node) return;
  const key=input.dataset.changeParam||input.dataset.numberParam,p=NODE_TYPES[node.type].params.find(p=>p.key===key);
  if(!p) return;
  if(!parameterGesture) {remember();parameterGesture=true;}
  let value=input.value;
  if(!p.options) {
    value=Number(value);
    if(!Number.isFinite(value)) return;
    if(p.scale==='log'&&input.dataset.changeParam) value=p.min*Math.pow(p.max/p.min,value/1000);
    value=clamp(Math.round(value/p.step)*p.step,p.min,p.max);
    value=Number(value.toFixed(8));
  }
  node.params[key]=value;
  const performance = performanceBinding();
  if(performance?.mode === 'live' && performance.node === node && performance.param.key === key) performance.metadata.manualValue = clamp(value,0,1);
  const output=$('inspector').querySelector(ui`[data-param-output="${key}"]`);
  if(output) output.textContent=formatValue(value,p);
  const number=$('inspector').querySelector(ui`[data-number-param="${key}"]`);
  if(number&&number!==input) number.value=value;
  const slider=$('inspector').querySelector(ui`[data-change-param="${key}"]`);
  if(slider&&slider!==input) slider.value=p.scale==='log'?Math.round(Math.log(value/p.min)/Math.log(p.max/p.min)*1000):value;
  updateNodeValue(node,key);
  if((node.type==='grain'&&key==='wave')||(node.type==='response'&&key==='mode')||(node.type==='math'&&key==='operation')){const focused=document.activeElement===input;renderNodes();renderInspector();if(focused)$('inspector').querySelector(`[data-change-param="${key}"]`)?.focus();parameterGesture=false;}
  commit();
}

function updateEdgeAmount(input) {
  const c=graph.connections.find(c=>c.id===selectedEdge);
  if(!c) return;
  const value=Number(input.value);
  if(!Number.isFinite(value)) return;
  if(!parameterGesture) {remember();parameterGesture=true;}
  c.amount=clamp(value,-100000,100000);
  const to=graph.nodes.find(n=>n.id===c.to),p=NODE_TYPES[to.type].params.find(p=>p.key===c.target);
  $('edgeAmountValue').textContent=formatValue(c.amount,p);
  if(input.id==='edgeAmount') $('edgeAmountNumber').value=c.amount;
  else $('edgeAmount').value=c.amount;
  commit();
}

function setCurvePreset(kind) {
  console.info('[Sona] Forme de courbe',kind);
  const node=graph.nodes.find(n=>n.id===selectedCurve);
  if(!node) return;
  remember();
  node.curve=({ramp:[[0,0],[.2,.1],[.8,1],[1,.1]],pluck:[[0,0],[.04,1],[.18,.45],[.5,.05],[1,0]],flat:[[0,.5],[1,.5]]})[kind];
  renderCurve();updateCurveThumbnail(node);commit();
}

function updateCurveThumbnail(node) {
  if (NODE_TYPES[node.type].category === 'control') { modulationPreviews.setGraph(graph); return; }
  const el=nodeElements.get(node.id)?.querySelector('.node-preview');
  if(el) el.outerHTML=miniPreview(node);
}

function curveCoordinates(event) {
  const r=$('curveEditor').getBoundingClientRect(),b=curveBounds();
  return [clamp((event.clientX-r.left-b.left)/(b.right-b.left),0,1),clamp(1-(event.clientY-r.top-b.top)/(b.bottom-b.top),0,1)];
}

function bindEvents() {
  console.info('[Sona] Initialisation des commandes');
  $('languageSelect').addEventListener('change',event=>setLanguage(event.target.value));
  $('presetPicker').addEventListener('change',event=>loadPreset(event.target.value));
  $('reloadPreset').addEventListener('click',()=>loadPreset(presetId||'v12'));
  $('nodeSearch').addEventListener('input',renderLibrary);
  $('nodeLibrary').addEventListener('click',event=>{const button=event.target.closest('[data-add]');if(button)addNode(button.dataset.add);});
  $('presetList').addEventListener('click',event=>{const button=event.target.closest('[data-preset]');if(button)loadPreset(button.dataset.preset);});
  $('playButton').addEventListener('click',toggleAudio);
  $('stopButton').addEventListener('click',stopAudio);
  $('performanceLive').addEventListener('click',()=>setPerformanceMode('live'));
  $('performanceCurve').addEventListener('click',()=>setPerformanceMode('curve'));
  $('performanceValue').addEventListener('input',event=>setPerformanceValue(Number(event.target.value)/100));
  ['change','pointerup','pointercancel','blur'].forEach(eventName=>$('performanceValue').addEventListener(eventName,()=>{performanceGesture=false;}));
  $('editPerformanceCurve').addEventListener('click',editPerformanceCurve);
  $('startAndDrive').addEventListener('click',startAndDrive);
  $('loopButton').addEventListener('click',()=>{console.info('[Sona] Boucle');remember();graph.loop=!graph.loop;updateTransport();commit();});
  $('durationInput').addEventListener('change',event=>{
    console.info('[Sona] Durée');
    const value=Number(event.target.value);
    if(!Number.isFinite(value)||value<.5||value>30) {event.target.value=graph.duration;return showToast(tx('Choisissez une durée entre 0,5 et 30 secondes.'),true);}
    remember();graph.duration=value;renderCurve();commit();
  });
  $('masterVolume').addEventListener('input',event=>{master=Number(event.target.value)/100;audio.setMaster(master);comparison.setMaster(master);$('volumeValue').textContent=Math.round(master*100)+' %';commit({audioChanged:false});});
  $('fitButton').addEventListener('click',fitGraph);
  $('zoomIn').addEventListener('click',()=>zoom(1.2));
  $('zoomOut').addEventListener('click',()=>zoom(1/1.2));
  $('undoButton').addEventListener('click',undo);
  $('redoButton').addEventListener('click',redo);
  $('exportButton').addEventListener('click',exportWav);
  $('projectButton').addEventListener('click',()=>{console.info('[Sona] Menu projet');$('projectMenu').hidden=!$('projectMenu').hidden;$('projectButton').setAttribute('aria-expanded',String(!$('projectMenu').hidden));});
  $('saveProject').addEventListener('click',saveProject);
  $('importProject').addEventListener('click',()=>{$('projectMenu').hidden=true;$('fileInput').click();});
  $('fileInput').addEventListener('change',event=>importProject(event.target.files[0]));
  $('newProject').addEventListener('click',newProject);
  $('helpButton').addEventListener('click',()=>{console.info('[Sona] Guide');$('helpDialog').showModal();});
  $('closeHelp').addEventListener('click',()=>{$('helpDialog').close();});
  $('helpDialog').addEventListener('click',event=>{if(event.target===$('helpDialog'))$('helpDialog').close();});
  $('scopeWave').addEventListener('click',()=>{scopeMode='wave';$('scopeWave').classList.add('active');$('scopeSpectrum').classList.remove('active');});
  $('scopeSpectrum').addEventListener('click',()=>{scopeMode='spectrum';$('scopeSpectrum').classList.add('active');$('scopeWave').classList.remove('active');});
  $('curveSelect').addEventListener('change',event=>{selectedCurve=event.target.value;selectNode(selectedCurve);});
  document.querySelectorAll('.curve-preset').forEach(button=>button.addEventListener('click',()=>setCurvePreset(button.dataset.curve)));
  $('inspector').addEventListener('input',event=>{
    if(event.target.matches('[data-change-param],[data-number-param]')) updateParameter(event.target);
    if(['edgeAmount','edgeAmountNumber'].includes(event.target.id)) updateEdgeAmount(event.target);
  });
  $('inspector').addEventListener('change',event=>{
    if(event.target.matches('select[data-change-param]')) updateParameter(event.target);
    if(event.target.id==='nodeLabel') {
      console.info('[Sona] Renommer');
      const node=graph.nodes.find(n=>n.id===selectedNode);
      if(node){remember();node.label=event.target.value.trim().slice(0,60)||NODE_TYPES[node.type].name;node.labelCustom=!!event.target.value.trim();renderNodes();renderInspector();renderCurveSelect();commit({audioChanged:false});}
    }
    parameterGesture=false;
  });
  $('inspector').addEventListener('click',event=>{
    if(event.target.closest('#toggleNodeEnabled'))toggleNodeEnabled();
    if(event.target.closest('#duplicateNode'))duplicateNode();
    if(event.target.closest('#deleteNode'))deleteNode();
    if(event.target.closest('#deleteEdge'))deleteEdge();
    const pill=event.target.closest('[data-select-edge]');if(pill)selectEdge(pill.dataset.selectEdge);
  });
  $('graphViewport').addEventListener('wheel',event=>{
    event.preventDefault();
    const r=$('graphViewport').getBoundingClientRect();
    zoom(Math.exp(-event.deltaY*.0012),event.clientX-r.left,event.clientY-r.top);
  },{passive:false});
  $('graphViewport').addEventListener('dblclick', event => {
    const header = event.target.closest('.node-header');
    if (header && !event.target.closest('button')) focusNode(header.closest('[data-node]').dataset.node);
  });
  $('graphViewport').addEventListener('pointerdown',event=>{
    if(event.button!==0&&event.button!==1)return;
    const r=$('graphViewport').getBoundingClientRect();
    cursor={x:event.clientX-r.left,y:event.clientY-r.top};
    const output=event.target.closest('[data-output]');
    if(output){
      console.info('[Sona] Début de liaison');
      const node=graph.nodes.find(n=>n.id===output.dataset.output);
      pendingCable={from:node.id,kind:NODE_TYPES[node.type].category==='control'?'control':'audio'};
      document.body.classList.add('connecting');event.preventDefault();renderEdges();return;
    }
    const input=event.target.closest('[data-input]');
    if(input){
      if(pendingCable)connect(input.dataset.input,input.dataset.target);
      else{const existing=graph.connections.find(c=>c.to===input.dataset.input&&c.target===input.dataset.target);if(existing)selectEdge(existing.id);else showToast(tx('Cliquez sur une sortie, puis sur cette entrée.'));}
      event.preventDefault();return;
    }
    const expand=event.target.closest('[data-expand]');
    if(expand){toggleNodePorts(expand.dataset.expand);return;}
    const remove=event.target.closest('[data-delete]');
    if(remove){deleteNode(remove.dataset.delete);return;}
    const edge=event.target.closest('[data-edge]');
    if(edge){selectEdge(edge.dataset.edge);return;}
    const nodeElement=event.target.closest('[data-node]');
    if(nodeElement){
      selectNode(nodeElement.dataset.node);
      if(event.target.closest('.node-header')){
        const node=graph.nodes.find(n=>n.id===selectedNode);
        gesture={type:'node',id:node.id,startX:event.clientX,startY:event.clientY,x:node.x,y:node.y,moved:false};
        event.preventDefault();
      }
      return;
    }
    if(event.target.closest('button'))return;
    if(pendingCable){pendingCable=null;document.body.classList.remove('connecting');renderEdges();return;}
    gesture={type:'pan',startX:event.clientX,startY:event.clientY,x:view.x,y:view.y,moved:false};
    $('graphViewport').classList.add('panning');event.preventDefault();
  });
  document.addEventListener('pointermove',event=>{
    if(pendingCable){const r=$('graphViewport').getBoundingClientRect();cursor={x:event.clientX-r.left,y:event.clientY-r.top};renderEdges();}
    if(gesture){
      const dx=event.clientX-gesture.startX,dy=event.clientY-gesture.startY;
      if(Math.abs(dx)+Math.abs(dy)>3&&!gesture.moved){if(gesture.type==='node')remember();gesture.moved=true;}
      if(gesture.type==='pan'){view.x=gesture.x+dx;view.y=gesture.y+dy;updateView();}
      else if(gesture.moved){const node=graph.nodes.find(n=>n.id===gesture.id);if(node){node.x=gesture.x+dx/view.scale;node.y=gesture.y+dy/view.scale;const el=nodeElements.get(node.id);el.style.left=node.x+'px';el.style.top=node.y+'px';renderEdges();}}
    }
    if(curvePoint){
      const node=graph.nodes.find(n=>n.id===curvePoint.nodeId);
      if(!node)return;
      const [x,y]=curveCoordinates(event),i=curvePoint.index;
      node.curve[i]=[i===0?0:i===node.curve.length-1?1:clamp(x,node.curve[i-1][0]+.002,node.curve[i+1][0]-.002),y];
      renderCurve();updateCurveThumbnail(node);commit();
    }
  });
  document.addEventListener('pointerup',event=>{
    if(pendingCable){const input=event.target.closest('[data-input]');if(input)connect(input.dataset.input,input.dataset.target);}
    if(gesture?.type==='node'&&gesture.moved)commit({audioChanged:false});
    gesture=null;curvePoint=null;parameterGesture=false;performanceGesture=false;
    $('graphViewport').classList.remove('panning');
  });
  $('curveEditor').addEventListener('pointerdown',event=>{
    if(event.button!==0)return;
    const point=event.target.closest('[data-curve-index]');
    if(point&&selectedCurve){remember();curvePoint={nodeId:selectedCurve,index:Number(point.dataset.curveIndex)};event.preventDefault();}
  });
  $('curveEditor').addEventListener('dblclick',event=>{
    console.info('[Sona] Ajouter un point');
    const node=graph.nodes.find(n=>n.id===selectedCurve);
    if(!node||node.curve.length>=64||event.target.closest('[data-curve-index]'))return;
    const point=curveCoordinates(event);
    if(point[0]<.002||point[0]>.998)return;
    remember();node.curve.push(point);node.curve.sort((a,b)=>a[0]-b[0]);renderCurve();updateCurveThumbnail(node);commit();
  });
  $('curveEditor').addEventListener('contextmenu',event=>{
    const el=event.target.closest('[data-curve-index]');
    if(!el)return;
    event.preventDefault();
    const node=graph.nodes.find(n=>n.id===selectedCurve),i=Number(el.dataset.curveIndex);
    if(!node||i===0||i===node.curve.length-1)return showToast(tx('Les deux extrémités restent en place.'));
    console.info('[Sona] Retirer un point');
    remember();node.curve.splice(i,1);renderCurve();updateCurveThumbnail(node);commit();
  });
  $('curveEditor').addEventListener('keydown',event=>{
    const el=event.target.closest('[data-curve-index]');
    if(!el||!['ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(event.key))return;
    event.preventDefault();event.stopPropagation();
    const node=graph.nodes.find(n=>n.id===selectedCurve),i=Number(el.dataset.curveIndex);
    remember();
    if(event.key==='ArrowUp'||event.key==='ArrowDown')node.curve[i][1]=clamp(node.curve[i][1]+(event.key==='ArrowUp'?.02:-.02),0,1);
    else if(i>0&&i<node.curve.length-1)node.curve[i][0]=clamp(node.curve[i][0]+(event.key==='ArrowRight'?.01:-.01),node.curve[i-1][0]+.002,node.curve[i+1][0]-.002);
    renderCurve();$('curveEditor').querySelector(ui`[data-curve-index="${i}"]`)?.focus();updateCurveThumbnail(node);commit();
  });
  $('graphViewport').addEventListener('keydown',event=>{
    const edge=event.target.closest('[data-edge]');
    if(edge&&(event.key==='Enter'||event.key===' ')){event.preventDefault();event.stopPropagation();selectEdge(edge.dataset.edge);}
    const out=event.target.closest('[data-output]');
    if(out&&(event.key==='Enter'||event.key===' ')){event.preventDefault();event.stopPropagation();const n=graph.nodes.find(n=>n.id===out.dataset.output);pendingCable={from:n.id,kind:NODE_TYPES[n.type].category==='control'?'control':'audio'};document.body.classList.add('connecting');showToast(tx('Choisissez une entrée avec Tab, puis Entrée.'));}
    const input=event.target.closest('[data-input]');
    if(input&&pendingCable&&(event.key==='Enter'||event.key===' ')){event.preventDefault();event.stopPropagation();connect(input.dataset.input,input.dataset.target);}
  });
  document.addEventListener('keydown',event=>{
    const editing=event.target.matches('input,textarea,select,[contenteditable=true]');
    if(event.key==='Escape'){pendingCable=null;document.body.classList.remove('connecting');$('projectMenu').hidden=true;renderEdges();}
    if(editing||$('helpDialog').open||$('comparisonDialog').open)return;
    if(event.code==='Space'&&!event.target.matches('button')){event.preventDefault();toggleAudio();}
    if(event.key==='Delete'||event.key==='Backspace'){event.preventDefault();if(selectedEdge)deleteEdge();else if(selectedNode)deleteNode();}
    if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='z'){event.preventDefault();event.shiftKey?redo():undo();}
    if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='s'){event.preventDefault();saveProject();}
    if(event.key.toLowerCase()==='f'&&!event.ctrlKey&&!event.metaKey)fitGraph();
  });
  document.addEventListener('click',event=>{if(!event.target.closest('#projectMenu')&&!event.target.closest('#projectButton'))$('projectMenu').hidden=true;});
  document.addEventListener('visibilitychange',()=>{if(document.hidden)saveLocal();});
  new ResizeObserver(()=>{renderEdges();renderCurve();}).observe($('graphViewport'));
  new ResizeObserver(()=>renderCurve()).observe($('curveEditor'));
}

const scopeData = new Float32Array(2048);
const spectrumData = new Uint8Array(1024);
function drawScope(now) {
  requestAnimationFrame(drawScope);
  if(now-lastScopeAt<40)return;
  lastScopeAt=now;
  const canvas=$('scope'),dpr=Math.min(window.devicePixelRatio||1,2),w=canvas.clientWidth,h=canvas.clientHeight;
  if(!w||!h)return;
  if(canvas.width!==Math.round(w*dpr)||canvas.height!==Math.round(h*dpr)){canvas.width=Math.round(w*dpr);canvas.height=Math.round(h*dpr);}
  const context=canvas.getContext('2d');context.setTransform(dpr,0,0,dpr,0,0);context.clearRect(0,0,w,h);
  context.strokeStyle='#2b3033';context.lineWidth=1;context.beginPath();context.moveTo(0,h/2);context.lineTo(w,h/2);context.stroke();
  const analyser=audio.getAnalyser();
  if(audio.isPlaying&&analyser){
    analyser.getFloatTimeDomainData(scopeData);
    let peak=0;for(let i=0;i<scopeData.length;i++)peak=Math.max(peak,Math.abs(scopeData[i]));
    $('peakValue').textContent=(peak>0?Math.max(-90,20*Math.log10(peak)).toLocaleString(locale(),{minimumFractionDigits:1,maximumFractionDigits:1}):'−∞')+' dBFS';
    if(scopeMode==='wave'){
      context.strokeStyle='#d8f78a';context.lineWidth=1.3;context.beginPath();
      for(let i=0;i<w;i++){const sample=scopeData[Math.floor(i/w*scopeData.length)],y=h/2-sample*h*.43;if(!i)context.moveTo(i,y);else context.lineTo(i,y);}context.stroke();
    }else{
      analyser.getByteFrequencyData(spectrumData);
      const bars=42,gap=3,bw=(w-gap*(bars-1))/bars;context.fillStyle='#a6d4d1';
      for(let i=0;i<bars;i++){const lo=Math.floor(Math.pow(i/bars,2)*spectrumData.length),hi=Math.max(lo+1,Math.floor(Math.pow((i+1)/bars,2)*spectrumData.length));let v=0;for(let j=lo;j<hi;j++)v=Math.max(v,spectrumData[j]);const bar=v/255*(h-4);context.fillRect(i*(bw+gap),h-bar,bw,bar);}
    }
  }else{
    $('peakValue').textContent='−∞ dBFS';context.strokeStyle='#65724c';context.beginPath();context.moveTo(0,h/2);context.lineTo(w,h/2);context.stroke();
  }
}

function refreshLanguage() {
  console.info('[Sona] Refresh interface language');
  applyStaticTranslations();
  $('languageSelect').value=getLanguage();
  $('languageSelect').setAttribute('aria-label',t('language'));
  $('saveState').textContent=tr(saveStatus);
  $('toast').textContent=typeof toastSource==='function'?toastSource():translateError(toastSource);
  // Rendering labels does not commit, recreate audio, or change undo history.
  renderLibrary();renderAll();comparison.localeChanged();
  $('exportButton').textContent=exporting?tr('Rendu en cours…'):tr('Exporter WAV');
}

function init() {
  console.info('[Sona] Atelier sonore prêt');
  registerStaticTranslations(document);
  applyStaticTranslations();
  $('languageSelect').value=getLanguage();
  $('languageSelect').setAttribute('aria-label',t('language'));
  loadLocal();renderLibrary();renderAll();bindEvents();audio.setMaster(master);fitGraph();
  $('saveState').textContent=tr(saveStatus);
  onLanguageChange(refreshLanguage);
  requestAnimationFrame(()=>{fitGraph();requestAnimationFrame(drawScope);});
}
init();
