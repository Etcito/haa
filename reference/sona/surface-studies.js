import { TEXTURE_CURVES } from './texture-curves.js';

function buildFootstep(n, c, config) {
  console.info('[Sona] Building a surface study', config.id);
  const { id, curves, duration, frequency, resonance, density, decay, detailCutoff, impactTime, bodyLevel, detailLevel } = config;
  const nodes = [
    n(`${id}-gesture`, 'curve', 30, 370, {}, 'Footstep gesture', curves.body),
    n(`${id}-body-envelope`, 'response', 330, 50, {mode:'follow',attack:0,release:0,delay:0,shape:1,gain:1}, 'Contact pressure'),
    n(`${id}-detail-envelope`, 'response', 330, 370, {mode:'follow',...config.responses.detail}, 'Surface release'),
    n(`${id}-clusters`, 'grain', 330, 50, {wave:'noise',rate:density,attack:.0002,decay,jitter:1,variation:.85,width:.08,level:bodyLevel,delay:0}, 'Irregular contact grains'),
    n(`${id}-impact`, 'grain', 330, 370, {wave:'noise',rate:0,attack:.0001,decay:.0018,jitter:0,variation:0,width:0,level:.85,delay:impactTime}, 'Initial contact'),
    n(`${id}-body-filter`, 'filter', 630, 50, {mode:'bandpass',cutoff:frequency,resonance}, 'Surface body'),
    n(`${id}-body-gain`, 'gain', 930, 50, {gain:0}, 'Pressure envelope'),
    n(`${id}-detail`, 'grain', 330, 680, {wave:'noise',rate:config.detailDensity||210,attack:.0001,decay:.0012,jitter:1,variation:.9,width:.2,level:detailLevel,delay:0}, 'Fine crackle'),
    n(`${id}-detail-filter`, 'filter', 630, 370, {mode:'bandpass',cutoff:detailCutoff,resonance:.8}, 'Friction band'),
    n(`${id}-detail-gain`, 'gain', 930, 370, {gain:0}, 'Release envelope'),
    n(`${id}-mix`, 'gain', 1230, 50, {gain:config.mixGain||1.1}, 'Surface mix'),
    n(`${id}-rolloff`, 'filter', 1230, 370, {mode:'lowpass',cutoff:config.rolloff||9500,resonance:.65}, 'Air absorption'),
    n(`${id}-space`, 'space', 1530, 50, {size:.08,damping:.8,mix:.025,width:.22,distance:.015}, 'Close perspective'),
    n(`${id}-output`, 'output', 1530, 370, {gain:.86}),
  ];
  const edges = [
    c(`${id}-gesture-pressure`,`${id}-gesture`,`${id}-body-envelope`,'input'),
    c(`${id}-gesture-release`,`${id}-gesture`,`${id}-detail-envelope`,'input'),
    c(`${id}-body-shape`,`${id}-body-envelope`,`${id}-body-gain`,'gain',1.7),
    c(`${id}-detail-shape`,`${id}-detail-envelope`,`${id}-detail-gain`,'gain',1.5),
    c(`${id}-cluster-body`,`${id}-clusters`,`${id}-body-filter`),
    c(`${id}-impact-body`,`${id}-impact`,`${id}-body-filter`,'audio',config.impactGain||1),
    c(`${id}-body-control`,`${id}-body-filter`,`${id}-body-gain`),
    c(`${id}-crackle-colour`,`${id}-detail`,`${id}-detail-filter`),
    c(`${id}-detail-control`,`${id}-detail-filter`,`${id}-detail-gain`),
    c(`${id}-body-mix`,`${id}-body-gain`,`${id}-mix`),
    c(`${id}-detail-mix`,`${id}-detail-gain`,`${id}-mix`),
    c(`${id}-air`,`${id}-mix`,`${id}-rolloff`),
    c(`${id}-perspective`,`${id}-rolloff`,`${id}-space`),
    c(`${id}-listen`,`${id}-space`,`${id}-output`,'audio',config.listenGain||1),
  ];
  if (curves.bass) {
    nodes.push(n(`${id}-bass-envelope`,'response',330,680,{mode:'follow',...config.responses.bass},'Weight transfer'));
    nodes.push(n(`${id}-bass`,'noise',630,680,{color:'brown',level:config.bassLevel||.15},'Low contact texture'));
    nodes.push(n(`${id}-bass-filter`,'filter',930,680,{mode:'bandpass',cutoff:config.bassFrequency||180,resonance:.9},'Weight band'));
    nodes.push(n(`${id}-bass-gain`,'gain',1230,680,{gain:0},'Weight envelope'));
    edges.push(c(`${id}-gesture-weight`,`${id}-gesture`,`${id}-bass-envelope`,'input'));
    edges.push(c(`${id}-bass-shape`,`${id}-bass-envelope`,`${id}-bass-gain`,'gain',.9));
    edges.push(c(`${id}-bass-colour`,`${id}-bass`,`${id}-bass-filter`));
    edges.push(c(`${id}-bass-control`,`${id}-bass-filter`,`${id}-bass-gain`));
    edges.push(c(`${id}-bass-mix`,`${id}-bass-gain`,`${id}-mix`));
  }
  if (config.modes) for (let index=0;index<config.modes.length;index++) {
    const mode=config.modes[index], source=`${id}-mode-${index}`;
    nodes.push(n(source,'grain',630+index*300,990,{wave:'sine',frequency:mode[0],rate:0,attack:.001,decay:mode[1],spread:.15,jitter:0,variation:.08,glide:-.035,width:0,level:mode[2],delay:impactTime},`Damped mode ${index+1}`));
    edges.push(c(`${source}-mix`,source,`${id}-mix`));
  }
  for (const node of nodes) if (node.type !== 'curve' && node.type !== 'response') node.x += 300;
  return {id,name:config.name,subtitle:config.subtitle,description:config.description,icon:'noise',color:config.color,
    graph:{duration,loop:true,nodes,connections:edges}};
}

export function buildSurfaceStudies(n,c) {
  console.info('[Sona] Assembling three editable footstep studies');
  return [
    buildFootstep(n,c,{id:'snow',name:'Snow · step 2',subtitle:'Crush · compact · release',color:'#bddbe4',duration:1.1,curves:TEXTURE_CURVES.snow,
      description:'One gesture drives contact pressure, surface release and weight. Shape each response with its own timing and curvature; the noise grains and resonances remain editable.',
      responses:{detail:{attack:0,release:0,delay:0,shape:.70,gain:.96},bass:{attack:.0005,release:.0009,delay:0,shape:.93,gain:.94}},
      frequency:760,resonance:2.1,density:520,decay:.0035,detailCutoff:2900,bodyLevel:.72,detailLevel:.18,impactTime:.157,impactGain:1.6,bassLevel:.08,bassFrequency:240,mixGain:1.4,listenGain:3}),
    buildFootstep(n,c,{id:'footsteps-st2',name:'Footstep · ST2',subtitle:'Impact · body · scuff',color:'#dfc798',duration:.9,curves:TEXTURE_CURVES.st2,
      description:'One gesture drives the pressure, weight and delayed surface scuff. Adjust the response timing and curvature to change the contact without redrawing three envelopes.',
      responses:{detail:{attack:.0035,release:0,delay:.0667,shape:.615,gain:1.118},bass:{attack:.0023,release:0,delay:.0002,shape:.782,gain:1.422}},
      frequency:320,resonance:1.5,density:620,decay:.004,detailCutoff:3800,bodyLevel:.82,detailLevel:.24,impactTime:.042,impactGain:1.1,bassLevel:.19,bassFrequency:130,mixGain:1.4,listenGain:4,
      modes:[[258,.015,.055],[350,.009,.028],[445,.006,.016]]}),
    buildFootstep(n,c,{id:'gravel',name:'Dirt & gravel',subtitle:'Weight · friction · small stones',color:'#c9b394',duration:6,curves:TEXTURE_CURVES.gravel,
      description:'One walking gesture controls the body and fine friction through separate responses. The two noise-grain layers stay editable; background wind is excluded.',
      responses:{detail:{attack:0,release:.0014,delay:0,shape:.882,gain:.748}},
      frequency:1180,resonance:1.05,density:760,decay:.0026,detailCutoff:3600,bodyLevel:.6,detailLevel:.23,impactTime:.37,impactGain:.25,detailDensity:410,mixGain:1.7,rolloff:6200,listenGain:1.5}),
  ];
}

export function buildWindStudy(n,c) {
  console.info('[Sona] Building an editable wind texture');
  return {id:'wind',name:'Soft wind',subtitle:'Gusts · air · buffeting',icon:'wind',color:'#abc5c7',
    description:'Slow gust envelopes drive two filtered noise layers. Low pressure movement and the higher airflow band evolve separately, without a tonal oscillator.',
    graph:{duration:8,loop:true,nodes:[
      n('wind-pressure','curve',30,50,{},'Gust pressure',TEXTURE_CURVES.wind.body),
      n('wind-air-envelope','curve',30,370,{},'Air movement',TEXTURE_CURVES.wind.detail),
      n('wind-random','random',30,680,{rate:1.7,depth:1,smooth:.95},'Turbulent drift'),
      n('wind-low','noise',330,50,{color:'brown',level:.78},'Pressure noise'),
      n('wind-low-filter','filter',630,50,{mode:'lowpass',cutoff:34,resonance:.65},'Low pressure band'),
      n('wind-low-gain','gain',930,50,{gain:.035},'Gust strength'),
      n('wind-air','grain',330,370,{wave:'noise',rate:340,attack:.008,decay:.085,jitter:1,variation:.6,width:.9,level:.68,delay:0},'Diffuse airflow'),
      n('wind-air-filter','filter',630,370,{mode:'bandpass',cutoff:960,resonance:4.1},'Airflow band'),
      n('wind-air-gain','gain',930,370,{gain:.015},'Air strength'),
      n('wind-mix','gain',1230,50,{gain:1.1},'Wind layers'),
      n('wind-output','output',1230,370,{gain:.8}),
    ],connections:[
      c('wind-pressure-level','wind-pressure','wind-low-gain','gain',1.65),
      c('wind-air-level','wind-air-envelope','wind-air-gain','gain',.42),
      c('wind-moving-band','wind-random','wind-air-filter','cutoff',65),
      c('wind-low-motion','wind-random','wind-low-filter','cutoff',6),
      c('wind-low-colour','wind-low','wind-low-filter'),
      c('wind-low-envelope','wind-low-filter','wind-low-gain'),
      c('wind-low-mix','wind-low-gain','wind-mix'),
      c('wind-air-colour','wind-air','wind-air-filter'),
      c('wind-air-shape','wind-air-filter','wind-air-gain'),
      c('wind-air-mix','wind-air-gain','wind-mix'),
      c('wind-listen','wind-mix','wind-output','audio',2),
    ]}};
}
