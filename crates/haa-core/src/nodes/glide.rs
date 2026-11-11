//! Reusable smooth transitions for any control signal. No oscillator inside.
use crate::catalog::{Category,NodeTypeDef,ParamDef,ParamKind,Scale};
use crate::node::{Node,NodeInit,NodeOut,SampleCtx};
const fn number(key: &'static str, min:f64,max:f64,default:f64,label_en:&'static str,label_fr:&'static str)->ParamDef {
 ParamDef{key,kind:ParamKind::Number{min,max,default,step:0.01,scale:Scale::Lin},unit:"",label_en,label_fr,modulatable:true,smooth:true}
}

static PARAMS:&[ParamDef]=&[number("input",-20000.0,20000.0,0.0,"Input","Entrée"),number("time",0.0,2.0,0.06,"Glide (seconds)","Glissé (secondes)")];
pub static DEF:NodeTypeDef=NodeTypeDef{type_name:"glide",category:Category::Control,label_en:"Glide",label_fr:"Glissé",audio_in:false,audio_out:false,control_out:true,control_outputs:&[],uses_curve:false,hidden:false,params:PARAMS,create};
struct Glide{value:Option<f64>}
fn create(_: &NodeInit<'_>)->Box<dyn Node>{Box::new(Glide{value:None})}
impl Glide{fn step(&mut self,target:f64,time:f64,dt:f64)->f64{let old=self.value.unwrap_or(target);let alpha=if time<=0.0{1.0}else{1.0-(-3.0*dt/time).exp()};let y=old+alpha*(target-old);self.value=Some(y);y}}
impl Node for Glide {
 fn process_sample(&mut self,ctx:&SampleCtx<'_>,_input:(f64,f64),p:&[f64])->NodeOut{if ctx.looped{self.value=None}NodeOut{l:0.0,r:0.0,value:self.step(p.first().copied().unwrap_or(0.0),p.get(1).copied().unwrap_or(0.06).max(0.0),ctx.dt)}}
 fn reset(&mut self){self.value=None;}
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn reaches_95_percent_at_requested_time_for_all_rates(){for rate in [44100,48000,96000]{let mut g=Glide{value:Some(0.0)};for _ in 0..rate/10{g.step(1.0,0.1,1.0/rate as f64);}assert!((g.value.unwrap()-(1.0-(-3.0f64).exp())).abs()<1e-10);}}
 #[test]fn starts_without_ramp_and_zero_time_bypasses(){let mut g=Glide{value:None};assert_eq!(g.step(0.7,1.0,0.001),0.7);assert_eq!(g.step(0.2,0.0,0.001),0.2);g.reset();assert_eq!(g.step(0.9,1.0,0.001),0.9);}
}
