import { UI_TRANSLATIONS } from './ui-translations.js';
import { CONTENT_TRANSLATIONS } from './content-translations.js';

const STORAGE_KEY = 'sona-language';
const supported = new Set(['en', 'fr']);
let language = 'en';
try { const saved = globalThis.localStorage?.getItem(STORAGE_KEY); if (supported.has(saved)) language = saved; } catch { /* Storage can be disabled. */ }
const listeners = new Set();
const englishToFrench = new Map(Object.entries(UI_TRANSLATIONS).map(([fr, en]) => [en, fr]));
const frenchToEnglish = new Map(Object.entries(UI_TRANSLATIONS));
for (const [en, fr] of Object.entries(CONTENT_TRANSLATIONS)) { englishToFrench.set(en, fr); frenchToEnglish.set(fr, en); }
const literalCache = new Map();
const staticEntries = [];
const registeredText = new WeakSet();
const registeredAttributes = new WeakMap();
const dynamicIds = new Set(['projectTitle','saveState','nodeCount','presetList','presetPicker','presetDescription','nodeLibrary','world','edges','inspector','curveSelect','curveHint','curveEditor','audioState','peakValue','playButton','timeDisplay','volumeValue','statusText','toast','referenceName','referenceMetadata','comparisonNote','comparisonMessage','comparisonDuration','referenceRms','synthesisRms','comparisonTime','comparisonBands','referenceCredit']);
const escapeRegExp = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const patterns = {};
for (const [code, mapping] of [['en', frenchToEnglish], ['fr', englishToFrench]]) {
  const keys = [...mapping.keys()].filter(key => key && mapping.get(key) !== key).sort((a,b) => b.length-a.length);
  patterns[code] = new RegExp('(?<![\\p{L}\\p{N}_-])(?:' + keys.map(escapeRegExp).join('|') + ')(?![\\p{L}\\p{N}_-])', 'gu');
}
const messages = {
  nodeCount: { en: '{nodes} {nodeLabel} · {connections} {connectionLabel}', fr: '{nodes} {nodeLabel} · {connections} {connectionLabel}' },
  moduleCount: { en: '{count} reusable modules · no samples in the graph', fr: '{count} briques réutilisables · aucun sample dans le graphe' },
  choosePreset: { en: 'Choose a preset', fr: 'Choisir un preset' },
  language: { en: 'Interface language', fr: 'Langue de l’interface' },
  noCurve: { en: 'No curves', fr: 'Aucune courbe' },
  moduleAdded: { en: '{name} added · connect its ports to hear it.', fr: '{name} ajouté · reliez ses ports pour l’entendre.' },
  inputOf: { en: 'Audio input of {name}', fr: 'Entrée audio de {name}' },
  outputOf: { en: '{kind} output of {name}', fr: 'Sortie {kind} de {name}' },
  modulate: { en: 'Modulate {parameter} of {name}', fr: 'Moduler {parameter} de {name}' },
  outputPort: { en: 'Single audio input — a new cable replaces the previous input', fr: 'Entrée audio unique — un nouveau câble remplace le précédent' },
  point: { en: 'Point {number}, {position} percent of the duration', fr: 'Point {number}, {position} pour cent de la durée' },
  exportDone: { en: 'WAV exported · {seconds} seconds.', fr: 'WAV exporté · {seconds} secondes.' },
  duration: { en: 'Compared region: {seconds} s. Synthesis starts at the beginning of the graph, without time stretching.', fr: 'Passage comparé : {seconds} s. Synthèse depuis le début du graphe, sans étirement.' },
  channels: { en: '{count} channels', fr: '{count} canaux' },
};

// These formatting/translation helpers are called from render loops.
export const getLanguage = () => language;
export const locale = () => language === 'fr' ? 'fr-FR' : 'en-US';
export function tr(source, target = language) {
  if (source == null) return '';
  if (typeof source === 'object') return source[target] ?? source.en ?? source.fr ?? '';
  const text = String(source);
  if (target === 'en') return frenchToEnglish.get(text) ?? text;
  if (frenchToEnglish.has(text)) return text;
  return englishToFrench.get(text) ?? text;
}
export function t(key, values = {}) {
  if (key === 'nodeCount') values = {...values,
    nodeLabel: language === 'fr' ? (values.nodes > 1 ? 'nœuds' : 'nœud') : (values.nodes === 1 ? 'node' : 'nodes'),
    connectionLabel: language === 'fr' ? (values.connections > 1 ? 'liaisons' : 'liaison') : (values.connections === 1 ? 'connection' : 'connections')
  };
  const template = messages[key]?.[language] ?? tr(key);
  return template.replace(/\{([\w]+)\}/g, (_, name) => String(values[name] ?? ''));
}
export function tx(literal, target = language) {
  const source = String(literal);
  const cacheKey = target + '\0' + source;
  if (literalCache.has(cacheKey)) return literalCache.get(cacheKey);
  const exact = tr(source, target);
  const mapping = target === 'en' ? frenchToEnglish : englishToFrench;
  const translated = exact !== source ? exact : source.replace(patterns[target], match => mapping.get(match) ?? match);
  literalCache.set(cacheKey, translated);
  return translated;
}
export function html(strings, ...values) {
  // Only source-code literals are translated. Interpolated labels, filenames,
  // URLs and user-entered content pass through unchanged.
  return strings.reduce((result, literal, index) => result + tx(literal) + (index < values.length ? values[index] : ''), '');
}
export function translateError(error) {
  const text = typeof error === 'string' ? error : error?.message || String(error);
  const exact = tr(text);
  if (exact !== text || frenchToEnglish.has(text) || englishToFrench.has(text)) return exact;
  const prefixes = ['Paramètre invalide : ', 'Paramètre hors limites : ', 'Type de nœud inconnu : ', 'Valeur invalide pour ', 'Cette modulation est incompatible : ', 'Preset inconnu : '];
  for (const prefix of prefixes) for (const variant of [prefix, tr(prefix, 'en')]) if (text.startsWith(variant)) return tr(prefix) + text.slice(variant.length);
  const download = text.match(/^Le téléchargement du son a échoué \(HTTP (\d+)\)\. Vous pouvez importer le fichier audio directement\.$/);
  if (download) return language === 'en' ? `Audio download failed (HTTP ${download[1]}). You can import the audio file directly.` : text;
  const finite = text.match(/^(.+) doit être un nombre fini\.$/);
  if (finite) return language === 'en' ? `${tr(finite[1])} must be a finite number.` : text;
  return text;
}

export function setLanguage(next) {
  console.info('[Sona] Set interface language', next);
  if (!supported.has(next) || next === language) return;
  language = next;
  try { globalThis.localStorage?.setItem(STORAGE_KEY, next); } catch { /* Session language still works. */ }
  if (globalThis.document) document.documentElement.lang = next;
  for (const listener of listeners) listener(next);
}
export function onLanguageChange(listener) {
  console.info('[Sona] Register language listener');
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function registerStaticTranslations(root) {
  console.info('[Sona] Register owned static UI translations');
  if (!root || !globalThis.document) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
  let node = root;
  do {
    const element = node.nodeType === 1 ? node : node.parentElement;
    if (!element || element.closest('script,style,[data-user-content]')) continue;
    let ancestor = element, dynamic = false;
    while (ancestor && ancestor !== root.parentElement) { if (dynamicIds.has(ancestor.id)) { dynamic = true; break; } ancestor = ancestor.parentElement; }
    if (dynamic && (node.nodeType !== 1 || !dynamicIds.has(node.id))) continue;
    if (node.nodeType === 3 && node.nodeValue.trim() && !registeredText.has(node)) {
      const source = node.nodeValue;
      if (tx(source, 'en') !== tx(source, 'fr')) { staticEntries.push({ node, source }); registeredText.add(node); }
    } else if (node.nodeType === 1) {
      const used = registeredAttributes.get(node) || new Set();
      for (const attribute of ['title','aria-label','placeholder','content']) {
        if (!node.hasAttribute(attribute) || used.has(attribute)) continue;
        const source = node.getAttribute(attribute);
        if (tx(source, 'en') !== tx(source, 'fr')) { staticEntries.push({ node, source, attribute }); used.add(attribute); }
      }
      registeredAttributes.set(node, used);
    }
  } while ((node = walker.nextNode()));
}
export function applyStaticTranslations() {
  console.info('[Sona] Apply interface language', language);
  if (!globalThis.document) return;
  document.documentElement.lang = language;
  for (const entry of staticEntries) {
    if (!entry.node.isConnected) continue;
    if (entry.attribute) entry.node.setAttribute(entry.attribute, tx(entry.source));
    else entry.node.nodeValue = tx(entry.source);
  }
}
