//! `clip.*`: clips on the playlist (project.md section 10.3).
//!
//! A pattern clip plays its pattern from the clip's start; a clip longer than the pattern repeats it. Clips may
//! overlap on a track. Locked tracks refuse every clip edit.

use std::collections::{BTreeMap, BTreeSet};

use bodo_kit_core::Tick;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{Clip, ClipId, TrackId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{by_clips, pattern_length, unique_name, whole};
use crate::error::CommandError;
use crate::palette;
use crate::params::{ClipRef, ColorText, Length, PatternRef, Position, TrackRef, format_position};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Place {
    /// The track; the selected track (or the first) by default.
    track: Option<TrackRef>,
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// Where the clip starts.
    start: Position,
    /// How long it plays; the pattern's length by default. Longer clips repeat the pattern.
    length: Option<Length>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Clips {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Move {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// Where the earliest clip goes; the others keep their distance to it.
    start: Option<Position>,
    /// Or: ticks to move every clip by.
    by: Option<i64>,
    /// The track every clip goes to; they stay on their tracks by default.
    track: Option<TrackRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resize {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// The new length of every clip.
    length: Length,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mute {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// Muted or not; toggles each clip when left out.
    muted: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Slice {
    /// Where to cut.
    at: Position,
    /// Cut only these clips; by default every clip the cut crosses on `tracks`, or else the selected clips.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// Cut every clip these tracks have at that point.
    #[serde(default)]
    tracks: Vec<TrackRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Slip {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// Ticks to slide the content by: positive plays later material, negative earlier. Pattern content wraps around
    /// the pattern; audio stops at its start.
    by: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MakeUnique {
    /// The pattern clip; the selected clip by default.
    clip: Option<ClipRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetColor {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// The color, or null to use the source's color again.
    color: Option<ColorText>,
}

/// Fails when track `id` is locked.
fn unlocked(ctx: &Ctx<'_>, id: TrackId) -> Result<(), CommandError> {
    match ctx.project.tracks.get(&id) {
        Some(track) if track.locked => {
            Err(CommandError::rejected(format!("{} is locked", track.name)).with_hint("unlock it with playlist.set_track_props"))
        }
        Some(_) => Ok(()),
        None => Err(CommandError::not_found("track", &id.to_string())),
    }
}

/// Takes the clips `targets` off their tracks and returns them.
fn take(ctx: &mut Ctx<'_>, targets: &[(TrackId, ClipId)]) -> Result<Vec<(TrackId, Clip)>, CommandError> {
    let mut by_track: BTreeMap<TrackId, BTreeSet<ClipId>> = BTreeMap::new();
    for (track, clip) in targets {
        unlocked(ctx, *track)?;
        by_track.entry(*track).or_default().insert(*clip);
    }
    let mut taken = Vec::with_capacity(targets.len());
    for (track, ids) in by_track {
        ctx.edit_track(track, |t| {
            let (out, keep): (Vec<Clip>, Vec<Clip>) = t.clips.iter().cloned().partition(|c| ids.contains(&c.id));
            t.clips = keep.into_iter().collect();
            taken.extend(out.into_iter().map(|c| (track, c)));
            Ok(())
        })?;
    }
    Ok(taken)
}

/// Puts `clips` on their tracks, each in its sorted place.
fn put(ctx: &mut Ctx<'_>, clips: Vec<(TrackId, Clip)>) -> Result<(), CommandError> {
    for (track, clip) in clips {
        ctx.edit_track(track, |t| {
            t.insert_clip(clip);
            Ok(())
        })?;
    }
    Ok(())
}

fn place(ctx: &mut Ctx<'_>, p: Place) -> Result<Value, CommandError> {
    let track = ctx.track(p.track.as_ref())?;
    unlocked(ctx, track)?;
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let start = p.start.resolve(&ctx.project)?;
    if start < Tick::ZERO {
        return Err(CommandError::invalid("a clip can't start before the song"));
    }
    let len = match &p.length {
        Some(length) => length.resolve(&ctx.project, start)?,
        None => pattern_length(&ctx.project, pattern),
    };
    let id = ctx.ids().clip();
    let clip = Clip { id, start, len, offset: ClipOffset::default(), muted: false, color: None, source: ClipSource::Pattern(pattern), fades: None };
    put(ctx, vec![(track, clip)])?;
    ctx.editor.track = Some(track);
    ctx.editor.clips = BTreeSet::from([id]);
    let name = ctx.project.patterns.get(&pattern).map(|p| p.name.clone()).unwrap_or_default();
    let at = format_position(&ctx.project, start);
    ctx.set_label(format!("Place {name} at {at}"));
    Ok(json!({ "clip": id, "track": track, "pattern": pattern, "start": at, "ticks": start.0, "length": len.0 }))
}

fn remove(ctx: &mut Ctx<'_>, p: Clips) -> Result<Value, CommandError> {
    let targets = ctx.clips(&p.clips)?;
    let removed = take(ctx, &targets)?;
    for (_, clip) in &removed {
        ctx.editor.clips.remove(&clip.id);
    }
    ctx.set_label(if removed.len() == 1 { String::from("Delete clip") } else { format!("Delete {} clips", removed.len()) });
    Ok(json!({ "removed": removed.len() }))
}

fn move_clips(ctx: &mut Ctx<'_>, p: Move) -> Result<Value, CommandError> {
    let targets = ctx.clips(&p.clips)?;
    let to_track = p.track.as_ref().map(|t| ctx.track(Some(t))).transpose()?;
    if let Some(track) = to_track {
        unlocked(ctx, track)?;
    }
    let mut clips = take(ctx, &targets)?;
    let earliest = clips.iter().map(|(_, c)| c.start).min().unwrap_or(Tick::ZERO);
    let delta = match (&p.start, p.by) {
        (Some(start), _) => start.resolve(&ctx.project)?.0 - earliest.0,
        (None, Some(by)) => by,
        (None, None) => 0,
    };
    if earliest.0 + delta < 0 {
        return Err(CommandError::rejected("that would move a clip before the song's start"));
    }
    for (track, clip) in &mut clips {
        clip.start = Tick(clip.start.0 + delta);
        if let Some(to) = to_track {
            *track = to;
        }
    }
    let count = clips.len();
    put(ctx, clips)?;
    ctx.set_label(if count == 1 { String::from("Move clip") } else { format!("Move {count} clips") });
    Ok(json!({ "moved": count, "by": delta }))
}

fn resize(ctx: &mut Ctx<'_>, p: Resize) -> Result<Value, CommandError> {
    let targets = ctx.clips(&p.clips)?;
    let mut clips = take(ctx, &targets)?;
    for (_, clip) in &mut clips {
        clip.len = p.length.resolve(&ctx.project, clip.start)?;
    }
    let count = clips.len();
    put(ctx, clips)?;
    ctx.set_label(if count == 1 { String::from("Resize clip") } else { format!("Resize {count} clips") });
    Ok(json!({ "resized": count }))
}

fn mute(ctx: &mut Ctx<'_>, p: Mute) -> Result<Value, CommandError> {
    let targets = ctx.clips(&p.clips)?;
    let mut clips = take(ctx, &targets)?;
    for (_, clip) in &mut clips {
        clip.muted = p.muted.unwrap_or(!clip.muted);
    }
    let count = clips.len();
    put(ctx, clips)?;
    ctx.set_label(if count == 1 { String::from("Mute clip") } else { format!("Mute {count} clips") });
    Ok(json!({ "changed": count }))
}

/// The offset of the part of `clip` from `at` on: its offset moved forward by `at - start`.
fn offset_after(project: &bodo_model::Project, clip: &Clip, at: Tick) -> ClipOffset {
    let ticks = at.0 - clip.start.0;
    match clip.offset {
        ClipOffset::Ticks(t) => ClipOffset::Ticks(Tick(t.0 + ticks)),
        ClipOffset::Frames(f) => {
            let seconds = project.tempo.seconds_at(at).0 - project.tempo.seconds_at(clip.start).0;
            ClipOffset::Frames(bodo_kit_core::Frames(f.0 + project.settings.sample_rate.seconds_to_frames(bodo_kit_core::Seconds(seconds)).0))
        }
    }
}

/// Slices every clip crossing `at` on `tracks` (given as JSON ids), for commands that cut the song.
pub(crate) fn slice_tracks(ctx: &mut Ctx<'_>, at: Tick, tracks: &[Value]) -> Result<Value, CommandError> {
    let tracks: Vec<TrackRef> = tracks.iter().filter_map(|t| serde_json::from_value(t.clone()).ok()).collect();
    slice(ctx, Slice { at: Position::Ticks(at.0), clips: Vec::new(), tracks })
}

fn slice(ctx: &mut Ctx<'_>, p: Slice) -> Result<Value, CommandError> {
    let at = p.at.resolve(&ctx.project)?;
    let crosses = |c: &Clip| c.start < at && c.end() > at;
    let targets: Vec<(TrackId, ClipId)> = if !p.clips.is_empty() {
        ctx.clips(&p.clips)?
    } else if !p.tracks.is_empty() {
        let mut found = Vec::new();
        for r in &p.tracks {
            let track = ctx.track(Some(r))?;
            if let Some(t) = ctx.project.tracks.get(&track) {
                found.extend(t.clips.iter().filter(|c| crosses(c)).map(|c| (track, c.id)));
            }
        }
        found
    } else {
        ctx.clips(&[])?
    };
    let mut pieces = Vec::new();
    for (track, clip) in targets {
        unlocked(ctx, track)?;
        let Some(original) = ctx.project.tracks.get(&track).and_then(|t| t.clips.iter().find(|c| c.id == clip)).cloned() else { continue };
        if !crosses(&original) {
            continue;
        }
        let id = ctx.ids().clip();
        let offset = offset_after(&ctx.project, &original, at);
        // A fade-in stays on the left half, a fade-out moves to the right half.
        let fades = original.fades;
        let right = Clip {
            id,
            start: at,
            len: Tick(original.end().0 - at.0),
            offset,
            fades: fades.map(|f| bodo_model::playlist::Fades { fade_in: Tick::ZERO, ..f }),
            ..original
        };
        ctx.edit_track(track, |t| {
            if let Some(left) = t.clips.iter_mut().find(|c| c.id == clip) {
                left.len = Tick(at.0 - left.start.0);
                left.fades = fades.map(|f| bodo_model::playlist::Fades { fade_out: Tick::ZERO, ..f });
            }
            t.insert_clip(right);
            Ok(())
        })?;
        pieces.push(id);
    }
    let cut = pieces.len();
    if cut == 0 {
        return Err(CommandError::rejected(format!("no clip crosses {}", format_position(&ctx.project, at))));
    }
    ctx.set_label(if cut == 1 { String::from("Slice clip") } else { format!("Slice {cut} clips") });
    Ok(json!({ "sliced": cut, "at": format_position(&ctx.project, at), "newClips": pieces }))
}

fn slip(ctx: &mut Ctx<'_>, p: Slip) -> Result<Value, CommandError> {
    let targets = ctx.clips(&p.clips)?;
    let project = ctx.project.clone();
    let mut clips = take(ctx, &targets)?;
    for (_, clip) in &mut clips {
        clip.offset = match (clip.offset, clip.source) {
            (ClipOffset::Ticks(t), ClipSource::Pattern(pattern)) => {
                let length = pattern_length(&project, pattern).0.max(1);
                ClipOffset::Ticks(Tick((t.0 + p.by).rem_euclid(length)))
            }
            (ClipOffset::Ticks(t), _) => ClipOffset::Ticks(Tick((t.0 + p.by).max(0))),
            (ClipOffset::Frames(f), _) => {
                let seconds = project.tempo.seconds_at(Tick(clip.start.0 + p.by)).0 - project.tempo.seconds_at(clip.start).0;
                let frames = project.settings.sample_rate.seconds_to_frames(bodo_kit_core::Seconds(seconds.abs())).0;
                ClipOffset::Frames(bodo_kit_core::Frames((f.0 + if seconds < 0.0 { -frames } else { frames }).max(0)))
            }
        };
    }
    let count = clips.len();
    put(ctx, clips)?;
    ctx.set_label(if count == 1 { String::from("Slip clip") } else { format!("Slip {count} clips") });
    Ok(json!({ "slipped": count, "by": p.by }))
}

fn make_unique(ctx: &mut Ctx<'_>, p: MakeUnique) -> Result<Value, CommandError> {
    let refs: Vec<ClipRef> = p.clip.into_iter().collect();
    let targets = ctx.clips(&refs)?;
    let [(track, clip)] = targets.as_slice() else {
        return Err(CommandError::invalid("give one clip, or select one"));
    };
    let (track, clip) = (*track, *clip);
    let source = ctx.project.tracks.get(&track).and_then(|t| t.clips.iter().find(|c| c.id == clip)).map(|c| c.source);
    let Some(ClipSource::Pattern(pattern)) = source else {
        return Err(CommandError::rejected("only pattern clips can be made unique"));
    };
    let original = ctx.project.patterns.get(&pattern).cloned().ok_or_else(|| CommandError::not_found("pattern", &pattern.to_string()))?;
    let copy = ctx.ids().pattern();
    let name = unique_name(&original.name, ctx.project.patterns.values().map(|p| p.name.as_str()));
    let color = palette::color(ctx.project.patterns.len());
    ctx.project.patterns.insert(copy, std::sync::Arc::new(bodo_model::Pattern { id: copy, name: name.clone(), color, ..(*original).clone() }));
    ctx.edit_track(track, |t| {
        if let Some(c) = t.clips.iter_mut().find(|c| c.id == clip) {
            c.source = ClipSource::Pattern(copy);
        }
        Ok(())
    })?;
    ctx.editor.pattern = Some(copy);
    ctx.editor.notes.clear();
    ctx.set_label(format!("Make {} unique", original.name));
    Ok(json!({ "clip": clip, "pattern": copy, "name": name }))
}

fn set_color(ctx: &mut Ctx<'_>, p: SetColor) -> Result<Value, CommandError> {
    let targets = ctx.clips(&p.clips)?;
    let color = p.color.map(|c| c.resolve()).transpose()?;
    let mut clips = take(ctx, &targets)?;
    for (_, clip) in &mut clips {
        clip.color = color;
    }
    let count = clips.len();
    put(ctx, clips)?;
    ctx.set_label("Color clips");
    Ok(json!({ "changed": count, "color": color }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("clip.place", "Place clip", "Places a pattern on a playlist track; returns the clip's id.", Effects::UNDOABLE, place)
        .example(json!({ "track": 1, "pattern": "pattern:1", "start": "1:1:0" }))
        .merge(whole);
    r.edit("clip.remove", "Delete clips", "Removes clips from the playlist.", Effects::UNDOABLE, remove).example(json!({ "clips": [] })).merge(whole);
    r.edit("clip.move", "Move clips", "Moves clips in time and optionally to another track.", Effects::UNDOABLE, move_clips)
        .example(json!({ "start": "5:1:0", "track": 2 }))
        .merge(by_clips);
    r.edit("clip.resize", "Resize clips", "Sets clips' length; a clip longer than its pattern repeats it.", Effects::UNDOABLE, resize)
        .example(json!({ "length": "4 bars" }))
        .merge(by_clips);
    r.edit("clip.mute", "Mute clips", "Mutes or unmutes clips; toggles each without `muted`.", Effects::UNDOABLE, mute)
        .example(json!({ "muted": true }))
        .merge(whole);
    r.edit("clip.slice", "Slice clips", "Cuts clips in two at a position; both halves keep playing the same source.", Effects::UNDOABLE, slice)
        .example(json!({ "at": "4:1:0", "tracks": [1] }))
        .merge(whole);
    r.edit("clip.slip", "Slip clips", "Slides the content inside clips without moving or resizing them.", Effects::UNDOABLE, slip)
        .example(json!({ "by": 240 }))
        .merge(by_clips);
    r.edit(
        "clip.make_unique",
        "Make unique",
        "Gives a pattern clip its own copy of the pattern, to edit without changing the other clips.",
        Effects::UNDOABLE,
        make_unique,
    )
    .example(json!({}));
    r.edit("clip.set_color", "Set clip color", "Colors clips, or gives them their source's color again with null.", Effects::UNDOABLE, set_color)
        .example(json!({ "color": "#22d3ee" }));
}
