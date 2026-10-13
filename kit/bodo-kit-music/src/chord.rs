//! Chord symbols, such as `C`, `Am7`, `Fmaj7`, `Bm7b5` or `C/E`, voiced in close position for the piano roll's stamp
//! tool (project.md section 10.4).

use core::fmt;
use core::str::FromStr;

use crate::pitch::{self, PitchClass};

/// Why text isn't a chord symbol.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a chord symbol; try C, Cm, C7, Cmaj7, Cm7b5, Cdim7, Csus4, Cadd9, C9, C13 or C/E")]
pub struct ChordError(pub String);

/// A chord: a root, the intervals above it, and an optional bass note below it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The root.
    pub root: PitchClass,
    /// Semitones above the root, ascending from 0.
    pub intervals: Vec<u8>,
    /// The bass of a slash chord, played below the root.
    pub bass: Option<PitchClass>,
    /// The quality as written, such as `m7b5`, for display.
    pub quality: String,
}

/// Chord qualities and their intervals. Longer spellings come first so `maj7` isn't read as `m`.
const QUALITIES: &[(&str, &[u8])] = &[
    ("maj13", &[0, 4, 7, 11, 14, 21]),
    ("maj9", &[0, 4, 7, 11, 14]),
    ("maj7", &[0, 4, 7, 11]),
    ("mmaj7", &[0, 3, 7, 11]),
    ("mM7", &[0, 3, 7, 11]),
    ("M7", &[0, 4, 7, 11]),
    ("m7b5", &[0, 3, 6, 10]),
    ("madd9", &[0, 3, 7, 14]),
    ("m13", &[0, 3, 7, 10, 14, 21]),
    ("m11", &[0, 3, 7, 10, 14, 17]),
    ("m9", &[0, 3, 7, 10, 14]),
    ("m7", &[0, 3, 7, 10]),
    ("m6", &[0, 3, 7, 9]),
    ("min7", &[0, 3, 7, 10]),
    ("min", &[0, 3, 7]),
    ("m", &[0, 3, 7]),
    ("-7", &[0, 3, 7, 10]),
    ("-", &[0, 3, 7]),
    ("dim7", &[0, 3, 6, 9]),
    ("dim", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("+", &[0, 4, 8]),
    ("7sus4", &[0, 5, 7, 10]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("sus", &[0, 5, 7]),
    ("add9", &[0, 4, 7, 14]),
    ("13", &[0, 4, 7, 10, 14, 21]),
    ("11", &[0, 4, 7, 10, 14, 17]),
    ("9", &[0, 4, 7, 10, 14]),
    ("7", &[0, 4, 7, 10]),
    ("6", &[0, 4, 7, 9]),
    ("maj", &[0, 4, 7]),
    ("", &[0, 4, 7]),
];

/// The qualities [`Chord::detect`] names, in the order it prefers when two spellings fit the same keys.
const DETECTED: &[&str] = &[
    "", "m", "7", "maj7", "m7", "dim", "dim7", "aug", "sus2", "sus4", "7sus4", "m7b5", "6", "m6", "add9", "9", "m9", "maj9", "11", "m11", "13",
    "maj13",
];

impl Chord {
    /// Names the chord that `keys` sound, whatever their octaves and order: the lowest key is the bass, and a chord
    /// whose root isn't the bass is written as a slash chord. Fewer than three different pitch classes, or a set no
    /// quality fits, give `None`.
    ///
    /// Complexity: O(k) in the keys, plus a constant 12 roots times the qualities.
    pub fn detect(keys: &[u8]) -> Option<Self> {
        let bass = PitchClass::of(*keys.iter().min()?);
        let set = keys.iter().fold(0u16, |set, key| set | 1 << (key % 12));
        if set.count_ones() < 3 {
            return None;
        }
        let roots = core::iter::once(bass.get()).chain((0..12).filter(|r| *r != bass.get() && set & 1 << r != 0));
        let mut slash = None;
        for root in roots {
            for quality in DETECTED {
                let Some((_, intervals)) = QUALITIES.iter().find(|(name, _)| name == quality) else { continue };
                let mask = intervals.iter().fold(0u16, |mask, i| mask | 1 << ((root + i) % 12));
                if mask != set {
                    continue;
                }
                let chord = Self { root: PitchClass::new(root), intervals: intervals.to_vec(), bass: None, quality: (*quality).to_owned() };
                if root == bass.get() {
                    return Some(chord);
                }
                slash.get_or_insert(Self { bass: Some(bass), ..chord });
            }
        }
        slash
    }

    /// The MIDI keys of the chord in close position, the root in `octave` (C4 is 60) and a slash chord's bass just
    /// below the root. Keys outside 0..=127 are left out.
    pub fn keys(&self, octave: i32) -> Vec<u8> {
        let Some(root) = pitch::key(self.root, octave) else { return Vec::new() };
        let mut keys = Vec::with_capacity(self.intervals.len() + 1);
        if let Some(bass) = self.bass {
            let below = (12 + self.root.get() - bass.get()) % 12;
            let below = if below == 0 { 12 } else { below };
            if let Some(key) = root.checked_sub(below) {
                keys.push(key);
            }
        }
        keys.extend(self.intervals.iter().filter_map(|i| root.checked_add(*i)).filter(|k| *k <= 127));
        keys
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    /// Reads a chord symbol: a root such as `C`, `F#` or `Bb`, a quality, and optionally `/` and a bass note.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ChordError(text.to_owned());
        let text = text.trim();
        let (symbol, bass) = match text.split_once('/') {
            Some((symbol, bass)) => (symbol, Some(bass.parse::<PitchClass>().map_err(|_| error())?)),
            None => (text, None),
        };
        // The root is a letter and an optional accidental.
        let root_len = symbol.char_indices().nth(1).map_or(symbol.len(), |(i, c)| if c == '#' || c == 'b' { i + 1 } else { i });
        let (root, quality) = symbol.split_at(root_len.min(symbol.len()));
        let root: PitchClass = root.parse().map_err(|_| error())?;
        let intervals = QUALITIES.iter().find(|(name, _)| *name == quality).map(|(_, i)| i.to_vec()).ok_or_else(error)?;
        Ok(Self { root, intervals, bass, quality: quality.to_owned() })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.root, self.quality)?;
        if let Some(bass) = self.bass {
            write!(f, "/{bass}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(symbol: &str) -> Vec<u8> {
        symbol.parse::<Chord>().unwrap().keys(4)
    }

    #[test]
    fn every_quality_voices_in_close_position() {
        assert_eq!(keys("C"), [60, 64, 67]);
        assert_eq!(keys("Cm"), [60, 63, 67]);
        assert_eq!(keys("C7"), [60, 64, 67, 70]);
        assert_eq!(keys("Cmaj7"), [60, 64, 67, 71]);
        assert_eq!(keys("CM7"), [60, 64, 67, 71]);
        assert_eq!(keys("Cm7"), [60, 63, 67, 70]);
        assert_eq!(keys("Cm7b5"), [60, 63, 66, 70]);
        assert_eq!(keys("Cdim"), [60, 63, 66]);
        assert_eq!(keys("Cdim7"), [60, 63, 66, 69]);
        assert_eq!(keys("Caug"), [60, 64, 68]);
        assert_eq!(keys("Csus2"), [60, 62, 67]);
        assert_eq!(keys("Csus4"), [60, 65, 67]);
        assert_eq!(keys("C7sus4"), [60, 65, 67, 70]);
        assert_eq!(keys("Cadd9"), [60, 64, 67, 74]);
        assert_eq!(keys("C6"), [60, 64, 67, 69]);
        assert_eq!(keys("Cm6"), [60, 63, 67, 69]);
        assert_eq!(keys("C9"), [60, 64, 67, 70, 74]);
        assert_eq!(keys("Cm9"), [60, 63, 67, 70, 74]);
        assert_eq!(keys("Cmaj9"), [60, 64, 67, 71, 74]);
        assert_eq!(keys("C11"), [60, 64, 67, 70, 74, 77]);
        assert_eq!(keys("C13"), [60, 64, 67, 70, 74, 81]);
        assert_eq!(keys("Cmmaj7"), [60, 63, 67, 71]);
        assert_eq!(keys("CmM7"), [60, 63, 67, 71]);
        assert_eq!(keys("Cm11"), [60, 63, 67, 70, 74, 77]);
        assert_eq!(keys("Cm13"), [60, 63, 67, 70, 74, 81]);
        assert_eq!(keys("Cmaj13"), [60, 64, 67, 71, 74, 81]);
        assert_eq!(keys("Cmadd9"), [60, 63, 67, 74]);
        assert_eq!(keys("C-7"), keys("Cm7"));
        assert_eq!(keys("Cmin"), keys("Cm"));
        assert_eq!(keys("C+"), keys("Caug"));
        assert_eq!(keys("Csus"), keys("Csus4"));
        assert_eq!(keys("Cmaj"), keys("C"));
    }

    #[test]
    fn roots_accidentals_and_slash_basses() {
        assert_eq!(keys("F#m"), [66, 69, 73]);
        assert_eq!(keys("Bbmaj7"), [70, 74, 77, 81]);
        assert_eq!(keys("C/E"), [52, 60, 64, 67], "the bass sits below the root");
        assert_eq!(keys("Am/G"), [67, 69, 72, 76]);
        assert_eq!(keys("C/C"), [48, 60, 64, 67]);
        assert_eq!("Bbm7b5/Ab".parse::<Chord>().map(|c| c.to_string()), Ok(String::from("A#m7b5/G#")));
        assert!("H7".parse::<Chord>().is_err() && "Cxyz".parse::<Chord>().is_err() && "C/X".parse::<Chord>().is_err());
        assert!("G13".parse::<Chord>().unwrap().keys(9).iter().all(|k| *k <= 127), "keys past 127 are left out");
    }

    #[test]
    fn detect_names_chords_from_any_voicing() {
        let name = |keys: &[u8]| Chord::detect(keys).map(|c| c.to_string());
        assert_eq!(name(&[60, 64, 67]), Some(String::from("C")));
        assert_eq!(name(&[64, 55, 72]), Some(String::from("C/G")), "the lowest key is the bass");
        assert_eq!(name(&[57, 60, 64, 67]), Some(String::from("Am7")), "a root in the bass wins over C6");
        assert_eq!(name(&[60, 63, 66, 70]), Some(String::from("Cm7b5")));
        assert_eq!(name(&[62, 65, 69, 72, 76]), Some(String::from("Dm9")));
        assert_eq!(name(&[60, 64]), None, "two notes are an interval");
        assert_eq!(name(&[60, 61, 62]), None);
        assert_eq!(name(&[]), None);
    }
}
