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

use super::{by_clips, pattern_length};
use crate::error::CommandError;
use crate::params::{ClipRef, Length, PatternRef, Position, TrackRef, format_position};
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

pub(super) fn register(r: &mut Registry) {
    r.edit("clip.place", "Place clip", "Places a pattern on a playlist track; returns the clip's id.", Effects::UNDOABLE, place)
        .example(json!({ "track": 1, "pattern": "pattern:1", "start": "1:1:0" }));
    r.edit("clip.remove", "Delete clips", "Removes clips from the playlist.", Effects::UNDOABLE, remove).example(json!({ "clips": [] }));
    r.edit("clip.move", "Move clips", "Moves clips in time and optionally to another track.", Effects::UNDOABLE, move_clips)
        .example(json!({ "start": "5:1:0", "track": 2 }))
        .merge(by_clips);
    r.edit("clip.resize", "Resize clips", "Sets clips' length; a clip longer than its pattern repeats it.", Effects::UNDOABLE, resize)
        .example(json!({ "length": "4 bars" }))
        .merge(by_clips);
    r.edit("clip.mute", "Mute clips", "Mutes or unmutes clips; toggles each without `muted`.", Effects::UNDOABLE, mute)
        .example(json!({ "muted": true }));
}
