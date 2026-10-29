/** A single drop assembled from reusable sources, curves and filters. */
export function buildWaterPreset(n, c) {
  const duration = 1.8;
  const seconds = points => points.map(([time, value]) => [time / duration, value]);
  return {
    id: 'water', name: 'Goutte d’eau', subtitle: 'Impulsion & résonance', icon: 'droplet', color: '#82c4f8',
    description: 'Une impulsion de bruit excite un filtre résonant. Un sinus glisse et s’amortit, tandis qu’une petite portion de bruit aigu forme l’éclaboussure. Chaque source, courbe et traitement se règle séparément.',
    graph: {
      duration, loop: true,
      nodes: [
        n('water-impact-curve', 'curve', 50, 60, { amount: 1 }, 'Impulsion · 12 ms', seconds([
          [0, 0], [.060, 0], [.0606, 1], [.0625, .4], [.067, .08], [.072, 0], [duration, 0],
        ])),
        n('water-amplitude-curve', 'curve', 50, 365, { amount: 1 }, 'Décroissance du tintement', seconds([
          [0, 0], [.063, 0], [.0645, 1], [.074, .7], [.094, .36], [.128, .12], [.178, .023], [.235, 0], [duration, 0],
        ])),
        n('water-frequency-curve', 'curve', 50, 670, { amount: 1 }, 'Glissement de fréquence', seconds([
          [0, 0], [.063, 0], [.070, .22], [.086, .62], [.118, .9], [.180, 1], [duration, 1],
        ])),
        n('water-noise', 'noise', 340, 60, { color: 'white', level: 0 }, 'Bruit blanc'),
        n('water-sine', 'oscillator', 340, 365, { wave: 'sine', frequency: 2100, level: 0 }, 'Sinus amorti'),
        n('water-impact-filter', 'filter', 630, 60, { mode: 'bandpass', cutoff: 1750, resonance: 9 }, 'Résonance de l’impact'),
        n('water-splash-filter', 'filter', 630, 365, { mode: 'highpass', cutoff: 4300, resonance: .65 }, 'Détail aigu du bruit'),
        n('water-resonator', 'resonator', 920, 60, { frequency: 1750, decay: .034, color: .32, mix: .38 }, 'Résonances amorties'),
        n('water-splash-gain', 'gain', 920, 365, { gain: .16 }, 'Niveau de l’éclaboussure'),
        n('water-mix', 'gain', 1210, 60, { gain: 1.12 }, 'Mélange des trois couches'),
        n('water-space', 'space', 1210, 365, { size: .12, damping: .72, mix: .075, width: .55, distance: .04 }, 'Petites réflexions'),
        n('water-output', 'output', 1500, 365, { gain: .9 }, 'Sortie audio'),
      ],
      connections: [
        c('water-excitation-level', 'water-impact-curve', 'water-noise', 'level', .9),
        c('water-sine-amplitude', 'water-amplitude-curve', 'water-sine', 'level', .68),
        c('water-sine-frequency', 'water-frequency-curve', 'water-sine', 'frequency', 700),
        c('water-impact-audio', 'water-noise', 'water-impact-filter'),
        c('water-ring-audio', 'water-impact-filter', 'water-resonator'),
        c('water-body-mix', 'water-resonator', 'water-mix'),
        c('water-splash-audio', 'water-noise', 'water-splash-filter'),
        c('water-splash-level', 'water-splash-filter', 'water-splash-gain'),
        c('water-splash-mix', 'water-splash-gain', 'water-mix'),
        c('water-tone-mix', 'water-sine', 'water-mix'),
        c('water-room-audio', 'water-mix', 'water-space'),
        c('water-output-audio', 'water-space', 'water-output'),
      ],
    },
  };
}
