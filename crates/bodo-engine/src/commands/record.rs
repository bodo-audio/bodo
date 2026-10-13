//! Arming inserts, recording and takes (project.md section 9.6).
//!
//! An armed insert with a hardware input monitors it, and records its record tap while the transport records
//! (`transport.record`). Whoever plays audio writes the takes and then calls `recording.add_takes`: each take becomes
//! a sample, an audio clip channel that plays into the insert it was recorded on, and a clip on the playlist at the
//! take's latency-compensated start.

use std::sync::Arc;

use bodo_kit_core::{Frames, Seconds, Tick};
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::mixer::{HwPort, RecordFormat, RecordTap};
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{Channel, ChannelId, Clip, PlaylistTrack, Project, SampleId, TrackId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{insert_number, unique_name};
use crate::error::CommandError;
use crate::palette;
use crate::params::{InsertRef, Position, format_position};
use crate::registry::{Effects, Registry};
use crate::samples;
use crate::session::Ctx;

/// The most device input channels an insert can name.
const MAX_INPUT_CHANNEL: u16 = 255;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Arm {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// Armed or not; toggles when left out.
    on: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetInput {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The device input channels it records and monitors, counted from 0: one for mono, two for stereo. Null for none.
    channels: Option<Vec<u16>>,
}

/// Where an armed insert records from.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Tap {
    /// The summed input.
    Input,
    /// Before the FX.
    PreFx,
    /// After the FX and EQ.
    PostFx,
    /// After the fader.
    PostFader,
}

impl From<Tap> for RecordTap {
    fn from(tap: Tap) -> Self {
        match tap {
            Tap::Input => Self::Input,
            Tap::PreFx => Self::PreFx,
            Tap::PostFx => Self::PostFx,
            Tap::PostFader => Self::PostFader,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetRecordTap {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// Where on the strip it records.
    tap: Tap,
}

/// The file format of recordings.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Format {
    /// 32-bit float WAV.
    Float32,
    /// 24-bit WAV.
    Int24,
    /// 24-bit FLAC, lossless and about half the size.
    Flac,
    /// 32-bit float Wave64, for takes longer than WAV's 4 GB.
    W64,
}

impl From<Format> for RecordFormat {
    fn from(format: Format) -> Self {
        match format {
            Format::Float32 => Self::Float32,
            Format::Int24 => Self::Int24,
            Format::Flac => Self::Flac,
            Format::W64 => Self::W64,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetRecordFormat {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The file format.
    format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TakeSpec {
    /// The insert that recorded it.
    insert: InsertRef,
    /// The recorded audio file.
    path: String,
    /// Where its first frame belongs after latency compensation. Ticks may be negative when the take began before
    /// the song; that part is skipped.
    start: Position,
    /// How many frames it holds.
    frames: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddTakes {
    /// The takes of one recording.
    takes: Vec<TakeSpec>,
}

/// An audio clip channel kind playing `sample` at its own speed, recorded at `tempo`.
fn audio_clip_kind(sample: SampleId, tempo: f64) -> ChannelKind {
    ChannelKind::AudioClip { sample, stretch: StretchMode::None, tempo: Some(tempo) }
}

/// Seconds of `frames` at the project's rate.
fn seconds(project: &Project, frames: u64) -> f64 {
    frames as f64 / f64::from(project.settings.sample_rate.hz())
}

/// Where a take of `frames` frames starting at tick `start` sits: the clip's start, its offset into the file, and its
/// length. A start before the song skips the part before it.
fn placement(project: &Project, start: Tick, frames: u64) -> Result<(Tick, Frames, Tick), CommandError> {
    let rate = f64::from(project.settings.sample_rate.hz());
    let (begin, skip) = if start >= Tick::ZERO {
        (start, 0)
    } else {
        let skip = (project.tempo.seconds_at(Tick(-start.0)).0 * rate).round() as u64;
        (Tick::ZERO, skip)
    };
    if skip >= frames {
        return Err(CommandError::rejected("the take ends before the song starts, so there is nothing to place"));
    }
    let from = project.tempo.seconds_at(begin).0;
    let end = project.tempo.ticks_at(Seconds(from + seconds(project, frames - skip))).round() as i64;
    Ok((begin, Frames(skip as i64), Tick((end - begin.0).max(1))))
}

/// The first track, in order, with no clip overlapping `start..end`; `None` when every track has one there.
///
/// Complexity: O(t * c) in tracks and their clips.
fn free_track(project: &Project, start: Tick, end: Tick) -> Option<TrackId> {
    project
        .track_order
        .iter()
        .filter_map(|id| project.tracks.get(id))
        .find(|track| !track.locked && !track.clips.iter().any(|c| c.start < end && c.end() > start))
        .map(|track| track.id)
}

fn arm(ctx: &mut Ctx<'_>, p: Arm) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let on = p.on.unwrap_or_else(|| !ctx.project.mixer.insert(id).is_some_and(|i| i.armed));
    ctx.edit_insert(id, |i| i.armed = on)?;
    let name = ctx.project.mixer.insert(id).map(|i| i.name.clone()).unwrap_or_default();
    ctx.set_label(format!("{} {name}", if on { "Arm" } else { "Disarm" }));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "armed": on }))
}

fn set_input(ctx: &mut Ctx<'_>, p: SetInput) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let port = match p.channels {
        None => None,
        Some(channels) if (1..=2).contains(&channels.len()) && channels.iter().all(|c| *c <= MAX_INPUT_CHANNEL) => Some(HwPort { channels }),
        Some(_) => {
            return Err(CommandError::invalid(format!("give one input channel for mono or two for stereo, each from 0 to {MAX_INPUT_CHANNEL}")));
        }
    };
    let channels = port.as_ref().map(|p| p.channels.clone());
    ctx.edit_insert(id, |i| i.input = port)?;
    ctx.set_label("Set insert input");
    Ok(json!({ "insert": insert_number(&ctx.project, id), "channels": channels }))
}

fn set_record_tap(ctx: &mut Ctx<'_>, p: SetRecordTap) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let tap = RecordTap::from(p.tap);
    ctx.edit_insert(id, |i| i.record_tap = tap)?;
    ctx.set_label("Set record tap");
    Ok(json!({ "insert": insert_number(&ctx.project, id), "tap": tap }))
}

fn set_record_format(ctx: &mut Ctx<'_>, p: SetRecordFormat) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let format = RecordFormat::from(p.format);
    ctx.edit_insert(id, |i| i.record_format = format)?;
    ctx.set_label("Set record format");
    Ok(json!({ "insert": insert_number(&ctx.project, id), "format": format }))
}

fn add_takes(ctx: &mut Ctx<'_>, p: AddTakes) -> Result<Value, CommandError> {
    if p.takes.is_empty() {
        return Err(CommandError::invalid("give at least one take"));
    }
    let mut placed = Vec::with_capacity(p.takes.len());
    for take in &p.takes {
        let insert = ctx.insert(Some(&take.insert))?;
        let insert_name = ctx.project.mixer.insert(insert).map(|i| i.name.clone()).unwrap_or_default();
        let start = take.start.resolve(&ctx.project)?;
        let (begin, skip, len) = placement(&ctx.project, start, take.frames)?;
        let sample = samples::import(ctx, &take.path)?;

        let base = format!("{insert_name} take");
        let count = ctx.project.channels.values().filter(|c| c.name.starts_with(&base)).count();
        let name = unique_name(&format!("{base} {}", count + 1), ctx.project.channels.values().map(|c| c.name.as_str()));
        let channel: ChannelId = ctx.ids().channel();
        let target = (insert != ctx.project.mixer.master.id).then_some(insert);
        ctx.project.channels.insert(
            channel,
            Arc::new(Channel {
                id: channel,
                name: name.clone(),
                color: palette::channel_color(ctx.project.channels.len()),
                kind: audio_clip_kind(sample, ctx.project.tempo.bpm_at(begin)),
                target,
                volume: 1.0,
                pan: 0.0,
                pitch_cents: 0.0,
                swing: 0.0,
                muted: false,
                midi_in: None,
                group: None,
            }),
        );
        ctx.project.rack_order.push_back(channel);

        let end = Tick(begin.0 + len.0);
        let track = match free_track(&ctx.project, begin, end) {
            Some(track) => track,
            None => {
                let id = ctx.ids().track();
                let track_name =
                    unique_name(&format!("Track {}", ctx.project.track_order.len() + 1), ctx.project.tracks.values().map(|t| t.name.as_str()));
                ctx.project.tracks.insert(id, Arc::new(PlaylistTrack::new(id, track_name, bodo_model::Color::GRAY)));
                ctx.project.track_order.push_back(id);
                id
            }
        };
        let clip = Clip {
            id: ctx.ids().clip(),
            start: begin,
            len,
            offset: ClipOffset::Frames(skip),
            muted: false,
            color: None,
            source: ClipSource::Audio(channel),
            fades: None,
        };
        let clip_id = clip.id;
        ctx.edit_track(track, |t| {
            t.insert_clip(clip);
            Ok(())
        })?;
        placed.push(json!({
            "channel": channel, "name": name, "clip": clip_id, "track": track, "start": format_position(&ctx.project, begin),
            "ticks": begin.0, "length": len.0, "skippedFrames": skip.0,
        }));
    }
    ctx.set_label(if placed.len() == 1 { String::from("Record take") } else { format!("Record {} takes", placed.len()) });
    Ok(json!({ "takes": placed }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "mixer.arm",
        "Arm insert",
        "Arms an insert for recording, or disarms it; toggles without `on`. An armed insert monitors its input.",
        Effects::UNDOABLE,
        arm,
    )
    .example(json!({ "insert": 1, "on": true }));
    r.edit(
        "mixer.set_input",
        "Set insert input",
        "Sets the device input channels an insert records and monitors, or none.",
        Effects::UNDOABLE,
        set_input,
    )
    .example(json!({ "insert": 1, "channels": [0, 1] }));
    r.edit("mixer.set_record_tap", "Set record tap", "Sets where on its strip an armed insert records from.", Effects::UNDOABLE, set_record_tap)
        .example(json!({ "insert": 1, "tap": "input" }));
    r.edit("mixer.set_record_format", "Set record format", "Sets the file format an insert records in.", Effects::UNDOABLE, set_record_format)
        .example(json!({ "insert": 1, "format": "int24" }));
    r.edit(
        "recording.add_takes",
        "Add recorded takes",
        "Places recorded takes on the playlist: a sample, an audio clip channel into the recording insert, and a clip for each.",
        Effects::UNDOABLE,
        add_takes,
    )
    .example(json!({ "takes": [{ "insert": 1, "path": "Recorded/Insert 1_1.wav", "start": "2:1:0", "frames": 96000 }] }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_before_the_song_skip_what_came_before() {
        let project = Project::new(120.0).unwrap();
        // 120 BPM: 960 ticks is half a second, 24 000 frames at 48 kHz.
        assert_eq!(placement(&project, Tick(3840), 48_000), Ok((Tick(3840), Frames(0), Tick(1920))));
        assert_eq!(placement(&project, Tick(-960), 48_000), Ok((Tick::ZERO, Frames(24_000), Tick(960))));
        assert!(placement(&project, Tick(-1920), 48_000).is_err(), "a take that ends before the song has nothing to place");
    }
}
