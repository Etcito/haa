// Bank bake (bake module, node only): renders each family's normalized variants, packs them
// into one strip, encodes it to `<outDir>/<id>.mp3` and writes the deterministic manifest.json
// (shared 'Bank manifest' format: fixed key order, 2-space JSON, trailing newline, no timestamp).
import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { packStrip, pcmHash } from "../bank/pack.js";
import { renderFamilyVariants } from "../bank/render.js";
import { activeMix } from "../registry/registry.js";
import { encodeMp3 } from "./mp3.js";
/** The only bake rate and bitrate of contract v0. */
const BAKE_RATE = 48000;
const BAKE_KBPS = 128;
/**
 * Bake families to MP3 strips plus manifest.json in opts.outDir and return the manifest.
 * Families are processed by id; preset defaults to activeMix(). The same inputs give
 * byte-identical MP3 files and manifest.
 */
export function bake(families, opts) {
    const rate = opts.rate ?? BAKE_RATE;
    const kbps = opts.kbps ?? BAKE_KBPS;
    if (rate !== BAKE_RATE || kbps !== BAKE_KBPS)
        throw new Error('[haa] bake supports 48000 Hz / 128 kbps in v0');
    const preset = opts.preset ?? activeMix();
    const log = opts.log ?? (() => { });
    mkdirSync(opts.outDir, { recursive: true });
    const sorted = [...families].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    const entries = [];
    let totalVariants = 0;
    let totalSamples = 0;
    let totalBytes = 0;
    for (const def of sorted) {
        const variants = renderFamilyVariants(def, BAKE_RATE, preset);
        const { strip, entries: slots, onset0 } = packStrip(variants);
        const mp3 = encodeMp3(strip, BAKE_RATE, BAKE_KBPS);
        const file = `${def.id}.mp3`;
        writeFileSync(join(opts.outDir, file), mp3);
        entries.push({
            id: def.id,
            file,
            bus: def.bus,
            category: def.category,
            variants: slots.map((s) => ({ start: s.start, length: s.length })),
            onset0,
            pcmHash: pcmHash(strip),
            mp3Sha256: createHash('sha256').update(mp3).digest('hex'),
        });
        totalVariants += variants.length;
        totalSamples += strip.length;
        totalBytes += mp3.length;
        log(`${def.id} ${variants.length} variants ${(strip.length / BAKE_RATE).toFixed(2)} s ${mp3.length} B`);
    }
    const manifest = {
        version: 1,
        sampleRate: BAKE_RATE,
        kbps: BAKE_KBPS,
        encoder: 'lamejs 1.2.1',
        preset: preset.id,
        families: entries,
    };
    writeFileSync(join(opts.outDir, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
    log(`total ${entries.length} families ${totalVariants} variants ${(totalSamples / BAKE_RATE).toFixed(2)} s ${totalBytes} B`);
    return manifest;
}
