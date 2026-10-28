import { buildEnginePreset } from './engine-preset.js';
import { buildEngineStartPreset } from './engine-start-preset.js';
import { buildLightRainPreset } from './light-rain-preset.js';
import { buildVoicePreset } from './voice-preset.js';
import { buildWaterStudies } from './water-studies.js';
import { buildSurfaceStudies, buildWindStudy } from './surface-studies.js';

/** Each example is an editable score built entirely from reusable primitives. */
export function buildPresets(node, connection) {
  console.info('[Sona] Building reusable sound studies');
  const presets = [
    buildEnginePreset(node, connection),
    buildEngineStartPreset(node, connection),
    buildVoicePreset(node, connection),
    ...buildWaterStudies(node, connection),
    buildLightRainPreset(node, connection),
    ...buildSurfaceStudies(node, connection),
    buildWindStudy(node, connection),
  ];
  const revisions = {'v12':7, 'engine-start':7, 'light-rain':8, 'snow':5, 'footsteps-st2':5, 'gravel':5};
  return presets.map(preset => ({...preset, revision:revisions[preset.id] ?? 4}));
}
