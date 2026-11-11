use haa_core::{catalog::ParamKind,node::{NodeInit,SampleCtx},patch::GraphMode,nodes::membrane::DEF};
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicUsize,Ordering},hint::black_box,time::Instant};
struct Counter;static ALLOCS:AtomicUsize=AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counter {unsafe fn alloc(&self,l:Layout)->*mut u8{ALLOCS.fetch_add(1,Ordering::Relaxed);unsafe{System.alloc(l)}}unsafe fn dealloc(&self,p:*mut u8,l:Layout){unsafe{System.dealloc(p,l)}}}
#[global_allocator]static ALLOC:Counter=Counter;
fn main(){
 let mut params:Vec<f64>=DEF.params.iter().map(|p|match p.kind{ParamKind::Number{default,..}=>default,ParamKind::Bool{default}=>f64::from(default),_=>0.}).collect();
 params[13]=1.;let texts=vec![None;params.len()];
 let init=NodeInit{id:"drum",seed:0,sample_rate:48000.,params:&params,texts:&texts,curve:None,inputs:&[],duration:4.};
 let mut ctx=SampleCtx{sample_rate:48000.,dt:1./48000.,t:0.,seconds:0.,duration:4.,mode:GraphMode::Live,live_frames:0,looped:false,perf_rebase:false,inputs:&[],triggers:&[]};
 for count in [1,8,32,64]{let mut times=vec![];let mut allocations=0;let mut checksum=0.;
  for _ in 0..3{let mut nodes:Vec<_>=(0..count).map(|_|(DEF.create)(&init)).collect();let before=ALLOCS.load(Ordering::Relaxed);let start=Instant::now();
   for frame in 0..48000 {ctx.live_frames=frame;for n in &mut nodes{checksum+=black_box(n.process_sample(&ctx,(0.,0.),&params).l);}}
   let elapsed=start.elapsed().as_secs_f64();allocations+=ALLOCS.load(Ordering::Relaxed)-before;times.push(elapsed);
  }
  times.sort_by(|a,b|a.total_cmp(b));println!("{{\"instances\":{count},\"secondsMedian\":{},\"allocations\":{allocations},\"checksum\":{checksum}}}",times[1]);
 }
 // Physical rebuild at an impact must also avoid allocating in the processing thread.
 let mut node=(DEF.create)(&init);let mut changed=params.clone();changed[14]=0.4;
 let before=ALLOCS.load(Ordering::Relaxed);node.process_sample(&ctx,(0.,0.),&changed);let allocations=ALLOCS.load(Ordering::Relaxed)-before;
 assert_eq!(allocations,0);println!("{{\"mechanicalChangeAllocations\":{allocations}}}");
}
