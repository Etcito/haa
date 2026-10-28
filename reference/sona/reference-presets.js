// Recordings are used only in the comparative listening view. They never feed
// the procedural graph or its WAV export. See references/CREDITS.md.
import { REFERENCE_STUDIES } from './reference-studies.js';
export const REFERENCES = {
  ...REFERENCE_STUDIES,
  'light-rain': {
    name: 'Light rain', author: 'Liecio',
    url: './references/light-rain-reference.wav',
    source: 'https://pixabay.com/sound-effects/nature-light-rain-109591/',
    excerpt: 'Excerpt 32–40 s', length: 8,
    note: 'Compare the short bright droplets against the softer diffuse bed. Levels are matched over 20 Hz–20 kHz; the recording stays unfiltered.',
  },
  'engine-start': {
    name: 'Engine start · Acceleration 1', author: 'biholao (Freesound)',
    url: './references/engine-start-reference.wav',
    source: 'https://pixabay.com/fr/sound-effects/ville-acceleration-1-6916/',
    excerpt: 'Excerpt 1:50.80–1:55.00', length: 4.2,
    note: 'Compare the starter pulses, the first firing, the brief rev flare and the return to idle.',
  },
  v12: {
    name: 'Ferrari Testarossa', author: 'AstonMartinVantageV12',
    url: './references/testarossa-scene-reference.wav',
    source: 'https://pixabay.com/fr/sound-effects/ville-ferrari-testarossa-sounds-384731/',
    excerpt: 'Excerpt 9.55–20.55 s', length: 11,
    note: 'Compare the change in engine character under load, the mechanical detail and the lift-off. One pedal drives the graph; its events are not time-aligned with this recording.',
  },
  elephant: {
    name: 'Gajah · barrissement', author: 'Sondang Sirait',
    url: './references/gajah-reference.wav',
    source: 'https://pixabay.com/fr/sound-effects/nature-gajah-220044/',
    excerpt: 'Cri de référence complet', length: 1.68,
    note: 'Une attaque brève, une hauteur tenue et une retombée finale. Comparez aussi le souffle et la rugosité.',
  },
};
