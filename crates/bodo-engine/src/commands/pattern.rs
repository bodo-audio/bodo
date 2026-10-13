//! `pattern.*` and `ui.select_pattern`: patterns and the step sequencer (project.md sections 10.2 and 14.1).
//!
//! A step is a sixteenth note. Step `n` (counted from 1) is on when the channel has a note starting inside it; turning
//! it on adds a one-step note at the channel's root key, and turning it off removes every note starting inside it.

use std::collections::BTreeSet;
use std::sync::Arc;

use bodo_kit_core::Tick;
use bodo_model::channel::ChannelKind;
use bodo_model::pattern::{NoteSeq, PatternLen};
use bodo_model::playlist::ClipSource;
use bodo_model::{ChannelId, Note, NoteId, Pattern, PatternId, Project};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{pattern_length, unique_name};
use crate::error::CommandError;
use crate::palette;
use crate::params::{ChannelRef, ColorText, Length, PatternRef, STEP, format_length};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

/// The velocity of new steps and notes, 0 to 1.
pub(crate) const DEFAULT_VELOCITY: f32 = 0.8;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct New {
    /// Its name; "Pattern N" by default.
    name: Option<String>,
    /// Its color; the next palette color by default.
    color: Option<ColorText>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct One {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Rename {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The new name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetColor {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The new color.
    color: ColorText,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetLength {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The length, or "auto" to follow the notes, rounded up to whole bars.
    length: Length,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetSteps {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The channel whose steps to set.
    channel: ChannelRef,
    /// One character per step (a sixteenth note) from the pattern's start: `x` on, `.` off. Spaces and `|` are
    /// ignored, so `"x... x... x... x..."` is four on the beat. Steps after the string are left as they are.
    steps: String,
    /// The velocity of steps that turn on, from 0 to 1; 0.8 by default.
    velocity: Option<f32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToggleStep {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The channel.
    channel: ChannelRef,
    /// The step, counted from 1.
    step: u32,
    /// On or off; toggles when left out.
    on: Option<bool>,
    /// The velocity if it turns on, from 0 to 1; 0.8 by default.
    velocity: Option<f32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Select {
    /// The pattern to make current.
    pattern: Option<PatternRef>,
    /// Or: move this many patterns forward (or back, when negative) from the current one.
    offset: Option<i32>,
}

/// A 0 to 1 velocity as the model's 16-bit value.
pub(crate) fn velocity(v: Option<f32>) -> Result<u16, CommandError> {
    let v = v.unwrap_or(DEFAULT_VELOCITY);
    super::in_range("velocity", f64::from(v), 0.0..=1.0)?;
    Ok((v * f32::from(u16::MAX)).round() as u16)
}

/// The key steps of channel `id` play: a sampler's root key, or C4.
fn step_key(project: &Project, id: ChannelId) -> u8 {
    match project.channels.get(&id).map(|c| &c.kind) {
        Some(ChannelKind::Sampler(params)) => params.root_key,
        _ => 60,
    }
}

/// Which steps of `seq` are on, over `steps` steps.
///
/// Complexity: O(log n + k) per step.
pub(crate) fn steps_of(seq: Option<&NoteSeq>, steps: usize) -> Vec<bool> {
    (0..steps)
        .map(|i| {
            let (start, end) = (Tick(i as i64 * STEP.0), Tick((i as i64 + 1) * STEP.0));
            seq.is_some_and(|seq| seq.overlapping(start, end).any(|n| n.start >= start && n.start < end))
        })
        .collect()
}

/// The step string of channel `channel` in `pattern`, such as `x...x...x...x...`.
pub fn step_text(project: &Project, pattern: &Pattern, channel: ChannelId) -> String {
    let steps = (pattern_length(project, pattern.id).0 / STEP.0).max(1) as usize;
    steps_of(pattern.notes.get(&channel), steps).into_iter().map(|on| if on { 'x' } else { '.' }).collect()
}

/// Sets step `index` (from 0) of `channel` in `pattern`; returns whether anything changed.
fn set_step(pattern: &mut Pattern, channel: ChannelId, index: usize, on: bool, key: u8, velocity: u16) -> bool {
    let start = Tick(index as i64 * STEP.0);
    let end = Tick(start.0 + STEP.0);
    let seq = pattern.notes.get(&channel);
    let present: Vec<NoteId> =
        seq.map(|s| s.overlapping(start, end).filter(|n| n.start >= start && n.start < end).map(|n| n.id).collect()).unwrap_or_default();
    match (on, present.is_empty()) {
        (true, true) => {
            let id = NoteId(pattern.next_note_id);
            pattern.next_note_id += 1;
            let mut note = Note::new(id, key, start, STEP);
            note.velocity = velocity;
            pattern.notes.entry(channel).or_default().insert(note);
            true
        }
        (false, false) => {
            if let Some(seq) = pattern.notes.get_mut(&channel) {
                for id in present {
                    seq.remove(id);
                }
                if seq.is_empty() {
                    pattern.notes.remove(&channel);
                }
            }
            true
        }
        _ => false,
    }
}

/// Checks that `channel` may have notes in `pattern`: a clip-mode pattern takes only its own channel.
fn check_binding(project: &Project, pattern: &Pattern, channel: ChannelId) -> Result<(), CommandError> {
    match pattern.binding {
        Some(bound) if bound != channel => {
            let name = |id| project.channels.get(&id).map(|c| c.name.clone()).unwrap_or_default();
            Err(CommandError::rejected(format!("{} is bound to {}, so it can't hold notes for {}", pattern.name, name(bound), name(channel))))
        }
        _ => Ok(()),
    }
}

fn new(ctx: &mut Ctx<'_>, p: New) -> Result<Value, CommandError> {
    let count = ctx.project.patterns.len();
    let base = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Pattern {}", count + 1));
    let name = unique_name(base.trim(), ctx.project.patterns.values().map(|p| p.name.as_str()));
    let color = match p.color {
        Some(c) => c.resolve()?,
        None => palette::color(count),
    };
    let id = ctx.ids().pattern();
    ctx.project.patterns.insert(id, Arc::new(Pattern::new(id, name.clone(), color)));
    ctx.editor.pattern = Some(id);
    ctx.editor.notes.clear();
    ctx.set_label(format!("New pattern {name}"));
    Ok(json!({ "pattern": id, "name": name }))
}

fn delete(ctx: &mut Ctx<'_>, p: One) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let name = ctx.project.patterns.get(&id).map(|p| p.name.clone()).unwrap_or_default();
    ctx.project.patterns.remove(&id);
    let tracks: Vec<_> = ctx.project.tracks.keys().copied().collect();
    let mut clips = 0;
    for track in tracks {
        clips += ctx.edit_track(track, |t| {
            let before = t.clips.len();
            t.clips.retain(|c| c.source != ClipSource::Pattern(id));
            Ok(before - t.clips.len())
        })?;
    }
    if ctx.editor.pattern == Some(id) {
        // The neighbor in creation order takes over.
        ctx.editor.pattern = ctx.project.patterns.range(id..).next().or_else(|| ctx.project.patterns.range(..id).next_back()).map(|(k, _)| *k);
        ctx.editor.notes.clear();
    }
    ctx.set_label(format!("Delete pattern {name}"));
    Ok(json!({ "deleted": id, "clipsRemoved": clips }))
}

fn clone(ctx: &mut Ctx<'_>, p: One) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let original = ctx.project.patterns.get(&id).cloned().ok_or_else(|| CommandError::not_found("pattern", &id.to_string()))?;
    let copy = ctx.ids().pattern();
    let name = unique_name(&original.name, ctx.project.patterns.values().map(|p| p.name.as_str()));
    let color = palette::color(ctx.project.patterns.len());
    ctx.project.patterns.insert(copy, Arc::new(Pattern { id: copy, name: name.clone(), color, ..(*original).clone() }));
    ctx.editor.pattern = Some(copy);
    ctx.editor.notes.clear();
    ctx.set_label(format!("Clone {}", original.name));
    Ok(json!({ "pattern": copy, "name": name }))
}

fn rename(ctx: &mut Ctx<'_>, p: Rename) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let name = p.name.trim().to_owned();
    if name.is_empty() {
        return Err(CommandError::invalid("a pattern name can't be empty"));
    }
    ctx.edit_pattern(id, |pattern| {
        let changed = pattern.name != name;
        pattern.name.clone_from(&name);
        Ok((changed, ()))
    })?;
    ctx.set_label(format!("Rename pattern to {name}"));
    Ok(json!({ "pattern": id, "name": name }))
}

fn set_color(ctx: &mut Ctx<'_>, p: SetColor) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let color = p.color.resolve()?;
    ctx.edit_pattern(id, |pattern| {
        let changed = pattern.color != color;
        pattern.color = color;
        Ok((changed, ()))
    })?;
    Ok(json!({ "pattern": id, "color": color }))
}

fn set_length(ctx: &mut Ctx<'_>, p: SetLength) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let length = match &p.length {
        Length::Text(text) if text.trim().eq_ignore_ascii_case("auto") => PatternLen::Auto,
        other => PatternLen::Fixed(other.resolve(&ctx.project, Tick::ZERO)?),
    };
    ctx.edit_pattern(id, |pattern| {
        let changed = pattern.length != length;
        pattern.length = length;
        Ok((changed, ()))
    })?;
    let length = pattern_length(&ctx.project, id);
    ctx.set_label("Set pattern length");
    Ok(json!({ "pattern": id, "length": format_length(&ctx.project, length), "ticks": length.0 }))
}

fn set_steps(ctx: &mut Ctx<'_>, p: SetSteps) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let channel = ctx.channel(Some(&p.channel))?;
    let mut steps = Vec::with_capacity(p.steps.len());
    for c in p.steps.chars() {
        match c {
            'x' | 'X' | '1' | 'o' | 'O' | '#' | '*' => steps.push(true),
            '.' | '-' | '0' | '_' => steps.push(false),
            ' ' | '|' | '\t' => {}
            other => return Err(CommandError::invalid(format!("`{other}` is not a step; use x for on and . for off"))),
        }
    }
    let key = step_key(&ctx.project, channel);
    let velocity = velocity(p.velocity)?;
    let project = ctx.project.clone();
    ctx.edit_pattern(id, |pattern| {
        check_binding(&project, pattern, channel)?;
        let mut changed = false;
        for (index, on) in steps.iter().enumerate() {
            changed |= set_step(pattern, channel, index, *on, key, velocity);
        }
        Ok((changed, ()))
    })?;
    let text = ctx.project.patterns.get(&id).map(|pattern| step_text(&ctx.project, pattern, channel)).unwrap_or_default();
    ctx.editor.channel = Some(channel);
    ctx.set_label("Set steps");
    Ok(json!({ "pattern": id, "channel": channel, "steps": text }))
}

fn toggle_step(ctx: &mut Ctx<'_>, p: ToggleStep) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let channel = ctx.channel(Some(&p.channel))?;
    if p.step == 0 {
        return Err(CommandError::invalid("steps count from 1"));
    }
    let index = (p.step - 1) as usize;
    let key = step_key(&ctx.project, channel);
    let velocity = velocity(p.velocity)?;
    let project = ctx.project.clone();
    let on = ctx.edit_pattern(id, |pattern| {
        check_binding(&project, pattern, channel)?;
        let was = steps_of(pattern.notes.get(&channel), index + 1).last().copied().unwrap_or(false);
        let on = p.on.unwrap_or(!was);
        Ok((set_step(pattern, channel, index, on, key, velocity), on))
    })?;
    ctx.set_label(if on { "Add step" } else { "Remove step" });
    Ok(json!({ "pattern": id, "channel": channel, "step": p.step, "on": on }))
}

fn select(ctx: &mut Ctx<'_>, p: Select) -> Result<Value, CommandError> {
    let target = match (&p.pattern, p.offset) {
        (Some(r), _) => ctx.pattern(Some(r))?,
        (None, Some(offset)) => {
            let order: Vec<PatternId> = ctx.project.patterns.keys().copied().collect();
            if order.is_empty() {
                return Err(CommandError::new(crate::error::ErrorCode::NotFound, "there are no patterns").with_hint("make one with pattern.new"));
            }
            let current = ctx.editor.pattern.and_then(|c| order.iter().position(|p| *p == c)).unwrap_or(0) as i64;
            let index = (current + i64::from(offset)).clamp(0, order.len() as i64 - 1);
            order.get(index as usize).copied().ok_or_else(|| CommandError::nothing_selected("pattern"))?
        }
        (None, None) => return Err(CommandError::invalid("give pattern or offset")),
    };
    if ctx.editor.pattern != Some(target) {
        ctx.editor.pattern = Some(target);
        ctx.editor.notes = BTreeSet::new();
    }
    let name = ctx.project.patterns.get(&target).map(|p| p.name.clone()).unwrap_or_default();
    Ok(json!({ "pattern": target, "name": name }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("pattern.new", "New pattern", "Makes a new empty pattern and makes it current.", Effects::UNDOABLE, new)
        .example(json!({ "name": "Verse" }))
        .menu("Patterns/New pattern");
    r.edit("pattern.delete", "Delete pattern", "Deletes a pattern and every clip of it on the playlist.", Effects::UNDOABLE, delete)
        .example(json!({ "pattern": "pattern:1" }))
        .menu("Patterns/Delete pattern");
    r.edit("pattern.clone", "Clone pattern", "Copies a pattern with its notes into a new current pattern.", Effects::UNDOABLE, clone)
        .example(json!({}))
        .menu("Patterns/Clone pattern");
    r.edit("pattern.rename", "Rename pattern", "Renames a pattern.", Effects::UNDOABLE, rename)
        .example(json!({ "pattern": "pattern:1", "name": "Intro" }))
        .menu("Patterns/Rename pattern");
    r.edit("pattern.set_color", "Set pattern color", "Sets a pattern's color.", Effects::UNDOABLE, set_color).example(json!({ "color": "#60a5fa" }));
    r.edit("pattern.set_length", "Set pattern length", "Fixes a pattern's length, or sets it back to auto.", Effects::UNDOABLE, set_length)
        .example(json!({ "length": "2 bars" }));
    r.edit(
        "pattern.set_steps",
        "Set steps",
        "Sets a channel's step sequence in a pattern from a string such as x...x...x...x....",
        Effects::UNDOABLE,
        set_steps,
    )
    .example(json!({ "channel": "Kick", "steps": "x... x... x... x..." }));
    r.edit("pattern.toggle_step", "Toggle step", "Turns one step of a channel on or off.", Effects::UNDOABLE, toggle_step)
        .example(json!({ "channel": "Clap", "step": 5 }));
    r.edit(
        "ui.select_pattern",
        "Select pattern",
        "Makes a pattern current; Pattern mode plays it and the channel rack shows it.",
        Effects::NONE,
        select,
    )
    .example(json!({ "offset": 1 }));
}
