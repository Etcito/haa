/**
 * A procedural call assembled from ordinary oscillators, curves, noise and filters.
 * The broad pitch and partial envelopes were estimated from the reference. These
 * are editable control points; no recording or sample player is embedded here.
 * Independent partial articulation and stochastic phase/amplitude changes avoid
 * the fixed pulse waveform of the earlier horn-like version.
 */
export function buildVoicePreset(n, c) {
  console.info('[Sona] Building an articulated additive voice from generic modules');
  const duration = 2.2;
  const normalized = points => points.map(([time, value]) => [time / duration, value]);
  const pitchHz = [[0,490],[.07,500],[.17,530],[.3,534],[.36,524],[.44,533],[.53,533],[.64,541],[.72,546],[.82,540],[.96,529],[1.08,515],[1.17,477],[1.28,463],[1.38,447],[1.48,410],[1.58,388],[1.67,380],[2.2,380]];
  const secondPitchHz = [[0,520],[.1,528],[.18,554],[.36,561],[.56,569],[.7,560],[.84,550],[1,539],[1.12,522],[1.25,509],[1.4,505],[1.53,451],[1.64,430],[2.2,430]];
  // Six independently editable partial amplitudes, at a coarse control rate.
  const partials = [[[0,0],[0.04,0],[0.07,0.044],[0.12,0.152],[0.18,0.142],[0.24,0.153],[0.31,0.164],[0.38,0.212],[0.45,0.154],[0.52,0.093],[0.59,0.186],[0.66,0.195],[0.73,0.226],[0.8,0.172],[0.87,0.333],[0.94,0.186],[1.01,0.328],[1.08,0.505],[1.15,0.544],[1.22,0.479],[1.29,0.446],[1.36,0.579],[1.43,0.282],[1.5,0.162],[1.57,0.08],[1.64,0.04],[1.68,0],[2.2,0]],[[0,0],[0.04,0],[0.07,0.005],[0.12,0.054],[0.18,0.265],[0.24,0.166],[0.31,0.413],[0.38,0.231],[0.45,0.304],[0.52,0.486],[0.59,0.527],[0.66,0.323],[0.73,0.245],[0.8,0.35],[0.87,0.359],[0.94,0.359],[1.01,0.316],[1.08,0.27],[1.15,0.176],[1.22,0.203],[1.29,0.345],[1.36,0.342],[1.43,0.295],[1.5,0.238],[1.57,0.179],[1.64,0.052],[1.68,0],[2.2,0]],[[0,0],[0.04,0],[0.07,0.01],[0.12,0.019],[0.18,0.077],[0.24,0.128],[0.31,0.135],[0.38,0.221],[0.45,0.267],[0.52,0.254],[0.59,0.231],[0.66,0.268],[0.73,0.185],[0.8,0.183],[0.87,0.147],[0.94,0.201],[1.01,0.277],[1.08,0.361],[1.15,0.327],[1.22,0.368],[1.29,0.444],[1.36,0.306],[1.43,0.154],[1.5,0.168],[1.57,0.146],[1.64,0.048],[1.68,0],[2.2,0]],[[0,0],[0.04,0],[0.07,0.002],[0.12,0.008],[0.18,0.039],[0.24,0.071],[0.31,0.079],[0.38,0.175],[0.45,0.072],[0.52,0.135],[0.59,0.114],[0.66,0.099],[0.73,0.099],[0.8,0.107],[0.87,0.102],[0.94,0.109],[1.01,0.103],[1.08,0.104],[1.15,0.093],[1.22,0.107],[1.29,0.124],[1.36,0.105],[1.43,0.041],[1.5,0.031],[1.57,0.01],[1.64,0.007],[1.68,0],[2.2,0]],[[0,0],[0.04,0],[0.07,0.002],[0.12,0.007],[0.18,0.039],[0.24,0.052],[0.31,0.036],[0.38,0.041],[0.45,0.063],[0.52,0.091],[0.59,0.094],[0.66,0.154],[0.73,0.135],[0.8,0.137],[0.87,0.135],[0.94,0.106],[1.01,0.091],[1.08,0.152],[1.15,0.067],[1.22,0.08],[1.29,0.075],[1.36,0.078],[1.43,0.031],[1.5,0.018],[1.57,0.014],[1.64,0.007],[1.68,0],[2.2,0]],[[0,0],[0.04,0],[0.07,0.001],[0.12,0.003],[0.18,0.03],[0.24,0.036],[0.31,0.04],[0.38,0.042],[0.45,0.032],[0.52,0.05],[0.59,0.075],[0.66,0.103],[0.73,0.121],[0.8,0.162],[0.87,0.085],[0.94,0.082],[1.01,0.081],[1.08,0.037],[1.15,0.049],[1.22,0.053],[1.29,0.052],[1.36,0.07],[1.43,0.029],[1.5,0.007],[1.57,0.01],[1.64,0.004],[1.68,0],[2.2,0]]];
  // A second, independently moving set of partials creates changing beating.
  // Splitting amplitude here preserves approximately the same squared energy.
  const secondPartials = partials.slice(0, 3).map((points, index) => points.map(([time, value]) => {
    const late = Math.max(0, Math.min(1, (time - 1.02) / .28));
    const share = index === 0 ? .4 + .3 * late : index === 1 ? .34 * (1 - .6 * late) : .45 * (1 - .72 * late);
    return [time, value * share];
  }));
  for (let i = 0; i < secondPartials.length; i++) {
    for (let point = 0; point < partials[i].length; point++) {
      const amplitude = partials[i][point][1], other = secondPartials[i][point][1];
      partials[i][point][1] = Math.sqrt(Math.max(0, amplitude * amplitude - other * other));
    }
  }
  const nodes = [
    n('trumpet-pitch', 'curve', 40, 40, {}, 'Pitch contour', normalized(pitchHz.map(([t,f]) => [t,(f-360)/220]))),
    n('trumpet-color', 'curve', 40, 350, {}, 'Growing instability', normalized([[0,.25],[.10,.28],[.23,.48],[.38,.35],[.52,.64],[.72,.48],[.9,.34],[1.1,.48],[1.3,.67],[1.47,.85],[1.65,1],[2.2,1]])),
    n('voice-second-pitch', 'curve', 40, 660, {}, 'Independent pitch contour', normalized(secondPitchHz.map(([t,f]) => [t,(f-360)/240]))),
    n('voice-phase-noise', 'random', 360, 40, {rate:380,depth:.2,smooth:.78}, 'Fast phase variation'),
    n('voice-odd-motion', 'random', 360, 350, {rate:61,depth:.9,smooth:.72}, 'Uneven lower partials'),
    n('voice-even-motion', 'random', 360, 660, {rate:97,depth:.9,smooth:.46}, 'Uneven upper partials'),
    n('trumpet-volume', 'curve', 40, 970, {}, 'Breath articulation', normalized([[0,0],[.035,0],[.06,.025],[.1,.09],[.18,.23],[.3,.32],[.48,.38],[.57,.6],[.72,.47],[.88,.45],[1.04,.51],[1.16,.68],[1.3,.78],[1.42,.7],[1.52,.58],[1.6,.32],[1.68,0],[2.2,0]])),
    n('voice-noise', 'noise', 680, 970, {color:'white',level:1}, 'Turbulent air'),
    n('voice-air-filter', 'filter', 1000, 970, {cutoff:1250,resonance:.75,mode:'bandpass'}, 'Breath band'),
    n('voice-air-level', 'gain', 1320, 970, {gain:0}, 'Breath level'),
    n('voice-level', 'gain', 2600, 40, {gain:.9}, 'Partial balance'),
    n('voice-rolloff', 'filter', 2600, 350, {cutoff:5200,resonance:.6,mode:'lowpass'}, 'Soft high-frequency loss'),
    n('voice-space', 'space', 2600, 660, {size:.12,damping:.72,mix:.025,width:.28,distance:.02}, 'Close reflections'),
    n('elephant-output', 'output', 2600, 970, {gain:.84}, 'Voice output'),
  ];
  const connections = [
    c('voice-instability', 'trumpet-color', 'voice-phase-noise', 'depth', .75),
    c('voice-breath-motion', 'voice-even-motion', 'voice-air-filter', 'cutoff', 220),
    c('voice-breath-envelope', 'trumpet-volume', 'voice-air-level', 'gain', 1.25),
    c('voice-breath-filter', 'voice-noise', 'voice-air-filter'),
    c('voice-breath-level', 'voice-air-filter', 'voice-air-level'),
    c('voice-breath-mix', 'voice-air-level', 'voice-level'),
    c('voice-rolloff-input', 'voice-level', 'voice-rolloff'),
    c('voice-room-input', 'voice-rolloff', 'voice-space'),
    c('voice-listen', 'voice-space', 'elephant-output'),
  ];
  for (let i = 0; i < partials.length; i++) {
    const harmonic = i + 1, oscillator = `voice-partial-${harmonic}`, envelope = `voice-partial-envelope-${harmonic}`;
    const motion = harmonic % 2 ? 'voice-odd-motion' : 'voice-even-motion';
    const column = Math.floor(i / 3), row = i % 3;
    nodes.push(n(envelope, 'curve', 680+column*640, 40+row*310, {amount:.8}, `Partial ${harmonic} articulation`, normalized(partials[i])));
    nodes.push(n(oscillator, 'oscillator', 1000+column*640, 40+row*310, {frequency:360*harmonic,level:0,wave:'sine'}, `Sine partial ${harmonic}`));
    connections.push(
      c(`voice-pitch-${harmonic}`, 'trumpet-pitch', oscillator, 'frequency', 220*harmonic),
      c(`voice-phase-${harmonic}`, 'voice-phase-noise', oscillator, 'frequency', 25*harmonic),
      c(`voice-drift-${harmonic}`, motion, oscillator, 'frequency', 5.6*harmonic),
      c(`voice-shimmer-${harmonic}`, motion, envelope, 'amount', .24),
      c(`voice-amplitude-${harmonic}`, envelope, oscillator, 'level', 1.12),
      c(`voice-mix-${harmonic}`, oscillator, 'voice-level'),
    );
  }
  for (let i = 0; i < secondPartials.length; i++) {
    const harmonic = i + 1, oscillator = `voice-second-${harmonic}`, envelope = `voice-second-envelope-${harmonic}`;
    nodes.push(n(envelope, 'curve', 1960, 40+i*310, {amount:.8}, `Secondary partial ${harmonic} articulation`, normalized(secondPartials[i])));
    nodes.push(n(oscillator, 'oscillator', 2280, 40+i*310, {frequency:360*harmonic,level:0,wave:'sine'}, `Secondary sine ${harmonic}`));
    connections.push(
      c(`voice-second-pitch-${harmonic}`, 'voice-second-pitch', oscillator, 'frequency', 240*harmonic),
      c(`voice-second-phase-${harmonic}`, 'voice-phase-noise', oscillator, 'frequency', -12*harmonic),
      c(`voice-second-drift-${harmonic}`, 'voice-even-motion', oscillator, 'frequency', 9*harmonic),
      c(`voice-second-shimmer-${harmonic}`, 'voice-odd-motion', envelope, 'amount', -.22),
      c(`voice-second-amplitude-${harmonic}`, envelope, oscillator, 'level', 1.12),
      c(`voice-second-mix-${harmonic}`, oscillator, 'voice-level'),
    );
  }
  return {
    id:'elephant', name:'Elephant', subtitle:'Articulated partials · turbulent air', icon:'wind', color:'#c2a5ff',
    description:'Two small groups of sine oscillators follow independent pitch contours, with a separate articulation for each partial. Fast random modulation adds changing roughness, and filtered noise supplies the breath. Every layer is a reusable building block.',
    graph:{duration,loop:true,nodes,connections},
  };
}
