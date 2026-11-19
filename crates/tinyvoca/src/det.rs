//! Transcendentals through the `libm` crate instead of the platform's: the same Rust code runs natively and in wasm,
//! so a native render (`haa speak`, `haa render`) and the browser's give the same bits. (std's `f64::cos` is glibc's
//! natively and compiler-builtins' libm in wasm; they differ in the last ulp, and the network spreads that.)

pub(crate) trait Det: Sized {
    fn dcos(self) -> Self;
    fn dsin(self) -> Self;
    fn dexp(self) -> Self;
    fn dln(self) -> Self;
    fn dlog2(self) -> Self;
    fn dpowf(self, e: Self) -> Self;
    fn dexp_m1(self) -> Self;
    fn dln_1p(self) -> Self;
}

impl Det for f64 {
    fn dcos(self) -> f64 {
        libm::cos(self)
    }
    fn dsin(self) -> f64 {
        libm::sin(self)
    }
    fn dexp(self) -> f64 {
        libm::exp(self)
    }
    fn dln(self) -> f64 {
        libm::log(self)
    }
    fn dlog2(self) -> f64 {
        libm::log2(self)
    }
    fn dpowf(self, e: f64) -> f64 {
        libm::pow(self, e)
    }
    fn dexp_m1(self) -> f64 {
        libm::expm1(self)
    }
    fn dln_1p(self) -> f64 {
        libm::log1p(self)
    }
}

impl Det for f32 {
    fn dcos(self) -> f32 {
        libm::cosf(self)
    }
    fn dsin(self) -> f32 {
        libm::sinf(self)
    }
    fn dexp(self) -> f32 {
        libm::expf(self)
    }
    fn dln(self) -> f32 {
        libm::logf(self)
    }
    fn dlog2(self) -> f32 {
        libm::log2f(self)
    }
    fn dpowf(self, e: f32) -> f32 {
        libm::powf(self, e)
    }
    fn dexp_m1(self) -> f32 {
        libm::expm1f(self)
    }
    fn dln_1p(self) -> f32 {
        libm::log1pf(self)
    }
}
