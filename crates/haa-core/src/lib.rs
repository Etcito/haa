//! haa-core: the HAA sound engine, a port of Sona v11's graph model.
//! Pure runtime code: no I/O, no clock, and randomness only from `rng`.

pub mod catalog;
pub mod consts;
pub mod graph;
pub mod issues;
pub mod node;
pub mod patch;
pub mod rng;
pub mod validate;

pub use catalog::catalog_json;
pub use graph::Voice;
pub use issues::Issue;
pub use patch::{Patch, load_patch};
pub use validate::validate;

pub mod nodes {
    include!(concat!(env!("OUT_DIR"), "/nodes_gen.rs"));
}

pub mod dsp {
    include!(concat!(env!("OUT_DIR"), "/dsp_gen.rs"));
}
