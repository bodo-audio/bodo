//! Preset files (project.md section 14.1, ADR 0029): a channel, a mixer insert or a score saved on its own, so it can
//! be loaded into any project from the Browser.
//!
//! Each preset is a small JSON file with a `format` and a `version`, like `project.json`:
//!
//! - **`.bodochannel`**, a [`ChannelPreset`]: the channel as the project stores it, the samples it plays by path and
//!   hash, its plugin states, and for a layer the presets of its children.
//! - **`.bodomixer`**, a [`MixerPreset`]: a mixer insert's settings and FX slots, with their plugin states.
//! - **`.bodoscore`**, a [`ScorePreset`]: the notes of one channel in a pattern.
//! - **`.bodopreset`**, a [`PluginPreset`]: one plugin's or native effect's settings, for an FX slot.
//!
//! The ids inside a preset are the ones it had where it was saved; loading gives everything new ids. Plugin states,
//! which `project.json` keeps outside the JSON, are base64 text here so a preset stays a single file. Writes are
//! atomic.

use std::collections::BTreeMap;
use std::path::Path;

use bodo_kit_core::Tick;
use bodo_model::channel::PluginFormat;
use bodo_model::mixer::ParamValue;
use bodo_model::{Channel, MixerInsert, Note, SampleId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::error::IoError;

/// The extension of channel presets.
pub const CHANNEL_PRESET_EXTENSION: &str = "bodochannel";
/// The extension of mixer presets.
pub const MIXER_PRESET_EXTENSION: &str = "bodomixer";
/// The extension of scores.
pub const SCORE_EXTENSION: &str = "bodoscore";
/// The extension of plugin presets.
pub const PLUGIN_PRESET_EXTENSION: &str = "bodopreset";
/// The newest preset version this Bodo writes and reads.
pub const PRESET_VERSION: u32 = 1;

/// The `format` of each kind of preset.
const CHANNEL_FORMAT: &str = "bodo.channel";
const MIXER_FORMAT: &str = "bodo.mixer";
const SCORE_FORMAT: &str = "bodo.score";
const PLUGIN_FORMAT: &str = "bodo.plugin";

/// A sample a preset plays, by the id it had where the preset was saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSample {
    /// The id the preset's channel refers to it by.
    pub id: SampleId,
    /// Where it is: a `factory:` path, or an absolute path.
    pub path: String,
    /// Its BLAKE3 hash, to find it again if it moves.
    #[serde(default)]
    pub hash: Option<String>,
}

/// A channel saved on its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPreset {
    /// The channel, as the project stored it. Its id, target insert and MIDI input are not loaded.
    pub channel: Channel,
    /// The samples it plays.
    #[serde(default)]
    pub samples: Vec<PresetSample>,
    /// The saved states of its plugins, by instance id.
    #[serde(default, with = "states")]
    pub states: BTreeMap<String, Vec<u8>>,
    /// For a layer, the presets of the channels it plays, in order.
    #[serde(default)]
    pub children: Vec<ChannelPreset>,
}

/// A mixer insert saved on its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerPreset {
    /// The insert, as the project stored it. Its id, name, routing, hardware ports and recording settings are not
    /// loaded.
    pub insert: MixerInsert,
    /// The saved states of its plugins, by instance id.
    #[serde(default, with = "states")]
    pub states: BTreeMap<String, Vec<u8>>,
}

/// The notes of one channel in a pattern, saved on their own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScorePreset {
    /// The name it was saved with, such as the pattern's.
    pub name: String,
    /// How long the score is: the pattern's length when it was saved.
    pub length: Tick,
    /// The notes, from the score's start.
    pub notes: Vec<Note>,
}

/// One plugin's or native effect's settings, saved on their own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreset {
    /// The name it was saved with.
    pub name: String,
    /// The plugin's format.
    pub plugin_format: PluginFormat,
    /// The format's unique id for the plugin, such as `bodo.delay` for a native effect.
    pub uid: String,
    /// A native effect's parameter values, by parameter id.
    #[serde(default)]
    pub params: Vec<ParamValue>,
    /// A plugin's saved state, under the key `plugin`.
    #[serde(default, with = "states")]
    pub states: BTreeMap<String, Vec<u8>>,
}

/// The envelope every preset file has: its kind and version, then the preset.
#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    format: String,
    version: u32,
    #[serde(flatten)]
    preset: T,
}

/// Writes `preset` to `path` atomically, making its folder if needed.
///
/// Does I/O. Complexity: O(size of the preset).
pub fn write_channel_preset(path: &Path, preset: &ChannelPreset) -> Result<(), IoError> {
    write(path, CHANNEL_FORMAT, preset)
}

/// Reads the channel preset at `path`.
///
/// Does I/O. Complexity: O(size of the file).
pub fn read_channel_preset(path: &Path) -> Result<ChannelPreset, IoError> {
    read(path, CHANNEL_FORMAT, "a channel preset")
}

/// Writes `preset` to `path` atomically, making its folder if needed.
///
/// Does I/O. Complexity: O(size of the preset).
pub fn write_mixer_preset(path: &Path, preset: &MixerPreset) -> Result<(), IoError> {
    write(path, MIXER_FORMAT, preset)
}

/// Reads the mixer preset at `path`.
///
/// Does I/O. Complexity: O(size of the file).
pub fn read_mixer_preset(path: &Path) -> Result<MixerPreset, IoError> {
    read(path, MIXER_FORMAT, "a mixer preset")
}

/// Writes `score` to `path` atomically, making its folder if needed.
///
/// Does I/O. Complexity: O(notes).
pub fn write_score(path: &Path, score: &ScorePreset) -> Result<(), IoError> {
    write(path, SCORE_FORMAT, score)
}

/// Reads the score at `path`.
///
/// Does I/O. Complexity: O(size of the file).
pub fn read_score(path: &Path) -> Result<ScorePreset, IoError> {
    read(path, SCORE_FORMAT, "a score")
}

/// Writes `preset` to `path` atomically, making its folder if needed.
///
/// Does I/O. Complexity: O(size of the preset).
pub fn write_plugin_preset(path: &Path, preset: &PluginPreset) -> Result<(), IoError> {
    write(path, PLUGIN_FORMAT, preset)
}

/// Reads the plugin preset at `path`.
///
/// Does I/O. Complexity: O(size of the file).
pub fn read_plugin_preset(path: &Path) -> Result<PluginPreset, IoError> {
    read(path, PLUGIN_FORMAT, "a plugin preset")
}

fn write<T: Serialize>(path: &Path, format: &str, preset: &T) -> Result<(), IoError> {
    let envelope = Envelope { format: format.to_owned(), version: PRESET_VERSION, preset };
    let mut text = serde_json::to_string_pretty(&envelope).map_err(|e| IoError::Encode { message: e.to_string() })?;
    text.push('\n');
    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).map_err(|e| IoError::io("create", folder, e))?;
    }
    atomic::write(path, text.as_bytes())
}

fn read<T: DeserializeOwned>(path: &Path, format: &str, what: &str) -> Result<T, IoError> {
    let text = std::fs::read_to_string(path).map_err(|e| IoError::io("read", path, e))?;
    let preset_error = |message: String| IoError::Preset { path: path.to_path_buf(), message };
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| preset_error(format!("is not valid JSON: {e}")))?;
    let found = value.get("format").and_then(serde_json::Value::as_str).unwrap_or("missing");
    if found != format {
        return Err(preset_error(format!("is not {what} (its format is {found}, not {format})")));
    }
    let version = value.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if version == 0 || version > u64::from(PRESET_VERSION) {
        return Err(preset_error(format!("has preset version {version}; this Bodo reads versions 1 to {PRESET_VERSION}")));
    }
    let envelope: Envelope<T> = serde_json::from_value(value).map_err(|e| preset_error(format!("has bad preset data: {e}")))?;
    Ok(envelope.preset)
}

/// Plugin states as base64 text, by instance id.
mod states {
    use std::collections::BTreeMap;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(states: &BTreeMap<String, Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(states.iter().map(|(instance, state)| (instance, STANDARD.encode(state))))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, Vec<u8>>, D::Error> {
        let text = BTreeMap::<String, String>::deserialize(deserializer)?;
        text.into_iter()
            .map(|(instance, state)| STANDARD.decode(state.as_bytes()).map(|bytes| (instance, bytes)).map_err(serde::de::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::sync::Arc;

    use bodo_model::channel::{ChannelKind, SamplerParams};
    use bodo_model::{ChannelId, Color, InsertId, NoteId};

    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bodo-preset-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn channel() -> Channel {
        let params = SamplerParams { sample: Some(SampleId::from_u128(9)), root_key: 48, ..SamplerParams::default() };
        Channel {
            id: ChannelId::from_u128(1),
            name: String::from("Kick"),
            color: Color::GRAY,
            kind: ChannelKind::Sampler(Arc::new(params)),
            target: None,
            volume: 0.5,
            pan: -0.25,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
            group: None,
        }
    }

    #[test]
    fn every_kind_of_preset_round_trips() {
        let dir = temp_dir("round-trip");
        let channel = ChannelPreset {
            channel: channel(),
            samples: vec![PresetSample { id: SampleId::from_u128(9), path: String::from("factory:Packs/Bodo Drums/Kick.wav"), hash: None }],
            states: BTreeMap::from([(String::from("plugin"), vec![0, 1, 2, 255])]),
            children: Vec::new(),
        };
        let path = dir.join("Kick.bodochannel");
        write_channel_preset(&path, &channel).unwrap();
        assert_eq!(read_channel_preset(&path).unwrap(), channel);
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"format\": \"bodo.channel\""));

        let mixer = MixerPreset { insert: MixerInsert::new(InsertId::from_u128(3), "Drums"), states: BTreeMap::new() };
        let path = dir.join("Drums.bodomixer");
        write_mixer_preset(&path, &mixer).unwrap();
        assert_eq!(read_mixer_preset(&path).unwrap(), mixer);

        let score = ScorePreset { name: String::from("Riff"), length: Tick(3840), notes: vec![Note::new(NoteId(4), 60, Tick(0), Tick(240))] };
        let path = dir.join("nested").join("Riff.bodoscore");
        write_score(&path, &score).unwrap();
        assert_eq!(read_score(&path).unwrap(), score, "the folder is made");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_wrong_kind_or_a_newer_version_is_refused() {
        let dir = temp_dir("refused");
        let path = dir.join("Riff.bodoscore");
        write_score(&path, &ScorePreset { name: String::from("Riff"), length: Tick(3840), notes: Vec::new() }).unwrap();
        let error = read_channel_preset(&path).unwrap_err().to_string();
        assert!(error.contains("is not a channel preset (its format is bodo.score"), "{error}");
        std::fs::write(&path, r#"{ "format": "bodo.score", "version": 99, "name": "x", "length": 0, "notes": [] }"#).unwrap();
        let error = read_score(&path).unwrap_err().to_string();
        assert!(error.contains("preset version 99"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_plugin_preset_round_trips() {
        let dir = temp_dir("plugin");
        let path = dir.join("Delay").join("Slapback.bodopreset");
        let preset = PluginPreset {
            name: String::from("Slapback"),
            plugin_format: PluginFormat::Native,
            uid: String::from("bodo.delay"),
            params: vec![ParamValue { id: 1, value: 0.25 }],
            states: BTreeMap::from([(String::from("plugin"), vec![1, 2, 3])]),
        };
        write_plugin_preset(&path, &preset).unwrap();
        assert_eq!(read_plugin_preset(&path).unwrap(), preset);
        assert!(read_score(&path).is_err(), "a plugin preset isn't a score");
        let _ = std::fs::remove_dir_all(dir);
    }
}
