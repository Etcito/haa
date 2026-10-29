/**
 * Five editable studies of water, assembled only from general synthesis nodes.
 * The scores contain measured event times / envelope landmarks, never samples.
 * Reference excerpts: 6223 0–6.384 s; 26638 18–26 s; 212173 1–9 s;
 * 8978 10–18 s; 8977 5–13 s. Reference playback belongs to the A/B player.
 */

// time, initial Hz, final Hz, amplitude time constant, level, attack,
// secondary-mode ratio, secondary level, glide curvature.
const SMALL_DROPS = [
  [.207, 2550, 2410, .0059, .325, .0011, 1.94, .055, .6],
  [.551, 1250, 3200, .0133, .394, .0010, 3.85, .040, 1.8],
  [1.114, 2500, 2720, .0064, .130, .0009, 1.93, .090, .7],
  [1.519, 2720, 3400, .0070, .238, .0005, 1.35, .480, .5],
  [1.749, 1900, 2700, .0048, .688, .0013, 2.12, .070, .45],
  [1.869, 3950, 5250, .0036, .167, .0008, 1.31, .180, .55],
  [2.122, 1780, 2650, .0091, .584, .0014, 2.04, .060, .8],
  [2.280, 2550, 3650, .0053, .044, .0008, 1.36, .420, .7],
  [2.879, 3720, 4250, .0029, .084, .0005, 1.24, .200, .6],
  [3.393, 1840, 2410, .0082, .807, .0012, 2.55, .045, .45],
  [3.729, 2490, 2770, .0090, .438, .0011, 1.91, .100, .6],
  [3.901, 1300, 2150, .0155, .070, .0011, 2.62, .070, 1.5],
  [3.983, 3950, 5900, .0031, .382, .0006, 1.24, .400, .65],
  [4.332, 1770, 2460, .0047, .820, .0009, 2.47, .055, .5],
  [4.463, 4450, 5350, .0028, .103, .0005, 1.17, .480, .7],
  [4.773, 3220, 4120, .0034, .054, .0006, 1.18, .300, .6],
  [5.132, 1800, 2710, .0072, .438, .0010, 2.20, .080, .6],
  [5.459, 2450, 2870, .0083, .067, .0008, 1.90, .100, .7],
];

const HOLLOW_DROPS = [
  [1.555, 460, 1100, .0075, .045, .0020, 2.11, .330, .7],
  [1.624, 1050, 1550, .0170, .528, .0035, 1.94, .045, .55],
  [1.725, 1620, 2800, .0069, .059, .0012, 1.43, .270, .6],
  [3.872, 1980, 3250, .0089, .820, .0012, 1.33, .260, .55],
  [4.049, 1020, 1250, .0130, .339, .0017, 2.10, .035, .5],
  [6.057, 490, 1040, .0060, .109, .0017, 1.91, .450, .55],
  [6.122, 1040, 1560, .0180, .568, .0045, 2.06, .055, .55],
];

// 250 ms RMS landmarks from the selected recordings. Fine texture is synthesized.
const DETAIL_ENVELOPE = [16.039,18.531,13.343,13.282,15.733,13.980,20.080,15.866,
  13.484,13.810,17.833,15.194,14.173,12.935,13.998,13.005,10.387,15.469,
  16.275,10.862,12.148,23.827,12.588,8.871,11.210,16.539,21.003,16.126,
  15.561,11.298,13.276,16.195];
const RAIN_TWO_ENVELOPE = [23.689,22.593,29.914,31.974,27.970,27.258,25.025,25.051,
  27.166,24.054,28.969,32.229,26.561,24.664,22.300,26.244,24.395,26.843,
  28.732,28.177,25.927,22.783,25.040,24.310,23.570,24.863,23.314,23.659,
  27.148,23.545,22.878,27.560];
const RAIN_ONE_ENVELOPE = [24.794,21.460,19.711,28.816,23.740,32.212,24.696,66.519,
  27.901,29.472,25.793,20.811,21.185,23.533,31.061,24.137,35.608,36.321,
  23.621,20.479,19.145,23.251,21.750,25.646,21.738,26.153,21.941,25.358,
  25.053,21.554,17.298,21.032];

const DETAIL_CLICKS = [[.25708,.5585],[.44542,.4083],[1.16821,.5242],[1.58054,.3429],
  [2.48042,.4134],[2.67862,.3675],[3.82450,.3466],[4.41079,.4104],
  [5.16500,.3621],[5.25421,1],[5.38929,.3521],[6.32717,.4169],
  [6.35738,.4339],[6.63183,.7298],[7.15388,.3685],[7.80525,.5149]];
const RAIN_TWO_CLICKS = [[.66842,.6073],[.75154,.4937],[.87825,.5647],[.91996,.4909],
  [2.39954,.4375],[2.52496,.5390],[2.95146,.7668],[2.97917,.4687],
  [3.35871,.6353],[4.14512,.4435],[4.42554,.5191],[4.63508,.4581],
  [4.72921,1],[4.97129,.4462],[5.44854,.4500],[7.15225,.5215]];
const RAIN_ONE_CLICKS = [[.00725,.4067],[.94875,.5182],[.97871,.5874],[1.33392,.4217],
  [1.48562,.3883],[1.78271,.5538],[1.81675,1],[1.859,.5676],
  [1.94279,.6877],[1.97254,.6746],[2.38713,.4662],[3.57308,.4405],
  [4.07167,.5400],[4.12304,.4048],[4.28496,.6554],[6.88921,.4113]];

const clamp = (value, low, high) => Math.min(high, Math.max(low, value));
const tidy = (points, duration) => points
  .map(([time, value]) => [clamp(time / duration, 0, 1), clamp(value, 0, 1)])
  .sort((a, b) => a[0] - b[0]);

function dampedEnvelope(score, duration, secondary = false) {
  const points = [[0, 0]];
  for (let i = 0; i < score.length; i++) {
    const [time, , , decay, level, attack, , partial] = score[i];
    const tau = decay * (secondary ? .43 : 1);
    const peak = level * (secondary ? partial : 1);
    const end = Math.min(duration, (score[i + 1]?.[0] ?? duration) - .0001);
    points.push([time, 0], [time + attack * (secondary ? .45 : 1), peak]);
    for (const t of [1, 2, 4, 6]) {
      const moment = time + attack + t * tau;
      if (moment < end) points.push([moment, t === 6 ? 0 : peak * Math.exp(-t)]);
    }
    if (time + attack + 6 * tau >= end) points.push([end, 0]);
  }
  points.push([duration, 0]);
  return tidy(points, duration);
}

function pitchEnvelope(score, duration, secondary = false) {
  const points = [[0, (score[0][1] - 20) / 11000]];
  for (const [time, initial, final, decay, , attack, ratio, , curvature] of score) {
    for (const portion of [0, .12, .33, .66, 1]) {
      const hz = (initial + (final - initial) * Math.pow(portion, curvature)) * (secondary ? ratio : 1);
      points.push([time + attack + portion * decay * (secondary ? 2.6 : 6), (Math.min(11000, hz) - 20) / 11000]);
    }
  }
  points.push([duration, points[points.length - 1][1]]);
  return tidy(points, duration);
}

function attackEnvelope(score, duration, hollow, extras = []) {
  const points = [[0, 0]];
  const impacts = [...score.map(event => [event[0], event[4] * (hollow ? .28 : .85)]), ...extras];
  for (const [time, level] of impacts) {
    points.push([time, 0], [time + .0003, level], [time + (hollow ? .0015 : .003), level * .23], [time + (hollow ? .005 : .012), 0]);
  }
  points.push([duration, 0]);
  return tidy(points, duration);
}

function sparseStudy(n, c, { id, name, subtitle, description, score, duration, hollow }) {
  const key = suffix => `${id}-${suffix}`;
  const node = (suffix, type, x, y, params, label, curve) => n(key(suffix), type, x, y, params, label, curve);
  const edge = (from, to, target = 'audio', amount = 1) => c(key(`${from}-${to}-${target}`), key(from), key(to), target, amount);
  // Keep every envelope landmark: two alternating voices fit the editor's
  // 64-point curve budget without flattening attacks or shortening tails.
  const groups = score.length > 9 ? [score.filter((_, i) => i % 2 === 0), score.filter((_, i) => i % 2 === 1)] : [score];
  const nodes = [], connections = [];
  groups.forEach((events, index) => {
    const voice = groups.length > 1 ? (index ? 'B' : 'A') : '';
    const suffix = voice ? ` · ${voice}` : '';
    const part = name => `${name}${voice ? '-' + voice : ''}`;
    const top = 50 + index * 1350;
    const extras = hollow ? [] : index ? [[5.278, .18]] : [[.359, .20]];
    nodes.push(
      node(part('body-envelope'), 'curve', 40, top, { amount: 1 }, 'Main decay score' + suffix, dampedEnvelope(events, duration)),
      node(part('body-pitch'), 'curve', 40, top + 250, { amount: 1 }, 'Changing drop pitch' + suffix, pitchEnvelope(events, duration)),
      node(part('partial-envelope'), 'curve', 40, top + 500, { amount: 1 }, 'Short secondary decay' + suffix, dampedEnvelope(events, duration, true)),
      node(part('partial-pitch'), 'curve', 40, top + 750, { amount: 1 }, 'Inharmonic partial pitch' + suffix, pitchEnvelope(events, duration, true)),
      node(part('attack-score'), 'curve', 40, top + 1000, { amount: 1 }, 'Impact and splash score' + suffix, attackEnvelope(events, duration, hollow, extras)),
      node(part('body'), 'oscillator', 340, top, { wave: 'sine', frequency: 20, level: 0 }, 'Main damped mode' + suffix),
      node(part('partial'), 'oscillator', 340, top + 500, { wave: 'sine', frequency: 20, level: 0 }, 'Short inharmonic mode' + suffix),
      node(part('attack'), 'noise', 340, top + 1000, { color: 'white', level: 0 }, 'Brief broadband impact' + suffix),
    );
    connections.push(
      edge(part('body-envelope'), part('body'), 'level'), edge(part('body-pitch'), part('body'), 'frequency', 11000),
      edge(part('partial-envelope'), part('partial'), 'level'), edge(part('partial-pitch'), part('partial'), 'frequency', 11000),
      edge(part('attack-score'), part('attack'), 'level'), edge(part('attack'), 'attack-hp'),
      edge(part('body'), 'mix'), edge(part('partial'), 'mix'),
    );
  });
  nodes.push(
    node('micro-grains', 'grain', 640, 1550, { wave: 'noise', frequency: 5000, rate: hollow ? 1.6 : 5.5, attack: .0001, decay: hollow ? .004 : .0012, spread: 0, jitter: 1, variation: .95, glide: 0, delay: .11, width: 0, level: hollow ? .009 : .007 }, 'Quiet micro-impacts'),
    node('attack-hp', 'filter', 640, 1050, { mode: 'highpass', cutoff: hollow ? 1300 : 3100, resonance: .65 }, 'Splash high-pass'),
    node('attack-lp', 'filter', 930, 1050, { mode: 'lowpass', cutoff: hollow ? 6600 : 10700, resonance: .65 }, 'Splash bandwidth'),
    node('attack-weight', 'gain', 1230, 1050, { gain: hollow ? 1 : 1.55 }, 'Splash detail level'),
    node('background', 'noise', 640, 1300, { color: hollow ? 'white' : 'brown', level: hollow ? .0008 : .012 }, 'Quiet recording floor'),
    node('mix', 'gain', 1230, 300, { gain: 1 }, 'Body, splash and background'),
    node('output', 'output', 1530, 300, { gain: .88 }, 'Water output'),
  );
  connections.push(edge('micro-grains', 'attack-hp'), edge('attack-hp', 'attack-lp'), edge('attack-lp', 'attack-weight'), edge('attack-weight', 'mix'), edge('background', 'mix'), edge('mix', 'output'));
  return {
    id, name, subtitle, description, icon: 'droplet', color: hollow ? '#9ecbe5' : '#82c4f8',
    graph: { duration, loop: true, nodes, connections },
  };
}

function measuredEnvelope(values, duration = 8) {
  const maximum = Math.max(...values);
  return tidy([[0, values[0] / maximum], ...values.map((value, index) => [index * .25 + .125, value / maximum]), [duration, values[values.length - 1] / maximum]], duration);
}

function clickEnvelope(events, duration = 8) {
  const points = [[0, 0]];
  // Foreground landmarks complement the random grain population. Four points
  // for each of the 14 strongest accents stay below the editor's 64-point limit.
  const strongest = [...events].sort((a, b) => b[1] - a[1]).slice(0, 14);
  for (const [time, level] of strongest) points.push([time - .0004, 0], [time, level], [time + .00045, level * .24], [time + .0025, 0]);
  points.push([duration, 0]);
  return tidy(points, duration);
}

function textureStudy(n, c, config) {
  const { id, detail, envelope } = config;
  const key = suffix => `${id}-${suffix}`;
  const node = (suffix, type, x, y, params, label, curve) => n(key(suffix), type, x, y, params, label, curve);
  const edge = (from, to, target = 'audio', amount = 1) => c(key(`${from}-${to}-${target}`), key(from), key(to), target, amount);
  const brighter = id === 'rain-city-1';
  const nodes = [
    node('activity', 'curve', 40, 50, { amount: 1 }, 'Measured activity contour', measuredEnvelope(envelope)),
    node('density-drift', 'random', 40, 320, { rate: detail ? 19 : 7, depth: 1, smooth: .8 }, 'Irregular impact density'),
    node('bed-low', detail ? 'noise' : 'grain', 340, 50, detail ? { color: 'brown', level: .050 } : { wave: 'noise', frequency: 180, rate: 700, attack: .0003, decay: .045, spread: 0, jitter: 1, variation: .7, glide: 0, delay: 0, width: 1, level: brighter ? .80 : .90 }, 'Low continuous wash'),
    node('bed-low-hp', 'filter', 640, 50, { mode: 'highpass', cutoff: detail ? 160 : 55, resonance: .65 }, 'Remove subsonic wash'),
    node('bed-low-lp', 'filter', 930, 50, { mode: 'lowpass', cutoff: detail ? 510 : brighter ? 230 : 170, resonance: .8 }, 'Low wash bandwidth'),
    node('bed-mid', 'noise', 340, 320, { color: 'pink', level: detail ? .10 : brighter ? .30 : .32 }, 'Diffuse middle layer'),
    node('bed-mid-filter', 'filter', 640, 320, { mode: 'bandpass', cutoff: detail ? 1500 : brighter ? 700 : 720, resonance: detail ? .65 : brighter ? 1 : 1.2 }, 'Middle texture colour'),
    node('body-grains', 'grain', 340, 590, { wave: 'noise', frequency: 1400, rate: detail ? 145 : 280, attack: .00015, decay: detail ? .0014 : .006, spread: 0, jitter: .96, variation: .9, glide: 0, delay: .003, width: .95, level: detail ? .37 : brighter ? .33 : .23 }, 'Overlapping noise impacts'),
    node('body-filter', 'filter', 640, 590, { mode: 'bandpass', cutoff: detail ? 1500 : brighter ? 1350 : 950, resonance: detail ? .55 : brighter ? 1 : .7 }, 'Impact body band'),
    node('fine-grains', 'grain', 340, 860, { wave: 'noise', frequency: 6000, rate: detail ? 190 : 470, attack: .0001, decay: detail ? .0011 : .0022, spread: 0, jitter: 1, variation: .95, glide: 0, delay: .008, width: 1, level: detail ? .20 : .09 }, 'Fine irregular splashes'),
    node('fine-hp', 'filter', 640, 860, { mode: 'highpass', cutoff: detail ? 2500 : 2100, resonance: .6 }, 'Fine splash high-pass'),
    node('fine-lp', 'filter', 930, 860, { mode: 'lowpass', cutoff: detail ? 8000 : 14800, resonance: .7 }, 'Air and recording bandwidth'),
    node('ring-grains', 'grain', 340, 1130, { wave: 'sine', frequency: detail ? 1750 : 1400, rate: detail ? 31 : 16, attack: .0004, decay: detail ? .0032 : .0024, spread: detail ? 17 : 23, jitter: 1, variation: .95, glide: .35, delay: .019, width: .85, level: detail ? .040 : .017 }, 'Tiny pitched resonances'),
    node('accent-grains', 'grain', 640, 1130, { wave: 'noise', frequency: 2700, rate: detail ? 7.8 : brighter ? 2.7 : 1.6, attack: .0001, decay: detail ? .0015 : .0016, spread: 0, jitter: 1, variation: .9, glide: 0, delay: .12, width: .85, level: detail ? .50 : .28 }, 'Larger scattered impacts'),
    node('accent-filter', 'filter', 930, 1130, { mode: detail ? 'highpass' : 'lowpass', cutoff: detail ? 900 : 6500, resonance: .6 }, 'Foreground impact colour'),
    node('mix', 'gain', 1220, 320, { gain: 0 }, 'Layer balance and activity'),
    node('space', 'space', 1510, 320, { size: .035, damping: .45, mix: detail ? .04 : .10, width: .9, distance: 0 }, 'Small diffuse reflections'),
    node('output', 'output', 1800, 320, { gain: detail ? .92 : .88 }, 'Water texture output'),
  ];
  const connections = [
    edge('bed-low', 'bed-low-hp'), edge('bed-low-hp', 'bed-low-lp'), edge('bed-low-lp', 'mix'),
    edge('bed-mid', 'bed-mid-filter'), edge('bed-mid-filter', 'mix'),
    edge('body-grains', 'body-filter'), edge('body-filter', 'mix'),
    edge('fine-grains', 'fine-hp'), edge('fine-hp', 'fine-lp'), edge('fine-lp', 'mix'),
    edge('ring-grains', 'mix'), edge('accent-grains', 'accent-filter'), edge('accent-filter', 'mix'),
    edge('density-drift', 'body-grains', 'rate', detail ? 65 : 95),
    edge('density-drift', 'fine-grains', 'rate', detail ? 90 : 180),
    edge('activity', 'mix', 'gain', detail ? 1.18 : brighter ? 1.6 : 1.10),
    edge('mix', 'space'), edge('space', 'output'),
  ];
  nodes.push(
    node('near-score', 'curve', 40, 1400, { amount: 1 }, 'Nearest impact accents', clickEnvelope(detail ? DETAIL_CLICKS : brighter ? RAIN_ONE_CLICKS : RAIN_TWO_CLICKS)),
    node('near-noise', 'noise', 340, 1400, { color: 'white', level: 0 }, 'Very short foreground clicks'),
    node('near-hp', 'filter', 640, 1400, { mode: 'highpass', cutoff: detail ? 1200 : 550, resonance: .65 }, 'Foreground click high-pass'),
    node('near-weight', 'gain', 930, 1400, { gain: detail ? 1.75 : 1.2 }, 'Close impact weight'),
  );
  connections.push(edge('near-score', 'near-noise', 'level', detail ? .92 : .68), edge('near-noise', 'near-hp'), edge('near-hp', 'near-weight'), edge('near-weight', 'fine-lp'));
  return { id, name: config.name, subtitle: config.subtitle, description: config.description,
    icon: 'droplet', color: detail ? '#6fd7d1' : brighter ? '#a4c4ef' : '#80a9c4',
    graph: { duration: 8, loop: true, nodes, connections } };
}

export function buildWaterStudies(n, c) {
  return [
    sparseStudy(n, c, { id: 'water', name: 'Scattered water drops', subtitle: 'Short changing resonances', duration: 6.384, score: SMALL_DROPS, hollow: false,
      description: 'An irregular six-second score with changing pitch, very short damping, independent secondary modes and brief noisy impacts. Each envelope and sound layer stays editable.' }),
    sparseStudy(n, c, { id: 'water-hollow', name: 'Hollow water drops', subtitle: 'Three impacts and small rebounds', duration: 8, score: HOLLOW_DROPS, hollow: true,
      description: 'Three separated groups of low water resonances, with little preceding and following impacts. Long quiet gaps preserve the recording’s sparse character.' }),
    textureStudy(n, c, { id: 'water-detail', detail: true, envelope: DETAIL_ENVELOPE, name: 'Water micro-splashes', subtitle: 'Dense texture and close impacts',
      description: 'Several densities of short noise grains, tiny pitched resonances and occasional foreground clicks form a continuous water texture. A measured activity curve shapes its movement.' }),
    textureStudy(n, c, { id: 'rain-city-2', detail: false, envelope: RAIN_TWO_ENVELOPE, name: 'City rain · soft', subtitle: 'Low wash with scattered splashes',
      description: 'A low continuous wash, diffuse middle frequencies and independent fine impacts follow the selected city-rain excerpt. Density, impact length, colour and background balance are separate controls.' }),
    textureStudy(n, c, { id: 'rain-city-1', detail: false, envelope: RAIN_ONE_ENVELOPE, name: 'City rain · lively', subtitle: 'Uneven bursts and bright details',
      description: 'A more active rain study with short foreground bursts, changing density and a measured rise and fall in level. Continuous background and individual impacts can be sculpted independently.' }),
  ];
}
