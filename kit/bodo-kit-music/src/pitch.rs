//! Pitch classes and note names. Middle C is C4, MIDI key 60.

use core::fmt;
use core::str::FromStr;

/// Why text isn't a note name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a note name such as C, F# or Bb")]
pub struct PitchError(pub String);

/// One of the twelve pitch classes, 0 for C up to 11 for B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PitchClass(u8);

/// The names of the pitch classes, with sharps.
const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

impl PitchClass {
    /// The pitch class `n` stands for, wrapping around the octave.
    pub const fn new(n: u8) -> Self {
        Self(n % 12)
    }

    /// The pitch class of MIDI key `key`.
    pub const fn of(key: u8) -> Self {
        Self(key % 12)
    }

    /// The number, 0 for C up to 11 for B.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The name, with a sharp where one is needed.
    pub fn name(self) -> &'static str {
        NAMES[usize::from(self.0)]
    }

    /// Reads a note name with an optional octave, such as `C`, `F#`, `Bb3` or `c#4`, into its pitch class and octave.
    pub fn parse_with_octave(text: &str) -> Result<(Self, Option<i32>), PitchError> {
        let error = || PitchError(text.to_owned());
        let mut chars = text.trim().chars();
        let base = match chars.next().ok_or_else(error)?.to_ascii_uppercase() {
            'C' => 0,
            'D' => 2,
            'E' => 4,
            'F' => 5,
            'G' => 7,
            'A' => 9,
            'B' => 11,
            _ => return Err(error()),
        };
        let rest = chars.as_str();
        let (shift, octave) = if let Some(rest) = rest.strip_prefix('#') {
            (1, rest)
        } else if let Some(rest) = rest.strip_prefix('b').filter(|r| !r.starts_with(|c: char| c.is_ascii_alphabetic())) {
            (-1, rest)
        } else {
            (0, rest)
        };
        let octave = if octave.is_empty() { None } else { Some(octave.parse::<i32>().map_err(|_| error())?) };
        Ok((Self(((base + shift + 12) % 12) as u8), octave))
    }
}

impl FromStr for PitchClass {
    type Err = PitchError;

    /// Reads a note name without an octave, such as `C`, `F#` or `Bb`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match Self::parse_with_octave(text)? {
            (class, None) => Ok(class),
            (_, Some(_)) => Err(PitchError(text.to_owned())),
        }
    }
}

impl fmt::Display for PitchClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The MIDI key of pitch class `class` in octave `octave` (C4 is 60), if it lies in 0..=127.
pub fn key(class: PitchClass, octave: i32) -> Option<u8> {
    let key = (octave + 1) * 12 + i32::from(class.get());
    u8::try_from(key).ok().filter(|k| *k <= 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_both_ways() {
        assert_eq!("F#".parse::<PitchClass>(), Ok(PitchClass::new(6)));
        assert_eq!("Bb".parse::<PitchClass>(), Ok(PitchClass::new(10)));
        assert_eq!("b".parse::<PitchClass>(), Ok(PitchClass::new(11)));
        assert_eq!("Cb".parse::<PitchClass>(), Ok(PitchClass::new(11)));
        assert_eq!(PitchClass::parse_with_octave("C4"), Ok((PitchClass::new(0), Some(4))));
        assert_eq!(PitchClass::parse_with_octave("Bb-1"), Ok((PitchClass::new(10), Some(-1))));
        assert!("H".parse::<PitchClass>().is_err() && "C4".parse::<PitchClass>().is_err());
        assert_eq!(key(PitchClass::new(0), 4), Some(60));
        assert_eq!(key(PitchClass::new(9), -2), None);
        assert_eq!(PitchClass::of(61).name(), "C#");
    }
}
