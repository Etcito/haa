// Static arrangement renderer (ARCHITECTURE §8, S17), exported as `@etcito/haa/arranger`.
// Realm-neutral: no DOM, no Node builtins. It ports the timing semantics of the v0.1 arranger
// (legacy/src/music/arranger.ts) onto static arrangements: beats, bars and equal-power stereo.
// Its generative state machine is not ported.
// Rules:
// - each instrument patch (an object or a JSON text) is parsed once;
// - notes are enumerated in flat order (order position, then track, then note), so the
//   noteIndex seeds `variantSeed(a.id, noteIndex)`;
// - a note starts at `beat · 60/bpm` within its section and renders for
//   `len · 60/bpm + releaseS`;
// - note-off: past `len · 60/bpm` the note's render fades linearly to 0 over `releaseS`, so a
//   sustaining live-mode instrument ends without a click;
// - gain and equal-power pan apply per channel: L·cos((pan+1)π/4), R·sin((pan+1)π/4);
// - `loop: true` adds every sample past the end at index `(i - total)`; `loop: false`
//   truncates at the end (authors leave a closing rest bar);
// - the output length is the sum of `bars · beatsPerBar · 60/bpm` over `order`.

import { OFFLINE_RATE, renderPatch } from './offline.ts';
import type { HaaPatch, PatchIssue, StereoRender } from './offline.ts';
import { variantSeed } from './bank.ts';

/** One loaded instrument: its patch, the published inputs it reads and its release. */
export interface Instrument {
  id: string;
  patch: HaaPatch | string;
  noteInput: string;
  velocityInput?: string;
  releaseS: number;
}

/** One static note: its beat inside the section, length in beats, MIDI note and velocity. */
export interface NoteEvent {
  beat: number;
  len: number;
  note: number;
  vel: number;
}

/** One section track: its instrument id, mix and notes. */
export interface Track {
  instrument: string;
  gain: number;
  pan: number;
  notes: NoteEvent[];
}

/** One arrangement section: a number of bars and its tracks. */
export interface Section {
  id: string;
  bars: number;
  tracks: Track[];
}

/** A `haa.arrangement` file (S17). The patch strings resolve against the page base. */
export interface Arrangement {
  format: 'haa.arrangement';
  version: 0;
  id: string;
  bpm: number;
  beatsPerBar: number;
  loop: boolean;
  seed: number;
  instruments: {
    id: string;
    patch: string;
    noteInput: string;
    velocityInput?: string;
    releaseS: number;
  }[];
  sections: Section[];
  order: string[];
}

/** The render rate rule of `renderPatch` (8000..192000, fallback `OFFLINE_RATE`). */
function renderRate(sampleRate: number | undefined): number {
  if (sampleRate === undefined || !Number.isFinite(sampleRate) || sampleRate === 0) return OFFLINE_RATE;
  return Math.min(192_000, Math.max(8_000, sampleRate));
}

/** One readable message per problem, in a stable order (S17, HAA §15 style). */
export function validateArrangement(a: Arrangement, instruments: readonly Instrument[]): string[] {
  if (a === null || typeof a !== 'object') return ['arrangement must be an object'];
  const messages: string[] = [];
  if (a.format !== 'haa.arrangement') messages.push('format must be "haa.arrangement"');
  if (a.version !== 0) messages.push('version must be 0');
  if (!Number.isFinite(a.bpm) || a.bpm <= 0) messages.push('bpm must be a finite number greater than 0');
  if (!Number.isInteger(a.beatsPerBar) || a.beatsPerBar < 1) {
    messages.push('beatsPerBar must be an integer of at least 1');
  }

  const sections = Array.isArray(a.sections) ? a.sections : [];
  const order = Array.isArray(a.order) ? a.order : [];
  const declared = Array.isArray(a.instruments) ? a.instruments : [];
  const supplied = new Map(instruments.map((instrument) => [instrument.id, instrument]));

  const byId = new Map<string, Section>();
  for (const section of sections) {
    if (byId.has(section.id)) messages.push(`duplicate section "${section.id}"`);
    else byId.set(section.id, section);
  }
  for (const id of order) {
    if (!byId.has(id)) messages.push(`order names no section "${id}"`);
  }

  const declaredIds = new Set<string>();
  const badRelease = new Set<string>();
  for (const instrument of declared) {
    declaredIds.add(instrument.id);
    if (!(instrument.releaseS >= 0)) badRelease.add(instrument.id);
  }
  for (const instrument of instruments) {
    if (!(instrument.releaseS >= 0)) badRelease.add(instrument.id);
  }
  for (const id of badRelease) messages.push(`instrument "${id}": releaseS must be at least 0`);

  const beatsPerBar = a.beatsPerBar;
  for (const section of sections) {
    const beats = section.bars * beatsPerBar;
    const tracks = Array.isArray(section.tracks) ? section.tracks : [];
    for (let t = 0; t < tracks.length; t++) {
      const track = tracks[t];
      const where = `section "${section.id}" track ${t}`;
      if (!declaredIds.has(track.instrument) || !supplied.has(track.instrument)) {
        messages.push(`${where}: unknown instrument "${track.instrument}"`);
      }
      if (!(track.pan >= -1 && track.pan <= 1)) messages.push(`${where}: pan ${track.pan} is outside -1..1`);
      if (!(track.gain >= 0)) messages.push(`${where}: gain must be at least 0`);
      const notes = Array.isArray(track.notes) ? track.notes : [];
      for (let n = 0; n < notes.length; n++) {
        const note = notes[n];
        const at = `${where} note ${n}`;
        if (!(note.note >= 0 && note.note <= 127)) messages.push(`${at}: note ${note.note} is outside 0..127`);
        if (!(note.vel >= 0 && note.vel <= 1)) messages.push(`${at}: vel ${note.vel} is outside 0..1`);
        if (!(note.len > 0)) messages.push(`${at}: len must be greater than 0`);
        if (!(note.beat >= 0 && note.beat < beats)) messages.push(`${at}: beat ${note.beat} is outside [0, ${beats})`);
      }
    }
  }

  return messages;
}

/**
 * Render a static arrangement to a stereo buffer. Every note renders its instrument patch with
 * the note's inputs; the mix applies the track gain and equal-power pan. The same arrangement and
 * instruments always give the same samples. Throws `[haa] arrangement <id>: <first message>` when
 * the arrangement is invalid.
 */
export function renderArrangement(
  module: WebAssembly.Module,
  a: Arrangement,
  instruments: readonly Instrument[],
  o: { sampleRate?: number } = {},
): StereoRender {
  const messages = validateArrangement(a, instruments);
  if (messages.length > 0) throw new Error(`[haa] arrangement ${a.id}: ${messages[0]}`);

  const rate = renderRate(o.sampleRate);
  const supplied = new Map(instruments.map((instrument) => [instrument.id, instrument]));
  const patches = new Map<string, HaaPatch>();
  for (const instrument of instruments) {
    const parsed = typeof instrument.patch === 'string' ? (JSON.parse(instrument.patch) as HaaPatch) : instrument.patch;
    patches.set(instrument.id, parsed);
  }

  const byId = new Map(a.sections.map((section) => [section.id, section]));
  const secondsPerBeat = 60 / a.bpm;
  let totalSeconds = 0;
  for (const id of a.order) {
    const section = byId.get(id);
    if (section) totalSeconds += section.bars * a.beatsPerBar * secondsPerBeat;
  }

  const total = Math.round(totalSeconds * rate);
  const left = new Float32Array(total);
  const right = new Float32Array(total);
  const issues: PatchIssue[] = [];
  const seen = new Set<string>();
  let noteIndex = 0;
  let cursor = 0;

  for (const id of a.order) {
    const section = byId.get(id);
    if (!section) continue;
    for (const track of section.tracks) {
      const instrument = supplied.get(track.instrument);
      const patch = instrument ? patches.get(instrument.id) : undefined;
      if (!instrument || !patch) continue;
      const angle = ((track.pan + 1) * Math.PI) / 4;
      const gainL = track.gain * Math.cos(angle);
      const gainR = track.gain * Math.sin(angle);
      for (const note of track.notes) {
        const seed = variantSeed(a.id, noteIndex);
        noteIndex++;
        const noteSeconds = note.len * secondsPerBeat;
        const inputs: Record<string, number> = { [instrument.noteInput]: note.note };
        if (instrument.velocityInput !== undefined) inputs[instrument.velocityInput] = note.vel;
        const render = renderPatch(module, patch, {
          sampleRate: rate,
          seconds: noteSeconds + instrument.releaseS,
          seed,
          inputs,
        });
        for (const issue of render.issues) {
          const key = `${issue.severity}|${issue.code}|${issue.path}|${issue.node ?? ''}|${issue.message}`;
          if (seen.has(key)) continue;
          seen.add(key);
          issues.push(issue);
        }

        const fadeFrames = instrument.releaseS * rate;
        const offFrame = noteSeconds * rate;
        const startFrame = Math.round((cursor + note.beat * secondsPerBeat) * rate);
        for (let i = 0; i < render.left.length; i++) {
          let frame = startFrame + i;
          if (frame >= total) {
            if (!a.loop || total <= 0) break;
            frame %= total;
          }
          let envelope = 1;
          if (fadeFrames > 0 && i >= offFrame) envelope = Math.max(0, 1 - (i - offFrame) / fadeFrames);
          if (envelope === 0) continue;
          left[frame] += render.left[i] * gainL * envelope;
          right[frame] += render.right[i] * gainR * envelope;
        }
      }
    }
    cursor += section.bars * a.beatsPerBar * secondsPerBeat;
  }

  return { left, right, sampleRate: rate, issues };
}
