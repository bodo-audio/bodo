//! Parameter types shared by many commands: positions, lengths, keys, colors and references to entities.
//!
//! Agents and people name things the way they think of them (project.md sections 8.2 and 12.1):
//! - a position as ticks, `bar:beat:tick` or seconds;
//! - a length as ticks or `2 bars`, `1 beat`, `4 steps`;
//! - a key as a MIDI number or a note name such as `C4` or `F#3`;
//! - an entity by id (`ch_01J...`), by 1-based number (`channel:3`, `insert:2`, or just `3`), or by name (`Kick`).

use std::borrow::Cow;
use std::str::FromStr;

use bodo_kit_core::{PPQ, Seconds, Tick};
use bodo_model::{Color, Project};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::error::CommandError;

/// The length of one step of the step sequencer: a sixteenth note.
pub const STEP: Tick = Tick(PPQ / 4);

/// A point in time.
///
/// As JSON: a number of ticks (960 per quarter note), or text: `"bar:beat:tick"` with bar and beat counted from 1
/// (`"3:1:0"` is the start of bar 3), `"bar:beat"`, `"bar"`, or seconds such as `"1.5s"`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Position {
    /// Ticks from the start.
    Ticks(i64),
    /// Bars, beats and ticks, or seconds.
    Text(String),
}

impl Position {
    /// The tick this position names in `project`.
    ///
    /// Complexity: O(log m) in meter and tempo changes.
    pub fn resolve(&self, project: &Project) -> Result<Tick, CommandError> {
        match self {
            Self::Ticks(ticks) => Ok(Tick(*ticks)),
            Self::Text(text) => parse_position(project, text.trim()).ok_or_else(|| {
                CommandError::invalid(format!("`{text}` is not a position"))
                    .with_hint("use ticks (960 per beat), \"bar:beat:tick\" counted from 1 such as \"2:1:0\", or seconds such as \"1.5s\"")
            }),
        }
    }
}

fn parse_position(project: &Project, text: &str) -> Option<Tick> {
    if let Some(seconds) = text.strip_suffix('s') {
        let seconds: f64 = seconds.trim().parse().ok().filter(|s: &f64| s.is_finite() && *s >= 0.0)?;
        return Some(Tick(project.tempo.ticks_at(Seconds(seconds)).round() as i64));
    }
    let mut parts = text.split(':').map(|p| p.trim().parse::<i64>());
    let bar = parts.next()?.ok()?;
    let beat = parts.next().transpose().ok()?.unwrap_or(1);
    let tick = parts.next().transpose().ok()?.unwrap_or(0);
    if parts.next().is_some() || bar < 1 || beat < 1 || tick < 0 {
        return None;
    }
    let start = project.meter.bar_start(bar - 1);
    let beat_ticks = project.meter.signature_at(start).beat_ticks();
    Some(Tick(start.0 + (beat - 1) * beat_ticks + tick))
}

/// `tick` as `bar:beat:tick` text, counted from 1 like the song position display: `1:1:0` is the start.
///
/// Complexity: O(log m) in meter changes.
pub fn format_position(project: &Project, tick: Tick) -> String {
    let at = project.meter.position(tick);
    format!("{}:{}:{}", at.bar + 1, at.beat + 1, at.tick)
}

/// `len` in the largest whole unit that fits: `2 bars`, `3 beats`, `5 steps` or `100 ticks`.
pub fn format_length(project: &Project, len: Tick) -> String {
    let bar = project.meter.signature_at(Tick::ZERO).bar_ticks();
    let beat = project.meter.signature_at(Tick::ZERO).beat_ticks();
    let plural = |n: i64, unit: &str| if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") };
    if bar > 0 && len.0 % bar == 0 {
        plural(len.0 / bar, "bar")
    } else if beat > 0 && len.0 % beat == 0 {
        plural(len.0 / beat, "beat")
    } else if len.0 % STEP.0 == 0 {
        plural(len.0 / STEP.0, "step")
    } else {
        plural(len.0, "tick")
    }
}

impl JsonSchema for Position {
    fn schema_name() -> Cow<'static, str> {
        "Position".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A point in time: ticks as a number (960 per quarter note), or text: \"bar:beat:tick\" counted from 1 (\"3:1:0\" is the start of bar 3), \"bar:beat\", \"bar\", or seconds such as \"1.5s\".",
            "anyOf": [{ "type": "integer" }, { "type": "string" }],
        })
    }
}

/// A duration.
///
/// As JSON: a number of ticks (960 per quarter note), or text: a count and a unit, such as `"2 bars"`, `"1 beat"`,
/// `"4 steps"` (sixteenth notes) or `"120 ticks"`. Counts may be fractional (`"0.5 bar"`).
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Length {
    /// Ticks.
    Ticks(i64),
    /// A count and a unit.
    Text(String),
}

impl Length {
    /// The length in ticks, measured with the meter in force at `at`. Always positive.
    ///
    /// Complexity: O(log m) in meter changes.
    pub fn resolve(&self, project: &Project, at: Tick) -> Result<Tick, CommandError> {
        let ticks = match self {
            Self::Ticks(ticks) => Some(*ticks),
            Self::Text(text) => parse_length(project, at, text),
        };
        match ticks {
            Some(ticks) if ticks > 0 => Ok(Tick(ticks)),
            Some(_) => Err(CommandError::invalid("a length must be positive")),
            None => Err(CommandError::invalid(format!("`{}` is not a length", self.text()))
                .with_hint("use ticks (960 per beat) or a count and a unit such as \"2 bars\", \"1 beat\" or \"4 steps\"")),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Ticks(ticks) => ticks.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

fn parse_length(project: &Project, at: Tick, text: &str) -> Option<i64> {
    let text = text.trim().to_ascii_lowercase();
    let split = text.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(text.len());
    let count: f64 = text[..split].trim().parse().ok().filter(|c: &f64| c.is_finite())?;
    let signature = project.meter.signature_at(at);
    let unit = match text[split..].trim().trim_end_matches('s') {
        "bar" => signature.bar_ticks(),
        "beat" => signature.beat_ticks(),
        "step" => STEP.0,
        "tick" | "" => 1,
        _ => return None,
    };
    Some((count * unit as f64).round() as i64)
}

impl JsonSchema for Length {
    fn schema_name() -> Cow<'static, str> {
        "Length".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A duration: ticks as a number (960 per quarter note), or a count and a unit such as \"2 bars\", \"1 beat\", \"4 steps\" (sixteenth notes) or \"120 ticks\".",
            "anyOf": [{ "type": "integer", "exclusiveMinimum": 0 }, { "type": "string" }],
        })
    }
}

/// A MIDI key: a number from 0 to 127, or a note name such as `C4` (60, middle C), `F#3` or `Bb2`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Key {
    /// The MIDI key number.
    Number(i64),
    /// A note name with an octave, where C4 is 60.
    Name(String),
}

impl Key {
    /// The MIDI key number, 0 to 127.
    pub fn resolve(&self) -> Result<u8, CommandError> {
        let key = match self {
            Self::Number(n) => Some(*n),
            Self::Name(name) => parse_key(name.trim()),
        };
        key.and_then(|k| u8::try_from(k).ok()).filter(|k| *k <= 127).ok_or_else(|| {
            let text = match self {
                Self::Number(n) => n.to_string(),
                Self::Name(name) => name.clone(),
            };
            CommandError::invalid(format!("`{text}` is not a key from 0 to 127")).with_hint("use a MIDI number or a note name such as C4 (60) or F#3")
        })
    }
}

fn parse_key(name: &str) -> Option<i64> {
    let mut chars = name.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (shift, octave) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') if rest.len() > 1 => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let octave: i64 = octave.parse().ok()?;
    Some((octave + 1) * 12 + base + shift)
}

/// The name of MIDI key `key`, such as `C4` for 60.
pub fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[usize::from(key % 12)], i32::from(key / 12) - 1)
}

impl JsonSchema for Key {
    fn schema_name() -> Cow<'static, str> {
        "Key".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A MIDI key: a number from 0 to 127, or a note name such as \"C4\" (60, middle C), \"F#3\" or \"Bb2\".",
            "anyOf": [{ "type": "integer", "minimum": 0, "maximum": 127 }, { "type": "string" }],
        })
    }
}

/// A color as `#rrggbb` text.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct ColorText(pub String);

impl ColorText {
    /// The color.
    pub fn resolve(&self) -> Result<Color, CommandError> {
        Color::from_str(self.0.trim()).map_err(|e| CommandError::invalid(e.to_string()))
    }
}

impl JsonSchema for ColorText {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "description": "A color as \"#rrggbb\".", "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" })
    }
}

/// Text that names an entity: an id, a 1-based number, or a name. JSON numbers are numbers.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum RefText {
    /// A 1-based number.
    Number(u64),
    /// An id, `kind:N`, a number, or a name.
    Text(String),
}

impl RefText {
    /// The reference as text, numbers included.
    pub fn as_text(&self) -> Cow<'_, str> {
        match self {
            Self::Number(n) => Cow::Owned(n.to_string()),
            Self::Text(text) => Cow::Borrowed(text.trim()),
        }
    }
}

macro_rules! entity_ref {
    ($(#[$doc:meta])* $name:ident, $schema:literal, $description:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
        #[serde(transparent)]
        pub struct $name(pub RefText);

        impl $name {
            /// A reference by id, number or name.
            pub fn new(text: impl Into<String>) -> Self {
                Self(RefText::Text(text.into()))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                $schema.into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "description": $description, "anyOf": [{ "type": "string" }, { "type": "integer", "minimum": 1 }] })
            }
        }
    };
}

entity_ref!(
    /// A channel of the channel rack.
    ChannelRef,
    "ChannelRef",
    "A channel: its id (\"ch_01J...\"), its number in the channel rack (\"channel:3\" or 3), or its name (\"Kick\")."
);
entity_ref!(
    /// A pattern.
    PatternRef,
    "PatternRef",
    "A pattern: its id (\"pat_01J...\"), its number (\"pattern:2\" or 2), or its name (\"Pattern 2\")."
);
entity_ref!(
    /// A playlist track.
    TrackRef,
    "TrackRef",
    "A playlist track: its id (\"trk_01J...\"), its number (\"track:1\" or 1), or its name (\"Track 1\")."
);
entity_ref!(
    /// A mixer insert or the master.
    InsertRef,
    "InsertRef",
    "A mixer insert: its id (\"ins_01J...\"), its number (\"insert:3\" or 3; 0 or \"master\" is the master), or its name."
);
entity_ref!(
    /// A clip on the playlist.
    ClipRef,
    "ClipRef",
    "A clip on the playlist, by its id (\"clip_01J...\")."
);
entity_ref!(
    /// The source of a mixer route: a channel or an insert.
    RouteFromRef,
    "RouteFromRef",
    "Where a route starts: a channel or an insert, by id, \"channel:N\", \"insert:N\" or name."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_read_ticks_bars_and_seconds() {
        let project = Project::new(120.0).unwrap();
        let at = |text: &str| Position::Text(text.into()).resolve(&project);
        assert_eq!(Position::Ticks(42).resolve(&project), Ok(Tick(42)));
        assert_eq!(at("1:1:0"), Ok(Tick(0)));
        assert_eq!(at("3"), Ok(Tick(2 * 3840)));
        assert_eq!(at("2:3:120"), Ok(Tick(3840 + 2 * 960 + 120)));
        assert_eq!(at("1s"), Ok(Tick(1920)), "two beats per second at 120 BPM");
        assert!(at("0:1").is_err() && at("x").is_err() && at("1:1:1:1").is_err());
        assert_eq!(format_position(&project, Tick(3840 + 2 * 960 + 120)), "2:3:120");
    }

    #[test]
    fn lengths_read_units() {
        let project = Project::new(120.0).unwrap();
        let len = |text: &str| Length::Text(text.into()).resolve(&project, Tick::ZERO);
        assert_eq!(len("2 bars"), Ok(Tick(7680)));
        assert_eq!(len("1 beat"), Ok(Tick(960)));
        assert_eq!(len("4 steps"), Ok(Tick(960)));
        assert_eq!(len("0.5 bar"), Ok(Tick(1920)));
        assert_eq!(len("120"), Ok(Tick(120)));
        assert!(len("0 bars").is_err() && len("2 parsecs").is_err());
        assert_eq!(format_length(&project, Tick(7680)), "2 bars");
        assert_eq!(format_length(&project, Tick(720)), "3 steps");
    }

    #[test]
    fn keys_read_numbers_and_names() {
        assert_eq!(Key::Name("C4".into()).resolve(), Ok(60));
        assert_eq!(Key::Name("F#3".into()).resolve(), Ok(54));
        assert_eq!(Key::Name("Bb2".into()).resolve(), Ok(46));
        assert_eq!(Key::Name("C-1".into()).resolve(), Ok(0));
        assert_eq!(Key::Number(127).resolve(), Ok(127));
        assert!(Key::Number(128).resolve().is_err() && Key::Name("H2".into()).resolve().is_err());
        assert_eq!(key_name(61), "C#4");
    }
}
