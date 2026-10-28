//! haa-core: the HAA sound engine, a port of Sona v11's graph model.
//! Pure runtime code: no I/O, no clock, and randomness only from `rng`.

pub mod consts;
pub mod rng;

pub mod dsp {
    include!(concat!(env!("OUT_DIR"), "/dsp_gen.rs"));
}
