//! Property tests for the tempo map.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use bodo_kit_core::tempo::{MAX_BPM, MIN_BPM, TempoCurve, TempoMap, TempoPoint};
use bodo_kit_core::{PPQ, Seconds, Tick};
use proptest::prelude::*;

fn curve() -> impl Strategy<Value = TempoCurve> {
    prop_oneof![Just(TempoCurve::Jump), Just(TempoCurve::Linear)]
}

/// Points with tempos drawn from `bpm`, gaps of one tick up to 64 beats, and up to 16 changes.
fn points(bpm: impl Strategy<Value = f64> + Clone) -> impl Strategy<Value = Vec<TempoPoint>> {
    (bpm.clone(), curve(), prop::collection::vec((1..=PPQ * 64, bpm, curve()), 0..16)).prop_map(|(bpm0, curve0, rest)| {
        let mut points = vec![TempoPoint::new(Tick(0), bpm0, curve0)];
        let mut at = 0;
        for (gap, bpm, curve) in rest {
            at += gap;
            points.push(TempoPoint::new(Tick(at), bpm, curve));
        }
        points
    })
}

fn any_bpm() -> impl Strategy<Value = f64> + Clone {
    prop_oneof![MIN_BPM..=MAX_BPM, 40.0..=240.0, Just(MIN_BPM), Just(MAX_BPM)]
}

fn map(points: Vec<TempoPoint>) -> TempoMap {
    TempoMap::new(points).unwrap()
}

/// Last point plus 64 beats, so queries also cover the open-ended tail.
fn horizon(points: &[TempoPoint]) -> i64 {
    points.last().map_or(0, |p| p.at.0) + PPQ * 64
}

/// The tempo at a fractional tick, computed straight from the points as an independent reference.
fn reference_bpm(points: &[TempoPoint], ticks: f64) -> f64 {
    let index = points.iter().rposition(|p| p.at.0 as f64 <= ticks).unwrap_or(0);
    let point = points[index];
    match (point.curve, points.get(index + 1)) {
        (TempoCurve::Linear, Some(next)) if ticks >= point.at.0 as f64 => {
            let fraction = (ticks - point.at.0 as f64) / (next.at.0 - point.at.0) as f64;
            point.bpm + (next.bpm - point.bpm) * fraction
        }
        _ => point.bpm,
    }
}

/// Seconds from tick 0 to `end`, by composite Simpson integration of 60 / (PPQ * bpm) over each segment.
fn integrated_seconds(points: &[TempoPoint], end: f64) -> f64 {
    let mut bounds: Vec<f64> = points.iter().map(|p| p.at.0 as f64).filter(|&at| at < end).collect();
    bounds.push(end);
    let dt = |x: f64| 60.0 / (PPQ as f64 * reference_bpm(points, x));
    let mut total = 0.0;
    for pair in bounds.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        // Integrate on [a, b) only: nudge the right edge below b so a jump at b isn't sampled.
        let b_inner = b - (b - a) * 1e-12;
        let n = 2048;
        let h = (b_inner - a) / n as f64;
        let mut sum = dt(a) + dt(b_inner);
        for i in 1..n {
            let weight = if i % 2 == 1 { 4.0 } else { 2.0 };
            sum += weight * dt(a + i as f64 * h);
        }
        total += sum * h / 3.0 + (b - b_inner) * dt(b_inner);
    }
    total
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn tick_to_seconds_round_trips(points in points(any_bpm()), t in 0.0..1.0f64) {
        let end = horizon(&points);
        let tick = (-PPQ * 16) + ((end + PPQ * 16) as f64 * t) as i64;
        let map = map(points);
        let back = map.ticks_at(map.seconds_at(Tick(tick)));
        prop_assert!((back - tick as f64).abs() <= 1e-6 + 1e-12 * tick.abs() as f64, "tick {tick} came back as {back}");
    }

    #[test]
    fn seconds_to_ticks_round_trips(points in points(any_bpm()), t in -0.1..1.1f64) {
        let end = horizon(&points);
        let map = map(points);
        let time = Seconds(map.seconds_at(Tick(end)).0 * t);
        let back = map.seconds_at_ticks(map.ticks_at(time));
        prop_assert!((back.0 - time.0).abs() <= 1e-9 + 1e-12 * time.0.abs(), "{time:?} came back as {back:?}");
    }

    #[test]
    fn time_strictly_increases_with_ticks(points in points(any_bpm()), a in 0.0..1.0f64, b in 0.0..1.0f64) {
        let end = horizon(&points);
        let (lo, hi) = (a.min(b), a.max(b));
        let t1 = Tick((end as f64 * lo) as i64);
        let t2 = Tick((end as f64 * hi) as i64);
        let map = map(points);
        if t1 < t2 {
            prop_assert!(map.seconds_at(t1) < map.seconds_at(t2));
        } else {
            prop_assert_eq!(map.seconds_at(t1), map.seconds_at(t2));
        }
    }

    #[test]
    fn bpm_matches_reference_and_stays_between_neighbors(points in points(any_bpm()), t in 0.0..1.0f64) {
        let tick = (horizon(&points) as f64 * t) as i64;
        let expected = reference_bpm(&points, tick as f64);
        let actual = map(points.clone()).bpm_at(Tick(tick));
        prop_assert!((actual - expected).abs() <= 1e-9 * expected, "bpm {actual}, reference {expected}");
        let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.bpm), hi.max(p.bpm)));
        prop_assert!(actual >= lo * (1.0 - 1e-12) && actual <= hi * (1.0 + 1e-12));
    }

    #[test]
    fn constant_tempo_is_proportional(bpm in any_bpm(), tick in -(PPQ * 1_000_000)..PPQ * 1_000_000) {
        let map = TempoMap::constant(bpm).unwrap();
        let expected = 60.0 * tick as f64 / (PPQ as f64 * bpm);
        let actual = map.seconds_at(Tick(tick)).0;
        prop_assert!((actual - expected).abs() <= 1e-12 * expected.abs().max(1.0));
    }

    #[test]
    fn flat_linear_ramp_equals_jump(bpm in any_bpm(), gap in 1..PPQ * 64, t in 0.0..2.0f64) {
        let jump = TempoMap::new(vec![
            TempoPoint::new(Tick(0), bpm, TempoCurve::Jump),
            TempoPoint::new(Tick(gap), bpm, TempoCurve::Jump),
        ]).unwrap();
        let linear = TempoMap::new(vec![
            TempoPoint::new(Tick(0), bpm, TempoCurve::Linear),
            TempoPoint::new(Tick(gap), bpm, TempoCurve::Jump),
        ]).unwrap();
        let ticks = gap as f64 * t;
        prop_assert_eq!(jump.seconds_at_ticks(ticks), linear.seconds_at_ticks(ticks));
    }
}

proptest! {
    // The Simpson reference is slow, so this property runs fewer cases.
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn closed_form_matches_numerical_integration(points in points(20.0..=400.0f64), t in 0.0..1.0f64) {
        let end = horizon(&points) as f64 * t;
        let expected = integrated_seconds(&points, end);
        let actual = map(points).seconds_at_ticks(end).0;
        prop_assert!((actual - expected).abs() <= 1e-7 * expected.max(1.0), "closed form {actual}, integrated {expected}");
    }
}

#[test]
fn the_default_map_is_a_constant_120_bpm() {
    assert_eq!(TempoMap::default(), TempoMap::constant(120.0).unwrap());
}
