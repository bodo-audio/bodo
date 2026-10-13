//! `edit.*`: undo, redo and the selection (project.md section 12.2).

use std::collections::BTreeSet;

use bodo_model::NoteId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::CommandError;
use std::sync::Arc;

use bodo_kit_core::Tick;
use bodo_model::{Clip, Note};

use crate::params::{ChannelRef, ClipRef, InsertRef, Position, TrackRef};
use crate::registry::{Effects, NoParams, Registry};
use crate::session::{Ctx, Session};
use crate::state::Clipboard;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Select {
    /// Channels to select in the channel rack.
    #[serde(default)]
    channels: Vec<ChannelRef>,
    /// The channel the piano roll edits.
    channel: Option<ChannelRef>,
    /// Clips to select on the playlist.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// Notes of the current pattern to select, by id.
    #[serde(default)]
    notes: Vec<u32>,
    /// The insert to select in the mixer.
    insert: Option<InsertRef>,
    /// The playlist track to select.
    track: Option<TrackRef>,
    /// Add to the selection instead of replacing it.
    #[serde(default)]
    add: bool,
    /// Flip each given item in or out of the selection, as Shift does with the select tool.
    #[serde(default)]
    toggle: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum CopyKind {
    /// The selected clips.
    Clips,
    /// The selected notes of the current pattern.
    Notes,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CopyParams {
    /// What to copy; the selected clips, or else the selected notes, by default.
    what: Option<CopyKind>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Paste {
    /// Where the copied items start; right after where they were copied from by default.
    at: Option<Position>,
    /// For clips: the track the first copied track lands on; the same track by default.
    track: Option<TrackRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Kind {
    /// Every channel.
    Channels,
    /// Every clip on the playlist.
    Clips,
    /// Every note of the current pattern.
    Notes,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectAll {
    /// What to select.
    what: Kind,
}

fn selection(ctx: &Ctx<'_>) -> Value {
    json!({
        "channels": ctx.editor.channels,
        "channel": ctx.editor.channel,
        "clips": ctx.editor.clips,
        "notes": ctx.editor.notes.iter().map(|n| n.0).collect::<Vec<_>>(),
        "insert": ctx.editor.insert,
        "track": ctx.editor.track,
    })
}

/// Puts `ids` into `set`: replacing it, adding to it, or flipping each one.
fn apply<T: Ord + Copy>(set: &mut BTreeSet<T>, ids: impl IntoIterator<Item = T>, add: bool, toggle: bool) {
    if !add && !toggle {
        set.clear();
    }
    for id in ids {
        if toggle && set.contains(&id) {
            set.remove(&id);
        } else {
            set.insert(id);
        }
    }
}

fn select(ctx: &mut Ctx<'_>, p: Select) -> Result<Value, CommandError> {
    if !p.channels.is_empty() {
        let ids = ctx.channels(&p.channels)?;
        apply(&mut ctx.editor.channels, ids, p.add, p.toggle);
    }
    if let Some(r) = &p.channel {
        ctx.editor.channel = Some(ctx.channel(Some(r))?);
    }
    if !p.clips.is_empty() {
        let ids = ctx.clips(&p.clips)?;
        apply(&mut ctx.editor.clips, ids.into_iter().map(|(_, clip)| clip), p.add, p.toggle);
    }
    if !p.notes.is_empty() {
        let pattern = ctx.pattern(None)?;
        let present: BTreeSet<NoteId> =
            ctx.project.patterns.get(&pattern).map(|p| p.notes.values().flat_map(|s| s.notes().iter().map(|n| n.id)).collect()).unwrap_or_default();
        if let Some(missing) = p.notes.iter().find(|n| !present.contains(&NoteId(**n))) {
            return Err(CommandError::not_found("note", &missing.to_string()));
        }
        apply(&mut ctx.editor.notes, p.notes.iter().map(|n| NoteId(*n)), p.add, p.toggle);
    }
    if let Some(r) = &p.insert {
        ctx.editor.insert = Some(ctx.insert(Some(r))?);
    }
    if let Some(r) = &p.track {
        ctx.editor.track = Some(ctx.track(Some(r))?);
    }
    Ok(selection(ctx))
}

fn select_all(ctx: &mut Ctx<'_>, p: SelectAll) -> Result<Value, CommandError> {
    match p.what {
        Kind::Channels => ctx.editor.channels = ctx.project.channels.keys().copied().collect(),
        Kind::Clips => ctx.editor.clips = ctx.project.tracks.values().flat_map(|t| t.clips.iter().map(|c| c.id)).collect(),
        Kind::Notes => {
            let pattern = ctx.pattern(None)?;
            ctx.editor.notes = ctx
                .project
                .patterns
                .get(&pattern)
                .map(|p| p.notes.values().flat_map(|s| s.notes().iter().map(|n| n.id)).collect())
                .unwrap_or_default();
        }
    }
    Ok(selection(ctx))
}

fn deselect(ctx: &mut Ctx<'_>) -> Value {
    ctx.editor.channels.clear();
    ctx.editor.clips.clear();
    ctx.editor.notes.clear();
    selection(ctx)
}

/// Copies the selected clips or notes into the clipboard; returns how many.
fn copy(ctx: &mut Ctx<'_>, what: Option<CopyKind>) -> Result<usize, CommandError> {
    let what = what.unwrap_or(if ctx.editor.clips.is_empty() && !ctx.editor.notes.is_empty() { CopyKind::Notes } else { CopyKind::Clips });
    match what {
        CopyKind::Clips => {
            let mut found: Vec<(usize, Clip)> = Vec::new();
            for (row, track) in ctx.project.track_order.iter().enumerate() {
                if let Some(t) = ctx.project.tracks.get(track) {
                    found.extend(t.clips.iter().filter(|c| ctx.editor.clips.contains(&c.id)).map(|c| (row, c.clone())));
                }
            }
            if found.is_empty() {
                return Err(CommandError::nothing_selected("clips"));
            }
            let origin = found.iter().map(|(_, c)| c.start).min().unwrap_or(Tick::ZERO);
            let end = found.iter().map(|(_, c)| c.end()).max().unwrap_or(origin);
            let first_track = found.iter().map(|(row, _)| *row).min().unwrap_or(0);
            let count = found.len();
            let clips = found.into_iter().map(|(row, c)| (row - first_track, Clip { start: Tick(c.start.0 - origin.0), ..c })).collect();
            ctx.editor.clipboard = Some(Arc::new(Clipboard::Clips { clips, origin, span: Tick(end.0 - origin.0), first_track }));
            Ok(count)
        }
        CopyKind::Notes => {
            let pattern = ctx.pattern(None)?;
            let selected = &ctx.editor.notes;
            let notes: Vec<(bodo_model::ChannelId, Note)> = ctx
                .project
                .patterns
                .get(&pattern)
                .map(|p| {
                    p.notes
                        .iter()
                        .flat_map(|(c, seq)| seq.notes().iter().filter(|n| selected.contains(&n.id)).map(move |n| (*c, n.clone())))
                        .collect()
                })
                .unwrap_or_default();
            if notes.is_empty() {
                return Err(CommandError::nothing_selected("notes"));
            }
            let origin = notes.iter().map(|(_, n)| n.start).min().unwrap_or(Tick::ZERO);
            let end = notes.iter().map(|(_, n)| n.end()).max().unwrap_or(origin);
            let count = notes.len();
            let notes = notes.into_iter().map(|(c, n)| (c, Note { start: Tick(n.start.0 - origin.0), ..n })).collect();
            ctx.editor.clipboard = Some(Arc::new(Clipboard::Notes { notes, origin, span: Tick(end.0 - origin.0) }));
            Ok(count)
        }
    }
}

fn cut(ctx: &mut Ctx<'_>, p: CopyParams) -> Result<Value, CommandError> {
    let count = copy(ctx, p.what)?;
    match ctx.editor.clipboard.as_deref() {
        Some(Clipboard::Clips { .. }) => {
            let clips: Vec<ClipRef> = ctx.editor.clips.iter().map(|c| ClipRef::new(c.to_string())).collect();
            let targets = ctx.clips(&clips)?;
            for (track, clip) in targets {
                ctx.edit_track(track, |t| {
                    t.clips.retain(|c| c.id != clip);
                    Ok(())
                })?;
            }
            ctx.editor.clips.clear();
        }
        Some(Clipboard::Notes { .. }) => {
            let pattern = ctx.pattern(None)?;
            let selected = ctx.editor.notes.clone();
            ctx.edit_pattern(pattern, |p| {
                let channels: Vec<_> = p.notes.keys().copied().collect();
                for channel in channels {
                    if let Some(seq) = p.notes.get(&channel) {
                        let kept: Vec<Note> = seq.notes().iter().filter(|n| !selected.contains(&n.id)).cloned().collect();
                        if kept.is_empty() {
                            p.notes.remove(&channel);
                        } else {
                            p.notes.insert(channel, bodo_model::pattern::NoteSeq::new(kept));
                        }
                    }
                }
                Ok((true, ()))
            })?;
            ctx.editor.notes.clear();
        }
        None => {}
    }
    ctx.set_label(format!("Cut {count}"));
    Ok(json!({ "cut": count }))
}

fn paste(ctx: &mut Ctx<'_>, p: Paste) -> Result<Value, CommandError> {
    let clipboard = ctx
        .editor
        .clipboard
        .clone()
        .ok_or_else(|| CommandError::rejected("the clipboard is empty").with_hint("copy something with edit.copy first"))?;
    match &*clipboard {
        Clipboard::Clips { clips, origin, span, first_track } => {
            let at = match &p.at {
                Some(at) => at.resolve(&ctx.project)?,
                None => Tick(origin.0 + span.0),
            };
            let first = match &p.track {
                Some(t) => {
                    let id = ctx.track(Some(t))?;
                    ctx.project.track_order.index_of(&id).unwrap_or(0)
                }
                None => *first_track,
            };
            let mut pasted = Vec::new();
            for (row, clip) in clips {
                let Some(track) = ctx.project.track_order.get(first + row).copied() else { continue };
                if ctx.project.tracks.get(&track).is_some_and(|t| t.locked) {
                    continue;
                }
                let id = ctx.ids().clip();
                let placed = Clip { id, start: Tick(at.0 + clip.start.0), ..clip.clone() };
                ctx.edit_track(track, |t| {
                    t.insert_clip(placed);
                    Ok(())
                })?;
                pasted.push(id);
            }
            ctx.editor.clips = pasted.iter().copied().collect();
            ctx.set_label(format!("Paste {} clips", pasted.len()));
            Ok(json!({ "pasted": pasted.len(), "clips": pasted }))
        }
        Clipboard::Notes { notes, origin, span } => {
            let at = match &p.at {
                Some(at) => at.resolve(&ctx.project)?,
                None => Tick(origin.0 + span.0),
            };
            let pattern = ctx.pattern(None)?;
            let channels = ctx.project.channels.clone();
            let pasted = ctx.edit_pattern(pattern, |pat| {
                let mut ids = Vec::new();
                for (channel, note) in notes {
                    if !channels.contains_key(channel) || pat.binding.is_some_and(|b| b != *channel) {
                        continue;
                    }
                    let id = bodo_model::NoteId(pat.next_note_id);
                    pat.next_note_id += 1;
                    pat.notes.entry(*channel).or_default().insert(Note { id, start: Tick(at.0 + note.start.0), ..note.clone() });
                    ids.push(id);
                }
                Ok((!ids.is_empty(), ids))
            })?;
            ctx.editor.notes = pasted.iter().copied().collect();
            ctx.set_label(format!("Paste {} notes", pasted.len()));
            Ok(json!({ "pasted": pasted.len(), "notes": pasted.iter().map(|n| n.0).collect::<Vec<_>>() }))
        }
    }
}

fn undo(session: &mut Session) -> Value {
    let undone = session.undo();
    json!({ "undone": undone, "canUndo": session.undo_labels().next().is_some() })
}

fn redo(session: &mut Session) -> Value {
    let redone = session.redo();
    json!({ "redone": redone, "canRedo": session.redo_labels().next().is_some() })
}

pub(super) fn register(r: &mut Registry) {
    r.session("edit.undo", "Undo", "Undoes the last change to the project.", Effects::NONE, |s, _: NoParams| Ok(undo(s)))
        .example(json!({}))
        .menu("Edit/Undo")
        .shortcut("Mod+Z");
    r.session("edit.redo", "Redo", "Redoes the last undone change.", Effects::NONE, |s, _: NoParams| Ok(redo(s)))
        .example(json!({}))
        .menu("Edit/Redo")
        .shortcut("Mod+Shift+Z");
    r.edit("edit.copy", "Copy", "Copies the selected clips, or the selected notes, for edit.paste.", Effects::NONE, |ctx, p: CopyParams| {
        let count = copy(ctx, p.what)?;
        Ok(json!({ "copied": count }))
    })
    .example(json!({ "what": "clips" }))
    .menu("Edit/Copy")
    .shortcut("Mod+C");
    r.edit("edit.cut", "Cut", "Copies the selected clips or notes and removes them.", Effects::UNDOABLE, cut)
        .example(json!({}))
        .menu("Edit/Cut")
        .shortcut("Mod+X");
    r.edit("edit.paste", "Paste", "Pastes what was copied: clips onto the playlist, notes into the current pattern.", Effects::UNDOABLE, paste)
        .example(json!({ "at": "9:1:0" }))
        .menu("Edit/Paste")
        .shortcut("Mod+V");
    r.edit(
        "edit.select",
        "Select",
        "Selects channels, clips, notes, an insert or a track; commands with empty params act on it.",
        Effects::NONE,
        select,
    )
    .example(json!({ "channels": ["Kick", "Snare"] }));
    r.edit("edit.select_all", "Select all", "Selects every channel, every clip, or every note of the current pattern.", Effects::NONE, select_all)
        .example(json!({ "what": "clips" }))
        .menu("Edit/Select all");
    r.edit("edit.deselect", "Deselect", "Clears the selection of channels, clips and notes.", Effects::NONE, |ctx, _: NoParams| Ok(deselect(ctx)))
        .example(json!({}))
        .menu("Edit/Deselect");
}
