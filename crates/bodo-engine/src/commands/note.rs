//! `note.*`: notes in patterns, as the piano roll edits them (project.md section 10.4).
//!
//! Notes are named by their id, a number unique within the pattern. Positions are pattern-local.

use std::collections::BTreeSet;

use bodo_kit_core::Tick;
use bodo_model::pattern::NoteSeq;
use bodo_model::{ChannelId, Note, NoteId, Pattern};
use serde::Deserialize;
use serde_json::{Value, json};

use super::by_notes;
use super::pattern::velocity;
use crate::error::CommandError;
use crate::params::{ChannelRef, Key, Length, PatternRef, Position, STEP};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NoteSpec {
    /// The key.
    key: Key,
    /// Where it starts, from the pattern's start.
    start: Position,
    /// How long it lasts; one step (a sixteenth note) by default.
    length: Option<Length>,
    /// Velocity from 0 to 1; 0.8 by default.
    velocity: Option<f32>,
    /// Pan from -1 to 1; 0 by default.
    pan: Option<f32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddMany {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The channel that plays them; the piano roll's channel by default.
    channel: Option<ChannelRef>,
    /// The notes to add.
    notes: Vec<NoteSpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Notes {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Move {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// Ticks to move them by, later when positive.
    #[serde(default)]
    ticks: i64,
    /// Semitones to move them by, up when positive.
    #[serde(default)]
    keys: i32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resize {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// The new length of every note.
    length: Option<Length>,
    /// Or: ticks to add to each note's length (negative shortens; a note keeps at least one tick).
    ticks: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetProps {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// A new start for every note.
    start: Option<Position>,
    /// A new key for every note.
    key: Option<Key>,
    /// A new length for every note.
    length: Option<Length>,
    /// A new velocity from 0 to 1.
    velocity: Option<f32>,
    /// A new pan from -1 to 1.
    pan: Option<f32>,
    /// Muted or not.
    muted: Option<bool>,
    /// A new release velocity from 0 to 1.
    release_velocity: Option<f32>,
    /// A new fine pitch in cents, from -1200 to 1200.
    fine_pitch_cents: Option<f32>,
    /// A new Mod X from 0 to 1.
    mod_x: Option<f32>,
    /// A new Mod Y from 0 to 1.
    mod_y: Option<f32>,
    /// Slide to the next note or not.
    slide: Option<bool>,
    /// Portamento or not.
    porta: Option<bool>,
}

/// The ids `ids` names, or else the selected notes.
pub(super) fn targets(ctx: &Ctx<'_>, ids: &[u32]) -> Result<BTreeSet<NoteId>, CommandError> {
    if ids.is_empty() {
        if ctx.editor.notes.is_empty() {
            return Err(CommandError::nothing_selected("notes"));
        }
        return Ok(ctx.editor.notes.clone());
    }
    Ok(ids.iter().map(|id| NoteId(*id)).collect())
}

/// Applies `f` to the notes `ids` of `pattern`, re-sorting each channel it touched. Every id must exist.
///
/// Complexity: O(n log n) in the notes of the channels touched.
fn edit_notes(pattern: &mut Pattern, ids: &BTreeSet<NoteId>, mut f: impl FnMut(&mut Note) -> Result<(), CommandError>) -> Result<bool, CommandError> {
    let mut found = BTreeSet::new();
    let mut changed = false;
    let channels: Vec<ChannelId> = pattern.notes.iter().filter(|(_, seq)| seq.notes().iter().any(|n| ids.contains(&n.id))).map(|(c, _)| *c).collect();
    for channel in channels {
        let Some(seq) = pattern.notes.get(&channel) else { continue };
        let mut notes: Vec<Note> = seq.notes().iter().cloned().collect();
        for note in notes.iter_mut().filter(|n| ids.contains(&n.id)) {
            let before = note.clone();
            f(note)?;
            changed |= *note != before;
            found.insert(note.id);
        }
        pattern.notes.insert(channel, NoteSeq::new(notes));
    }
    if let Some(missing) = ids.iter().find(|id| !found.contains(id)) {
        return Err(CommandError::not_found("note", &missing.0.to_string()).with_hint(format!("{} has no note with that id", pattern.name)));
    }
    Ok(changed)
}

fn add_many(ctx: &mut Ctx<'_>, p: AddMany) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let channel = ctx.channel(p.channel.as_ref())?;
    let mut prepared = Vec::with_capacity(p.notes.len());
    for spec in &p.notes {
        let start = spec.start.resolve(&ctx.project)?;
        if start < Tick::ZERO {
            return Err(CommandError::invalid("a note can't start before the pattern"));
        }
        let len = match &spec.length {
            Some(length) => length.resolve(&ctx.project, start)?,
            None => STEP,
        };
        let pan = spec.pan.unwrap_or(0.0);
        super::in_range("pan", f64::from(pan), -1.0..=1.0)?;
        prepared.push((spec.key.resolve()?, start, len, velocity(spec.velocity)?, pan));
    }
    let names = |ctx: &Ctx<'_>| ctx.project.channels.get(&channel).map(|c| c.name.clone()).unwrap_or_default();
    let channel_name = names(ctx);
    let added = ctx.edit_pattern(id, |pattern| {
        if let Some(bound) = pattern.binding.filter(|b| *b != channel) {
            return Err(CommandError::rejected(format!("{} is bound to another channel ({bound})", pattern.name)));
        }
        let mut added = Vec::with_capacity(prepared.len());
        for (key, start, len, velocity, pan) in &prepared {
            let note_id = NoteId(pattern.next_note_id);
            pattern.next_note_id += 1;
            let mut note = Note::new(note_id, *key, *start, *len);
            note.velocity = *velocity;
            note.pan = *pan;
            pattern.notes.entry(channel).or_default().insert(note);
            added.push(note_id);
        }
        Ok((!added.is_empty(), added))
    })?;
    ctx.editor.channel = Some(channel);
    ctx.editor.notes = added.iter().copied().collect();
    ctx.set_label(if added.len() == 1 { format!("Add note to {channel_name}") } else { format!("Add {} notes to {channel_name}", added.len()) });
    Ok(json!({ "pattern": id, "channel": channel, "notes": added.iter().map(|n| n.0).collect::<Vec<_>>() }))
}

fn remove(ctx: &mut Ctx<'_>, p: Notes) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let ids = targets(ctx, &p.notes)?;
    let removed = ctx.edit_pattern(id, |pattern| {
        let mut removed = 0;
        let channels: Vec<ChannelId> = pattern.notes.keys().copied().collect();
        for channel in channels {
            if let Some(seq) = pattern.notes.get(&channel)
                && seq.notes().iter().any(|n| ids.contains(&n.id))
            {
                let kept: Vec<Note> = seq.notes().iter().filter(|n| !ids.contains(&n.id)).cloned().collect();
                removed += seq.len() - kept.len();
                if kept.is_empty() {
                    pattern.notes.remove(&channel);
                } else {
                    pattern.notes.insert(channel, NoteSeq::new(kept));
                }
            }
        }
        if removed < ids.len() {
            return Err(CommandError::not_found("note", &format!("{:?}", ids.iter().map(|n| n.0).collect::<Vec<_>>())));
        }
        Ok((removed > 0, removed))
    })?;
    ctx.editor.notes.retain(|n| !ids.contains(n));
    ctx.set_label(if removed == 1 { String::from("Delete note") } else { format!("Delete {removed} notes") });
    Ok(json!({ "pattern": id, "removed": removed }))
}

fn move_notes(ctx: &mut Ctx<'_>, p: Move) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let ids = targets(ctx, &p.notes)?;
    ctx.edit_pattern(id, |pattern| {
        let changed = edit_notes(pattern, &ids, |note| {
            let start = note.start.0 + p.ticks;
            let key = i32::from(note.key) + p.keys;
            if start < 0 {
                return Err(CommandError::rejected("that would move a note before the pattern's start"));
            }
            note.key =
                u8::try_from(key).ok().filter(|k| *k <= 127).ok_or_else(|| CommandError::rejected("that would move a note out of the key range"))?;
            note.start = Tick(start);
            Ok(())
        })?;
        Ok((changed, ()))
    })?;
    ctx.set_label(if ids.len() == 1 { String::from("Move note") } else { format!("Move {} notes", ids.len()) });
    Ok(json!({ "pattern": id, "moved": ids.len() }))
}

fn resize(ctx: &mut Ctx<'_>, p: Resize) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let ids = targets(ctx, &p.notes)?;
    let length = p.length.as_ref().map(|l| l.resolve(&ctx.project, Tick::ZERO)).transpose()?;
    if length.is_none() && p.ticks.is_none() {
        return Err(CommandError::invalid("give length or ticks"));
    }
    ctx.edit_pattern(id, |pattern| {
        let changed = edit_notes(pattern, &ids, |note| {
            note.len = match (length, p.ticks) {
                (Some(len), _) => len,
                (None, Some(ticks)) => Tick((note.len.0 + ticks).max(1)),
                (None, None) => note.len,
            };
            Ok(())
        })?;
        Ok((changed, ()))
    })?;
    ctx.set_label(if ids.len() == 1 { String::from("Resize note") } else { format!("Resize {} notes", ids.len()) });
    Ok(json!({ "pattern": id, "resized": ids.len() }))
}

fn set_props(ctx: &mut Ctx<'_>, p: SetProps) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let ids = targets(ctx, &p.notes)?;
    let start = p.start.as_ref().map(|s| s.resolve(&ctx.project)).transpose()?;
    if start.is_some_and(|s| s < Tick::ZERO) {
        return Err(CommandError::invalid("a note can't start before the pattern"));
    }
    let key = p.key.as_ref().map(Key::resolve).transpose()?;
    let length = p.length.as_ref().map(|l| l.resolve(&ctx.project, start.unwrap_or(Tick::ZERO))).transpose()?;
    let vel = p.velocity.map(|v| velocity(Some(v))).transpose()?;
    let release = p.release_velocity.map(|v| velocity(Some(v))).transpose()?;
    if let Some(pan) = p.pan {
        super::in_range("pan", f64::from(pan), -1.0..=1.0)?;
    }
    if let Some(cents) = p.fine_pitch_cents {
        super::in_range("finePitchCents", f64::from(cents), -1200.0..=1200.0)?;
    }
    for (name, value) in [("modX", p.mod_x), ("modY", p.mod_y)] {
        if let Some(value) = value {
            super::in_range(name, f64::from(value), 0.0..=1.0)?;
        }
    }
    ctx.edit_pattern(id, |pattern| {
        let changed = edit_notes(pattern, &ids, |note| {
            if let Some(start) = start {
                note.start = start;
            }
            if let Some(key) = key {
                note.key = key;
            }
            if let Some(len) = length {
                note.len = len;
            }
            if let Some(v) = vel {
                note.velocity = v;
            }
            if let Some(pan) = p.pan {
                note.pan = pan;
            }
            if let Some(muted) = p.muted {
                note.muted = muted;
            }
            if let Some(v) = release {
                note.release_velocity = v;
            }
            if let Some(cents) = p.fine_pitch_cents {
                note.fine_pitch_cents = cents;
            }
            if let Some(x) = p.mod_x {
                note.mod_x = x;
            }
            if let Some(y) = p.mod_y {
                note.mod_y = y;
            }
            if let Some(slide) = p.slide {
                note.slide = slide;
            }
            if let Some(porta) = p.porta {
                note.porta = porta;
            }
            Ok(())
        })?;
        Ok((changed, ()))
    })?;
    ctx.set_label(if ids.len() == 1 { String::from("Edit note") } else { format!("Edit {} notes", ids.len()) });
    Ok(json!({ "pattern": id, "edited": ids.len() }))
}

/// Merge key: the pattern alone, so a brush stroke across a lane of many notes is one undo step; the gesture id keeps
/// separate strokes apart.
fn by_pattern(params: &Value) -> String {
    params.get("pattern").map(Value::to_string).unwrap_or_default()
}

pub(super) fn register(r: &mut Registry) {
    r.edit("note.add_many", "Add notes", "Adds notes for one channel to a pattern and selects them; returns their ids.", Effects::UNDOABLE, add_many)
        .example(json!({
            "channel": "Kick",
            "notes": [{ "key": "C4", "start": "1:1:0", "length": "1 beat" }, { "key": 64, "start": 960, "velocity": 0.6 }]
        }))
        .merge(by_pattern);
    r.edit("note.remove", "Delete notes", "Deletes notes from a pattern.", Effects::UNDOABLE, remove)
        .example(json!({ "notes": [0] }))
        .merge(by_pattern);
    r.edit("note.move", "Move notes", "Moves notes by ticks and semitones.", Effects::UNDOABLE, move_notes)
        .example(json!({ "notes": [0], "ticks": 240, "keys": 2 }))
        .merge(by_notes);
    r.edit("note.resize", "Resize notes", "Sets notes' length, or changes it by ticks.", Effects::UNDOABLE, resize)
        .example(json!({ "notes": [0], "length": "2 steps" }))
        .merge(by_notes);
    r.edit(
        "note.set_props",
        "Edit notes",
        "Sets start, key, length, velocity, release velocity, pan, fine pitch, Mod X and Y, slide, portamento or mute of notes.",
        Effects::UNDOABLE,
        set_props,
    )
    .example(json!({ "notes": [0], "velocity": 0.5, "modX": 0.25 }))
    .merge(by_pattern);
}
