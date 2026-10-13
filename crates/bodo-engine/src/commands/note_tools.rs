//! Piano roll tools beyond drawing: quantize, transpose, legato, chop, glue, strum, arpeggiate, flip, humanize, scales,
//! chord stamps and note expression (project.md sections 10.4 and 13.5).
//!
//! Each tool acts on the given notes, or the selected notes of the current pattern, channel by channel, and is one
//! undo step. Tools that make new notes (chop, arpeggiate, stamp) select them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use bodo_kit_core::{Seconds, Tick};
use bodo_kit_music::{ArpPattern, Chord, Direction, Humanizer, PitchClass, Scale, ScaleKind, arp_order, quantize, strum_offsets};
use bodo_model::pattern::{ExprPoint, NoteExpression, NoteSeq};
use bodo_model::{ChannelId, Note, NoteId, Pattern, PatternId, Project};
use serde::Deserialize;
use serde_json::{Value, json};

use super::in_range;
use super::note::targets;
use super::pattern::velocity;
use crate::error::CommandError;
use crate::params::{ChannelRef, Length, PatternRef, Position, STEP};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Quantize {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// The grid; one step (a sixteenth note) by default.
    grid: Option<Length>,
    /// How far notes move toward the grid, from 0 (not at all) to 1 (onto it); 1 by default.
    strength: Option<f64>,
    /// Quantize the ends too, not only the starts.
    #[serde(default)]
    ends: bool,
    /// Moves every second grid line later by up to half a grid, from 0 to 1; 0 by default.
    swing: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transpose {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// Semitones to move by, up when positive.
    semitones: i32,
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
struct Chop {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// The length of each piece.
    length: Length,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Slice {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// Where to cut. Notes that don't cross it stay whole.
    at: Position,
}

/// Which way a strum runs.
#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum StrumDirection {
    /// From the lowest key up.
    #[default]
    Up,
    /// From the highest key down.
    Down,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Strum {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// How much later each voice of a chord starts than the one before; 30 ticks by default.
    time: Option<Length>,
    /// Up from the lowest key, or down from the highest; up by default.
    #[serde(default)]
    direction: StrumDirection,
}

/// The order an arpeggio plays a chord in.
#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ArpOrder {
    /// Lowest to highest.
    #[default]
    Up,
    /// Highest to lowest.
    Down,
    /// Up, then down.
    UpDown,
    /// Down, then up.
    DownUp,
    /// A random voice each step, from the seed.
    Random,
    /// In the order the notes were drawn.
    AsPlayed,
}

impl From<ArpOrder> for ArpPattern {
    fn from(order: ArpOrder) -> Self {
        match order {
            ArpOrder::Up => Self::Up,
            ArpOrder::Down => Self::Down,
            ArpOrder::UpDown => Self::UpDown,
            ArpOrder::DownUp => Self::DownUp,
            ArpOrder::Random => Self::Random,
            ArpOrder::AsPlayed => Self::AsPlayed,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Arpeggiate {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// The order; up by default.
    #[serde(default)]
    order: ArpOrder,
    /// How often a new note starts; one step by default.
    rate: Option<Length>,
    /// How long each note lasts, as a fraction of the rate from 0 to 1; 1 by default.
    gate: Option<f64>,
    /// The seed of the random order; 1 by default.
    seed: Option<u64>,
}

/// What flip mirrors.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Axis {
    /// Reverse the notes in time within their span.
    Time,
    /// Mirror the keys around the middle of the notes' range.
    Keys,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Flip {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// Time or keys.
    axis: Axis,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Humanize {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// How far starts drift, in milliseconds (the drift's standard deviation), from 0 to 200; 10 by default.
    timing_ms: Option<f64>,
    /// How much velocities vary, as a fraction from 0 to 1; 0.1 by default.
    velocity: Option<f64>,
    /// The seed; the same seed gives the same performance. 1 by default.
    seed: Option<u64>,
}

/// A scale's root: a note name such as `C` or `F#`, or a MIDI key.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum Root {
    /// A MIDI key; only its pitch class counts.
    Key(u8),
    /// A note name, with or without an octave.
    Name(String),
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LimitToScale {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note ids; the selected notes by default.
    #[serde(default)]
    notes: Vec<u32>,
    /// The scale's root, such as "C" or "F#".
    root: Root,
    /// The scale: major, minor, harmonic minor, melodic minor, dorian, phrygian, lydian, mixolydian, locrian, major
    /// pentatonic, minor pentatonic, blues, whole tone or chromatic.
    scale: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stamp {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The channel that plays it; the piano roll's channel by default.
    channel: Option<ChannelRef>,
    /// The chord symbol, such as "Am7", "Fmaj7", "G7sus4" or "C/E".
    chord: String,
    /// The octave of the root, where C4 is middle C; 4 by default.
    octave: Option<i32>,
    /// Where the chord starts.
    start: Position,
    /// How long it lasts; one beat by default.
    length: Option<Length>,
    /// Velocity from 0 to 1; 0.8 by default.
    velocity: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurvePoint {
    /// Ticks from the note's start.
    at: i64,
    /// The value: semitones for pitch, 0 to 1 for pressure and timbre.
    value: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetExpression {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The note's id.
    note: u32,
    /// The pitch curve in semitones; an empty list clears it, leaving it out keeps it.
    pitch: Option<Vec<CurvePoint>>,
    /// The pressure curve, 0 to 1.
    pressure: Option<Vec<CurvePoint>>,
    /// The timbre (MPE slide) curve, 0 to 1.
    timbre: Option<Vec<CurvePoint>>,
}

/// Replaces the notes `ids` of `pattern` channel by channel: `f` gets one channel's chosen notes, sorted by start, key
/// and id, and the pattern's next note id, and returns the notes to put in their place. Every id must exist. Returns
/// whether anything changed and the ids of the notes that took their place.
///
/// Complexity: O(n log n) in the notes of the channels touched, plus `f`.
fn rewrite(
    pattern: &mut Pattern,
    ids: &BTreeSet<NoteId>,
    mut f: impl FnMut(ChannelId, Vec<Note>, &mut u32) -> Result<Vec<Note>, CommandError>,
) -> Result<(bool, BTreeSet<NoteId>), CommandError> {
    let mut found = 0;
    let mut changed = false;
    let mut placed = BTreeSet::new();
    let channels: Vec<ChannelId> = pattern.notes.iter().filter(|(_, seq)| seq.notes().iter().any(|n| ids.contains(&n.id))).map(|(c, _)| *c).collect();
    for channel in channels {
        let Some(seq) = pattern.notes.get(&channel) else { continue };
        let (chosen, mut kept): (Vec<Note>, Vec<Note>) = seq.notes().iter().cloned().partition(|n| ids.contains(&n.id));
        found += chosen.len();
        let mut next = pattern.next_note_id;
        let mut replaced = f(channel, chosen.clone(), &mut next)?;
        for note in &replaced {
            if note.start < Tick::ZERO {
                return Err(CommandError::rejected("that would move a note before the pattern's start"));
            }
            if note.key > 127 {
                return Err(CommandError::rejected("that would move a note out of the key range"));
            }
            if note.len <= Tick::ZERO {
                return Err(CommandError::rejected("that would leave a note with no length"));
            }
        }
        replaced.sort_by_key(Note::order);
        changed |= replaced != chosen || next != pattern.next_note_id;
        pattern.next_note_id = next;
        placed.extend(replaced.iter().map(|n| n.id));
        kept.extend(replaced);
        if kept.is_empty() {
            pattern.notes.remove(&channel);
        } else {
            pattern.notes.insert(channel, NoteSeq::new(kept));
        }
    }
    if found < ids.len() {
        let missing: Vec<u32> = ids.iter().map(|n| n.0).collect();
        return Err(CommandError::not_found("note", &format!("{missing:?}")).with_hint(format!("{} doesn't have all of those notes", pattern.name)));
    }
    Ok((changed, placed))
}

/// Runs a tool on the chosen notes of the chosen pattern and selects the notes that result.
fn run_tool(
    ctx: &mut Ctx<'_>,
    pattern: Option<&PatternRef>,
    notes: &[u32],
    label: String,
    f: impl FnMut(ChannelId, Vec<Note>, &mut u32) -> Result<Vec<Note>, CommandError>,
) -> Result<Value, CommandError> {
    let id = ctx.pattern(pattern)?;
    let ids = targets(ctx, notes)?;
    let (changed, placed) = ctx.edit_pattern(id, |p| rewrite(p, &ids, f).map(|(changed, placed)| (changed, (changed, placed))))?;
    ctx.editor.notes = placed.clone();
    ctx.set_label(label);
    Ok(json!({ "pattern": id, "notes": placed.iter().map(|n| n.0).collect::<Vec<_>>(), "changed": changed }))
}

/// The notes of a channel grouped into chords: notes that start together, in time order.
fn chords(notes: Vec<Note>) -> BTreeMap<Tick, Vec<Note>> {
    let mut chords: BTreeMap<Tick, Vec<Note>> = BTreeMap::new();
    for note in notes {
        chords.entry(note.start).or_default().push(note);
    }
    chords
}

/// A new note id from the pattern's counter.
fn fresh(next: &mut u32) -> NoteId {
    let id = NoteId(*next);
    *next += 1;
    id
}

/// Every chosen note of `pattern`, across its channels.
fn chosen<'a>(project: &'a Project, pattern: PatternId, ids: &'a BTreeSet<NoteId>) -> impl Iterator<Item = &'a Note> + 'a {
    project
        .patterns
        .get(&pattern)
        .into_iter()
        .flat_map(move |p| p.notes.values().flat_map(move |seq| seq.notes().iter().filter(move |n| ids.contains(&n.id))))
}

fn quantize_notes(ctx: &mut Ctx<'_>, p: Quantize) -> Result<Value, CommandError> {
    let grid = p.grid.as_ref().map(|g| g.resolve(&ctx.project, Tick::ZERO)).transpose()?.unwrap_or(STEP);
    let strength = p.strength.unwrap_or(1.0);
    let swing = p.swing.unwrap_or(0.0);
    in_range("strength", strength, 0.0..=1.0)?;
    in_range("swing", swing, 0.0..=1.0)?;
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Quantize notes"), |_, notes, _| {
        Ok(notes
            .into_iter()
            .map(|mut note| {
                let end = note.end();
                let start = quantize(note.start, grid, strength, swing).max(Tick::ZERO);
                note.quantized_start = Some(quantize(note.start, grid, 1.0, swing).max(Tick::ZERO));
                note.start = start;
                if p.ends {
                    let end = quantize(end, grid, strength, 0.0);
                    note.len = if end > start { Tick(end.0 - start.0) } else { grid };
                }
                note
            })
            .collect())
    })
}

fn transpose(ctx: &mut Ctx<'_>, p: Transpose) -> Result<Value, CommandError> {
    let label = format!("Transpose notes by {:+}", p.semitones);
    run_tool(ctx, p.pattern.as_ref(), &p.notes, label, |_, notes, _| {
        notes
            .into_iter()
            .map(|mut note| {
                let key = i32::from(note.key) + p.semitones;
                note.key = u8::try_from(key)
                    .ok()
                    .filter(|k| *k <= 127)
                    .ok_or_else(|| CommandError::rejected("that would move a note out of the key range"))?;
                Ok(note)
            })
            .collect()
    })
}

fn legato(ctx: &mut Ctx<'_>, p: Notes) -> Result<Value, CommandError> {
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Legato"), |_, notes, _| {
        let starts: BTreeSet<Tick> = notes.iter().map(|n| n.start).collect();
        Ok(notes
            .into_iter()
            .map(|mut note| {
                if let Some(next) = starts.range(Tick(note.start.0 + 1)..).next() {
                    note.len = Tick(next.0 - note.start.0);
                }
                note
            })
            .collect())
    })
}

fn chop(ctx: &mut Ctx<'_>, p: Chop) -> Result<Value, CommandError> {
    let piece = p.length.resolve(&ctx.project, Tick::ZERO)?;
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Chop notes"), |_, notes, next| {
        let mut out = Vec::with_capacity(notes.len());
        for note in notes {
            let end = note.end();
            let mut first = note.clone();
            first.len = piece.min(note.len);
            out.push(first);
            let mut at = note.start.0 + piece.0;
            while at < end.0 {
                let mut part = note.clone();
                part.id = fresh(next);
                part.start = Tick(at);
                part.len = Tick(piece.0.min(end.0 - at));
                part.quantized_start = None;
                part.expr = None;
                out.push(part);
                at += piece.0;
            }
        }
        Ok(out)
    })
}

fn slice(ctx: &mut Ctx<'_>, p: Slice) -> Result<Value, CommandError> {
    let at = p.at.resolve(&ctx.project)?;
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Slice notes"), |_, notes, next| {
        let mut out = Vec::with_capacity(notes.len() * 2);
        for note in notes {
            if note.start < at && at < note.end() {
                let mut right = note.clone();
                right.id = fresh(next);
                right.start = at;
                right.len = Tick(note.end().0 - at.0);
                right.quantized_start = None;
                right.expr = None;
                let mut left = note;
                left.len = Tick(at.0 - left.start.0);
                out.push(left);
                out.push(right);
            } else {
                out.push(note);
            }
        }
        Ok(out)
    })
}

fn glue(ctx: &mut Ctx<'_>, p: Notes) -> Result<Value, CommandError> {
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Glue notes"), |_, notes, _| {
        let mut by_key: BTreeMap<u8, Vec<Note>> = BTreeMap::new();
        for note in notes {
            by_key.entry(note.key).or_default().push(note);
        }
        let mut out = Vec::new();
        for (_, mut run) in by_key {
            run.sort_by_key(Note::order);
            let mut iter = run.into_iter();
            let Some(mut current) = iter.next() else { continue };
            for note in iter {
                if note.start <= current.end() {
                    let end = current.end().max(note.end());
                    current.len = Tick(end.0 - current.start.0);
                } else {
                    out.push(std::mem::replace(&mut current, note));
                }
            }
            out.push(current);
        }
        Ok(out)
    })
}

fn strum(ctx: &mut Ctx<'_>, p: Strum) -> Result<Value, CommandError> {
    let per_voice = p.time.as_ref().map(|t| t.resolve(&ctx.project, Tick::ZERO)).transpose()?.unwrap_or(Tick(30));
    let direction = match p.direction {
        StrumDirection::Up => Direction::Up,
        StrumDirection::Down => Direction::Down,
    };
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Strum"), |_, notes, _| {
        let mut out = Vec::new();
        for (_, mut chord) in chords(notes) {
            chord.sort_by_key(|n| (n.key, n.id));
            let offsets = strum_offsets(chord.len(), per_voice, direction);
            for (mut note, offset) in chord.into_iter().zip(offsets) {
                let end = note.end();
                note.start = Tick(note.start.0 + offset.0);
                note.len = Tick((end.0 - note.start.0).max(1));
                out.push(note);
            }
        }
        Ok(out)
    })
}

fn arpeggiate(ctx: &mut Ctx<'_>, p: Arpeggiate) -> Result<Value, CommandError> {
    let rate = p.rate.as_ref().map(|r| r.resolve(&ctx.project, Tick::ZERO)).transpose()?.unwrap_or(STEP);
    let gate = p.gate.unwrap_or(1.0);
    in_range("gate", gate, 0.0..=1.0)?;
    let seed = p.seed.unwrap_or(1);
    let order = ArpPattern::from(p.order);
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Arpeggiate"), |_, notes, next| {
        let mut out = Vec::new();
        for (start, mut chord) in chords(notes) {
            if order == ArpPattern::AsPlayed {
                chord.sort_by_key(|n| n.id);
            } else {
                chord.sort_by_key(|n| (n.key, n.id));
            }
            let end = chord.iter().map(Note::end).max().unwrap_or(start);
            let span = end.0 - start.0;
            let steps = usize::try_from((span + rate.0 - 1) / rate.0).unwrap_or(1).max(1);
            let length = ((rate.0 as f64 * gate).round() as i64).max(1);
            for (step, voice) in arp_order(chord.len(), order, steps, seed).into_iter().enumerate() {
                let Some(source) = chord.get(voice) else { continue };
                let at = start.0 + step as i64 * rate.0;
                let mut note = source.clone();
                note.id = fresh(next);
                note.start = Tick(at);
                note.len = Tick(length.min(end.0 - at).max(1));
                note.quantized_start = None;
                note.expr = None;
                out.push(note);
            }
        }
        Ok(out)
    })
}

fn flip(ctx: &mut Ctx<'_>, p: Flip) -> Result<Value, CommandError> {
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let ids = targets(ctx, &p.notes)?;
    let all: Vec<&Note> = chosen(&ctx.project, pattern, &ids).collect();
    let (first, last) = (all.iter().map(|n| n.start.0).min().unwrap_or(0), all.iter().map(|n| n.end().0).max().unwrap_or(0));
    let (low, high) = (all.iter().map(|n| n.key).min().unwrap_or(0), all.iter().map(|n| n.key).max().unwrap_or(0));
    let label = match p.axis {
        Axis::Time => "Flip notes in time",
        Axis::Keys => "Flip notes upside down",
    };
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from(label), |_, notes, _| {
        Ok(notes
            .into_iter()
            .map(|mut note| {
                match p.axis {
                    Axis::Time => note.start = Tick(first + last - note.end().0),
                    Axis::Keys => note.key = low + high - note.key,
                }
                note
            })
            .collect())
    })
}

fn humanize(ctx: &mut Ctx<'_>, p: Humanize) -> Result<Value, CommandError> {
    let timing_ms = p.timing_ms.unwrap_or(10.0);
    let amount = p.velocity.unwrap_or(0.1);
    in_range("timingMs", timing_ms, 0.0..=200.0)?;
    in_range("velocity", amount, 0.0..=1.0)?;
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let ids = targets(ctx, &p.notes)?;
    // One drift runs through the selection in time order, across channels, as one player's would.
    let mut order: Vec<(Tick, ChannelId, u8, NoteId)> = Vec::new();
    if let Some(pat) = ctx.project.patterns.get(&pattern) {
        for (channel, seq) in &pat.notes {
            order.extend(seq.notes().iter().filter(|n| ids.contains(&n.id)).map(|n| (n.quantized_start.unwrap_or(n.start), *channel, n.key, n.id)));
        }
    }
    order.sort();
    let humanizer = Humanizer::new(p.seed.unwrap_or(1));
    let (timing, velocity_noise) = (humanizer.timing(order.len()), humanizer.velocity(order.len()));
    let plan: HashMap<NoteId, (f64, f64)> =
        order.iter().zip(timing.iter().zip(velocity_noise.iter())).map(|((_, _, _, id), (t, v))| (*id, (*t, *v))).collect();
    let tempo = Arc::clone(&ctx.project.tempo);
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Humanize notes"), |_, notes, _| {
        Ok(notes
            .into_iter()
            .map(|mut note| {
                let (drift, spread) = plan.get(&note.id).copied().unwrap_or((0.0, 0.0));
                let base = note.quantized_start.unwrap_or(note.start);
                let seconds = tempo.seconds_at(base).0 + drift * timing_ms / 1000.0;
                note.start = Tick((tempo.ticks_at(Seconds(seconds.max(0.0))).round() as i64).max(0));
                note.quantized_start = Some(base);
                let velocity = f64::from(note.velocity) * (1.0 + amount * spread);
                note.velocity = velocity.round().clamp(1.0, f64::from(u16::MAX)) as u16;
                note
            })
            .collect())
    })
}

fn unhumanize(ctx: &mut Ctx<'_>, p: Notes) -> Result<Value, CommandError> {
    run_tool(ctx, p.pattern.as_ref(), &p.notes, String::from("Un-humanize notes"), |_, notes, _| {
        Ok(notes
            .into_iter()
            .map(|mut note| {
                if let Some(start) = note.quantized_start.take() {
                    note.start = start;
                }
                note
            })
            .collect())
    })
}

fn limit_to_scale(ctx: &mut Ctx<'_>, p: LimitToScale) -> Result<Value, CommandError> {
    let root = match &p.root {
        Root::Key(key) => PitchClass::of(*key),
        Root::Name(name) => PitchClass::parse_with_octave(name).map_err(|e| CommandError::invalid(e.to_string()))?.0,
    };
    let kind: ScaleKind = p.scale.parse().map_err(|e: bodo_kit_music::ScaleError| CommandError::invalid(e.to_string()))?;
    let scale = Scale::new(root, kind);
    run_tool(ctx, p.pattern.as_ref(), &p.notes, format!("Limit notes to {scale}"), |_, notes, _| {
        Ok(notes
            .into_iter()
            .map(|mut note| {
                note.key = scale.snap(note.key);
                note
            })
            .collect())
    })
}

fn stamp(ctx: &mut Ctx<'_>, p: Stamp) -> Result<Value, CommandError> {
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let channel = ctx.channel(p.channel.as_ref())?;
    let chord: Chord = p.chord.parse().map_err(|e: bodo_kit_music::ChordError| CommandError::invalid(e.to_string()))?;
    let keys = chord.keys(p.octave.unwrap_or(4));
    if keys.is_empty() {
        return Err(CommandError::invalid(format!("{chord} has no keys in that octave")));
    }
    let start = p.start.resolve(&ctx.project)?;
    if start < Tick::ZERO {
        return Err(CommandError::invalid("a chord can't start before the pattern"));
    }
    let length = match &p.length {
        Some(length) => length.resolve(&ctx.project, start)?,
        None => Tick(ctx.project.meter.signature_at(start).beat_ticks()),
    };
    let vel = velocity(p.velocity)?;
    let added = ctx.edit_pattern(pattern, |pat| {
        if pat.binding.is_some_and(|b| b != channel) {
            return Err(CommandError::rejected(format!("{} is bound to another channel", pat.name)));
        }
        let mut added = Vec::with_capacity(keys.len());
        for key in &keys {
            let id = fresh(&mut pat.next_note_id);
            let mut note = Note::new(id, *key, start, length);
            note.velocity = vel;
            pat.notes.entry(channel).or_default().insert(note);
            added.push(id);
        }
        Ok((true, added))
    })?;
    ctx.editor.channel = Some(channel);
    ctx.editor.notes = added.iter().copied().collect();
    ctx.set_label(format!("Stamp {chord}"));
    Ok(
        json!({ "pattern": pattern, "channel": channel, "chord": chord.to_string(), "keys": keys, "notes": added.iter().map(|n| n.0).collect::<Vec<_>>() }),
    )
}

fn curve(points: Vec<CurvePoint>) -> Vec<ExprPoint> {
    let mut curve: Vec<ExprPoint> = points.into_iter().map(|p| ExprPoint { at: Tick(p.at.max(0)), value: p.value }).collect();
    curve.sort_by_key(|p| p.at);
    curve
}

fn set_expression(ctx: &mut Ctx<'_>, p: SetExpression) -> Result<Value, CommandError> {
    for point in p.pitch.iter().chain(&p.pressure).chain(&p.timbre).flatten() {
        if !point.value.is_finite() {
            return Err(CommandError::invalid("expression values must be numbers"));
        }
    }
    let (pitch, pressure, timbre) = (p.pitch.map(curve), p.pressure.map(curve), p.timbre.map(curve));
    run_tool(ctx, p.pattern.as_ref(), &[p.note], String::from("Edit note expression"), |_, notes, _| {
        Ok(notes
            .into_iter()
            .map(|mut note| {
                let mut expr = note.expr.as_deref().cloned().unwrap_or_default();
                if let Some(pitch) = &pitch {
                    expr.pitch.clone_from(pitch);
                }
                if let Some(pressure) = &pressure {
                    expr.pressure.clone_from(pressure);
                }
                if let Some(timbre) = &timbre {
                    expr.timbre.clone_from(timbre);
                }
                let empty = expr == NoteExpression::default();
                note.expr = (!empty).then(|| Arc::new(expr));
                note
            })
            .collect())
    })
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "note.quantize",
        "Quantize notes",
        "Moves notes toward a grid, with strength and swing; remembers the grid position for un-humanize.",
        Effects::UNDOABLE,
        quantize_notes,
    )
    .example(json!({ "notes": [0, 1], "grid": "1 beat", "strength": 0.5 }));
    r.edit("note.transpose", "Transpose notes", "Moves notes up or down by semitones.", Effects::UNDOABLE, transpose)
        .example(json!({ "notes": [0, 1], "semitones": 2 }));
    r.edit("note.legato", "Legato", "Lengthens each note to the start of the next one on its channel.", Effects::UNDOABLE, legato)
        .example(json!({ "notes": [0, 1] }));
    r.edit("note.chop", "Chop notes", "Splits notes into pieces of a length.", Effects::UNDOABLE, chop)
        .example(json!({ "notes": [0], "length": "120 ticks" }));
    r.edit("note.slice", "Slice notes", "Cuts notes in two where they cross a position.", Effects::UNDOABLE, slice)
        .example(json!({ "notes": [0], "at": 60 }))
        .merge(super::whole);
    r.edit("note.glue", "Glue notes", "Joins notes of the same key and channel that touch or overlap.", Effects::UNDOABLE, glue)
        .example(json!({ "notes": [0, 1] }));
    r.edit("note.strum", "Strum", "Rolls chords so each voice starts a little after the one before.", Effects::UNDOABLE, strum)
        .example(json!({ "notes": [0, 1], "time": 30, "direction": "down" }));
    r.edit("note.arpeggiate", "Arpeggiate", "Turns each chord into a run of single notes over its length.", Effects::UNDOABLE, arpeggiate)
        .example(json!({ "notes": [0], "order": "up", "rate": "120 ticks", "gate": 0.5 }));
    r.edit("note.flip", "Flip notes", "Reverses notes in time, or mirrors their keys.", Effects::UNDOABLE, flip)
        .example(json!({ "notes": [0, 1], "axis": "keys" }));
    r.edit("note.humanize", "Humanize notes", "Moves starts by a player-like drift and varies velocities, from a seed.", Effects::UNDOABLE, humanize)
        .example(json!({ "notes": [0, 1], "timingMs": 10, "velocity": 0.1, "seed": 7 }));
    r.edit("note.unhumanize", "Un-humanize notes", "Puts notes back where quantize or humanize found them.", Effects::UNDOABLE, unhumanize)
        .example(json!({ "notes": [0, 1] }));
    r.edit("note.limit_to_scale", "Limit to scale", "Moves notes to the nearest key of a scale.", Effects::UNDOABLE, limit_to_scale)
        .example(json!({ "notes": [0, 1], "root": "E", "scale": "major" }));
    r.edit("note.stamp", "Stamp chord", "Adds a chord from its symbol, such as Am7 or C/E, and selects it.", Effects::UNDOABLE, stamp)
        .example(json!({ "channel": "808", "chord": "Cmaj7", "start": "1:3:0" }));
    r.edit(
        "note.set_expression",
        "Edit note expression",
        "Sets a note's pitch, pressure and timbre curves (MPE).",
        Effects::UNDOABLE,
        set_expression,
    )
    .example(json!({ "note": 1, "pitch": [{ "at": 0, "value": 0.0 }, { "at": 120, "value": 1.0 }] }));
}
