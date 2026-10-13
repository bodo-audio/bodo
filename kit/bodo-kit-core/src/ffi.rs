//! The only unsafe code in this crate: switching the FPU's handling of denormal numbers.

/// Runs `f` with denormal floats flushed to zero on this thread, and restores the previous mode afterwards, also
/// when `f` panics.
pub(crate) fn flush_denormals<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: `no_denormals` sets FTZ and DAZ in MXCSR on x86, or FZ in FPCR on aarch64, for the duration of `f`,
    // and its guard restores the previous register value on return and on unwind. Rust assumes the default
    // floating-point environment, so inside `f` a denormal result can differ from what constant folding would
    // produce. Audio code accepts this on purpose: denormals become zero, which is the point, and nothing here relies
    // on denormals being preserved. The register is per-thread, so other threads are unaffected.
    unsafe { no_denormals::no_denormals(f) }
}
