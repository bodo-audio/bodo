//! `playlist.*`: playlist tracks (project.md section 10.3).

use std::collections::BTreeSet;
use std::sync::Arc;

use bodo_model::{Color, PlaylistTrack, TrackId};
use serde::Deserialize;
use serde_json::{Value, json};

use bodo_kit_core::Tick;
use bodo_model::Clip;

use super::{in_range, unique_name};
use crate::error::CommandError;
use crate::params::{ColorText, Length, Position, TrackRef, format_length, format_position};
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
    /// Soloed or not: while any track is soloed, only soloed tracks play.
    solo: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GroupTracks {
    /// The tracks to put in the group.
    tracks: Vec<TrackRef>,
    /// The group's name. Tracks join an existing group of that name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ungroup {
    /// The group to dissolve, by name.
    group: Option<String>,
    /// Or: tracks to take out of their groups.
    #[serde(default)]
    tracks: Vec<TrackRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameGroup {
    /// The group's current name.
    group: String,
    /// Its new name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GroupProps {
    /// The group, by name.
    group: String,
    /// Mutes or unmutes every track of the group.
    muted: Option<bool>,
    /// Solos or unsolos every track of the group.
    solo: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TimeSpan {
    /// Where the time is inserted or removed.
    at: Position,
    /// How much.
    length: Length,
}

/// Shifts every clip and marker at or after `at` by `by` ticks, on unlocked tracks. Clips that cross `at` stay.
fn shift_from(ctx: &mut Ctx<'_>, at: Tick, by: i64) -> Result<usize, CommandError> {
    let mut moved = 0;
    let tracks: Vec<_> = ctx.project.track_order.iter().copied().collect();
    for track in tracks {
        if ctx.project.tracks.get(&track).is_some_and(|t| t.locked) {
            continue;
        }
        moved += ctx.edit_track(track, |t| {
            let mut count = 0;
            let clips: Vec<Clip> = t
                .clips
                .iter()
                .map(|c| {
                    let mut c = c.clone();
                    if c.start >= at {
                        c.start = Tick(c.start.0 + by);
                        count += 1;
                    }
                    c
                })
                .collect();
            t.clips = imbl::Vector::new();
            for clip in clips {
                t.insert_clip(clip);
            }
            Ok(count)
        })?;
    }
    if ctx.project.markers.iter().any(|m| m.at >= at) {
        ctx.project.markers = ctx
            .project
            .markers
            .iter()
            .cloned()
            .map(|mut m| {
                if m.at >= at {
                    m.at = Tick(m.at.0 + by);
                }
                m
            })
            .collect();
    }
    Ok(moved)
}

fn insert_time(ctx: &mut Ctx<'_>, p: TimeSpan) -> Result<Value, CommandError> {
    let at = p.at.resolve(&ctx.project)?.max(Tick::ZERO);
    let length = p.length.resolve(&ctx.project, at)?;
    // Clips that cross the point are cut there first, so their second half moves with the rest.
    if ctx.project.tracks.values().any(|t| !t.locked && t.clips.iter().any(|c| c.start < at && c.end() > at)) {
        let tracks: Vec<serde_json::Value> = ctx.project.track_order.iter().map(|t| json!(t)).collect();
        super::clip_slice(ctx, at, &tracks)?;
    }
    let moved = shift_from(ctx, at, length.0)?;
    ctx.set_label(format!("Insert {} at {}", format_length(&ctx.project, length), format_position(&ctx.project, at)));
    Ok(json!({ "moved": moved }))
}

fn delete_time(ctx: &mut Ctx<'_>, p: TimeSpan) -> Result<Value, CommandError> {
    let at = p.at.resolve(&ctx.project)?.max(Tick::ZERO);
    let length = p.length.resolve(&ctx.project, at)?;
    let end = Tick(at.0 + length.0);
    // Cut clips at both edges, drop what lies inside, then close the gap.
    for edge in [at, end] {
        if ctx.project.tracks.values().any(|t| !t.locked && t.clips.iter().any(|c| c.start < edge && c.end() > edge)) {
            let tracks: Vec<serde_json::Value> = ctx.project.track_order.iter().map(|t| json!(t)).collect();
            super::clip_slice(ctx, edge, &tracks)?;
        }
    }
    let mut removed = 0;
    let tracks: Vec<_> = ctx.project.track_order.iter().copied().collect();
    for track in tracks {
        if ctx.project.tracks.get(&track).is_some_and(|t| t.locked) {
            continue;
        }
        removed += ctx.edit_track(track, |t| {
            let before = t.clips.len();
            t.clips.retain(|c| !(c.start >= at && c.end() <= end));
            Ok(before - t.clips.len())
        })?;
    }
    ctx.project.markers.retain(|m| !(m.at >= at && m.at < end));
    let moved = shift_from(ctx, end, -length.0)?;
    ctx.set_label(format!("Delete {} at {}", format_length(&ctx.project, length), format_position(&ctx.project, at)));
    Ok(json!({ "removed": removed, "moved": moved }))
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
        if let Some(solo) = p.solo {
            track.solo = solo;
        }
        Ok(())
    })?;
    let track = ctx.project.tracks.get(&id).cloned().ok_or_else(|| CommandError::not_found("track", &id.to_string()))?;
    ctx.set_label(format!("Edit track {}", track.name));
    Ok(json!({ "track": id, "name": track.name, "muted": track.muted, "locked": track.locked, "solo": track.solo }))
}

/// The tracks of group `name`, in playlist order, or an error when there are none.
///
/// Complexity: O(t) in tracks.
fn members(ctx: &Ctx<'_>, name: &str) -> Result<Vec<TrackId>, CommandError> {
    let found: Vec<TrackId> =
        ctx.project.track_order.iter().copied().filter(|id| ctx.project.tracks.get(id).is_some_and(|t| t.group.as_deref() == Some(name))).collect();
    if found.is_empty() {
        return Err(CommandError::not_found("track group", name));
    }
    Ok(found)
}

fn group_tracks(ctx: &mut Ctx<'_>, p: GroupTracks) -> Result<Value, CommandError> {
    let name = p.name.trim().to_owned();
    if name.is_empty() {
        return Err(CommandError::invalid("a track group needs a name"));
    }
    if p.tracks.is_empty() {
        return Err(CommandError::invalid("give the tracks to group"));
    }
    let mut chosen = Vec::new();
    for r in &p.tracks {
        let id = ctx.track(Some(r))?;
        if !chosen.contains(&id) {
            chosen.push(id);
        }
    }
    // The group sits where its first track is (an existing group's first track, or else the first joining track),
    // with the joining tracks after its current ones, in playlist order.
    let mut group: Vec<TrackId> = members(ctx, &name).unwrap_or_default();
    let mut joining: Vec<TrackId> = ctx.project.track_order.iter().copied().filter(|t| chosen.contains(t) && !group.contains(t)).collect();
    let anchor = group.first().or(joining.first()).copied();
    let at = anchor.and_then(|a| ctx.project.track_order.index_of(&a)).unwrap_or(0);
    group.append(&mut joining);
    let before = ctx.project.track_order.iter().take(at).filter(|t| !group.contains(t)).count();
    let mut order: Vec<TrackId> = ctx.project.track_order.iter().copied().filter(|t| !group.contains(t)).collect();
    for (offset, id) in group.iter().enumerate() {
        order.insert(before + offset, *id);
    }
    ctx.project.track_order = order.into_iter().collect();
    for id in &group {
        ctx.edit_track(*id, |t| {
            t.group = Some(name.clone());
            Ok(())
        })?;
    }
    ctx.set_label(format!("Group tracks into {name}"));
    Ok(json!({ "group": name, "tracks": group }))
}

fn ungroup(ctx: &mut Ctx<'_>, p: Ungroup) -> Result<Value, CommandError> {
    let tracks = match (&p.group, p.tracks.is_empty()) {
        (Some(name), true) => members(ctx, name)?,
        (None, false) => p.tracks.iter().map(|r| ctx.track(Some(r))).collect::<Result<Vec<_>, _>>()?,
        _ => return Err(CommandError::invalid("give a group or tracks, not both")),
    };
    for id in &tracks {
        ctx.edit_track(*id, |t| {
            t.group = None;
            Ok(())
        })?;
    }
    ctx.set_label(String::from("Ungroup tracks"));
    Ok(json!({ "tracks": tracks }))
}

fn rename_group(ctx: &mut Ctx<'_>, p: RenameGroup) -> Result<Value, CommandError> {
    let name = p.name.trim().to_owned();
    if name.is_empty() {
        return Err(CommandError::invalid("a track group needs a name"));
    }
    if name != p.group && members(ctx, &name).is_ok() {
        return Err(CommandError::rejected(format!("there is already a track group named {name}")));
    }
    for id in members(ctx, &p.group)? {
        ctx.edit_track(id, |t| {
            t.group = Some(name.clone());
            Ok(())
        })?;
    }
    ctx.set_label(format!("Rename track group to {name}"));
    Ok(json!({ "group": name }))
}

fn set_group_props(ctx: &mut Ctx<'_>, p: GroupProps) -> Result<Value, CommandError> {
    let tracks = members(ctx, &p.group)?;
    for id in &tracks {
        ctx.edit_track(*id, |t| {
            if let Some(muted) = p.muted {
                t.muted = muted;
            }
            if let Some(solo) = p.solo {
                t.solo = solo;
            }
            Ok(())
        })?;
    }
    ctx.set_label(format!("Edit track group {}", p.group));
    Ok(json!({ "group": p.group, "tracks": tracks.len() }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "playlist.insert_time",
        "Insert time",
        "Pushes everything from a position later by a length, cutting clips that cross it.",
        Effects::UNDOABLE,
        insert_time,
    )
    .example(json!({ "at": "3:1:0", "length": "1 bar" }));
    r.edit(
        "playlist.delete_time",
        "Delete time",
        "Removes a span of the song and closes the gap, cutting clips at its edges.",
        Effects::UNDOABLE,
        delete_time,
    )
    .example(json!({ "at": "2:1:0", "length": "1 bar" }));
    r.edit("playlist.add_track", "Add track", "Adds a playlist track.", Effects::UNDOABLE, add_track)
        .example(json!({ "name": "Drums" }))
        .menu("Add/Playlist track");
    r.edit("playlist.remove_track", "Remove track", "Removes a playlist track and its clips.", Effects::UNDOABLE, remove_track)
        .example(json!({ "track": "track:16" }));
    r.edit("playlist.set_track_props", "Edit track", "Sets a track's name, color, mute, solo, lock or height.", Effects::UNDOABLE, set_track_props)
        .example(json!({ "track": 1, "name": "Drums", "muted": false, "solo": true }));
    r.edit(
        "playlist.group_tracks",
        "Group tracks",
        "Puts tracks in a named group that folds under one header; they move next to each other.",
        Effects::UNDOABLE,
        group_tracks,
    )
    .example(json!({ "tracks": [1, 3], "name": "Drums" }));
    r.edit("playlist.ungroup_tracks", "Ungroup tracks", "Dissolves a track group, or takes tracks out of theirs.", Effects::UNDOABLE, ungroup)
        .example(json!({ "group": "Drums" }));
    r.edit("playlist.rename_track_group", "Rename track group", "Renames a track group.", Effects::UNDOABLE, rename_group)
        .example(json!({ "group": "Drums", "name": "Beat" }));
    r.edit(
        "playlist.set_group_props",
        "Edit track group",
        "Mutes, unmutes, solos or unsolos every track of a group.",
        Effects::UNDOABLE,
        set_group_props,
    )
    .example(json!({ "group": "Drums", "solo": true }));
}
