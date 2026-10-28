/** A starter, compression and twelve pressure releases share one moving shaft. */
export function buildEngineStartPreset(n, c) {
  console.info('[Sona] Building a mechanically coupled engine start from reusable modules');
  const duration = 4.2;
  const seconds = points => points.map(([time, value]) => [time / duration, value]);
  const key = seconds([[0,0],[.38,0],[.392,1],[.43,.82],[.58,.76],[.76,.88],[.9,.96],[.965,.87],[1.015,0],[duration,0]]);
  const pressure = seconds([[0,0],[.855,0],[.878,.14],[.902,.06],[.928,.29],[.946,.17],[.976,.56],[1.01,.43],[1.09,.82],[1.22,.96],[1.48,1],[1.72,.84],[1.96,.48],[2.3,.28],[2.75,.19],[3.2,.19],[3.55,.182],[duration,.182]]);
  // The authored shaft speed is shared by compression, gear mesh, firing and
  // valve contact. In particular, combustion no longer jumps from a stationary
  // clock to an unrelated tone when the key is released.
  const speed = seconds([[0,0],[.38,0],[.405,.10],[.50,.1167],[.66,.125],[.79,.1361],[.86,.15],[.915,.2389],[.98,.3833],[1.09,.5833],[1.28,.80],[1.50,.8389],[1.68,.8056],[1.94,.6778],[2.25,.5667],[2.6,.5139],[3.05,.4806],[3.55,.4722],[duration,.4722]]);
  const nodes = [
    n('start-key','curve',40,40,{},'Starter key',key),
    n('start-pressure','curve',40,370,{},'First firing and pressure',pressure),
    n('start-speed','curve',40,700,{},'Shaft run-up and settling',speed),
    n('start-engage','response',340,40,{mode:'rise',input:0,attack:.00025,release:.010,delay:0,shape:1,gain:1,sensitivity:.045},'Starter engagement'),
    n('start-disengage','response',340,370,{mode:'fall',input:0,attack:.0003,release:.018,delay:0,shape:1,gain:1,sensitivity:.04},'Starter disengagement'),
    n('start-bank-rate','math',340,700,{a:0,b:15,operation:'multiply'},'Shaft speed / 120'),
    n('start-release-width','math',340,1030,{a:0,b:.00175,operation:'multiply'},'Pressure duration · milliseconds'),
    n('start-compression-motion','lfo',640,40,{rate:.05,depth:1,wave:'sine'},'Compression drag on starter'),
    n('start-gear-mesh','pulse',640,700,{frequency:480,width:.13,phase:.1,jitter:.0015,shimmer:.055,level:.18,shape:'exponential'},'Starter gear mesh'),
    n('start-friction','noise',640,1030,{color:'white',level:.4},'Continuous starter friction'),
    n('start-compression','pulse',640,1360,{frequency:0,width:.16,phase:.21,jitter:.025,shimmer:.24,level:.27,shape:'rounded'},'Compression strokes'),
    n('start-contact','noise',640,370,{color:'white',level:0},'Engagement and release contact'),
    n('start-mechanism-band','filter',940,40,{cutoff:1350,resonance:.75,mode:'bandpass'},'Starter mechanism colour'),
    n('start-mechanism-gate','gain',940,370,{gain:0},'Starter key envelope'),
    n('start-header-a','comb',2440,700,{time:.00102,feedback:-.28,damping:.35,mix:.78},'Bank A · short reflection'),
    n('start-header-b','comb',2440,1360,{time:.00108,feedback:-.30,damping:.38,mix:.78},'Bank B · short reflection'),
    n('start-pipe-a','comb',2740,700,{time:.00275,feedback:-.47,damping:.51,mix:.84},'Bank A · open-end reflection'),
    n('start-pipe-b','comb',2740,1360,{time:.00290,feedback:-.50,damping:.55,mix:.84},'Bank B · open-end reflection'),
    n('start-clatter','grain',940,700,{wave:'noise',frequency:2400,rate:0,attack:.0001,decay:.0018,spread:0,jitter:.24,variation:.72,glide:0,delay:.86,width:.12,level:0},'Valve and mechanism contact'),
    n('start-air','noise',940,1030,{color:'white',level:0},'Gas-flow texture'),
    n('start-air-band','filter',1240,370,{cutoff:1150,resonance:.65,mode:'highpass'},'Running airflow'),
    n('start-body','filter',2740,1030,{cutoff:380,resonance:.7,mode:'lowpass'},'Low pressure body'),
    n('start-eq','equalizer',3040,40,{low:3,mid:0,high:1.5,frequency:780},'Start body and mechanical detail'),
    n('start-pressure-gain','gain',3040,370,{gain:.9},'Firing pressure build-up'),
    n('start-dynamics','compressor',3040,700,{threshold:-7,ratio:3,attack:.0015,release:.065,makeup:1.5},'Catch transient balance'),
    n('start-rolloff','filter',3040,1030,{cutoff:5800,resonance:.65,mode:'lowpass'},'Close absorption'),
    n('start-space','space',3340,1030,{size:.14,damping:.68,mix:.04,width:.42,distance:.04},'Close reflections'),
    n('start-output','output',3340,1360,{gain:.68},'Engine start output'),
  ];
  const connections = [
    c('start-key-engage','start-key','start-engage','input',1),
    c('start-key-release','start-key','start-disengage','input',1),
    c('start-rpm-clock','start-speed','start-bank-rate','a',1),
    c('start-pressure-width','start-bank-rate','start-release-width','a',1),
    c('start-compression-clock','start-bank-rate','start-compression-motion','rate',6),
    c('start-gear-speed','start-bank-rate','start-gear-mesh','frequency',105),
    c('start-gear-droop','start-compression-motion','start-gear-mesh','frequency',-90),
    c('start-friction-load','start-compression-motion','start-friction','level',.23),
    c('start-compression-strokes','start-bank-rate','start-compression','frequency',6),
    c('start-contact-engage','start-engage','start-contact','level',.82),
    c('start-contact-release','start-disengage','start-contact','level',.24),
    c('start-gear-colour','start-gear-mesh','start-mechanism-band'),
    c('start-friction-colour','start-friction','start-mechanism-band','audio',.82),
    c('start-mechanism-envelope','start-mechanism-band','start-mechanism-gate','audio',1.5),
    c('start-compression-envelope','start-compression','start-mechanism-gate','audio',.55),
    c('start-key-gate','start-key','start-mechanism-gate','gain',1.65),
    c('start-cranking-mix','start-mechanism-gate','start-eq'),
    c('start-contact-detail','start-contact','start-eq','audio',.72),
    c('start-header-tail-a','start-header-a','start-pipe-a'),
    c('start-header-tail-b','start-header-b','start-pipe-b'),
    c('start-pressure-mix-a','start-pipe-a','start-eq','audio',.65),
    c('start-pressure-mix-b','start-pipe-b','start-eq','audio',.62),
    c('start-pressure-body-a','start-pipe-a','start-body'),
    c('start-pressure-body-b','start-pipe-b','start-body','audio',.96),
    c('start-contact-weight','start-contact','start-body','audio',2.8),
    c('start-pressure-body-mix','start-body','start-eq','audio',2.05),
    c('start-running-contact-clock','start-bank-rate','start-clatter','rate',4),
    c('start-running-contact-pressure','start-pressure','start-clatter','level',.52),
    c('start-running-contact-mix','start-clatter','start-eq','audio',.65),
    c('start-air-pressure','start-pressure','start-air','level',.028),
    c('start-air-colour','start-air','start-air-band'),
    c('start-air-mix','start-air-band','start-eq','audio',.8),
    c('start-pressure-build-up','start-pressure','start-pressure-gain','gain',1.1),
    c('start-pressure-level','start-eq','start-pressure-gain'),
    c('start-catch-dynamics','start-pressure-gain','start-dynamics'),
    c('start-colour-absorption','start-dynamics','start-rolloff'),
    c('start-acoustic-space','start-rolloff','start-space'),
    c('start-listen','start-space','start-output'),
  ];
  const strengths = [.84,1.10,.68,1.22,.95,1.04,1.16,.73,1.08,.86,1.19,.79];
  const runnerSeconds = [.00020,.00027,.00038,.00024,.00044,.00032];
  for (let event = 0; event < 12; event++) {
    const bank = event % 2, cylinder = Math.floor(event / 2), id = `start-cylinder-${event + 1}`;
    const column = bank * 2 + Math.floor(cylinder / 3), row = cylinder % 3;
    const strength = strengths[event];
    nodes.push(n(id,'pulse',1240 + column * 300,700 + row * 330,
      {frequency:0,width:.002,phase:.07 + event/12,jitter:.0038 + (cylinder%3)*.0007,shimmer:.12 + (cylinder%2)*.025,level:0,shape:'exponential'},
      `Bank ${bank?'B':'A'} · pressure ${cylinder+1}`));
    connections.push(
      c(`${id}-clock`,'start-bank-rate',id,'frequency',1),
      c(`${id}-duration`,'start-release-width',id,'width',1 + (cylinder%3-1)*.08),
      c(`${id}-runner`,'start-bank-rate',id,'phase',-runnerSeconds[cylinder]),
      c(`${id}-pressure`,'start-pressure',id,'level',.85*strength),
      c(`${id}-catch-scatter`,'start-key',id,'jitter',.018),
      c(`${id}-catch-variation`,'start-key',id,'shimmer',.22),
      c(`${id}-audio`,id,bank?'start-header-b':'start-header-a'),
    );
  }
  return {
    id:'engine-start',name:'Engine start',subtitle:'Engage · crank · catch · idle',icon:'engine',color:'#f3b77e',
    description:'The key engages a continuous starter mechanism. Compression modulates its pitch and friction; twelve uneven pressure releases then catch, accelerate and settle into idle. The same shaft gesture drives cranking, combustion and mechanical contact. Every source and connection remains editable.',
    graph:{duration,loop:false,nodes,connections},
  };
}
