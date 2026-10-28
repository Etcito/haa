import type { BedDef, FamilyDef, FamilySpec, MixPreset, MusicStyle } from '../contract.js';
/** Validate one family spec and return a new frozen definition with a seed filled in. */
export declare function defineFamily(spec: FamilySpec): FamilyDef;
/** Validate one bed definition and return a new frozen copy (parameter arrays included). */
export declare function defineBed(spec: BedDef): BedDef;
/** Validate one mix preset and return a new frozen copy. */
export declare function defineMix(p: MixPreset): MixPreset;
/** Validate one music style and return a new frozen copy. */
export declare function defineStyle(s: MusicStyle): MusicStyle;
/** Register a family; a duplicate id throws '[haa] duplicate family <id>'. */
export declare function registerFamily(def: FamilySpec): void;
/** Register several families in order; any invalid or duplicate entry throws. */
export declare function registerFamilies(defs: readonly FamilySpec[]): void;
/** Register a bed; a duplicate id throws '[haa] duplicate bed <id>'. */
export declare function registerBed(def: BedDef): void;
/** Register a mix preset; a duplicate id throws '[haa] duplicate mix <id>'. */
export declare function registerMix(p: MixPreset): void;
/** Register a music style; a duplicate id throws '[haa] duplicate style <id>'. */
export declare function registerStyle(s: MusicStyle): void;
/** The family with this id, or undefined. */
export declare function getFamily(id: string): FamilyDef | undefined;
/** Every registered family, ascending id. */
export declare function listFamilies(): readonly FamilyDef[];
/** The bed with this id, or undefined. */
export declare function getBed(id: string): BedDef | undefined;
/** Every registered bed, ascending id. */
export declare function listBeds(): readonly BedDef[];
/** The mix preset with this id, or undefined. */
export declare function getMix(id: string): MixPreset | undefined;
/** Every registered mix preset, ascending id. */
export declare function listMixes(): readonly MixPreset[];
/** The highest-priority preset (ties by lowest id); throws when none is registered. */
export declare function activeMix(): MixPreset;
/** The style with this id, or undefined. */
export declare function getStyle(id: string): MusicStyle | undefined;
/** Every registered style, ascending id; its index is the music 'style' param code. */
export declare function listStyles(): readonly MusicStyle[];
/** Index of a style in listStyles(), or -1 when absent. */
export declare function styleIndex(id: string): number;
/** Family trim of p for id (dB), 0 when the preset has none. */
export declare function familyTrimDb(p: MixPreset, id: string): number;
/** Bed trim of p for id (dB), 0 when the preset has none. */
export declare function bedTrimDb(p: MixPreset, id: string): number;
/** Upper bound of the bank in seconds: the sum of variants x maxSeconds. */
export declare function bankSeconds(defs?: readonly FamilyDef[]): number;
/** Drop every registered definition (tests only). */
export declare function clearRegistry(): void;
