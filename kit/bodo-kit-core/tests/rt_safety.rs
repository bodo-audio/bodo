//! Checks that the functions documented as RT-safe never allocate or free memory.
//!
//! This test binary installs `assert_no_alloc`'s allocator, which counts every allocation made inside an
//! `assert_no_alloc` closure on the current thread. The check only runs in debug builds.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_core::queue::spsc;
use bodo_kit_core::tempo::{TempoCurve, TempoMap, TempoPoint};
use bodo_kit_core::{SampleRate, Seconds, Tick};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Runs `f` with allocations forbidden and returns how many allocations it made on this thread.
fn allocations_in(f: impl FnOnce()) -> u32 {
    reset_violation_count();
    assert_no_alloc(f);
    violation_count()
}

#[test]
fn the_checker_catches_allocations() {
    assert!(allocations_in(|| drop(std::hint::black_box(vec![1u8; 16]))) > 0);
}

#[test]
fn tempo_queries_do_not_allocate() {
    let map = TempoMap::new(vec![
        TempoPoint::new(Tick(0), 90.0, TempoCurve::Linear),
        TempoPoint::new(Tick(3840), 140.0, TempoCurve::Jump),
        TempoPoint::new(Tick(7680), 70.0, TempoCurve::Jump),
    ])
    .unwrap();
    let rate = SampleRate::HZ_48000;
    let allocations = allocations_in(|| {
        for tick in (-960..20_000).step_by(97) {
            let seconds = map.seconds_at(Tick(tick));
            let frames = rate.seconds_to_frames(seconds);
            std::hint::black_box((map.ticks_at(rate.frames_to_seconds(frames)), map.bpm_at(Tick(tick)), map.ticks_at(Seconds(-1.0))));
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn queue_push_and_pop_do_not_allocate() {
    let (mut tx, mut rx) = spsc::<u64>(8);
    let allocations = allocations_in(|| {
        for i in 0..100 {
            tx.push_or_drop(i);
            if i % 3 == 0 {
                std::hint::black_box(rx.pop());
            }
            std::hint::black_box((tx.free_slots(), tx.overflows(), rx.len(), rx.is_abandoned()));
        }
    });
    assert_eq!(allocations, 0);
}
