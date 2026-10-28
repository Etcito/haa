//! haa-core: the HAA sound engine, a port of Sona v11's graph model.
//! Pure runtime code: no I/O, no clock, and randomness only from `rng`.

pub mod consts;
pub mod issues;
pub mod patch;
pub mod rng;

pub use issues::Issue;
pub use patch::Patch;

pub mod dsp {
    include!(concat!(env!("OUT_DIR"), "/dsp_gen.rs"));
}
