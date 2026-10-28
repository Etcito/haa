/** Measurement-only band limiting. Playback and the source PCM stay intact. */
function bandSection(kind, frequency, q, sampleRate) {
  const omega = 2 * Math.PI * frequency / sampleRate;
  const cosine = Math.cos(omega), alpha = Math.sin(omega) / (2 * q);
  const a0 = 1 + alpha;
  const side = kind === 'highpass' ? 1 : -1;
  const edge = 1 + side * cosine;
  return { b0: edge / (2 * a0), b1: -side * edge / a0,
    b2: edge / (2 * a0), a1: -2 * cosine / a0,
    a2: (1 - alpha) / a0, z1: 0, z2: 0 };
}

/** Fourth-order Butterworth edges; sum channel energy without mono cancellation. */
export function audibleBandRms(channels, sampleRate) {
  console.info('[Sona] Measuring comparison level between 20 Hz and 20 kHz');
  const length = Math.min(...channels.map(channel => channel.length));
  if (!length) return 0;
  let sumSquares = 0;
  const upper = Math.min(20000, sampleRate * .49);
  for (const channel of channels) {
    const sections = [];
    for (const q of [.541196100146197, 1.306562964876377]) {
      sections.push(bandSection('highpass', 20, q, sampleRate));
      sections.push(bandSection('lowpass', upper, q, sampleRate));
    }
    for (let i = 0; i < length; i++) {
      let value = Number.isFinite(channel[i]) ? channel[i] : 0;
      for (const section of sections) {
        const output = section.b0 * value + section.z1;
        section.z1 = section.b1 * value - section.a1 * output + section.z2;
        section.z2 = section.b2 * value - section.a2 * output;
        value = output;
      }
      sumSquares += value * value;
    }
  }
  return Math.sqrt(sumSquares / (length * channels.length));
}
