//! `playlist.*`: playlist tracks (project.md section 10.3).

use std::collections::BTreeSet;
use std::sync::Arc;

use bodo_model::{Color, PlaylistTrack};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{in_range, unique_name};
use crate::error::CommandError;
use crate::params::{ColorText, TrackRef};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddTrack {
    /// Its name; "Track N" by default.
    name: Option<String>,
    /// Its position, from 1; after the last track by default.
    at: Option<usize>,
    /// Its color.
    color: Option<ColorText>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveTrack {
    /// The track; the selected track by default.
    track: Option<TrackRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetTrackProps {
    /// The track; the selected track by default.
    track: Option<TrackRef>,
    /// A new name.
    name: Option<String>,
    /// A new color.
    color: Option<ColorText>,
    /// Muted or not.
    muted: Option<bool>,
    /// Locked against edits or not.
    locked: Option<bool>,
    /// Its height in the playlist, from 16 to 400 points.
    height: Option<f32>,
}

fn add_track(ctx: &mut Ctx<'_>, p: AddTrack) -> Result<Value, CommandError> {
    let count = ctx.project.track_order.len();
    let at = p.at.map_or(count, |at| at.saturating_sub(1).min(count));
    let base = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Track {}", count + 1));
    let name = unique_name(base.trim(), ctx.project.tracks.values().map(|t| t.name.as_str()));
    let color = p.color.map(|c| c.resolve()).transpose()?.unwrap_or(Color::GRAY);
    let id = ctx.ids().track();
    ctx.project.tracks.insert(id, Arc::new(PlaylistTrack::new(id, name.clone(), color)));
    ctx.project.track_order.insert(at, id);
    ctx.editor.track = Some(id);
    ctx.set_label(format!("Add track {name}"));
    Ok(json!({ "track": id, "name": name, "position": at + 1 }))
}

fn remove_track(ctx: &mut Ctx<'_>, p: RemoveTrack) -> Result<Value, CommandError> {
    let id = ctx.track(p.track.as_ref())?;
    let track = ctx.project.tracks.remove(&id).ok_or_else(|| CommandError::not_found("track", &id.to_string()))?;
    if track.locked {
        return Err(CommandError::rejected(format!("{} is locked", track.name)).with_hint("unlock it with playlist.set_track_props"));
    }
    ctx.project.track_order.retain(|t| *t != id);
    let gone: BTreeSet<_> = track.clips.iter().map(|c| c.id).collect();
    ctx.editor.clips.retain(|c| !gone.contains(c));
    ctx.set_label(format!("Remove track {}", track.name));
    Ok(json!({ "removed": id, "clipsRemoved": track.clips.len() }))
}

fn set_track_props(ctx: &mut Ctx<'_>, p: SetTrackProps) -> Result<Value, CommandError> {
    let id = ctx.track(p.track.as_ref())?;
    let color = p.color.map(|c| c.resolve()).transpose()?;
    if let Some(height) = p.height {
        in_range("height", f64::from(height), 16.0..=400.0)?;
    }
    let name = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
    ctx.edit_track(id, |track| {
        if let Some(name) = &name {
            track.name.clone_from(name);
        }
        if let Some(color) = color {
            track.color = color;
        }
        if let Some(muted) = p.muted {
            track.muted = muted;
        }
        if let Some(locked) = p.locked {
            track.locked = locked;
        }
        if let Some(height) = p.height {
            track.height = height;
        }
        Ok(())
    })?;
    let track = ctx.project.tracks.get(&id).cloned().ok_or_else(|| CommandError::not_found("track", &id.to_string()))?;
    ctx.set_label(format!("Edit track {}", track.name));
    Ok(json!({ "track": id, "name": track.name, "muted": track.muted, "locked": track.locked }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("playlist.add_track", "Add track", "Adds a playlist track.", Effects::UNDOABLE, add_track)
        .example(json!({ "name": "Drums" }))
        .menu("Add/Playlist track");
    r.edit("playlist.remove_track", "Remove track", "Removes a playlist track and its clips.", Effects::UNDOABLE, remove_track)
        .example(json!({ "track": "track:16" }));
    r.edit("playlist.set_track_props", "Edit track", "Sets a track's name, color, mute, lock or height.", Effects::UNDOABLE, set_track_props)
        .example(json!({ "track": 1, "name": "Drums", "muted": false }));
}
