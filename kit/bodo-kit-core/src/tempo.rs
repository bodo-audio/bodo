//! Tempo maps: converting between musical ticks and seconds.
//!
//! A [`TempoMap`] is a list of [`TempoPoint`]s sorted by tick. Each point sets a tempo and a [`TempoCurve`] that
//! says how the tempo moves until the next point: it either holds and then jumps, or ramps linearly in ticks.
//! The map caches the time in seconds at every point, so a conversion is a binary search for the segment followed
//! by a closed form inside it.
//!
//! For a linear ramp from `b0` to `b1` BPM over `L` ticks, the tempo at `x` ticks into the segment is
//! `b(x) = b0 + k x` with `k = (b1 - b0) / L`, and the time is
//!
//! ```text
//! t(x) = 60 / (PPQ k) * ln(1 + k x / b0)        x(t) = b0 / k * (exp(PPQ k t / 60) - 1)
//! ```
//!
//! Both are evaluated through `ln_1p` and `exp_m1` so they stay accurate when `k` is tiny or zero.
//!
//! Before tick 0 and after the last point the tempo is constant: the first point's tempo extends backwards, and the
//! last point's tempo extends forwards whatever its curve.

use crate::rt_safe;
use crate::time::{PPQ, Seconds, Tick};

/// The slowest tempo a tempo map accepts, in beats per minute.
pub const MIN_BPM: f64 = 1.0;
/// The fastest tempo a tempo map accepts, in beats per minute.
pub const MAX_BPM: f64 = 1000.0;

const PPQ_F64: f64 = PPQ as f64;

/// How the tempo moves from a point to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum TempoCurve {
    /// The tempo holds until the next point, then jumps to that point's tempo.
    #[default]
    Jump,
    /// The tempo changes linearly in ticks until it reaches the next point's tempo.
    Linear,
}

/// A tempo change at a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TempoPoint {
    /// Where the point sits.
    pub at: Tick,
    /// The tempo at the point, in beats (quarter notes) per minute.
    pub bpm: f64,
    /// How the tempo moves from this point to the next one.
    pub curve: TempoCurve,
}

impl TempoPoint {
    /// Returns a point at `at` with tempo `bpm` and curve `curve`.
    pub const fn new(at: Tick, bpm: f64, curve: TempoCurve) -> Self {
        Self { at, bpm, curve }
    }
}

/// Why a list of points can't form a tempo map.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
pub enum TempoMapError {
    /// The list has no points.
    #[error("a tempo map needs at least one point")]
    Empty,
    /// The first point is not at tick 0.
    #[error("the first tempo point must be at tick 0, not at tick {}", .0.0)]
    FirstPointNotAtZero(Tick),
    /// A point does not come strictly after the one before it.
    #[error("tempo point {index} at tick {} does not come after the previous point", .at.0)]
    NotIncreasing {
        /// Index of the offending point.
        index: usize,
        /// Its tick.
        at: Tick,
    },
    /// A tempo is not finite or is outside [`MIN_BPM`]..=[`MAX_BPM`].
    #[error("tempo point {index} has {bpm} BPM, outside {MIN_BPM}..={MAX_BPM}")]
    BpmOutOfRange {
        /// Index of the offending point.
        index: usize,
        /// Its tempo.
        bpm: f64,
    },
}

/// One segment of the map, from a point up to the next one, in the form the queries need.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    /// Tick where the segment starts.
    at: f64,
    /// Seconds at the start of the segment.
    start: f64,
    /// Tempo at the start of the segment.
    bpm: f64,
    /// Tempo change per tick. Zero for a jump and for the last segment.
    slope: f64,
}

impl Segment {
    /// Seconds from the start of the segment to `x` ticks into it.
    fn seconds_into(&self, x: f64) -> f64 {
        let constant = 60.0 * x / (PPQ_F64 * self.bpm);
        if x <= 0.0 { constant } else { constant * ln_1p_ratio(self.slope * x / self.bpm) }
    }

    /// Ticks from the start of the segment to `dt` seconds into it. The inverse of [`Segment::seconds_into`].
    fn ticks_into(&self, dt: f64) -> f64 {
        let constant = dt * PPQ_F64 * self.bpm / 60.0;
        if dt <= 0.0 { constant } else { constant * exp_m1_ratio(dt * PPQ_F64 * self.slope / 60.0) }
    }

    /// Tempo `x` ticks into the segment.
    fn bpm_into(&self, x: f64) -> f64 {
        if x <= 0.0 { self.bpm } else { self.bpm + self.slope * x }
    }
}

/// `ln(1 + r) / r`, accurate near `r = 0` where it tends to 1.
fn ln_1p_ratio(r: f64) -> f64 {
    if r.abs() < 1e-5 { 1.0 - r / 2.0 + r * r / 3.0 } else { r.ln_1p() / r }
}

/// `(exp(u) - 1) / u`, accurate near `u = 0` where it tends to 1.
fn exp_m1_ratio(u: f64) -> f64 {
    if u.abs() < 1e-5 { 1.0 + u / 2.0 + u * u / 6.0 } else { u.exp_m1() / u }
}

/// A validated tempo map with cached segment start times.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(try_from = "Vec<TempoPoint>", into = "Vec<TempoPoint>"))]
pub struct TempoMap {
    points: Vec<TempoPoint>,
    /// One segment per point, in the same order. Never empty.
    segments: Vec<Segment>,
    /// A copy of the first segment, so lookups never need a fallible index.
    origin: Segment,
}

impl TempoMap {
    /// Builds a map from points sorted by tick. The first point must be at tick 0.
    ///
    /// Complexity: O(n) in the number of points.
    pub fn new(points: Vec<TempoPoint>) -> Result<Self, TempoMapError> {
        let first = points.first().ok_or(TempoMapError::Empty)?;
        if first.at != Tick::ZERO {
            return Err(TempoMapError::FirstPointNotAtZero(first.at));
        }
        for (index, point) in points.iter().enumerate() {
            if !(MIN_BPM..=MAX_BPM).contains(&point.bpm) {
                return Err(TempoMapError::BpmOutOfRange { index, bpm: point.bpm });
            }
        }
        for (index, pair) in points.windows(2).enumerate() {
            if let [prev, next] = pair
                && next.at <= prev.at
            {
                return Err(TempoMapError::NotIncreasing { index: index + 1, at: next.at });
            }
        }

        let mut segments = Vec::with_capacity(points.len());
        let mut start = 0.0;
        for (index, point) in points.iter().enumerate() {
            let next = points.get(index + 1);
            let slope = match (point.curve, next) {
                (TempoCurve::Linear, Some(next)) => (next.bpm - point.bpm) / (next.at.0 - point.at.0) as f64,
                _ => 0.0,
            };
            let segment = Segment { at: point.at.0 as f64, start, bpm: point.bpm, slope };
            if let Some(next) = next {
                start += segment.seconds_into((next.at.0 - point.at.0) as f64);
            }
            segments.push(segment);
        }
        let origin = Segment { at: 0.0, start: 0.0, bpm: first.bpm, slope: segments.first().map_or(0.0, |s| s.slope) };
        Ok(Self { points, segments, origin })
    }

    /// Builds a map with one constant tempo.
    pub fn constant(bpm: f64) -> Result<Self, TempoMapError> {
        Self::new(vec![TempoPoint::new(Tick::ZERO, bpm, TempoCurve::Jump)])
    }

    /// The points of the map, sorted by tick.
    pub fn points(&self) -> &[TempoPoint] {
        &self.points
    }

    /// The time at `tick`.
    ///
    /// RT-safe. Complexity: O(log n) in the number of points.
    #[rt_safe]
    pub fn seconds_at(&self, tick: Tick) -> Seconds {
        self.seconds_at_ticks(tick.0 as f64)
    }

    /// The time at a fractional tick position.
    ///
    /// RT-safe. Complexity: O(log n) in the number of points.
    #[rt_safe]
    pub fn seconds_at_ticks(&self, ticks: f64) -> Seconds {
        let segment = self.segment_at_ticks(ticks);
        Seconds(segment.start + segment.seconds_into(ticks - segment.at))
    }

    /// The fractional tick position at `time`. The inverse of [`TempoMap::seconds_at_ticks`].
    ///
    /// RT-safe. Complexity: O(log n) in the number of points.
    #[rt_safe]
    pub fn ticks_at(&self, time: Seconds) -> f64 {
        let index = self.segments.partition_point(|s| s.start <= time.0).saturating_sub(1);
        let segment = self.segments.get(index).unwrap_or(&self.origin);
        segment.at + segment.ticks_into(time.0 - segment.start)
    }

    /// The tempo at `tick`, in beats per minute.
    ///
    /// RT-safe. Complexity: O(log n) in the number of points.
    #[rt_safe]
    pub fn bpm_at(&self, tick: Tick) -> f64 {
        let ticks = tick.0 as f64;
        let segment = self.segment_at_ticks(ticks);
        segment.bpm_into(ticks - segment.at)
    }

    fn segment_at_ticks(&self, ticks: f64) -> &Segment {
        let index = self.segments.partition_point(|s| s.at <= ticks).saturating_sub(1);
        self.segments.get(index).unwrap_or(&self.origin)
    }
}

impl Default for TempoMap {
    /// A constant 120 BPM.
    fn default() -> Self {
        let segment = Segment { at: 0.0, start: 0.0, bpm: 120.0, slope: 0.0 };
        Self { points: vec![TempoPoint::new(Tick::ZERO, 120.0, TempoCurve::Jump)], segments: vec![segment], origin: segment }
    }
}

impl TryFrom<Vec<TempoPoint>> for TempoMap {
    type Error = TempoMapError;

    fn try_from(points: Vec<TempoPoint>) -> Result<Self, Self::Error> {
        Self::new(points)
    }
}

impl From<TempoMap> for Vec<TempoPoint> {
    fn from(map: TempoMap) -> Self {
        map.points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn constant_tempo() {
        let map = TempoMap::constant(120.0).unwrap();
        assert_eq!(map.seconds_at(Tick::QUARTER), Seconds(0.5));
        assert_eq!(map.seconds_at(Tick(-960)), Seconds(-0.5));
        assert_eq!(map.ticks_at(Seconds(2.0)), 3840.0);
        assert_eq!(map.bpm_at(Tick(1_000_000)), 120.0);
    }

    #[test]
    fn jump_holds_then_jumps() {
        let map =
            TempoMap::new(vec![TempoPoint::new(Tick(0), 120.0, TempoCurve::Jump), TempoPoint::new(Tick(3840), 60.0, TempoCurve::Jump)]).unwrap();
        assert_eq!(map.bpm_at(Tick(3839)), 120.0);
        assert_eq!(map.bpm_at(Tick(3840)), 60.0);
        // Four beats at 120 BPM, then one beat at 60 BPM.
        assert_eq!(map.seconds_at(Tick(3840)), Seconds(2.0));
        assert_eq!(map.seconds_at(Tick(4800)), Seconds(3.0));
        assert_eq!(map.ticks_at(Seconds(3.0)), 4800.0);
    }

    #[test]
    fn linear_ramp_matches_closed_form() {
        // 60 to 120 BPM over four beats takes 4 * ln(2) seconds.
        let map =
            TempoMap::new(vec![TempoPoint::new(Tick(0), 60.0, TempoCurve::Linear), TempoPoint::new(Tick(3840), 120.0, TempoCurve::Jump)]).unwrap();
        let end = map.seconds_at(Tick(3840)).0;
        assert!(close(end, 4.0 * 2f64.ln()), "{end}");
        assert!(close(map.ticks_at(Seconds(end)), 3840.0));
        assert!(close(map.bpm_at(Tick(1920)), 90.0));
        // After the ramp the tempo holds at 120 BPM.
        assert!(close(map.seconds_at(Tick(4800)).0, end + 0.5));
    }

    #[test]
    fn last_point_holds_even_when_linear() {
        let linear = TempoMap::new(vec![TempoPoint::new(Tick(0), 90.0, TempoCurve::Linear)]).unwrap();
        let constant = TempoMap::constant(90.0).unwrap();
        for tick in [Tick(-960), Tick(0), Tick(100_000)] {
            assert_eq!(linear.seconds_at(tick), constant.seconds_at(tick));
            assert_eq!(linear.bpm_at(tick), 90.0);
        }
    }

    #[test]
    fn rejects_invalid_points() {
        assert_eq!(TempoMap::new(vec![]), Err(TempoMapError::Empty));
        assert_eq!(TempoMap::new(vec![TempoPoint::new(Tick(10), 120.0, TempoCurve::Jump)]), Err(TempoMapError::FirstPointNotAtZero(Tick(10))));
        let unsorted = vec![
            TempoPoint::new(Tick(0), 120.0, TempoCurve::Jump),
            TempoPoint::new(Tick(960), 120.0, TempoCurve::Jump),
            TempoPoint::new(Tick(960), 100.0, TempoCurve::Jump),
        ];
        assert_eq!(TempoMap::new(unsorted), Err(TempoMapError::NotIncreasing { index: 2, at: Tick(960) }));
        for bpm in [0.0, MAX_BPM + 1.0, f64::NAN, f64::INFINITY] {
            let err = TempoMap::constant(bpm).unwrap_err();
            assert!(matches!(err, TempoMapError::BpmOutOfRange { index: 0, .. }), "{err}");
        }
    }

    #[test]
    fn ratio_helpers_are_continuous_at_zero() {
        for r in [1e-5, -1e-5, 1e-6, -1e-6] {
            assert!(close(ln_1p_ratio(r), r.ln_1p() / r));
            assert!(close(exp_m1_ratio(r), r.exp_m1() / r));
        }
        assert_eq!(ln_1p_ratio(0.0), 1.0);
        assert_eq!(exp_m1_ratio(0.0), 1.0);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serializes_as_points() {
        let map =
            TempoMap::new(vec![TempoPoint::new(Tick(0), 60.0, TempoCurve::Linear), TempoPoint::new(Tick(3840), 120.0, TempoCurve::Jump)]).unwrap();
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"[{"at":0,"bpm":60.0,"curve":"linear"},{"at":3840,"bpm":120.0,"curve":"jump"}]"#);
        assert_eq!(serde_json::from_str::<TempoMap>(&json).unwrap(), map);
        assert!(serde_json::from_str::<TempoMap>(r#"[{"at":5,"bpm":60.0,"curve":"jump"}]"#).is_err());
    }
}
