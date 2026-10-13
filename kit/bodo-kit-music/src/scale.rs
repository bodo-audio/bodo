//! Scales and modes (project.md section 10.4): which keys belong, and snapping keys into the scale.

use core::fmt;
use core::str::FromStr;

use crate::pitch::{PitchClass, PitchError};

/// Why text isn't a scale.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScaleError {
    /// The root isn't a note name.
    #[error(transparent)]
    Root(#[from] PitchError),
    /// The kind isn't one Bodo knows.
    #[error("`{0}` is not a scale; try major, minor, dorian, harmonic minor, minor pentatonic or blues")]
    Kind(String),
}

/// A kind of scale, independent of its root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScaleKind {
    /// Major (Ionian).
    Major,
    /// Natural minor (Aeolian).
    Minor,
    /// Harmonic minor.
    HarmonicMinor,
    /// Melodic minor, ascending.
    MelodicMinor,
    /// Dorian.
    Dorian,
    /// Phrygian.
    Phrygian,
    /// Lydian.
    Lydian,
    /// Mixolydian.
    Mixolydian,
    /// Locrian.
    Locrian,
    /// Major pentatonic.
    MajorPentatonic,
    /// Minor pentatonic.
    MinorPentatonic,
    /// The blues scale: minor pentatonic with the flat fifth.
    Blues,
    /// Whole tone.
    WholeTone,
    /// Every key.
    Chromatic,
}

impl ScaleKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 14] = [
        Self::Major,
        Self::Minor,
        Self::HarmonicMinor,
        Self::MelodicMinor,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::Locrian,
        Self::MajorPentatonic,
        Self::MinorPentatonic,
        Self::Blues,
        Self::WholeTone,
        Self::Chromatic,
    ];

    /// The semitones above the root that belong to the scale, ascending from 0.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Self::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Self::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Self::MajorPentatonic => &[0, 2, 4, 7, 9],
            Self::MinorPentatonic => &[0, 3, 5, 7, 10],
            Self::Blues => &[0, 3, 5, 6, 7, 10],
            Self::WholeTone => &[0, 2, 4, 6, 8, 10],
            Self::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }

    /// Its name, such as `harmonic minor`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Minor => "minor",
            Self::HarmonicMinor => "harmonic minor",
            Self::MelodicMinor => "melodic minor",
            Self::Dorian => "dorian",
            Self::Phrygian => "phrygian",
            Self::Lydian => "lydian",
            Self::Mixolydian => "mixolydian",
            Self::Locrian => "locrian",
            Self::MajorPentatonic => "major pentatonic",
            Self::MinorPentatonic => "minor pentatonic",
            Self::Blues => "blues",
            Self::WholeTone => "whole tone",
            Self::Chromatic => "chromatic",
        }
    }
}

impl FromStr for ScaleKind {
    type Err = ScaleError;

    /// Reads a scale name, ignoring case and extra spaces; common aliases such as `ionian`, `aeolian`, `natural minor`
    /// and `pentatonic` are accepted.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let name = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        let kind = match name.as_str() {
            "major" | "ionian" | "maj" => Self::Major,
            "minor" | "natural minor" | "aeolian" | "min" | "m" => Self::Minor,
            "harmonic minor" => Self::HarmonicMinor,
            "melodic minor" => Self::MelodicMinor,
            "dorian" => Self::Dorian,
            "phrygian" => Self::Phrygian,
            "lydian" => Self::Lydian,
            "mixolydian" => Self::Mixolydian,
            "locrian" => Self::Locrian,
            "major pentatonic" | "pentatonic" => Self::MajorPentatonic,
            "minor pentatonic" => Self::MinorPentatonic,
            "blues" => Self::Blues,
            "whole tone" | "wholetone" => Self::WholeTone,
            "chromatic" => Self::Chromatic,
            _ => return Err(ScaleError::Kind(text.trim().to_owned())),
        };
        Ok(kind)
    }
}

/// A scale on a root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scale {
    /// The root.
    pub root: PitchClass,
    /// The kind.
    pub kind: ScaleKind,
}

impl Scale {
    /// The scale `kind` on `root`.
    pub const fn new(root: PitchClass, kind: ScaleKind) -> Self {
        Self { root, kind }
    }

    /// Whether MIDI key `key` belongs to the scale.
    pub fn contains(&self, key: u8) -> bool {
        let offset = (12 + PitchClass::of(key).get() - self.root.get()) % 12;
        self.kind.intervals().contains(&offset)
    }

    /// The nearest key in the scale to `key`; when two are equally near, the lower one. Keys stay in 0..=127.
    pub fn snap(&self, key: u8) -> u8 {
        if self.contains(key) {
            return key;
        }
        // Every scale has a note within six semitones on one side or the other.
        for distance in 1..=6u8 {
            if let Some(down) = key.checked_sub(distance).filter(|k| self.contains(*k)) {
                return down;
            }
            if let Some(up) = key.checked_add(distance).filter(|k| *k <= 127 && self.contains(*k)) {
                return up;
            }
        }
        key
    }

    /// The keys of the scale from `low` to `high`, both included.
    ///
    /// Complexity: O(high - low).
    pub fn keys(&self, low: u8, high: u8) -> Vec<u8> {
        (low..=high.min(127)).filter(|k| self.contains(*k)).collect()
    }
}

impl FromStr for Scale {
    type Err = ScaleError;

    /// Reads a root and a kind, such as `C minor`, `F# dorian` or `Bb major pentatonic`. A root alone means major.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let (root, kind) = text.split_once(char::is_whitespace).unwrap_or((text, "major"));
        Ok(Self { root: root.parse()?, kind: kind.parse()? })
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.root, self.kind.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcs(scale: &Scale) -> Vec<u8> {
        (60..72).filter(|k| scale.contains(*k)).map(|k| k - 60).collect()
    }

    #[test]
    fn every_scale_has_its_intervals_and_round_trips_its_name() {
        for kind in ScaleKind::ALL {
            let scale = Scale::new(PitchClass::new(0), kind);
            assert_eq!(pcs(&scale), kind.intervals(), "{}", kind.name());
            assert_eq!(kind.name().parse::<ScaleKind>(), Ok(kind));
            assert_eq!(scale.to_string().parse::<Scale>(), Ok(scale));
        }
    }

    #[test]
    fn scales_read_roots_and_aliases() {
        let a_minor: Scale = "A minor".parse().unwrap();
        assert_eq!((a_minor.root.get(), a_minor.kind), (9, ScaleKind::Minor));
        assert_eq!("F#  Dorian".parse::<Scale>().map(|s| s.root.get()), Ok(6));
        assert_eq!("Bb major pentatonic".parse::<Scale>().map(|s| s.kind), Ok(ScaleKind::MajorPentatonic));
        assert_eq!("D".parse::<Scale>().map(|s| s.kind), Ok(ScaleKind::Major));
        assert!(matches!("C bebop".parse::<Scale>(), Err(ScaleError::Kind(_))));
        assert!(matches!("H minor".parse::<Scale>(), Err(ScaleError::Root(_))));
    }

    #[test]
    fn snapping_finds_the_nearest_key_and_ties_go_down() {
        let c_major: Scale = "C major".parse().unwrap();
        assert_eq!(c_major.snap(61), 60, "C# is between C and D: down");
        assert_eq!(c_major.snap(66), 65, "F# is between F and G: down");
        assert_eq!(c_major.snap(64), 64);
        let c_minor_pent: Scale = "C minor pentatonic".parse().unwrap();
        assert_eq!(c_minor_pent.snap(62), 63, "D is nearer Eb than C");
        assert_eq!(c_minor_pent.snap(0), 0);
        assert_eq!(c_major.snap(127), 127);
        assert_eq!(c_major.keys(60, 72), [60, 62, 64, 65, 67, 69, 71, 72]);
    }
}
