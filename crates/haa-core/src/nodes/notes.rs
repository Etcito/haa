//! Editable monophonic note events. Pitch and gate are separate graph instances.
use crate::catalog::{Category,NodeTypeDef,ParamDef,ParamKind,Scale};
use crate::node::{Node,NodeInit,NodeOut,SampleCtx};
const fn number(key: &'static str, min:f64,max:f64,default:f64,label_en:&'static str,label_fr:&'static str)->ParamDef {
 ParamDef{key,kind:ParamKind::Number{min,max,default,step:0.01,scale:Scale::Lin},unit:"",label_en,label_fr,modulatable:true,smooth:true}
}

use serde::Deserialize;
#[derive(Clone,Deserialize,Debug)]
#[serde(deny_unknown_fields)]
struct Note { start:f64, duration:f64, midi:f64, level:f64 }
static PARAMS: &[ParamDef]=&[
 ParamDef{key:"score",kind:ParamKind::Text{default:r#"[{"start":0,"duration":0.8,"midi":81,"level":1},{"start":1,"duration":0.8,"midi":84,"level":0.9}]"#},unit:"",label_en:"Notes",label_fr:"Notes",modulatable:false,smooth:false},
 ParamDef{key:"channel",kind:ParamKind::Enum{values:&["pitch","gate"],default:0},unit:"",label_en:"Output",label_fr:"Sortie",modulatable:false,smooth:false},
 number("low",1.0,10000.0,60.0,"Low frequency","Fréquence basse"),
 number("high",1.0,20000.0,3000.0,"High frequency","Fréquence haute"),
 ParamDef{key:"phrase",kind:ParamKind::Text{default:"phrase-1"},unit:"",label_en:"Linked phrase",label_fr:"Phrase liée",modulatable:false,smooth:false},
];
pub static DEF:NodeTypeDef=NodeTypeDef{type_name:"notes",category:Category::Control,label_en:"Note sequence",label_fr:"Suite de notes",audio_in:false,audio_out:false,control_out:true,control_outputs:&[],uses_curve:false,hidden:false,params:PARAMS,create};
fn parse(s:&str)->Vec<Note>{
 let Ok(mut notes)=serde_json::from_str::<Vec<Note>>(s) else{return vec![]};
 if notes.len()>512 || notes.iter().any(|n| !n.start.is_finite() || !n.duration.is_finite() || !n.midi.is_finite() || !n.level.is_finite() || n.start<0.0 || n.duration<0.005 || n.duration>3600.0 || !(0.0..=127.0).contains(&n.midi) || !(0.0..=1.0).contains(&n.level)){return vec![]}
 notes.sort_by(|a,b|a.start.total_cmp(&b.start));
 if notes.windows(2).any(|p|p[0].start+p[0].duration>p[1].start+1e-6){return vec![]}
 notes
}
struct Notes { notes:Vec<Note> }
fn create(init:&NodeInit<'_>)->Box<dyn Node>{let mut n=Notes{notes:vec![]};n.configure(init);Box::new(n)}
impl Notes {
 fn value_at(&self,t:f64,gate:bool,low:f64,high:f64)->f64 {
  if self.notes.is_empty(){return 0.0}
  let i=self.notes.partition_point(|n|n.start<=t).saturating_sub(1);let n=&self.notes[i];
  if gate {if t>=n.start && t<n.start+n.duration {n.level}else{0.0}}
  else {let hz=440.0*2.0f64.powf((n.midi-69.0)/12.0);((hz-low)/(high-low).max(1.0)).clamp(0.0,1.0)}
 }
}
impl Node for Notes {
 fn configure(&mut self,init:&NodeInit<'_>){self.notes=parse(init.texts.first().and_then(|s|s.as_deref()).unwrap_or("[]"));}
 fn process_sample(&mut self,ctx:&SampleCtx<'_>,_input:(f64,f64),p:&[f64])->NodeOut{
  NodeOut{l:0.0,r:0.0,value:self.value_at(ctx.t*ctx.duration,p.get(1).copied().unwrap_or(0.0).round()==1.0,p.get(2).copied().unwrap_or(60.0),p.get(3).copied().unwrap_or(3000.0))}
 }
 fn reset(&mut self){}
}
#[cfg(test)] mod tests {
 use super::*;
 fn score()->Notes{Notes{notes:parse(r#"[{"start":0.2,"duration":0.5,"midi":69,"level":0.8},{"start":1,"duration":0.5,"midi":81,"level":1}]"#)}}
 #[test]fn gate_respects_notes_and_rests(){let n=score();assert_eq!(n.value_at(0.1,true,60.0,3000.0),0.0);assert_eq!(n.value_at(0.2,true,60.0,3000.0),0.8);assert_eq!(n.value_at(0.7,true,60.0,3000.0),0.0);assert_eq!(n.value_at(1.5,true,60.0,3000.0),0.0);}
 #[test]fn pitch_is_normalized_and_held_in_rests(){let n=score();assert!((60.0+2940.0*n.value_at(0.9,false,60.0,3000.0)-440.0).abs()<1e-9);assert!((60.0+2940.0*n.value_at(1.1,false,60.0,3000.0)-880.0).abs()<1e-9);}
 #[test]fn invalid_or_overlapping_score_is_silent(){assert!(parse("not json").is_empty());assert!(parse(r#"[{"start":0,"duration":2,"midi":69,"level":1},{"start":1,"duration":1,"midi":72,"level":1}]"#).is_empty());}
}
