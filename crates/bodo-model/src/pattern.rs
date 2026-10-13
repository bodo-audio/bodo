//! Patterns and notes (project.md sections 8.3 and 10.4).
//!
//! A pattern is a reusable musical idea: a page of notes for one or more channels, placed on the playlist as clips.

use std::sync::Arc;

use bodo_kit_core::Tick;
use imbl::{OrdMap, Vector};

use crate::automation::AutomationLane;
use crate::color::Color;
use crate::id::{ChannelId, NoteId, PatternId};

/// A pattern.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    /// The pattern's id.
    pub id: PatternId,
    /// Its name.
    pub name: String,
    /// Its color.
    pub color: Color,
    /// How long it is.
    #[serde(default)]
    pub length: PatternLen,
    /// `Some` makes it a clip-mode pattern, where only this channel may have notes.
    #[serde(default)]
    pub binding: Option<ChannelId>,
    /// Notes per channel.
    #[serde(default)]
    pub notes: OrdMap<ChannelId, NoteSeq>,
    /// Automation that travels with the pattern. At most one lane per target.
    #[serde(default)]
    pub automation: Vector<Arc<AutomationLane>>,
    /// The id the next new note gets.
    #[serde(default)]
    pub next_note_id: u32,
}

/// The length of a pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "ticks")]
pub enum PatternLen {
    /// Rounded up to whole bars around the last note.
    #[default]
    Auto,
    /// A fixed length.
    Fixed(Tick),
}

/// A note.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// Unique within its pattern; maps directly to a CLAP note id.
    pub id: NoteId,
    /// The MIDI key, 0 to 127.
    pub key: u8,
    /// Where it starts.
    pub start: Tick,
    /// How long it lasts; always positive.
    pub len: Tick,
    /// Velocity, 16-bit as in MIDI 2.0.
    #[serde(default = "default_velocity")]
    pub velocity: u16,
    /// Release velocity, 16-bit.
    #[serde(default = "default_velocity")]
    pub release_velocity: u16,
    /// Fine pitch in cents.
    #[serde(default)]
    pub fine_pitch_cents: f32,
    /// Pan, -1 to 1.
    #[serde(default)]
    pub pan: f32,
    /// Modulation X, 0 to 1.
    #[serde(default)]
    pub mod_x: f32,
    /// Modulation Y, 0 to 1.
    #[serde(default)]
    pub mod_y: f32,
    /// The note group, which doubles as the MIDI channel.
    #[serde(default)]
    pub color: u8,
    /// Whether the note is muted.
    #[serde(default)]
    pub muted: bool,
    /// A slide note.
    #[serde(default)]
    pub slide: bool,
    /// A portamento note.
    #[serde(default)]
    pub porta: bool,
    /// The grid position before humanization, so it can be undone.
    #[serde(default)]
    pub quantized_start: Option<Tick>,
    /// Per-note expression curves (MPE, CLAP note expressions).
    #[serde(default)]
    pub expr: Option<Arc<NoteExpression>>,
}

fn default_velocity() -> u16 {
    0x8000
}

impl Note {
    /// A plain note of `key` at `start` lasting `len`, at the default velocity.
    pub fn new(id: NoteId, key: u8, start: Tick, len: Tick) -> Self {
        Self {
            id,
            key,
            start,
            len,
            velocity: default_velocity(),
            release_velocity: default_velocity(),
            fine_pitch_cents: 0.0,
            pan: 0.0,
            mod_x: 0.0,
            mod_y: 0.0,
            color: 0,
            muted: false,
            slide: false,
            porta: false,
            quantized_start: None,
            expr: None,
        }
    }

    /// Where it ends.
    pub fn end(&self) -> Tick {
        self.start + self.len
    }

    /// The sort key of notes within a sequence: start, then key, then id.
    pub fn order(&self) -> (Tick, u8, NoteId) {
        (self.start, self.key, self.id)
    }
}

/// A point of an expression curve.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExprPoint {
    /// Ticks from the note's start.
    pub at: Tick,
    /// The value.
    pub value: f32,
}

/// Expression curves of one note.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteExpression {
    /// Pitch in semitones.
    pub pitch: Vec<ExprPoint>,
    /// Pressure, 0 to 1.
    pub pressure: Vec<ExprPoint>,
    /// Timbre (MPE slide), 0 to 1.
    pub timbre: Vec<ExprPoint>,
}

/// One channel's notes in a pattern, sorted by start, key and id.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "Vector<Note>", into = "Vector<Note>")]
pub struct NoteSeq {
    notes: Vector<Note>,
    /// The longest note, which bounds how far back an overlap query must look.
    max_len: Tick,
}

impl NoteSeq {
    /// A sequence of `notes`, sorted as needed.
    ///
    /// Complexity: O(n log n) in the number of notes.
    pub fn new(notes: impl IntoIterator<Item = Note>) -> Self {
        let mut list: Vec<Note> = notes.into_iter().collect();
        list.sort_by_key(Note::order);
        let max_len = list.iter().map(|n| n.len).max().unwrap_or(Tick::ZERO);
        Self { notes: list.into_iter().collect(), max_len }
    }

    /// The notes, sorted by start, key and id.
    pub fn notes(&self) -> &Vector<Note> {
        &self.notes
    }

    /// The number of notes.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Whether there are no notes.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// The longest note.
    pub fn max_len(&self) -> Tick {
        self.max_len
    }

    /// Adds a note in its sorted place.
    ///
    /// Complexity: O(log n) in the number of notes.
    pub fn insert(&mut self, note: Note) {
        let index = self.notes.binary_search_by(|n| n.order().cmp(&note.order())).unwrap_or_else(|i| i);
        self.max_len = self.max_len.max(note.len);
        self.notes.insert(index, note);
    }

    /// Removes the note with `id`, and returns it.
    ///
    /// Complexity: O(n) in the number of notes, to find it by id.
    pub fn remove(&mut self, id: NoteId) -> Option<Note> {
        let index = self.notes.iter().position(|n| n.id == id)?;
        let note = self.notes.remove(index);
        if note.len == self.max_len {
            self.max_len = self.notes.iter().map(|n| n.len).max().unwrap_or(Tick::ZERO);
        }
        Some(note)
    }

    /// Every note that sounds during `start..end`: it starts before `end` and ends after `start`.
    ///
    /// Complexity: O(log n + k), scanning only starts in `start - max_len..end` (project.md section 10.1).
    pub fn overlapping(&self, start: Tick, end: Tick) -> impl Iterator<Item = &Note> {
        let earliest = start - self.max_len;
        let from = self
            .notes
            .binary_search_by(|n| if n.start <= earliest { core::cmp::Ordering::Less } else { core::cmp::Ordering::Greater })
            .unwrap_or_else(|i| i);
        self.notes.focus().narrow(from..).into_iter().take_while(move |n| n.start < end).filter(move |n| n.end() > start)
    }

    /// Whether the notes are sorted and every length is positive.
    pub(crate) fn is_well_formed(&self) -> bool {
        self.notes.iter().all(|n| n.len > Tick::ZERO)
            && self.notes.iter().zip(self.notes.iter().skip(1)).all(|(a, b)| a.order() < b.order())
            && self.notes.iter().map(|n| n.len).max().unwrap_or(Tick::ZERO) <= self.max_len
    }
}

impl From<Vector<Note>> for NoteSeq {
    fn from(notes: Vector<Note>) -> Self {
        let max_len = notes.iter().map(|n| n.len).max().unwrap_or(Tick::ZERO);
        // Files are expected to be sorted already; validation reports it if they aren't.
        Self { notes, max_len }
    }
}

impl From<NoteSeq> for Vector<Note> {
    fn from(seq: NoteSeq) -> Self {
        seq.notes
    }
}

impl Pattern {
    /// An empty pattern.
    pub fn new(id: PatternId, name: impl Into<String>, color: Color) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            length: PatternLen::Auto,
            binding: None,
            notes: OrdMap::new(),
            automation: Vector::new(),
            next_note_id: 0,
        }
    }

    /// The end of the last note, or zero for an empty pattern.
    ///
    /// Complexity: O(c log n) in the channels and notes, since each sequence is sorted by start.
    pub fn content_end(&self) -> Tick {
        self.notes.values().filter_map(|seq| seq.notes.iter().map(Note::end).max()).max().unwrap_or(Tick::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32, key: u8, start: i64, len: i64) -> Note {
        Note::new(NoteId(id), key, Tick(start), Tick(len))
    }

    #[test]
    fn insert_keeps_order_and_overlap_finds_long_notes() {
        let mut seq = NoteSeq::new([note(0, 60, 0, 3840), note(1, 64, 960, 240)]);
        seq.insert(note(2, 62, 480, 240));
        let starts: Vec<i64> = seq.notes().iter().map(|n| n.start.0).collect();
        assert_eq!(starts, [0, 480, 960]);
        let found: Vec<u32> = seq.overlapping(Tick(2000), Tick(2100)).map(|n| n.id.0).collect();
        assert_eq!(found, [0], "the long first note is still sounding");
        let found: Vec<u32> = seq.overlapping(Tick(500), Tick(1000)).map(|n| n.id.0).collect();
        assert_eq!(found, [0, 2, 1]);
        assert!(seq.is_well_formed());
        assert_eq!(seq.remove(NoteId(0)).map(|n| n.id), Some(NoteId(0)));
        assert_eq!(seq.max_len(), Tick(240));
    }
}
