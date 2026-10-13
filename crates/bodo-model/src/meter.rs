//! Time signatures and bar positions (project.md section 8.1).

use bodo_kit_core::{PPQ, Tick};

/// A time signature change at the start of a bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterChange {
    /// The bar it starts at, counting from zero.
    pub bar: u32,
    /// Beats per bar, 1 to 64.
    pub numerator: u8,
    /// The beat unit: 1, 2, 4, 8, 16 or 32.
    pub denominator: u8,
}

impl MeterChange {
    /// The length of one beat in ticks.
    pub fn beat_ticks(&self) -> i64 {
        PPQ * 4 / i64::from(self.denominator.max(1))
    }

    /// The length of one bar in ticks.
    pub fn bar_ticks(&self) -> i64 {
        self.beat_ticks() * i64::from(self.numerator)
    }

    fn is_valid(&self) -> bool {
        (1..=64).contains(&self.numerator) && matches!(self.denominator, 1 | 2 | 4 | 8 | 16 | 32)
    }
}

/// Why a list of changes can't form a meter map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeterMapError {
    /// There are no changes, or the first isn't at bar 0.
    #[error("a meter map needs a first change at bar 0")]
    NoStart,
    /// Changes aren't in strictly increasing bar order.
    #[error("meter change {0} does not come after the previous one")]
    NotIncreasing(usize),
    /// A change has an impossible signature.
    #[error("meter change {0} has an invalid signature")]
    Invalid(usize),
}

/// A position in bars and beats, all counted from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BarBeat {
    /// The bar.
    pub bar: i64,
    /// The beat within the bar.
    pub beat: i64,
    /// Ticks into the beat.
    pub tick: i64,
}

/// Time signature changes, with the tick at which each one starts cached.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "Vec<MeterChange>", into = "Vec<MeterChange>")]
pub struct MeterMap {
    changes: Vec<MeterChange>,
    starts: Vec<Tick>,
}

impl MeterMap {
    /// Builds a map from changes sorted by bar, starting at bar 0.
    ///
    /// Complexity: O(n) in the number of changes.
    pub fn new(changes: Vec<MeterChange>) -> Result<Self, MeterMapError> {
        if changes.first().is_none_or(|c| c.bar != 0) {
            return Err(MeterMapError::NoStart);
        }
        let mut starts = Vec::with_capacity(changes.len());
        for (index, change) in changes.iter().enumerate() {
            if !change.is_valid() {
                return Err(MeterMapError::Invalid(index));
            }
            let start = match (index.checked_sub(1).and_then(|i| changes.get(i)), starts.last()) {
                (Some(previous), Some(&Tick(previous_start))) => {
                    if change.bar <= previous.bar {
                        return Err(MeterMapError::NotIncreasing(index));
                    }
                    Tick(previous_start + i64::from(change.bar - previous.bar) * previous.bar_ticks())
                }
                _ => Tick::ZERO,
            };
            starts.push(start);
        }
        Ok(Self { changes, starts })
    }

    /// 4/4 from the start.
    pub fn common_time() -> Self {
        Self { changes: vec![MeterChange { bar: 0, numerator: 4, denominator: 4 }], starts: vec![Tick::ZERO] }
    }

    /// The changes, sorted by bar.
    pub fn changes(&self) -> &[MeterChange] {
        &self.changes
    }

    /// The tick at which `bar` starts. Bars before zero extend the first signature backwards.
    ///
    /// Complexity: O(log n) in the number of changes.
    pub fn bar_start(&self, bar: i64) -> Tick {
        let index = self.changes.partition_point(|c| i64::from(c.bar) <= bar).saturating_sub(1);
        let (Some(change), Some(start)) = (self.changes.get(index), self.starts.get(index)) else { return Tick::ZERO };
        Tick(start.0 + (bar - i64::from(change.bar)) * change.bar_ticks())
    }

    /// The bar, beat and tick of `tick`.
    ///
    /// Complexity: O(log n) in the number of changes.
    pub fn position(&self, tick: Tick) -> BarBeat {
        let index = self.starts.partition_point(|s| *s <= tick).saturating_sub(1);
        let (Some(change), Some(start)) = (self.changes.get(index), self.starts.get(index)) else {
            return BarBeat { bar: 0, beat: 0, tick: 0 };
        };
        let offset = tick.0 - start.0;
        let bar_ticks = change.bar_ticks().max(1);
        let beat_ticks = change.beat_ticks().max(1);
        let bars = offset.div_euclid(bar_ticks);
        let in_bar = offset.rem_euclid(bar_ticks);
        BarBeat { bar: i64::from(change.bar) + bars, beat: in_bar / beat_ticks, tick: in_bar % beat_ticks }
    }

    /// The signature in force at `tick`.
    ///
    /// Complexity: O(log n) in the number of changes.
    pub fn signature_at(&self, tick: Tick) -> MeterChange {
        let index = self.starts.partition_point(|s| *s <= tick).saturating_sub(1);
        self.changes.get(index).copied().unwrap_or(MeterChange { bar: 0, numerator: 4, denominator: 4 })
    }
}

impl Default for MeterMap {
    fn default() -> Self {
        Self::common_time()
    }
}

impl TryFrom<Vec<MeterChange>> for MeterMap {
    type Error = MeterMapError;

    fn try_from(changes: Vec<MeterChange>) -> Result<Self, Self::Error> {
        Self::new(changes)
    }
}

impl From<MeterMap> for Vec<MeterChange> {
    fn from(map: MeterMap) -> Self {
        map.changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(bar: u32, numerator: u8, denominator: u8) -> MeterChange {
        MeterChange { bar, numerator, denominator }
    }

    #[test]
    fn bars_and_beats_follow_the_changes() {
        // Two bars of 4/4, then 6/8.
        let map = MeterMap::new(vec![change(0, 4, 4), change(2, 6, 8)]).unwrap();
        assert_eq!(map.bar_start(1), Tick(3840));
        assert_eq!(map.bar_start(2), Tick(7680));
        assert_eq!(map.bar_start(3), Tick(7680 + 6 * 480));
        assert_eq!(map.position(Tick(7680 + 480 * 7 + 5)), BarBeat { bar: 3, beat: 1, tick: 5 });
        assert_eq!(map.position(Tick(-960)), BarBeat { bar: -1, beat: 3, tick: 0 });
        assert_eq!(map.signature_at(Tick(9000)), change(2, 6, 8));
    }

    #[test]
    fn rejects_bad_maps() {
        assert_eq!(MeterMap::new(vec![]), Err(MeterMapError::NoStart));
        assert_eq!(MeterMap::new(vec![change(1, 4, 4)]), Err(MeterMapError::NoStart));
        assert_eq!(MeterMap::new(vec![change(0, 4, 4), change(0, 3, 4)]), Err(MeterMapError::NotIncreasing(1)));
        assert_eq!(MeterMap::new(vec![change(0, 4, 3)]), Err(MeterMapError::Invalid(0)));
    }
}
