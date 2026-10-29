/** Twelve editable pressure releases, load-dependent breath and mechanical contact. */
export function buildEnginePreset(n, c) {
  console.info('[Sona] Building a single-pedal engine with speed-dependent effort and pressure');
  const duration = 11;
  // Only this gesture is authored. A generic inertial motion, its positive
  // slope and a pedal × movement product estimate effort. This is a compact
  // sound-control model, not an exact vehicle or transmission simulation.
  const pedal = [[0,0],[.10,0],[.16,.82],[.27,.9],[.31,.08],[.36,.94],[.48,1],[.53,.1],[.58,.96],[.70,1],[.76,.12],[.82,0],[1,0]];
  const nodes = [
    n('pedal-curve','curve',40,50,{},'Pedal gesture',pedal),
    n('engine-pedal','response',40,370,{input:0,mode:'follow',attack:.018,release:.03,delay:0,shape:1,gain:1},'Pedal'),
    n('engine-motion','motion',40,690,{input:0,acceleration:.36,resistance:.07,return:1.7,ceiling:.98,reset:.64,stages:0,progression:.76,hold:.14,inertia:.065,rest:.025},'Inferred rotational movement'),
    n('engine-acceleration','response',40,1010,{input:0,mode:'rise',attack:.035,release:.10,delay:0,shape:1,gain:1,sensitivity:1},'Acceleration effort'),
    n('engine-load','response',340,1010,{input:0,mode:'follow',attack:.05,release:.08,delay:0,shape:1,gain:1},'Pressure from inferred effort'),
    n('engine-effort','math',340,1330,{a:0,b:.52,operation:'multiply'},'Pedal × moving resistance'),
    n('engine-release-response','response',40,1330,{input:0,mode:'fall',attack:.001,release:.18,delay:0,shape:1,gain:1.1,sensitivity:.12},'Release from pedal motion'),
    n('engine-bank-rate','math',340,50,{a:850,b:1/120,operation:'multiply'},'RPM / 120 · cylinder cycle'),
    n('engine-shaft-rate','math',340,370,{a:0,b:2,operation:'multiply'},'Cycle × 2 · crankshaft'),
    n('engine-release-width','math',340,690,{a:0,b:.0021,operation:'multiply'},'Pressure duration · softer idle, sharper load'),
    n('engine-header-a','comb',1880,50,{time:.00102,feedback:-.28,damping:.35,mix:.78},'Bank A · short reflection'),
    n('engine-header-b','comb',1880,690,{time:.00108,feedback:-.30,damping:.38,mix:.78},'Bank B · short reflection'),
    n('engine-pipe-a','comb',2180,50,{time:.00275,feedback:-.47,damping:.51,mix:.84},'Bank A · open-end reflection'),
    n('engine-pipe-b','comb',2180,690,{time:.00290,feedback:-.50,damping:.55,mix:.84},'Bank B · open-end reflection'),
    n('engine-shaft-pulse','pulse',680,1010,{frequency:0,width:.065,phase:.23,jitter:.012,shimmer:.22,level:.29,shape:'exponential'},'Mechanical order · crankshaft'),
    n('engine-mechanics','filter',980,1010,{cutoff:260,resonance:.7,mode:'bandpass'},'Engine block weight'),
    n('engine-clatter','grain',1280,1010,{wave:'noise',frequency:2400,rate:0,attack:.0001,decay:.0011,spread:0,jitter:.32,variation:.72,glide:0,delay:0,width:.18,level:.18},'Mechanical ticks · short grains'),
    n('engine-clatter-filter','filter',1580,1010,{cutoff:2200,resonance:.8,mode:'bandpass'},'Metal contact band'),
    n('engine-air','noise',1880,1010,{color:'white',level:.004},'Turbulent gas flow'),
    n('engine-air-filter','filter',2180,1010,{cutoff:1200,resonance:.65,mode:'highpass'},'Intake and exhaust air'),
    n('engine-overrun','grain',680,1330,{wave:'noise',frequency:500,rate:8,attack:.0001,decay:.005,spread:0,jitter:1,variation:.85,glide:0,delay:0,width:.3,level:0},'Lift-off micro-impacts'),
    n('engine-overrun-filter','filter',980,1330,{cutoff:1100,resonance:.7,mode:'lowpass'},'Damped release crackles'),
    n('engine-intake','filter',2180,1330,{cutoff:1500,resonance:.65,mode:'bandpass'},'Induction pressure · throttle opening'),
    n('engine-eq','equalizer',2480,50,{low:1.5,mid:0,high:1.5,frequency:780},'Body and mechanical detail'),
    n('engine-rolloff','filter',2480,370,{cutoff:7600,resonance:.6,mode:'lowpass'},'High-frequency absorption'),
    n('engine-compressor','compressor',2480,690,{threshold:-18,ratio:1.8,attack:.012,release:.065,makeup:4.5},'Pressure and transient balance'),
    n('engine-space','space',2780,690,{size:.14,damping:.68,mix:.06,width:.58,distance:.04},'Close reflections'),
    n('engine-output','output',2780,1010,{gain:.82}),
  ];
  const connections = [
    c('pedal-automation','pedal-curve','engine-pedal','input',1),
    c('engine-drive','engine-pedal','engine-motion','input',1),
    c('engine-sense-acceleration','engine-motion','engine-acceleration','input',1),
    c('engine-demand','engine-pedal','engine-effort','a',1),
    c('engine-moving-resistance','engine-motion','engine-effort','b',.44),
    c('engine-pressure-demand','engine-effort','engine-load','input',1),
    c('engine-pressure-inertia','engine-acceleration','engine-load','input',.55),
    c('engine-release-detect','engine-pedal','engine-release-response','input',1),
    c('engine-rpm','engine-motion','engine-bank-rate','a',7900),
    c('engine-shaft-conversion','engine-bank-rate','engine-shaft-rate','a',1),
    c('engine-release-duration','engine-bank-rate','engine-release-width','a',1),
    c('engine-release-shortening','engine-motion','engine-release-width','b',-.00045),
    c('engine-pressure-edge','engine-load','engine-release-width','b',-.0016),
    c('engine-header-tail-a','engine-header-a','engine-pipe-a'),
    c('engine-header-tail-b','engine-header-b','engine-pipe-b'),
    c('engine-mix-a','engine-pipe-a','engine-eq','audio',1.2),
    c('engine-mix-b','engine-pipe-b','engine-eq','audio',.72),
    c('engine-hot-header-a','engine-load','engine-header-a','time',-.00008),
    c('engine-hot-header-b','engine-load','engine-header-b','time',-.00009),
    c('engine-hot-pipe-a','engine-load','engine-pipe-a','time',-.0002),
    c('engine-hot-pipe-b','engine-load','engine-pipe-b','time',-.00023),
    c('engine-shaft-frequency','engine-shaft-rate','engine-shaft-pulse','frequency',1),
    c('engine-load-mechanics','engine-load','engine-shaft-pulse','level',.36),
    c('engine-metal','engine-shaft-pulse','engine-mechanics'),
    c('engine-mix-mechanics','engine-mechanics','engine-eq','audio',1.15),
    c('engine-clatter-rate','engine-shaft-rate','engine-clatter','rate',3),
    c('engine-clatter-load','engine-load','engine-clatter','level',.12),
    c('engine-clatter-colour','engine-clatter','engine-clatter-filter'),
    c('engine-clatter-mix','engine-clatter-filter','engine-eq','audio',.85),
    c('engine-loaded-metal','engine-load','engine-clatter-filter','cutoff',1400),
    c('engine-load-air','engine-load','engine-air','level',.025),
    c('engine-speed-air','engine-motion','engine-air','level',.025),
    c('engine-flow','engine-air','engine-air-filter'),
    c('engine-mix-air','engine-air-filter','engine-eq','audio',.8),
    c('engine-release-strength','engine-release-response','engine-overrun','level',.34),
    c('engine-overrun-density','engine-motion','engine-overrun','rate',180),
    c('engine-release-colour','engine-overrun','engine-overrun-filter'),
    c('engine-release-mix','engine-overrun-filter','engine-eq','audio',.8),
    c('engine-intake-mix','engine-intake','engine-eq','audio',1.15),
    c('engine-intake-open','engine-load','engine-intake','cutoff',1100),
    c('engine-intake-speed','engine-motion','engine-intake','cutoff',250),
    c('engine-top-end','engine-eq','engine-rolloff'),
    c('engine-load-colour','engine-load','engine-rolloff','cutoff',1600),
    c('engine-density','engine-rolloff','engine-compressor'),
    c('engine-room','engine-compressor','engine-space'),
    c('engine-listen','engine-space','engine-output'),
  ];
  const strengths = [.97,1.05,.91,1.02,.95,1.10,.94,1.03,.88,.99,1.08,.96];
  const runners = [.20,.24,.28,.32,.37,.41];
  // Alternating banks, six events each per two crankshaft revolutions. Each
  // source is the same ordinary Pulse node. Its duration and delay are visible
  // conversions of the common cycle frequency, so it retains a finite pressure
  // release at high RPM instead of shrinking into a narrow pitched buzz.
  for (let event=0;event<12;event++) {
    const bank=event%2, cylinder=Math.floor(event/2), id=`engine-cylinder-${event+1}`;
    const column=bank*2+Math.floor(cylinder/3), row=cylinder%3;
    const strength=strengths[event];
    nodes.push(n(id,'pulse',680+column*300,50+row*320,
      {frequency:0,width:.002,phase:.07+event/12,jitter:.0035+(cylinder%3)*.0008,shimmer:.09+(cylinder%2)*.025,level:.13*strength,shape:'exponential'},
      `Bank ${bank?'B':'A'} · pressure ${cylinder+1}`));
    connections.push(
      c(`${id}-clock`,'engine-bank-rate',id,'frequency',1),
      c(`${id}-width`,'engine-release-width',id,'width',1),
      c(`${id}-runner`,'engine-bank-rate',id,'phase',-runners[cylinder]/520),
      c(`${id}-load`,'engine-load',id,'level',.48*strength),
      c(`${id}-pressure`,id,bank?'engine-header-b':'engine-header-a'),
      c(`${id}-induction`,id,'engine-intake','audio',bank?.65:1),
    );
  }
  return {
    id:'v12',name:'Italian V12',subtitle:'One pedal · speed and effort',icon:'engine',color:'#f3b77e',
    description:'One pedal drives speed and inferred effort. Inertia, acceleration and moving resistance shape pressure; stronger effort sharpens its pulses, opens the induction sound and changes exhaust reflections. Mechanical ticks and lift-off texture remain separate editable layers. Use Live or one automation curve. Automatic stages are disabled.',
    graph:{duration,loop:true,nodes,connections,performance:{node:'engine-pedal',parameter:'input',curveNode:'pedal-curve',edge:'pedal-automation',mode:'curve',manualValue:0}},
  };
}
