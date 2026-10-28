import { buildPresets } from './presets.js';

export const SOUND_REVISION = 8;
const sourceColor = '#f3b77e';
const controlColor = '#c2a5ff';
const effectColor = '#82c4f8';
const outputColor = '#d8fa84';

const number = (key, label, min, max, step, value, unit = '', extra = {}) => ({
  key, label, min, max, step, default: value, unit, modulatable: true, ...extra,
});
const choice = (key, label, value, options) => ({
  key, label, default: value, modulatable: false,
  options: options.map(([optionValue, optionLabel]) => ({ value: optionValue, label: optionLabel })),
});

export const NODE_TYPES = {
  grain: {
    name: 'Grain cloud', category: 'source', icon: 'noise', color: sourceColor,
    description: 'Short synthesized sine, noise or filtered-noise grains with independent pitch, decay and timing. Filtered noise gives each grain its own moving resonance. Build impacts, friction or dense textures. Rate zero creates one grain; overlap is level-compensated.',
    inputs: false,
    params: [
      choice('wave', 'Grain shape', 'sine', [['sine', 'Sine'], ['noise', 'Noise'], ['filtered-noise', 'Filtered noise']]),
      number('frequency', 'Pitch', 40, 12000, 1, 1800, 'Hz', {scale:'log'}),
      number('resonance', 'Grain resonance', .25, 4, .05, .7, 'Q'),
      number('rate', 'Density', 0, 2000, .1, 12, 'Hz'),
      number('attack', 'Attack', .0001, .1, .0001, .001, 's'),
      number('decay', 'Decay', .001, 2, .0001, .035, 's'),
      number('spread', 'Pitch spread', 0, 36, .1, 5, 'st'),
      number('jitter', 'Timing variation', 0, 1, .01, .8),
      number('variation', 'Grain variation', 0, 1, .01, .5),
      number('glide', 'Pitch glide', -.9, 3, .01, .05, '×'),
      number('delay', 'Start delay', 0, 20, .001, .02, 's'),
      number('width', 'Stereo spread', 0, 1, .01, .25),
      number('level', 'Level', 0, 1, .01, .3),
    ],
  },
  pulse: {
    name: 'Train d’impulsions', category: 'source', icon: 'pulse', color: sourceColor,
    description: 'Des impulsions périodiques dont vous réglez la cadence, la largeur et la forme. Une même source peut exciter un tube, articuler une voix, faire vibrer un objet ou produire un rythme.',
    inputs: false,
    params: [
      number('frequency', 'Fréquence', 0, 4000, .1, 100, 'Hz'),
      number('width', 'Largeur', .002, .95, .001, .08, 'cycle'),
      number('phase', 'Phase', 0, 1, .01, 0, 'cycle'),
      number('jitter', 'Écart de temps', 0, .25, .001, .02, 'cycle'),
      number('shimmer', 'Écart d’amplitude', 0, 1, .01, .05),
      number('level', 'Niveau', 0, 1, .01, .5),
      choice('shape', 'Forme', 'rounded', [['rounded', 'Arrondie'], ['exponential', 'Décroissance rapide']]),
    ],
  },
  engine: {
    legacy: true,
    name: 'Moteur V12', category: 'source', icon: 'engine', color: sourceColor,
    description: 'Douze combustions individuelles, deux collecteurs et leurs réflexions de pression. La charge, l’inertie et la longueur d’échappement donnent son caractère au moteur.',
    inputs: false,
    params: [
      number('rpm', 'Régime', 650, 9500, 10, 1100, 'tr/min'),
      number('throttle', 'Charge', 0, 1, .01, .6),
      number('roughness', 'Irrégularité', 0, 1, .01, .25),
      number('inertia', 'Inertie', .02, .8, .01, .12, 's'),
      number('mechanics', 'Mécanique', 0, 1, .01, .22),
      number('exhaust', 'Collecteur', .25, 2, .01, .75, 'm'),
      number('crackle', 'Crépitement', 0, 1, .01, .15),
    ],
  },
  reed: {
    legacy: true,
    name: 'Voix organique', category: 'source', icon: 'wind', color: sourceColor,
    description: 'Excitation sous pression, souffle turbulent, instabilités et résonances mobiles. Faites évoluer la pression et la hauteur pour articuler un cri.',
    inputs: false,
    params: [
      number('frequency', 'Hauteur', 60, 1500, 1, 380, 'Hz', { scale: 'log' }),
      number('pressure', 'Pression', 0, 1, .01, .7),
      number('breath', 'Souffle', 0, 1, .01, .35),
      number('growl', 'Rugosité', 0, 1, .01, .4),
      number('formant', 'Taille du conduit', .5, 2, .01, 1, '×'),
      number('instability', 'Instabilité', 0, 1, .01, .25),
    ],
  },
  bubble: {
    legacy: true,
    name: 'Goutte / bulle', category: 'source', icon: 'droplet', color: sourceColor,
    description: 'Impact humide, bulle principale et microbulles amorties. Chaque événement varie légèrement en taille, en énergie et en timing.',
    inputs: false,
    params: [
      number('frequency', 'Résonance', 100, 9000, 1, 2800, 'Hz', { scale: 'log' }),
      number('decay', 'Amortissement', .02, 2, .005, .11, 's'),
      number('tone', 'Clarté', 0, 1, .01, .55),
      number('repeat', 'Intervalle', .2, 5, .01, 1.2, 's', { modulatable: false }),
      number('scatter', 'Dispersion', 0, 1, .01, .5),
      number('splash', 'Éclaboussure', 0, 1, .01, .35),
    ],
  },
  oscillator: {
    name: 'Oscillateur', category: 'source', icon: 'waves', color: sourceColor,
    description: 'La brique de base : une forme d’onde périodique. Une modulation de fréquence crée vibratos et glissandos.',
    inputs: false,
    params: [
      number('frequency', 'Fréquence', 20, 12000, 1, 220, 'Hz', { scale: 'log' }),
      choice('wave', 'Forme', 'sine', [['sine', 'Sinus'], ['triangle', 'Triangle'], ['saw', 'Dent de scie'], ['square', 'Carré']]),
      number('level', 'Niveau', 0, 1, .01, .35),
    ],
  },
  noise: {
    name: 'Bruit', category: 'source', icon: 'noise', color: sourceColor,
    description: 'Five noise colours: white, pink, brown, blue and violet. Stereo spread moves from a centred source to a diffuse field. Shape airflow, friction and fine surface detail with filters and envelopes.',
    inputs: false,
    params: [
      choice('color', 'Couleur', 'pink', [['white', 'Blanc'], ['pink', 'Rose'], ['brown', 'Brun'], ['blue', 'Blue'], ['violet', 'Violet']]),
      number('level', 'Niveau', 0, 1, .01, .15),
      number('width', 'Stereo spread', 0, 1, .01, 0),
    ],
  },
  curve: {
    name: 'Courbe temporelle', category: 'control', icon: 'curve', color: controlColor,
    description: 'Dessinez l’évolution d’un paramètre pendant toute la durée du son. Reliez cette sortie à un port violet.',
    inputs: false,
    params: [number('amount', 'Amplitude', 0, 1, .01, 1)],
  },
  response: {
    name: 'Control response', category: 'control', icon: 'envelope', color: controlColor,
    description: 'Derive several envelopes from one control signal. Follow its level or extract rising and falling motion, then adjust delay, attack, release and curve power. The output stays between zero and one.',
    inputs: false,
    params: [
      number('input', 'Control input', 0, 1, .001, 0),
      choice('mode', 'Response mode', 'follow', [['follow', 'Follow level'], ['rise', 'Rising motion'], ['fall', 'Falling motion']]),
      number('attack', 'Attack', 0, 2, .0001, .003, 's'),
      number('release', 'Release', 0, 3, .0001, .035, 's'),
      number('delay', 'Response delay', 0, 2, .0001, 0, 's'),
      number('shape', 'Curve power', .25, 4, .01, 1, '×'),
      number('gain', 'Response gain', 0, 4, .01, 1, '×'),
      number('sensitivity', 'Edge window', .001, 1, .001, .04, 's'),
    ],
  },
  motion: {
    name: 'Inertial motion', category: 'control', icon: 'activity', color: controlColor,
    description: 'Build movement from one drive signal. Acceleration adds momentum, resistance and release drag remove it. Optional threshold resets create a limited sequence of stages. This control continues through timeline loops and returns to its initial stage at rest.',
    inputs: false,
    params: [
      number('input', 'Drive input', 0, 1, .001, 0),
      number('acceleration', 'Acceleration', 0, 4, .01, .36, '/s'),
      number('resistance', 'Resistance', 0, 8, .01, .07, '/s'),
      number('return', 'Release drag', 0, 12, .01, 1.7, '/s'),
      number('ceiling', 'Upper threshold', .05, 1, .01, .98),
      number('reset', 'Reset ratio', 0, .95, .01, .64),
      number('stages', 'Reset limit', 0, 12, 1, 0),
      number('progression', 'Stage scaling', .05, 1, .01, .76),
      number('hold', 'Reset pause', 0, 2, .001, .14, 's'),
      number('inertia', 'Inertia', 0, 2, .001, .065, 's'),
      number('rest', 'Rest threshold', 0, .25, .001, .025),
    ],
  },
  lfo: {
    name: 'LFO', category: 'control', icon: 'activity', color: controlColor,
    description: 'A control oscillator from slow movement to audio-rate modulation. Use it for vibrato, tremolo, roughness or frequency modulation.',
    inputs: false,
    params: [
      number('rate', 'Vitesse', .05, 2000, .05, 2, 'Hz', {scale:'log'}),
      number('depth', 'Amplitude', 0, 1, .01, .5),
      choice('wave', 'Forme', 'sine', [['sine', 'Sinus'], ['triangle', 'Triangle'], ['square', 'Carré']]),
    ],
  },
  envelope: {
    name: 'Enveloppe ADSR', category: 'control', icon: 'envelope', color: controlColor,
    description: 'Une attaque, une décroissance, un maintien et un relâchement. Idéal pour articuler le volume d’un son.',
    inputs: false,
    params: [
      number('attack', 'Attaque', .001, 3, .001, .08, 's'),
      number('decay', 'Décroissance', .001, 3, .001, .25, 's'),
      number('sustain', 'Soutien', 0, 1, .01, .7),
      number('hold', 'Maintien', .01, 8, .01, 1.2, 's'),
      number('release', 'Relâchement', .01, 4, .01, .5, 's'),
    ],
  },
  random: {
    name: 'Variation organique', category: 'control', icon: 'random', color: controlColor,
    description: 'Des variations aléatoires continues, sans répétition périodique. Ajoutez de petites irrégularités à la pression, la hauteur ou la matière.',
    inputs: false,
    params: [
      number('rate', 'Vitesse', .05, 2000, .05, 2.5, 'Hz', {scale:'log'}),
      number('depth', 'Amplitude', 0, 1, .01, .5),
      number('smooth', 'Souplesse', 0, 1, .01, .8),
    ],
  },
  math: {
    name: 'Calcul', category: 'control', icon: 'math', color: controlColor,
    description: 'Combine modulation with add, multiply, minimum or maximum. Smoothstep reshapes A into a smooth 0–1 transition between two edges. Connected signals add to the input base; set the base to zero to use a signal directly.',
    inputs: false,
    params: [
      number('a', 'Valeur A', -20000, 20000, .001, 0),
      number('b', 'Valeur B', -20000, 20000, .000001, 1),
      choice('operation', 'Opération', 'multiply', [['multiply', 'A × B'], ['add', 'A + B'], ['subtract', 'A − B'], ['divide', 'A ÷ B'], ['min', 'Minimum'], ['max', 'Maximum'], ['smoothstep', 'Smoothstep']]),
      number('edge0', 'Start edge', -20000, 20000, .001, 0),
      number('edge1', 'End edge', -20000, 20000, .001, 1),
    ],
  },
  comb: {
    name: 'Retard court / peigne', category: 'effect', icon: 'comb', color: effectColor,
    description: 'Un retard très court réinjecte une partie du signal et crée des résonances espacées. Réutilisez-le pour un tube, une corde, du métal ou un collecteur.',
    inputs: true,
    params: [
      number('time', 'Retard', .0002, .05, .0001, .004, 's'),
      number('feedback', 'Réinjection', -.95, .95, .01, .55),
      number('damping', 'Absorption', 0, 1, .01, .4),
      number('mix', 'Mélange', 0, 1, .01, .5),
    ],
  },
  resonator: {
    name: 'Résonateur de matière', category: 'effect', icon: 'resonator', color: effectColor,
    description: 'Plusieurs résonances amorties et inharmoniques ajoutent un corps à une excitation : cavité, collecteur, surface ou liquide.',
    inputs: true,
    params: [
      number('frequency', 'Fondamentale', 80, 6000, 1, 450, 'Hz', { scale: 'log' }),
      number('decay', 'Résonance', .005, 1, .005, .055, 's'),
      number('color', 'Matière', 0, 1, .01, .45),
      number('mix', 'Mélange', 0, 1, .01, .25),
    ],
  },
  formant: {
    name: 'Formants', category: 'effect', icon: 'formant', color: effectColor,
    description: 'Un ensemble de résonances mobiles façonne un conduit vocal. Modulez légèrement sa taille pour faire respirer le timbre.',
    inputs: true,
    params: [
      number('frequency', 'Résonance basse', 120, 1600, 1, 560, 'Hz', { scale: 'log' }),
      number('spread', 'Ouverture', .5, 2, .01, 1, '×'),
      number('resonance', 'Précision', .3, 6, .05, 1.2, 'Q'),
      number('mix', 'Mélange', 0, 1, .01, .35),
    ],
  },
  equalizer: {
    name: 'Égaliseur 3 bandes', category: 'effect', icon: 'equalizer', color: effectColor,
    description: 'Équilibrez le poids des graves, le corps des médiums et le détail des aigus. La fréquence des médiums est réglable.',
    inputs: true,
    params: [
      number('low', 'Graves', -18, 18, .5, 0, 'dB'),
      number('mid', 'Médiums', -18, 18, .5, 0, 'dB'),
      number('high', 'Aigus', -18, 18, .5, 0, 'dB'),
      number('frequency', 'Centre médiums', 150, 8000, 10, 1200, 'Hz', { scale: 'log' }),
    ],
  },
  compressor: {
    name: 'Compresseur', category: 'effect', icon: 'compressor', color: effectColor,
    description: 'Contrôlez les crêtes et la densité avec une transition douce. L’attaque préserve ou atténue les transitoires ; le gain compense la réduction.',
    inputs: true,
    params: [
      number('threshold', 'Seuil', -48, 0, .5, -20, 'dB'),
      number('ratio', 'Ratio', 1, 12, .1, 2.5, ':1'),
      number('attack', 'Attaque', .001, .2, .001, .015, 's'),
      number('release', 'Retour', .02, 1, .01, .15, 's'),
      number('makeup', 'Gain', 0, 18, .5, 3, 'dB'),
    ],
  },
  space: {
    name: 'Espace acoustique', category: 'effect', icon: 'space', color: effectColor,
    description: 'Premières réflexions, diffusion stéréo et absorption des aigus. Une faible quantité installe la source dans un lieu sans la noyer.',
    inputs: true,
    params: [
      number('size', 'Volume du lieu', 0, 1, .01, .3),
      number('damping', 'Absorption', 0, 1, .01, .6),
      number('mix', 'Réflexions', 0, 1, .01, .13),
      number('width', 'Largeur', 0, 1, .01, .7),
      number('distance', 'Distance', 0, 1, .01, .15),
    ],
  },
  filter: {
    name: 'Filtre', category: 'effect', icon: 'filter', color: effectColor,
    description: 'Sculptez les fréquences. Une courbe sur la coupure fait évoluer la couleur du son au fil du temps.',
    inputs: true,
    params: [
      number('cutoff', 'Coupure', 10, 18000, 1, 4000, 'Hz', { scale: 'log' }),
      number('resonance', 'Résonance', .1, 12, .1, .7, 'Q'),
      choice('mode', 'Mode', 'lowpass', [['lowpass', 'Passe-bas'], ['bandpass', 'Passe-bande'], ['highpass', 'Passe-haut']]),
    ],
  },
  drive: {
    name: 'Saturation', category: 'effect', icon: 'drive', color: effectColor,
    description: 'Épaississez les harmoniques pour ajouter du mordant. Le mélange conserve une partie du signal d’origine.',
    inputs: true,
    params: [
      number('amount', 'Intensité', 0, 1, .01, .25),
      number('mix', 'Mélange', 0, 1, .01, .45),
    ],
  },
  gain: {
    name: 'Amplificateur', category: 'effect', icon: 'volume', color: effectColor,
    description: 'Contrôlez le niveau ou modulez-le avec une enveloppe. Plusieurs entrées audio sont mélangées à ce point.',
    inputs: true,
    params: [number('gain', 'Gain', 0, 2, .01, 1, '×')],
  },
  delay: {
    name: 'Écho', category: 'effect', icon: 'delay', color: effectColor,
    description: 'Des répétitions espacées dans le temps. La réinjection règle le nombre d’échos.',
    inputs: true,
    params: [
      number('time', 'Délai', .01, 1, .01, .24, 's', { modulatable: false }),
      number('feedback', 'Réinjection', 0, .85, .01, .3),
      number('mix', 'Mélange', 0, 1, .01, .2),
    ],
  },
  reverb: {
    name: 'Réverbération', category: 'effect', icon: 'reverb', color: effectColor,
    description: 'Donnez de l’espace au son avec une queue diffuse. Une petite quantité suffit à éloigner la source.',
    inputs: true,
    params: [
      number('size', 'Espace', 0, 1, .01, .4, '', { modulatable: false }),
      number('mix', 'Mélange', 0, 1, .01, .18),
    ],
  },
  output: {
    name: 'Sortie audio', category: 'output', icon: 'speaker', color: outputColor,
    description: 'Le point d’écoute du graphe. Connectez au moins une source ici, directement ou à travers des effets.',
    inputs: true,
    params: [number('gain', 'Niveau', 0, 1, .01, .75)],
  },
};

export function makeNode(type, id, x = 80, y = 80) {
  console.info('[SonaLab] Création du nœud', type);
  const schema = NODE_TYPES[type];
  if (!schema) throw new Error(`Type de nœud inconnu : ${type}`);
  const node = {
    id, type, x, y, label: schema.name,
    params: Object.fromEntries(schema.params.map(parameter => [parameter.key, parameter.default])),
  };
  if (type === 'curve') node.curve = [[0, 0], [.2, .8], [.65, .8], [1, 0]];
  return node;
}

const presetNode = (id, type, x, y, params = {}, label, curve) => {
  const schema = NODE_TYPES[type];
  return {
    id, type, x, y, label: label || schema.name, compact: true,
    params: { ...Object.fromEntries(schema.params.map(parameter => [parameter.key, parameter.default])), ...params },
    ...(curve ? { curve } : {}),
  };
};
const connection = (id, from, to, target = 'audio', amount = 1) => ({ id, from, to, target, amount });

export const PRESETS = buildPresets(presetNode, connection);

export function clonePreset(id = 'v12') {
  console.info('[SonaLab] Chargement du preset', id);
  const preset = PRESETS.find(item => item.id === id);
  if (!preset) throw new Error(`Preset inconnu : ${id}`);
  return structuredClone(preset.graph);
}
