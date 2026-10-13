//! `preset.*`: channel presets, mixer presets and scores (project.md section 14.1, ADR 0029).
//!
//! Saving writes a file and leaves the project as it is; loading reads one and is a single undo step. Without a
//! path, presets are saved in the user library's folder for their kind, named after what they hold. A path can be
//! absolute, a `library:` key from the Browser, or a name inside that folder.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_io::preset::{
    CHANNEL_PRESET_EXTENSION, ChannelPreset, MIXER_PRESET_EXTENSION, MixerPreset, PresetSample, SCORE_EXTENSION, ScorePreset, read_channel_preset,
    read_mixer_preset, read_score, write_channel_preset, write_mixer_preset, write_score,
};
use bodo_kit_core::Tick;
use bodo_model::channel::ChannelKind;
use bodo_model::{Channel, ChannelId, InsertId, NoteId, SampleId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{insert_number, pattern_length, unique_name};
use crate::error::{CommandError, ErrorCode};
use crate::library::UserLibrary;
use crate::library::user::{LIBRARY_PREFIX, PresetFolder, unused_file};
use crate::palette;
use crate::params::{ChannelRef, InsertRef, PatternRef, Position};
use crate::registry::{Effects, Registry};
use crate::samples;
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveChannel {
    /// The channel; the piano roll's channel, or the one selected channel, by default.
    channel: Option<ChannelRef>,
    /// The preset's name; the channel's name by default.
    name: Option<String>,
    /// Where to write it: an absolute path or a name in the library's Channel presets folder. By default a new file
    /// there named after the preset.
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoadChannel {
    /// The `.bodochannel` file: an absolute path, a `library:` key, or a name in the library's Channel presets folder.
    path: String,
    /// A channel to turn into the preset, keeping its place in the rack, its mixer insert and its MIDI input. By
    /// default the preset becomes a new channel.
    channel: Option<ChannelRef>,
    /// For a new channel, the mixer insert it plays into; the first insert no channel uses by default.
    target: Option<InsertRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveMixer {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The preset's name; the insert's name by default.
    name: Option<String>,
    /// Where to write it: an absolute path or a name in the library's Mixer presets folder. By default a new file
    /// there named after the preset.
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoadMixer {
    /// The `.bodomixer` file: an absolute path, a `library:` key, or a name in the library's Mixer presets folder.
    path: String,
    /// The insert that takes the preset's settings and FX slots; the selected insert by default. It keeps its name,
    /// routing, hardware ports, recording settings, mute and solo.
    insert: Option<InsertRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveScore {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The channel whose notes to save; the piano roll's channel by default.
    channel: Option<ChannelRef>,
    /// The score's name; the pattern's and the channel's names by default.
    name: Option<String>,
    /// Where to write it: an absolute path or a name in the library's Scores folder. By default a new file there
    /// named after the score.
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoadScore {
    /// The `.bodoscore` file: an absolute path, a `library:` key, or a name in the library's Scores folder.
    path: String,
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// The channel that plays the notes; the piano roll's channel by default.
    channel: Option<ChannelRef>,
    /// Where in the pattern the score starts; its start by default.
    at: Option<Position>,
    /// Remove the channel's notes in the pattern first; false by default, which adds the score to them.
    #[serde(default)]
    replace: bool,
}

fn io_error(error: &bodo_io::IoError) -> CommandError {
    CommandError::new(ErrorCode::Io, error.to_string())
}

/// The user library, or an error saying a path is needed.
fn library(ctx: &Ctx<'_>) -> Result<UserLibrary, CommandError> {
    ctx.env()
        .user
        .as_ref()
        .map(UserLibrary::new)
        .ok_or_else(|| CommandError::invalid("there is no user library to keep presets in").with_hint("pass path, an absolute path to the file"))
}

/// The file `text` names: absolute, a `library:` key, or a name in the library's folder for `kind`. `extension` is
/// added when the name has none.
fn preset_file(ctx: &Ctx<'_>, text: &str, kind: PresetFolder, extension: &str) -> Result<PathBuf, CommandError> {
    let text = text.trim();
    let path = if text.starts_with(LIBRARY_PREFIX) {
        library(ctx)?.resolve(text).unwrap_or_default()
    } else if Path::new(text).is_absolute() {
        PathBuf::from(text)
    } else {
        library(ctx)?.presets(kind).join(text)
    };
    Ok(if path.extension().is_none() { path.with_extension(extension) } else { path })
}

/// Where to save a preset named `name`: `path` when given, or a new file in the library's folder for `kind`.
fn save_path(ctx: &Ctx<'_>, path: Option<&str>, name: &str, kind: PresetFolder, extension: &str) -> Result<PathBuf, CommandError> {
    match path {
        Some(text) => preset_file(ctx, text, kind, extension),
        None => Ok(unused_file(&library(ctx)?.presets(kind), name, extension)),
    }
}

/// The text the Browser and agents can give back to load the file at `path`: its `library:` key inside the user
/// library, its path otherwise.
fn shown(ctx: &Ctx<'_>, path: &Path) -> String {
    ctx.env().user.as_ref().and_then(|u| UserLibrary::new(u).key(path)).unwrap_or_else(|| path.display().to_string())
}

/// The preset of channel `id`, with its samples, plugin states and, for a layer, its children's presets.
///
/// Complexity: O(c) in the channel and its children.
fn channel_preset(ctx: &Ctx<'_>, id: ChannelId, depth: usize) -> Result<ChannelPreset, CommandError> {
    let channel = ctx.project.channels.get(&id).ok_or_else(|| CommandError::not_found("channel", &id.to_string()))?;
    let roots = ctx.sample_roots();
    let sample = |id: SampleId| -> Result<PresetSample, CommandError> {
        let stored = ctx.project.samples.get(&id).ok_or_else(|| CommandError::not_found("sample", &id.to_string()))?;
        let path = if bodo_io::paths::is_factory(&stored.path) { stored.path.clone() } else { roots.resolve(&stored.path).display().to_string() };
        Ok(PresetSample { id, path, hash: stored.hash.clone() })
    };
    let mut samples = Vec::new();
    let mut states = BTreeMap::new();
    let mut children = Vec::new();
    match &channel.kind {
        ChannelKind::Sampler(params) => samples.extend(params.sample.map(sample).transpose()?),
        ChannelKind::AudioClip { sample: id, .. } => samples.push(sample(*id)?),
        ChannelKind::Instrument { plugin } => {
            states.insert(plugin.instance.to_string(), plugin.state.to_vec());
        }
        ChannelKind::Layer { children: ids } => {
            if depth > 8 {
                return Err(CommandError::rejected("the layer nests too deeply to save"));
            }
            for child in ids {
                children.push(channel_preset(ctx, *child, depth + 1)?);
            }
        }
        ChannelKind::MidiOut { .. } => {}
        ChannelKind::Automation(_) => {
            return Err(CommandError::rejected(format!(
                "{} is an automation clip: it controls parameters of this project, so it can't be a preset",
                channel.name
            )));
        }
    }
    Ok(ChannelPreset { channel: (**channel).clone(), samples, states, children })
}

/// Makes the channel `preset` describes, with id `id`, and the channels of a layer's children, adding the samples it
/// plays to the project. Returns the channel; the children are added to the project and the rack.
///
/// Complexity: O(c + s) in the channels and samples of the preset.
fn instantiate(ctx: &mut Ctx<'_>, preset: &ChannelPreset, id: ChannelId, target: Option<InsertId>) -> Result<Channel, CommandError> {
    let mut sample_ids = BTreeMap::new();
    for sample in &preset.samples {
        let new = samples::import(ctx, &sample.path).map_err(|e| e.with_hint("the preset's sample is missing; put it back where it was"))?;
        sample_ids.insert(sample.id, new);
    }
    let map = |old: SampleId| sample_ids.get(&old).copied().unwrap_or(old);
    let mut channel = preset.channel.clone();
    channel.id = id;
    channel.target = target;
    channel.midi_in = None;
    channel.kind = match &preset.channel.kind {
        ChannelKind::Sampler(params) => {
            let mut params = (**params).clone();
            params.sample = params.sample.map(map);
            ChannelKind::Sampler(Arc::new(params))
        }
        ChannelKind::AudioClip { sample, stretch, tempo } => ChannelKind::AudioClip { sample: map(*sample), stretch: *stretch, tempo: *tempo },
        ChannelKind::Instrument { plugin } => {
            let mut plugin = plugin.clone();
            let state = preset.states.get(&plugin.instance.to_string()).cloned().unwrap_or_default();
            plugin.instance = ctx.ids().plugin();
            plugin.state = state.into();
            ChannelKind::Instrument { plugin }
        }
        ChannelKind::Layer { .. } => {
            let mut children = imbl::Vector::new();
            for child in &preset.children {
                let child_id = ctx.ids().channel();
                let mut made = instantiate(ctx, child, child_id, target)?;
                made.name = unique_name(made.name.trim(), ctx.project.channels.values().map(|c| c.name.as_str()));
                ctx.project.channels.insert(child_id, Arc::new(made));
                ctx.project.rack_order.push_back(child_id);
                children.push_back(child_id);
            }
            ChannelKind::Layer { children }
        }
        other @ (ChannelKind::MidiOut { .. } | ChannelKind::Automation(_)) => other.clone(),
    };
    Ok(channel)
}

fn save_channel(ctx: &mut Ctx<'_>, p: SaveChannel) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let preset = channel_preset(ctx, id, 0)?;
    let name = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| preset.channel.name.clone());
    let path = save_path(ctx, p.path.as_deref(), &name, PresetFolder::ChannelPresets, CHANNEL_PRESET_EXTENSION)?;
    write_channel_preset(&path, &preset).map_err(|e| io_error(&e))?;
    Ok(json!({ "path": shown(ctx, &path), "channel": id }))
}

fn load_channel(ctx: &mut Ctx<'_>, p: LoadChannel) -> Result<Value, CommandError> {
    let path = preset_file(ctx, &p.path, PresetFolder::ChannelPresets, CHANNEL_PRESET_EXTENSION)?;
    let preset = read_channel_preset(&path).map_err(|e| io_error(&e))?;
    // A channel the preset replaces keeps its place in the project: its insert, MIDI input and rack group.
    let (id, target, midi_in, group, replaced) = match &p.channel {
        Some(r) => {
            let id = ctx.channel(Some(r))?;
            let old = ctx.project.channels.get(&id).ok_or_else(|| CommandError::not_found("channel", &id.to_string()))?;
            (id, old.target, old.midi_in.clone(), old.group.clone(), true)
        }
        None => {
            let target = match &p.target {
                Some(r) => Some(ctx.insert(Some(r))?).filter(|i| *i != ctx.project.mixer.master.id),
                None => {
                    let used: BTreeSet<_> = ctx.project.channels.values().filter_map(|c| c.target).collect();
                    ctx.project.mixer.order.iter().copied().find(|i| !used.contains(i))
                }
            };
            (ctx.ids().channel(), target, None, None, false)
        }
    };
    let mut channel = instantiate(ctx, &preset, id, target)?;
    channel.midi_in = midi_in;
    channel.group = group;
    // The channel takes the preset's name, as the file is called.
    let base = path.file_stem().map_or_else(|| channel.name.clone(), |s| s.to_string_lossy().into_owned());
    let others = ctx.project.channels.values().filter(|c| c.id != id).map(|c| c.name.as_str());
    channel.name = unique_name(base.trim(), others);
    if !replaced {
        channel.color = palette::channel_color(ctx.project.channels.len());
        ctx.project.rack_order.push_back(id);
    }
    let name = channel.name.clone();
    ctx.project.channels.insert(id, Arc::new(channel));
    ctx.editor.channel = Some(id);
    ctx.editor.channels = BTreeSet::from([id]);
    ctx.set_label(format!("Load channel preset {name}"));
    let insert = target.and_then(|t| insert_number(&ctx.project, t)).unwrap_or(0);
    Ok(json!({ "channel": id, "name": name, "insert": insert, "replaced": replaced }))
}

fn save_mixer(ctx: &mut Ctx<'_>, p: SaveMixer) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let insert = ctx.project.mixer.insert(id).ok_or_else(|| CommandError::not_found("insert", &id.to_string()))?;
    let states = insert.slots.iter().filter_map(|s| s.plugin.as_ref()).map(|p| (p.instance.to_string(), p.state.to_vec())).collect();
    let preset = MixerPreset { insert: (**insert).clone(), states };
    let name = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| insert.name.clone());
    let path = save_path(ctx, p.path.as_deref(), &name, PresetFolder::MixerPresets, MIXER_PRESET_EXTENSION)?;
    write_mixer_preset(&path, &preset).map_err(|e| io_error(&e))?;
    Ok(json!({ "path": shown(ctx, &path), "insert": insert_number(&ctx.project, id) }))
}

fn load_mixer(ctx: &mut Ctx<'_>, p: LoadMixer) -> Result<Value, CommandError> {
    let path = preset_file(ctx, &p.path, PresetFolder::MixerPresets, MIXER_PRESET_EXTENSION)?;
    let preset = read_mixer_preset(&path).map_err(|e| io_error(&e))?;
    let id = ctx.insert(p.insert.as_ref())?;
    let mut slots = preset.insert.slots.clone();
    for slot in &mut slots {
        slot.sidechain = None;
        if let Some(plugin) = &mut slot.plugin {
            let state = preset.states.get(&plugin.instance.to_string()).cloned().unwrap_or_default();
            plugin.instance = ctx.ids().plugin();
            plugin.state = state.into();
        }
    }
    let loaded = preset.insert;
    ctx.edit_insert(id, |insert| {
        *insert = bodo_model::MixerInsert {
            id: insert.id,
            name: insert.name.clone(),
            slots,
            to_master: insert.to_master,
            input: insert.input.clone(),
            output: insert.output.clone(),
            armed: insert.armed,
            record_tap: insert.record_tap,
            record_format: insert.record_format,
            muted: insert.muted,
            solo: insert.solo,
            ..loaded
        };
    })?;
    let number = insert_number(&ctx.project, id).unwrap_or(0);
    ctx.set_label(format!("Load mixer preset on insert {number}"));
    Ok(json!({ "insert": number, "path": shown(ctx, &path) }))
}

fn save_score(ctx: &mut Ctx<'_>, p: SaveScore) -> Result<Value, CommandError> {
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let channel = ctx.channel(p.channel.as_ref())?;
    let pat = ctx.project.patterns.get(&pattern).ok_or_else(|| CommandError::not_found("pattern", &pattern.to_string()))?;
    let notes: Vec<_> = pat.notes.get(&channel).map(|seq| seq.notes().iter().cloned().collect()).unwrap_or_default();
    if notes.is_empty() {
        return Err(CommandError::rejected("the channel has no notes in the pattern to save"));
    }
    let channel_name = ctx.project.channels.get(&channel).map(|c| c.name.clone()).unwrap_or_default();
    let name = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} {channel_name}", pat.name));
    let score = ScorePreset { name: name.clone(), length: pattern_length(&ctx.project, pattern), notes };
    let path = save_path(ctx, p.path.as_deref(), &name, PresetFolder::Scores, SCORE_EXTENSION)?;
    write_score(&path, &score).map_err(|e| io_error(&e))?;
    Ok(json!({ "path": shown(ctx, &path), "notes": score.notes.len() }))
}

fn load_score(ctx: &mut Ctx<'_>, p: LoadScore) -> Result<Value, CommandError> {
    let path = preset_file(ctx, &p.path, PresetFolder::Scores, SCORE_EXTENSION)?;
    let score = read_score(&path).map_err(|e| io_error(&e))?;
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let channel = ctx.channel(p.channel.as_ref())?;
    let at = p.at.as_ref().map(|a| a.resolve(&ctx.project)).transpose()?.unwrap_or(Tick::ZERO).max(Tick::ZERO);
    let added = ctx.edit_pattern(pattern, |pat| {
        if pat.binding.is_some_and(|b| b != channel) {
            return Err(CommandError::rejected(format!("{} is bound to another channel", pat.name)));
        }
        if p.replace {
            pat.notes.remove(&channel);
        }
        let mut added = Vec::with_capacity(score.notes.len());
        for note in &score.notes {
            let id = NoteId(pat.next_note_id);
            pat.next_note_id += 1;
            let mut note = note.clone();
            note.id = id;
            note.start = Tick(note.start.0 + at.0);
            note.quantized_start = note.quantized_start.map(|q| Tick(q.0 + at.0));
            pat.notes.entry(channel).or_default().insert(note);
            added.push(id);
        }
        Ok((true, added))
    })?;
    ctx.editor.channel = Some(channel);
    ctx.editor.notes = added.iter().copied().collect();
    ctx.set_label(format!("Load score {}", score.name));
    Ok(json!({ "pattern": pattern, "channel": channel, "notes": added.iter().map(|n| n.0).collect::<Vec<_>>() }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "preset.save_channel",
        "Save channel preset",
        "Saves a channel, with its samples and plugin states, as a .bodochannel file in the library.",
        Effects::DISK,
        save_channel,
    )
    .example(json!({ "channel": "Kick" }));
    r.edit(
        "preset.load_channel",
        "Load channel preset",
        "Loads a .bodochannel file as a new channel, or into a channel to replace its sound.",
        Effects::UNDOABLE,
        load_channel,
    )
    .example(json!({ "path": "Kick" }));
    r.edit(
        "preset.save_mixer",
        "Save mixer preset",
        "Saves a mixer insert's settings and FX slots as a .bodomixer file in the library.",
        Effects::DISK,
        save_mixer,
    )
    .example(json!({ "insert": 1 }));
    r.edit(
        "preset.load_mixer",
        "Load mixer preset",
        "Gives a mixer insert the settings and FX slots of a .bodomixer file.",
        Effects::UNDOABLE,
        load_mixer,
    )
    .example(json!({ "path": "Insert 1", "insert": 2 }));
    r.edit(
        "preset.save_score",
        "Save score",
        "Saves one channel's notes in a pattern as a .bodoscore file in the library.",
        Effects::DISK,
        save_score,
    )
    .example(json!({ "channel": "Kick" }));
    r.edit("preset.load_score", "Load score", "Adds the notes of a .bodoscore file to a channel in a pattern.", Effects::UNDOABLE, load_score)
        .example(json!({ "path": "Pattern 1 Kick", "channel": "Clap", "at": "1:3:0" }));
}
