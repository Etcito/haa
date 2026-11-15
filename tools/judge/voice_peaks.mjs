// The judge's per-voice saturation check: every sfx preset rendered dry (the wasm engine, as the scene bakes it)
// at energy 0.3, 0.7 and 1, its declared inputs only. A voice is flagged when
//  - it reaches the voice limiter (haa-core LIMIT_PEAK, 0.92 = -0.7 dBFS): its transient is ridden down, a
//    saturated attack whatever the master's limiter says;
//  - it holds a flat top (more than 3 samples within 0.1 % of its peak in a row);
//  - its crest factor falls by more than 3 dB from energy 0.3 to 1 (a level that compresses its own peaks).
// Captures cannot show this: a voice clipped inside its own graph reaches the master clean and quiet.
//
//   node tools/judge/voice_peaks.mjs [preset ...]      (after `npm run build`; exit 1 when a voice is flagged)
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const node = await import(ROOT + 'packages/haa/dist/node.js');
const module = await node.loadModule();
const LIMIT_PEAK = 0.92;
const names = process.argv.slice(2).length
  ? process.argv.slice(2)
  : readdirSync(ROOT + 'patches/sfx').filter((f) => f.endsWith('.json')).map((f) => f.slice(0, -5));
let flagged = 0;
for (const name of names) {
  const bytes = readFileSync(`${ROOT}patches/sfx/${name}.json`);
  const declared = JSON.parse(bytes.toString()).inputs.map((i) => i.name);
  const rows = [];
  for (const energy of [0.3, 0.7, 1]) {
    const want = { energy, speed: energy, intensity: energy, hardness: 0.9, size: 0.5, flux: 1 };
    const inputs = Object.fromEntries(Object.entries(want).filter(([k]) => declared.includes(k)));
    const r = node.renderPatch(module, bytes, { seconds: 1, inputs });
    const L = r.left ?? r[0] ?? r.channels?.[0];
    let peak = 0;
    let sum = 0;
    for (const v of L) {
      peak = Math.max(peak, Math.abs(v));
      sum += v * v;
    }
    let run = 0;
    let flat = 0;
    for (const v of L) {
      run = peak > 0 && Math.abs(v) > peak * 0.999 ? run + 1 : 0;
      flat = Math.max(flat, run);
    }
    const crest = 20 * Math.log10(peak / Math.sqrt(sum / L.length + 1e-30) + 1e-30);
    rows.push({ energy, peak, flat, crest });
  }
  const why = [];
  if (rows.some((x) => x.peak >= LIMIT_PEAK * 0.999)) why.push('reaches the voice limiter');
  if (rows.some((x) => x.flat > 3)) why.push('flat top');
  if (rows[0].peak > 1e-4 && rows[0].crest - rows[2].crest > 3) why.push(`crest -${(rows[0].crest - rows[2].crest).toFixed(1)} dB at full energy`);
  if (why.length) flagged++;
  const cells = rows.map((x) => `e${x.energy}: ${(20 * Math.log10(x.peak + 1e-12)).toFixed(1)} dBFS crest ${x.crest.toFixed(1)}`).join(' | ');
  console.log(`${why.length ? 'FLAG' : 'ok  '} ${name.padEnd(22)} ${cells}${why.length ? '  <- ' + why.join(', ') : ''}`);
}
process.exit(flagged ? 1 : 0);
