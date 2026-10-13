//! `edit.*`: undo, redo and the selection (project.md section 12.2).

use std::collections::BTreeSet;

use bodo_model::NoteId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::params::{ChannelRef, ClipRef, InsertRef, TrackRef};
use crate::registry::{Effects, NoParams, Registry};
use crate::session::{Ctx, Session};

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

fn select(ctx: &mut Ctx<'_>, p: Select) -> Result<Value, CommandError> {
    if !p.channels.is_empty() {
        let ids = ctx.channels(&p.channels)?;
        if !p.add {
            ctx.editor.channels.clear();
        }
        ctx.editor.channels.extend(ids);
    }
    if let Some(r) = &p.channel {
        ctx.editor.channel = Some(ctx.channel(Some(r))?);
    }
    if !p.clips.is_empty() {
        let ids = ctx.clips(&p.clips)?;
        if !p.add {
            ctx.editor.clips.clear();
        }
        ctx.editor.clips.extend(ids.into_iter().map(|(_, clip)| clip));
    }
    if !p.notes.is_empty() {
        let pattern = ctx.pattern(None)?;
        let present: BTreeSet<NoteId> =
            ctx.project.patterns.get(&pattern).map(|p| p.notes.values().flat_map(|s| s.notes().iter().map(|n| n.id)).collect()).unwrap_or_default();
        if let Some(missing) = p.notes.iter().find(|n| !present.contains(&NoteId(**n))) {
            return Err(CommandError::not_found("note", &missing.to_string()));
        }
        if !p.add {
            ctx.editor.notes.clear();
        }
        ctx.editor.notes.extend(p.notes.iter().map(|n| NoteId(*n)));
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
