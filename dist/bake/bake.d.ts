import type { BankManifest, FamilyDef, MixPreset } from '../contract.js';
/**
 * Bake families to MP3 strips plus manifest.json in opts.outDir and return the manifest.
 * Families are processed by id; preset defaults to activeMix(). The same inputs give
 * byte-identical MP3 files and manifest.
 */
export declare function bake(families: readonly FamilyDef[], opts: {
    outDir: string;
    rate?: 48000;
    kbps?: 128;
    preset?: MixPreset;
    log?: (line: string) => void;
}): BankManifest;
