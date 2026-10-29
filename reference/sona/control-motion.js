/** Generic inertial control movement with optional threshold-reset stages. */
export const MOTION_PARAMS = {
  input: [0, 1, 0], acceleration: [0, 4, .36], resistance: [0, 8, .07],
  return: [0, 12, 1.7], ceiling: [.05, 1, .98], reset: [0, .95, .64],
  stages: [0, 12, 0], progression: [.05, 1, .76], hold: [0, 2, .14],
  inertia: [0, 2, .065], rest: [0, .25, .025],
};
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

export function createControlMotionState(sampleRate = 48000) {
  console.info('[Sona] Creating reusable inertial control motion');
  const sr = clamp(Number(sampleRate) || 48000, 8000, 192000);
  return {sampleRate:sr,dt:1/sr,position:0,value:0,stage:0,holdRemaining:0,
    lastFrame:-1,inertia:-1,inertiaCoefficient:1,progression:-1,stageScale:1};
}

// Called in the sample loop; the monotonic frame clock survives timeline loops.
function resetMotion(state) {
  state.position=0;state.value=0;state.stage=0;state.holdRemaining=0;
  state.stageScale=1;state.progression=-1;
}

/**
 * Drive is integrated into normalized movement. Resistance and release drag
 * remove momentum. Optional threshold resets create a finite series of cycles;
 * each cycle can accelerate more slowly. This module knows no audio sources,
 * engine units, cylinders, gear ratios or vehicle-specific rules.
 */
export function controlMotionSample(state, params, timeSeconds, frameIndex) {
  if (state.lastFrame>=0 && frameIndex<state.lastFrame) resetMotion(state);
  state.lastFrame=frameIndex;
  const drive=clamp(Number.isFinite(params.input)?params.input:0,0,1);
  const acceleration=clamp(Number.isFinite(params.acceleration)?params.acceleration:.36,0,4);
  const resistance=clamp(Number.isFinite(params.resistance)?params.resistance:.07,0,8);
  const release=clamp(Number.isFinite(params.return)?params.return:1.7,0,12);
  const ceiling=clamp(Number.isFinite(params.ceiling)?params.ceiling:.98,.05,1);
  const reset=clamp(Number.isFinite(params.reset)?params.reset:.64,0,.95);
  const stages=Math.round(clamp(Number.isFinite(params.stages)?params.stages:0,0,12));
  const progression=clamp(Number.isFinite(params.progression)?params.progression:.76,.05,1);
  const hold=clamp(Number.isFinite(params.hold)?params.hold:.14,0,2);
  const inertia=clamp(Number.isFinite(params.inertia)?params.inertia:.065,0,2);
  const rest=clamp(Number.isFinite(params.rest)?params.rest:.025,0,.25);
  if(progression!==state.progression){state.progression=progression;state.stageScale=Math.pow(progression,state.stage);}
  if(inertia!==state.inertia){state.inertia=inertia;state.inertiaCoefficient=inertia===0?1:-Math.expm1(-state.dt/inertia);}

  const effort=state.holdRemaining>0?0:drive*acceleration*state.stageScale;
  state.holdRemaining=Math.max(0,state.holdRemaining-state.dt);
  state.position=Math.max(0,state.position+(effort-(resistance+release*(1-drive))*state.position)*state.dt);
  if(state.position>=ceiling){
    if(state.stage<stages && drive>.02){
      state.position=ceiling*reset;state.stage++;state.stageScale=Math.pow(progression,state.stage);state.holdRemaining=hold;
    } else state.position=ceiling;
  }
  if(drive<.005 && state.position<=rest){
    state.position=0;state.stage=0;state.stageScale=1;state.holdRemaining=0;
  }
  state.value+= (state.position-state.value)*state.inertiaCoefficient;
  return clamp(state.value,0,1);
}
