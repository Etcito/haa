// Realm-neutral types and constants of the host engine: the patch format, published inputs,
// the node catalog, validation issues and the AudioWorklet player messages (ARCHITECTURE §5, §7–§9, §11, §12).
// Pure declarations and constants; the wasm engine and the host realms import from here.

/** Frames per `process` call, mirrors `BLOCK` of crates/haa-core/src/consts.rs. */
export const BLOCK = 128;

/** Sample rate of offline renders (CLI, Node, Worker), mirrors `OFFLINE_RATE`. */
export const OFFLINE_RATE = 48_000;

/** Rate of control trace packets in Hz, mirrors `TRACE_HZ`. */
export const TRACE_HZ = 25;

/** Length of a live patch's render when no seconds are given, mirrors `LIVE_DEFAULT_SECONDS`. */
export const LIVE_DEFAULT_SECONDS = 10;

/** Registered name of the AudioWorklet player processor. */
export const PLAYER_PROCESSOR = 'haa-player';

/** Every validation issue code of §12. Only the errors stop a patch from playing. */
export type PatchIssueCode =
  | 'not-json'
  | 'not-object'
  | 'format-too-new'
  | 'bad-field'
  | 'bad-duration'
  | 'duplicate-node-id'
  | 'duplicate-connection-id'
  | 'unknown-type'
  | 'legacy-type'
  | 'bad-enum'
  | 'unknown-node'
  | 'bad-port'
  | 'cycle'
  | 'too-many-nodes'
  | 'too-many-connections'
  | 'no-output'
  | 'unknown-input'
  | 'duplicate-input'
  | 'bad-input'
  | 'unknown-param'
  | 'param-clamped'
  | 'wrapped-sona-project'
  | 'wrapped-sona-graph'
  | 'performance-converted';

/** One problem found while loading or validating a patch, serialized as the loader returns it. */
export interface PatchIssue {
  severity: 'error' | 'warning' | 'info';
  code: PatchIssueCode;
  /** Dotted path with indices, e.g. `graph.nodes[3].params.cutoff`. */
  path: string;
  /** Node id, present when the issue concerns one node. */
  node?: string;
  message: string;
}

/** A published input the game sets by name (ARCHITECTURE §8). */
export interface HaaInput {
  name: string;
  kind: 'continuous' | 'trigger' | 'enum';
  /** Free text, default `""`. */
  unit?: string;
  /** Game units; `min < max`. */
  min: number;
  max: number;
  /** Clamped to the range, or an index into `values` for an enum. */
  default: number;
  /** One-pole time constant in seconds; 0 is unsmoothed. Default 0.05. */
  smooth?: number;
  /** Slider and `map` axis. Default `"lin"`. */
  curve?: 'lin' | 'log';
  /** Required and non-empty for an enum; the value is the index. */
  values?: string[];
  [key: string]: unknown;
}

/** A HAA patch file: the header, the published inputs and the Sona graph (ARCHITECTURE §7). */
export interface HaaPatch {
  /** Patch format number; 1 is `PATCH_FORMAT`. */
  haa: number;
  /** Lowercase-dash id; the file name equals it. */
  id: string;
  title: string;
  /** Default `[]`. */
  inputs?: HaaInput[];
  graph: SonaGraph;
  /** Sona's project master value, copied as-is; default unity. */
  master?: number;
  [key: string]: unknown;
}

/** Sona's graph, unchanged, plus HAA's `mode` (ARCHITECTURE §7). */
export interface SonaGraph {
  /** Seconds, greater than 0. */
  duration: number;
  /** Default false. */
  loop?: boolean;
  /** Default `"timeline"`. */
  mode?: 'timeline' | 'live';
  nodes: SonaNode[];
  /** Default `[]`. */
  connections?: SonaConnection[];
  performance?: SonaPerformance | null;
  [key: string]: unknown;
}

/** A value of a node param: a number, an enum or text as a string, or a bool. */
export type SonaParamValue = number | string | boolean;

/** One Sona node: its type, position, label and params (ARCHITECTURE §7). */
export interface SonaNode {
  id: string;
  type: string;
  x: number;
  y: number;
  label: string;
  params: Record<string, SonaParamValue>;
  /** Sona's breakpoint data, read by the nodes that use it (`curve`, `map`). */
  curve?: unknown;
  compact?: boolean;
  muted?: boolean;
  bypass?: boolean;
  [key: string]: unknown;
}

/** A wire between two nodes: audio when `target` is `"audio"`, otherwise a control wire to that param. */
export interface SonaConnection {
  id: string;
  from: string;
  to: string;
  target: string;
  /** Default 1. */
  amount?: number;
  [key: string]: unknown;
}

/** Sona's one-pedal macro, kept as-is and played with Sona's semantics (ARCHITECTURE §7). */
export interface SonaPerformance {
  node: string;
  parameter: string;
  curveNode?: string;
  edge?: string;
  mode: 'curve' | 'live';
  manualValue: number;
  [key: string]: unknown;
}

/** Facts a loaded voice reports about its plan (the `haa_voice_info` JSON). */
export interface VoiceInfo {
  /** Node ids in plan order. */
  nodes: string[];
  /** Published inputs in header order; the reserved inputs are not listed. */
  inputs: HaaInput[];
  mode: 'timeline' | 'live';
  duration: number;
  loop: boolean;
}

/** Outcome of loading a patch into a voice: whether it plays, and every issue found. */
export interface LoadResult {
  playable: boolean;
  issues: PatchIssue[];
}

/** One control trace packet: the last control value of every plan node, at a point in time. */
export interface TracePacket {
  /** Normalized time, 0..1 in timeline mode. */
  time: number;
  seconds: number;
  /** One value per `VoiceInfo.nodes` entry, in plan order. */
  values: Float32Array;
}

/** A label in English and French, as Sona's catalog provides. */
export interface CatalogLabel {
  en: string;
  fr: string;
}

/** Node categories of the catalog (ARCHITECTURE §11). */
export type CatalogCategory = 'source' | 'control' | 'effect' | 'output';

/** Fields every catalog param has, whatever its kind. */
export interface CatalogParamCommon {
  key: string;
  label: CatalogLabel;
  modulatable: boolean;
  smooth: boolean;
}

/** A numeric catalog param. */
export interface CatalogNumberParam extends CatalogParamCommon {
  kind: 'number';
  min: number;
  max: number;
  default: number;
  step: number;
  scale: 'lin' | 'log';
  unit: string;
}

/** An enum catalog param; `default` is one of `values`. */
export interface CatalogEnumParam extends CatalogParamCommon {
  kind: 'enum';
  values: string[];
  default: string;
}

/** A boolean catalog param. */
export interface CatalogBoolParam extends CatalogParamCommon {
  kind: 'bool';
  default: boolean;
}

/** A text catalog param. */
export interface CatalogTextParam extends CatalogParamCommon {
  kind: 'text';
  default: string;
}

/** A catalog param of any kind. */
export type CatalogParam = CatalogNumberParam | CatalogEnumParam | CatalogBoolParam | CatalogTextParam;

/** One node type of the catalog. */
export interface CatalogType {
  type: string;
  category: CatalogCategory;
  label: CatalogLabel;
  ports: {
    audioIn: boolean;
    audioOut: boolean;
    controlOut: boolean;
  };
  usesCurve: boolean;
  hidden: boolean;
  /** In declaration order, which is the interpreter's param index order. */
  params: CatalogParam[];
}

/** The node catalog JSON returned by `catalog_json()` and the `haa_catalog` export (ARCHITECTURE §11). */
export interface CatalogJson {
  format: number;
  soundRevision: number;
  /** Sorted by `type`. */
  types: CatalogType[];
}

/** Options a player passes to the AudioWorklet processor (ARCHITECTURE §9 `processorOptions`). */
export interface PlayerProcessorOptions {
  /** Compiled `haa.wasm`, instantiated synchronously in the processor. */
  module: WebAssembly.Module;
  /** UTF-8 JSON patch bytes. */
  patch: Uint8Array;
  seed: number;
  inputs: Record<string, number | string>;
  trace: boolean;
}

/** Messages the main thread posts to the player processor (ARCHITECTURE §9). */
export type PlayerInMsg =
  | { t: 'set'; name: string; value: number | string }
  | { t: 'trigger'; name: string }
  | { t: 'patch'; req: number; bytes: Uint8Array; keepState: boolean }
  | { t: 'restart' }
  | { t: 'trace'; on: boolean }
  | { t: 'dispose' };

/** Messages the player processor posts to the main thread (ARCHITECTURE §9). */
export type PlayerOutMsg =
  | { t: 'ready'; playable: boolean; issues: PatchIssue[]; info: VoiceInfo }
  | { t: 'loaded'; req: number; playable: boolean; issues: PatchIssue[]; info: VoiceInfo }
  | { t: 'trace'; time: number; seconds: number; values: Float32Array }
  | { t: 'ended' }
  | { t: 'error'; message: string };
