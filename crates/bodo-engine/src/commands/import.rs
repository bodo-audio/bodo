//! MIDI file import and export (project.md section 18.2).
//!
//! Importing makes one new pattern named after the file, with one new sampler channel per track and MIDI channel of
//! the file, and places it on the playlist. Exporting writes a pattern, or the whole playlist as it plays, as a
//! Standard MIDI File. Relative paths start at the project's folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_core::{TempoMap, Tick};
use bodo_kit_midi::smf::EventKind;
use bodo_kit_midi::{MidiMessage, Smf};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::pattern::NoteSeq;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{Channel, ChannelId, Clip, Color, MeterMap, Note, NoteId, Pattern, PlaylistTrack};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{insert_number, pattern_length, unique_name};
use crate::error::{CommandError, ErrorCode};
use crate::palette;
use crate::params::{PatternRef, Position, TrackRef, format_position};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Import {
    /// The `.mid` file: an absolute path, or one relative to the project's folder.
    path: String,
    /// Make the file's first tempo and time signature the project's; true by default.
    #[serde(default = "yes")]
    tempo: bool,
    /// Place a clip of the new pattern on the playlist; true by default.
    #[serde(default = "yes")]
    place: bool,
    /// The track for the clip; the first track with no clips by default (a new track when none is empty).
    track: Option<TrackRef>,
    /// Where the clip starts; bar 1 by default.
    at: Option<Position>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportPattern {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// Where to write the `.mid` file: an absolute path, or one relative to the project's folder. `.mid` is added
    /// when the path has no extension.
    path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportProject {
    /// Where to write the `.mid` file: an absolute path, or one relative to the project's folder. `.mid` is added
    /// when the path has no extension.
    path: String,
}

/// Where `text` points: absolute, or relative to the project's folder (the current folder for an unsaved project).
fn locate(ctx: &Ctx<'_>, text: &str) -> PathBuf {
    let path = PathBuf::from(text.trim());
    if path.is_absolute() { path } else { ctx.sample_roots().project.join(path) }
}

/// Where to write a MIDI file named by `text`, with `.mid` added when it has no extension.
fn target(ctx: &Ctx<'_>, text: &str) -> PathBuf {
    let path = locate(ctx, text);
    if path.extension().is_none() { path.with_extension("mid") } else { path }
}

fn io_error(error: &bodo_io::IoError) -> CommandError {
    CommandError::new(ErrorCode::Io, error.to_string())
}

/// The number of notes in `smf`.
fn notes_in(smf: &Smf) -> usize {
    smf.tracks
        .iter()
        .flat_map(|t| t.events.iter())
        .filter(|e| matches!(e.kind, EventKind::Channel { message: MidiMessage::NoteOn { .. }, .. }))
        .count()
}

/// Writes `smf` to `path`, making its folder when needed.
fn write(path: &Path, smf: &Smf) -> Result<Value, CommandError> {
    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).map_err(|e| CommandError::io(folder, e))?;
    }
    bodo_io::write_midi_file(path, smf).map_err(|e| io_error(&e))?;
    Ok(json!({ "path": path.display().to_string(), "tracks": smf.tracks.len().saturating_sub(1), "notes": notes_in(smf) }))
}

fn import(ctx: &mut Ctx<'_>, p: Import) -> Result<Value, CommandError> {
    let path = locate(ctx, &p.path);
    let midi = bodo_io::read_midi_file(&path).map_err(|e| io_error(&e))?;
    if midi.parts.is_empty() {
        return Err(CommandError::rejected(format!("{} has no notes", path.display())));
    }
    let base = path.file_stem().map_or_else(|| String::from("MIDI"), |s| s.to_string_lossy().trim().to_owned());
    let name = unique_name(if base.is_empty() { "MIDI" } else { &base }, ctx.project.patterns.values().map(|p| p.name.as_str()));

    // The file's opening tempo and time signature.
    let mut tempo = None;
    if p.tempo {
        if let Some(first) = midi.tempo.first() {
            let mut points = ctx.project.tempo.points().to_vec();
            if let Some(start) = points.first_mut() {
                start.bpm = first.bpm;
            }
            let map = TempoMap::new(points).map_err(|e| CommandError::invalid(e.to_string()))?;
            if map != *ctx.project.tempo {
                ctx.project.tempo = Arc::new(map);
            }
            tempo = Some(first.bpm);
        }
        if let Some(first) = midi.meter.first() {
            let mut changes = ctx.project.meter.changes().to_vec();
            match changes.first_mut() {
                Some(change) => *change = *first,
                None => changes.push(*first),
            }
            let meter = MeterMap::new(changes).map_err(|e| CommandError::invalid(e.to_string()))?;
            if meter != *ctx.project.meter {
                ctx.project.meter = Arc::new(meter);
            }
        }
    }

    // One empty sampler channel per part, each on the next insert no channel uses.
    let mut used: BTreeSet<_> = ctx.project.channels.values().filter_map(|c| c.target).collect();
    let pattern_id = ctx.ids().pattern();
    let mut pattern = Pattern::new(pattern_id, name.clone(), palette::color(ctx.project.patterns.len()));
    let mut made = Vec::with_capacity(midi.parts.len());
    let mut first = None;
    for part in &midi.parts {
        let id: ChannelId = ctx.ids().channel();
        first = first.or(Some(id));
        let channel_name = unique_name(&part.name, ctx.project.channels.values().map(|c| c.name.as_str()));
        let target = ctx.project.mixer.order.iter().copied().find(|i| !used.contains(i));
        if let Some(insert) = target {
            used.insert(insert);
        }
        let channel = Channel {
            id,
            name: channel_name.clone(),
            color: palette::channel_color(ctx.project.channels.len()),
            kind: ChannelKind::Sampler(Arc::new(SamplerParams::default())),
            target,
            volume: 1.0,
            pan: 0.0,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
            group: None,
        };
        ctx.project.channels.insert(id, Arc::new(channel));
        ctx.project.rack_order.push_back(id);
        let notes = part.notes.iter().map(|n| {
            let mut note = Note::new(NoteId(pattern.next_note_id), n.key, n.start, n.len);
            pattern.next_note_id += 1;
            note.velocity = n.velocity;
            note.release_velocity = n.release_velocity;
            note.color = part.channel;
            note
        });
        let seq = NoteSeq::new(notes.collect::<Vec<_>>());
        made.push(json!({
            "channel": id,
            "name": channel_name,
            "insert": target.and_then(|t| insert_number(&ctx.project, t)).unwrap_or(0),
            "midiChannel": part.channel + 1,
            "program": part.program,
            "notes": seq.len(),
        }));
        pattern.notes.insert(id, seq);
    }
    ctx.project.patterns.insert(pattern_id, Arc::new(pattern));

    // The clip, on the chosen track or the first empty one.
    let mut placed = None;
    if p.place {
        let track = match &p.track {
            Some(r) => ctx.track(Some(r))?,
            None => match ctx.project.track_order.iter().copied().find(|t| ctx.project.tracks.get(t).is_some_and(|t| t.clips.is_empty())) {
                Some(track) => track,
                None => {
                    let id = ctx.ids().track();
                    let track_name = format!("Track {}", ctx.project.track_order.len() + 1);
                    let track_name = unique_name(&track_name, ctx.project.tracks.values().map(|t| t.name.as_str()));
                    ctx.project.tracks.insert(id, Arc::new(PlaylistTrack::new(id, track_name, Color::GRAY)));
                    ctx.project.track_order.push_back(id);
                    id
                }
            },
        };
        if ctx.project.tracks.get(&track).is_some_and(|t| t.locked) {
            return Err(CommandError::rejected("that track is locked").with_hint("unlock it with playlist.set_track_props, or pick another track"));
        }
        let start = match &p.at {
            Some(at) => at.resolve(&ctx.project)?.max(Tick::ZERO),
            None => Tick::ZERO,
        };
        let len = pattern_length(&ctx.project, pattern_id);
        let clip_id = ctx.ids().clip();
        let clip = Clip {
            id: clip_id,
            start,
            len,
            offset: ClipOffset::default(),
            muted: false,
            color: None,
            source: ClipSource::Pattern(pattern_id),
            fades: None,
        };
        ctx.edit_track(track, |t| {
            t.insert_clip(clip);
            Ok(())
        })?;
        ctx.editor.track = Some(track);
        ctx.editor.clips = BTreeSet::from([clip_id]);
        placed = Some(json!({ "clip": clip_id, "track": track, "start": format_position(&ctx.project, start) }));
    }

    ctx.editor.pattern = Some(pattern_id);
    ctx.editor.notes.clear();
    ctx.editor.channel = first;
    ctx.set_label(format!("Import {name}"));
    Ok(json!({ "pattern": pattern_id, "name": name, "channels": made, "notes": midi.note_count(), "tempo": tempo, "clip": placed }))
}

fn export_pattern(ctx: &mut Ctx<'_>, p: ExportPattern) -> Result<Value, CommandError> {
    let id = ctx.pattern(p.pattern.as_ref())?;
    let smf = bodo_io::export_pattern(&ctx.project, id).map_err(|e| io_error(&e))?;
    write(&target(ctx, &p.path), &smf)
}

fn export_project(ctx: &mut Ctx<'_>, p: ExportProject) -> Result<Value, CommandError> {
    if ctx.project.tracks.values().all(|t| t.clips.is_empty()) {
        return Err(CommandError::rejected("the playlist is empty").with_hint("place clips first, or export one pattern with pattern.export_midi"));
    }
    let smf = bodo_io::export_arrangement(&ctx.project);
    write(&target(ctx, &p.path), &smf)
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "project.import_midi",
        "Import MIDI file",
        "Imports a MIDI file as a new pattern, with an empty sampler channel per part, and places it on the playlist.",
        Effects::UNDOABLE,
        import,
    )
    .example(json!({ "path": "Groove.mid" }))
    .menu("File/Import MIDI file");
    r.edit("pattern.export_midi", "Export pattern as MIDI", "Writes a pattern as a MIDI file, one track per channel.", Effects::DISK, export_pattern)
        .example(json!({ "pattern": "pattern:1", "path": "Verse.mid" }))
        .menu("File/Export pattern as MIDI");
    r.edit(
        "project.export_midi",
        "Export song as MIDI",
        "Writes the whole playlist as a MIDI file, every clip as it plays, one track per channel.",
        Effects::DISK,
        export_project,
    )
    .example(json!({ "path": "Song.mid" }))
    .menu("File/Export song as MIDI");
}
