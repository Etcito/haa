import { BASE_PRESET } from "../mix/presets/base.js";
import { IMPACT_FAMILIES } from "./impacts.js";
import { STING_FAMILIES } from "./stings.js";
import { UI_FAMILIES } from "./ui.js";
import { WIND_BED, WIND_FAMILIES } from "./wind.js";
import { CALM_STYLE, UPBEAT_STYLE } from "./styles.js";
import { MUSIC_BED } from "./musicBed.js";
import { BEACON_BED } from "./beacon.js";
/** Every built-in one-shot family, sorted by id. */
export const BUILTIN_FAMILIES = [
    ...IMPACT_FAMILIES,
    ...UI_FAMILIES,
    ...STING_FAMILIES,
    ...WIND_FAMILIES,
].sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
/** Every built-in bed. */
export const BUILTIN_BEDS = [WIND_BED, MUSIC_BED, BEACON_BED];
/** Every built-in mix preset. */
export const BUILTIN_MIXES = [BASE_PRESET];
/** Every built-in music style: the M3 genre-neutral UPBEAT_STYLE and CALM_STYLE. */
export const BUILTIN_STYLES = [UPBEAT_STYLE, CALM_STYLE];
