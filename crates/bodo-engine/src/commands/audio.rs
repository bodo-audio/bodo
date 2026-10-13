//! `channel.add_audio`, `clip.place_audio`, `clip.place_sound`, `clip.set_stretch` and `clip.set_fades`: audio clips
//! on the playlist (project.md section 6.7).
//!
//! An audio clip channel plays one sample wherever its clips sit on the playlist. Its stretch mode says how the clips
//! follow the tempo: `none` plays the file at its own speed, `resample` speeds it up or slows it down (pitch and all)
//! so the sample's tempo matches the project's, and `stretch` does the same while keeping the pitch.

use std::path::Path;
use std::sync::Arc;

use bodo_kit_core::{PPQ, Seconds, Tick};
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::playlist::{ClipOffset, ClipSource, Fades};
use bodo_model::{Channel, ChannelId, Clip, SampleId, TrackId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{by_clips, in_range, insert_number, unique_name};
use crate::error::CommandError;
use crate::palette;
use crate::params::{ChannelRef, ClipRef, InsertRef, Length, Position, TrackRef, format_length, format_position};
use crate::registry::{Effects, Registry};
use crate::samples;
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddAudio {
    /// The audio file: a `factory:` path from browser.search, an absolute path, or a path inside the project's folder.
    sample: String,
    /// Its name; the file's name by default.
    name: Option<String>,
    /// The sample's own tempo in BPM, for resample and stretch; the project's current tempo by default.
    tempo: Option<f64>,
    /// The mixer insert it plays into; the master by default.
    target: Option<InsertRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaceAudio {
    /// The track; the selected track (or the first) by default.
    track: Option<TrackRef>,
    /// The audio clip channel whose sample the clip plays.
    channel: ChannelRef,
    /// Where the clip starts.
    start: Position,
    /// How long it plays; the whole sample by default.
    length: Option<Length>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaceSound {
    /// The track; the selected track (or the first) by default.
    track: Option<TrackRef>,
    /// The audio file: a `factory:` path, an absolute path, or a path inside the project's folder.
    sample: String,
    /// Where the clip starts.
    start: Position,
}

/// How an audio clip channel follows the tempo.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Mode {
    /// The file plays at its own speed.
    None,
    /// Faster or slower, pitch and all, so the sample's tempo matches the project's.
    Resample,
    /// Time-stretched to the project's tempo, keeping the pitch.
    Stretch,
}

impl From<Mode> for StretchMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::None => Self::None,
            Mode::Resample => Self::Resample,
            Mode::Stretch => Self::Stretch,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetStretch {
    /// The audio clip channel.
    channel: ChannelRef,
    /// How its clips follow the tempo.
    mode: Mode,
    /// The sample's own tempo in BPM; unchanged when left out.
    tempo: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetFades {
    /// The clips; the selected clips by default.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// The fade-in's length; 0 removes it, and it is unchanged when left out.
    fade_in: Option<Length>,
    /// The fade-out's length; 0 removes it, and it is unchanged when left out.
    fade_out: Option<Length>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Consolidate {
    /// The clips; the selected clips by default. Automation clips are left as they are.
    #[serde(default)]
    clips: Vec<ClipRef>,
    /// The new channel's name; "Consolidated" by default.
    name: Option<String>,
}

/// The project as heard with only `keep` of the pattern and audio clips playing: their tracks unmuted, every other
/// pattern and audio clip gone, automation as it is, and the master strip neutral, since the result plays through it
/// again.
///
/// Complexity: O(c) in clips.
fn only_clips(project: &bodo_model::Project, keep: &std::collections::BTreeSet<bodo_model::ClipId>) -> bodo_model::Project {
    let mut copy = project.clone();
    for track in project.tracks.values() {
        let mut t = (**track).clone();
        t.clips = t.clips.iter().filter(|c| matches!(c.source, ClipSource::Automation(_)) || keep.contains(&c.id)).cloned().collect();
        for clip in t.clips.iter_mut() {
            if keep.contains(&clip.id) {
                clip.muted = false;
            }
        }
        if t.clips.iter().any(|c| keep.contains(&c.id)) {
            t.muted = false;
        }
        // Other tracks hold only automation now, so no solo is needed to hear the clips alone.
        t.solo = false;
        copy.tracks.insert(t.id, Arc::new(t));
    }
    let mixer = Arc::make_mut(&mut copy.mixer);
    let master = Arc::make_mut(&mut mixer.master);
    *master =
        bodo_model::MixerInsert { id: master.id, name: master.name.clone(), color: master.color, ..bodo_model::MixerInsert::new(master.id, "") };
    copy
}

/// A file in `folder` named `base.wav`, or `base 2.wav` and so on, that doesn't exist yet.
fn fresh_file(folder: &Path, base: &str) -> std::path::PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{base}.wav") } else { format!("{base} {n}.wav") };
        let path = folder.join(name);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

fn consolidate(ctx: &mut Ctx<'_>, p: Consolidate) -> Result<Value, CommandError> {
    let mut chosen: Vec<(TrackId, Clip)> = Vec::new();
    for (track, id) in ctx.clips(&p.clips)? {
        let Some(t) = ctx.project.tracks.get(&track) else { continue };
        if t.locked {
            return Err(CommandError::rejected(format!("{} is locked", t.name)).with_hint("unlock it with playlist.set_track_props"));
        }
        if let Some(clip) = t.clips.iter().find(|c| c.id == id && !matches!(c.source, ClipSource::Automation(_))) {
            chosen.push((track, clip.clone()));
        }
    }
    let (Some(start), Some(end)) = (chosen.iter().map(|(_, c)| c.start).min(), chosen.iter().map(|(_, c)| c.end()).max()) else {
        return Err(CommandError::rejected("choose pattern or audio clips to consolidate"));
    };
    let keep: std::collections::BTreeSet<_> = chosen.iter().map(|(_, c)| c.id).collect();
    let options = bodo_mix::OfflineOptions { start, end, tail: Seconds(0.0), mode: bodo_mix::PlayMode::Song, block: 512, workers: 0 };
    let audio = bodo_mix::render_offline(&only_clips(&ctx.project, &keep), &ctx.sample_roots(), &options)
        .map_err(|e| CommandError::rejected(format!("the clips could not be rendered: {e}")))?;
    let folder = ctx.project_dir().map_or_else(|| std::env::temp_dir().join("Bodo Rendered"), |dir| dir.join("Rendered"));
    std::fs::create_dir_all(&folder).map_err(|e| CommandError::io(&folder, e.to_string()))?;
    let base = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Consolidated").to_owned();
    let file = fresh_file(&folder, &base);
    let channels: Vec<&[f32]> = audio.channels.iter().map(Vec::as_slice).collect();
    let frames = channels.first().map_or(0, |c| c.len());
    let mut writer = bodo_kit_audio::WavWriter::create(&file, ctx.project.settings.sample_rate, channels.len(), bodo_kit_audio::WavFormat::Float32)
        .map_err(|e| CommandError::io(&file, e.to_string()))?;
    writer.write(&channels, frames).map_err(|e| CommandError::io(&file, e.to_string()))?;
    writer.finish().map_err(|e| CommandError::io(&file, e.to_string()))?;
    // The new clip goes on the topmost track the clips were on, where they started.
    let track = ctx.project.track_order.iter().copied().find(|t| chosen.iter().any(|(c, _)| c == t)).unwrap_or(chosen[0].0);
    for (t, clip) in &chosen {
        ctx.edit_track(*t, |t| {
            t.clips.retain(|c| c.id != clip.id);
            Ok(())
        })?;
    }
    let tempo = ctx.project.tempo.bpm_at(start);
    let channel = make_channel(ctx, &file.display().to_string(), Some(base), Some(tempo), None)?;
    let length = Length::Ticks(end.0 - start.0);
    let clip = place(ctx, track, channel, start, Some(&length))?;
    ctx.set_label(format!("Consolidate {} clips", chosen.len()));
    let mut result = clip_json(ctx, &clip, track, channel);
    result["file"] = json!(file.display().to_string());
    result["replaced"] = json!(chosen.len());
    Ok(result)
}

/// The name of channel `id`, for labels.
fn name(ctx: &Ctx<'_>, id: ChannelId) -> String {
    ctx.project.channels.get(&id).map(|c| c.name.clone()).unwrap_or_default()
}

/// The sample, stretch mode and sample tempo of audio clip channel `id`, or an error naming it.
fn audio_channel(ctx: &Ctx<'_>, id: ChannelId) -> Result<(SampleId, StretchMode, Option<f64>), CommandError> {
    match ctx.project.channels.get(&id).map(|c| &c.kind) {
        Some(ChannelKind::AudioClip { sample, stretch, tempo }) => Ok((*sample, *stretch, *tempo)),
        _ => Err(CommandError::rejected(format!("{} is not an audio clip channel", name(ctx, id))).with_hint("make one with channel.add_audio")),
    }
}

/// The length in seconds of the file at `path`.
///
/// Does I/O: reads the file's header, or decodes it when the header doesn't say.
fn file_seconds(path: &Path) -> Result<f64, CommandError> {
    let decoder = bodo_kit_audio::Decoder::open(path).map_err(|e| CommandError::io(path, e))?;
    let info = decoder.info();
    let frames = match info.frames {
        Some(frames) => frames as f64,
        None => bodo_kit_audio::decode_file(path).map_err(|e| CommandError::io(path, e))?.frames() as f64,
    };
    Ok(frames / f64::from(info.rate.hz()))
}

/// How long a clip of channel `id` lasts when it plays the whole sample from `start`: real time through the tempo
/// map for files at their own speed, beats at the sample's tempo otherwise.
///
/// Does I/O: reads the sample's header.
fn sample_length(ctx: &Ctx<'_>, id: ChannelId, start: Tick) -> Result<Tick, CommandError> {
    let (sample, mode, tempo) = audio_channel(ctx, id)?;
    let reference = ctx.project.samples.get(&sample).ok_or_else(|| CommandError::not_found("sample", &sample.to_string()))?;
    let seconds = file_seconds(&ctx.sample_roots().resolve(&reference.path))?;
    let ticks = match (mode, tempo) {
        (StretchMode::None, _) | (_, None) => {
            let tempo = &ctx.project.tempo;
            tempo.ticks_at(Seconds(tempo.seconds_at(start).0 + seconds)) - start.0 as f64
        }
        (_, Some(bpm)) => seconds * bpm / 60.0 * PPQ as f64,
    };
    Ok(Tick((ticks.round() as i64).max(1)))
}

/// Makes an audio clip channel for the file `text` names.
fn make_channel(
    ctx: &mut Ctx<'_>,
    text: &str,
    name: Option<String>,
    tempo: Option<f64>,
    target: Option<&InsertRef>,
) -> Result<ChannelId, CommandError> {
    if let Some(tempo) = tempo {
        in_range("tempo", tempo, 10.0..=999.0)?;
    }
    let sample = samples::import(ctx, text)?;
    let base = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| samples::display_name(text));
    let name = unique_name(base.trim(), ctx.project.channels.values().map(|c| c.name.as_str()));
    let target = match target {
        Some(r) => Some(ctx.insert(Some(r))?).filter(|i| *i != ctx.project.mixer.master.id),
        None => None,
    };
    let id = ctx.ids().channel();
    let tempo = Some(tempo.unwrap_or_else(|| ctx.project.tempo.bpm_at(Tick::ZERO)));
    let channel = Channel {
        id,
        name,
        color: palette::channel_color(ctx.project.channels.len()),
        kind: ChannelKind::AudioClip { sample, stretch: StretchMode::None, tempo },
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
    Ok(id)
}

/// Puts a clip of channel `channel` on `track` from `start`, `length` long or the whole sample. Returns the clip.
fn place(ctx: &mut Ctx<'_>, track: TrackId, channel: ChannelId, start: Tick, length: Option<&Length>) -> Result<Clip, CommandError> {
    if let Some(t) = ctx.project.tracks.get(&track)
        && t.locked
    {
        return Err(CommandError::rejected(format!("{} is locked", t.name)).with_hint("unlock it with playlist.set_track_props"));
    }
    if start < Tick::ZERO {
        return Err(CommandError::invalid("a clip can't start before the song"));
    }
    let len = match length {
        Some(length) => length.resolve(&ctx.project, start)?,
        None => sample_length(ctx, channel, start)?,
    };
    audio_channel(ctx, channel)?;
    let clip = Clip {
        id: ctx.ids().clip(),
        start,
        len,
        offset: ClipOffset::default(),
        muted: false,
        color: None,
        source: ClipSource::Audio(channel),
        fades: None,
    };
    let placed = clip.clone();
    ctx.edit_track(track, |t| {
        t.insert_clip(clip);
        Ok(())
    })?;
    ctx.editor.track = Some(track);
    ctx.editor.clips = std::collections::BTreeSet::from([placed.id]);
    Ok(placed)
}

fn clip_json(ctx: &Ctx<'_>, clip: &Clip, track: TrackId, channel: ChannelId) -> Value {
    json!({
        "clip": clip.id,
        "track": track,
        "channel": channel,
        "start": format_position(&ctx.project, clip.start),
        "length": format_length(&ctx.project, clip.len),
        "ticks": clip.len.0,
    })
}

fn add_audio(ctx: &mut Ctx<'_>, p: AddAudio) -> Result<Value, CommandError> {
    let id = make_channel(ctx, &p.sample, p.name, p.tempo, p.target.as_ref())?;
    let (_, _, tempo) = audio_channel(ctx, id)?;
    let channel = ctx.project.channels.get(&id).cloned().ok_or_else(|| CommandError::not_found("channel", &id.to_string()))?;
    ctx.set_label(format!("Add audio channel {}", channel.name));
    let insert = channel.target.and_then(|t| insert_number(&ctx.project, t)).unwrap_or(0);
    Ok(json!({ "channel": id, "name": channel.name, "insert": insert, "tempo": tempo }))
}

fn place_audio(ctx: &mut Ctx<'_>, p: PlaceAudio) -> Result<Value, CommandError> {
    let track = ctx.track(p.track.as_ref())?;
    let channel = ctx.channel(Some(&p.channel))?;
    let start = p.start.resolve(&ctx.project)?;
    let clip = place(ctx, track, channel, start, p.length.as_ref())?;
    ctx.set_label(format!("Place {} at {}", name(ctx, channel), format_position(&ctx.project, start)));
    Ok(clip_json(ctx, &clip, track, channel))
}

fn place_sound(ctx: &mut Ctx<'_>, p: PlaceSound) -> Result<Value, CommandError> {
    let track = ctx.track(p.track.as_ref())?;
    let start = p.start.resolve(&ctx.project)?;
    let file = samples::locate(ctx, &p.sample);
    let stored = samples::stored_path(&file, ctx.project_dir().as_deref(), ctx.env().factory.as_deref());
    // A channel that already plays this file takes the clip; otherwise a new one does.
    let existing = ctx.project.rack_order.iter().filter_map(|id| ctx.project.channels.get(id)).find_map(|c| match &c.kind {
        ChannelKind::AudioClip { sample, .. } if ctx.project.samples.get(sample).is_some_and(|s| s.path == stored) => Some(c.id),
        _ => None,
    });
    let channel = match existing {
        Some(id) => id,
        None => make_channel(ctx, &p.sample, None, None, None)?,
    };
    let clip = place(ctx, track, channel, start, None)?;
    ctx.set_label(format!("Place {} at {}", name(ctx, channel), format_position(&ctx.project, start)));
    let mut out = clip_json(ctx, &clip, track, channel);
    if let Some(map) = out.as_object_mut() {
        map.insert(String::from("newChannel"), Value::Bool(existing.is_none()));
    }
    Ok(out)
}

fn set_stretch(ctx: &mut Ctx<'_>, p: SetStretch) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let (_, _, old_tempo) = audio_channel(ctx, id)?;
    if let Some(tempo) = p.tempo {
        in_range("tempo", tempo, 10.0..=999.0)?;
    }
    let mode = StretchMode::from(p.mode);
    let tempo = p.tempo.or(old_tempo).or_else(|| Some(ctx.project.tempo.bpm_at(Tick::ZERO)));
    ctx.edit_channel(id, |c| {
        if let ChannelKind::AudioClip { stretch, tempo: t, .. } = &mut c.kind {
            *stretch = mode;
            *t = tempo;
        }
    })?;
    ctx.set_label(format!("Set {} stretch mode", name(ctx, id)));
    Ok(json!({ "channel": id, "mode": mode, "tempo": tempo }))
}

/// A fade length: zero (`0`, `"0 beats"`), or a length.
fn fade_length(ctx: &Ctx<'_>, length: &Length, at: Tick) -> Result<Tick, CommandError> {
    let zero = match length {
        Length::Ticks(ticks) => *ticks == 0,
        Length::Text(text) => {
            text.trim().split(|c: char| c.is_ascii_alphabetic() || c.is_whitespace()).next().and_then(|n| n.parse::<f64>().ok()) == Some(0.0)
        }
    };
    if zero { Ok(Tick::ZERO) } else { length.resolve(&ctx.project, at) }
}

fn set_fades(ctx: &mut Ctx<'_>, p: SetFades) -> Result<Value, CommandError> {
    if p.fade_in.is_none() && p.fade_out.is_none() {
        return Err(CommandError::invalid("give fadeIn, fadeOut or both"));
    }
    let targets = ctx.clips(&p.clips)?;
    let mut by_track: std::collections::BTreeMap<TrackId, Vec<bodo_model::ClipId>> = std::collections::BTreeMap::new();
    for (track, clip) in &targets {
        if let Some(t) = ctx.project.tracks.get(track)
            && t.locked
        {
            return Err(CommandError::rejected(format!("{} is locked", t.name)));
        }
        by_track.entry(*track).or_default().push(*clip);
    }
    for (track, ids) in by_track {
        let mut updated = Vec::new();
        if let Some(t) = ctx.project.tracks.get(&track) {
            for clip in t.clips.iter().filter(|c| ids.contains(&c.id)) {
                let mut fades = clip.fades.unwrap_or_default();
                if let Some(fade) = &p.fade_in {
                    fades.fade_in = fade_length(ctx, fade, clip.start)?.min(clip.len);
                }
                if let Some(fade) = &p.fade_out {
                    fades.fade_out = fade_length(ctx, fade, clip.start)?.min(clip.len);
                }
                updated.push((clip.id, (fades != Fades::default()).then_some(fades)));
            }
        }
        ctx.edit_track(track, |t| {
            for clip in t.clips.iter_mut() {
                if let Some((_, fades)) = updated.iter().find(|(id, _)| *id == clip.id) {
                    clip.fades = *fades;
                }
            }
            Ok(())
        })?;
    }
    ctx.set_label(if targets.len() == 1 { String::from("Set clip fades") } else { format!("Set fades of {} clips", targets.len()) });
    Ok(json!({ "clips": targets.len() }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "clip.consolidate",
        "Consolidate to audio",
        "Renders clips to a WAV in the project's Rendered folder and replaces them with one audio clip.",
        Effects { undoable: true, destructive: false, disk: true, network: false },
        consolidate,
    )
    .example(json!({ "clips": ["Verse"] }));
    r.edit(
        "channel.add_audio",
        "Add audio clip channel",
        "Adds a channel that plays an audio file as clips on the playlist.",
        Effects::UNDOABLE,
        add_audio,
    )
    .example(json!({ "sample": "factory:Packs/Bodo Drums/Crash.wav", "name": "Crash" }))
    .menu("Add/Audio clip channel");
    r.edit(
        "clip.place_audio",
        "Place audio clip",
        "Places a clip of an audio clip channel on a playlist track; it lasts the whole sample by default.",
        Effects::UNDOABLE,
        place_audio,
    )
    .example(json!({ "track": 3, "channel": "Crash", "start": "2:1:0" }))
    .merge(super::whole);
    r.edit(
        "clip.place_sound",
        "Place sound",
        "Places an audio file on a playlist track as an audio clip, reusing the channel that already plays it or making one.",
        Effects::UNDOABLE,
        place_sound,
    )
    .example(json!({ "track": 3, "sample": "factory:Packs/Bodo Drums/Crash.wav", "start": "1:1:0" }));
    r.edit(
        "clip.set_stretch",
        "Set stretch mode",
        "Sets how an audio clip channel follows the tempo: none, resample or stretch.",
        Effects::UNDOABLE,
        set_stretch,
    )
    .example(json!({ "channel": "Crash", "mode": "stretch", "tempo": 120.0 }));
    r.edit("clip.set_fades", "Set clip fades", "Sets the fade-in and fade-out of clips; 0 removes a fade.", Effects::UNDOABLE, set_fades)
        .example(json!({ "fadeIn": "1 beat", "fadeOut": 0 }))
        .merge(by_clips);
}
