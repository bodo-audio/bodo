//! Real-time execution: the checks behind [`rt_safe`](crate::rt_safe) and denormal flushing.
//!
//! Code on the audio thread and the RT workers must not allocate, lock, block or do I/O (AGENTS.md section 4). Two
//! tools catch mistakes:
//!
//! - `assert_no_alloc`, in debug builds of programs that install its allocator, reports every allocation and
//!   deallocation made inside a guarded region.
//! - RealtimeSanitizer, in builds made with the `RTSAN_ENABLE` environment variable set, also catches locks, blocking
//!   calls and I/O.
//!
//! `#[rt_safe]` wraps a function body in [`guarded`], which turns both on. In normal release builds it compiles away.

/// Runs `f` as real-time code. `#[rt_safe]` wraps function bodies in this; call it directly only for closures.
///
/// RT-safe.
#[inline(always)]
pub fn guarded<T>(f: impl FnOnce() -> T) -> T {
    let _scope = RealtimeScope::enter();
    assert_no_alloc::assert_no_alloc(f)
}

/// Runs `f` with the real-time checks switched off, for the rare code on an RT thread that may allocate, such as
/// one-time setup.
#[inline(always)]
pub fn permit<T>(f: impl FnOnce() -> T) -> T {
    rtsan_standalone::disable();
    let result = assert_no_alloc::permit_alloc(f);
    rtsan_standalone::enable();
    result
}

/// Runs `f` with denormal floats flushed to zero on this thread, restoring the previous mode afterwards.
///
/// Denormals are tiny numbers near zero that some CPUs process up to a hundred times slower. Decaying filters and
/// reverbs produce them constantly, so every RT thread runs its audio code inside this (project.md section 6.3).
///
/// RT-safe.
#[inline]
pub fn flush_denormals<T>(f: impl FnOnce() -> T) -> T {
    crate::ffi::flush_denormals(f)
}

/// Marks the RealtimeSanitizer context for as long as it lives.
struct RealtimeScope;

impl RealtimeScope {
    #[inline(always)]
    fn enter() -> Self {
        rtsan_standalone::realtime_enter();
        Self
    }
}

impl Drop for RealtimeScope {
    #[inline(always)]
    fn drop(&mut self) {
        rtsan_standalone::realtime_exit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_and_permit_return_their_values() {
        assert_eq!(guarded(|| 2 + 2), 4);
        assert_eq!(permit(|| vec![1, 2]).len(), 2);
    }

    #[test]
    fn flush_denormals_flushes_and_restores() {
        let tiny = std::hint::black_box(f32::MIN_POSITIVE);
        let half = std::hint::black_box(0.5f32);
        assert!((tiny * half).is_subnormal(), "denormals are kept by default");
        let flushed = flush_denormals(|| std::hint::black_box(tiny) * std::hint::black_box(half));
        assert_eq!(flushed, 0.0, "denormal results are flushed inside");
        assert!((tiny * half).is_subnormal(), "the previous mode is restored");
    }
}
