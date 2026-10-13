//! Playlist tracks and clips (project.md sections 8.3 and 10.3).

use bodo_kit_core::{Frames, Tick};
use imbl::Vector;

use crate::color::Color;
use crate::id::{ChannelId, ClipId, PatternId, TrackId};

/// A playlist track. Any track can hold any kind of clip.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistTrack {
    /// The track's id.
    pub id: TrackId,
    /// Its name.
    pub name: String,
    /// Its color.
    pub color: Color,
    /// Its height in the playlist, in points.
    #[serde(default = "default_height")]
    pub height: f32,
    /// Whether the track is muted.
    #[serde(default)]
    pub muted: bool,
    /// Whether the track is locked against edits.
    #[serde(default)]
    pub locked: bool,
    /// Whether the track is soloed: while any track is, only soloed tracks play.
    #[serde(default, skip_serializing_if = "is_false")]
    pub solo: bool,
    /// The track group it belongs to, by name. A group's tracks sit next to each other in the playlist and fold under
    /// its header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The clips, sorted by start.
    #[serde(default)]
    pub clips: Vector<Clip>,
}

fn default_height() -> f32 {
    48.0
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl PlaylistTrack {
    /// An empty track.
    pub fn new(id: TrackId, name: impl Into<String>, color: Color) -> Self {
        Self { id, name: name.into(), color, height: default_height(), muted: false, locked: false, solo: false, group: None, clips: Vector::new() }
    }

    /// Whether the track plays: it isn't muted, and either it is soloed or no track is (see [`any_solo`]).
    pub fn plays(&self, any_solo: bool) -> bool {
        !self.muted && (self.solo || !any_solo)
    }

    /// Adds a clip in its sorted place, after any clips with the same start.
    ///
    /// Complexity: O(log n) in the number of clips.
    pub fn insert_clip(&mut self, clip: Clip) {
        let index = self
            .clips
            .binary_search_by(|c| if c.start <= clip.start { core::cmp::Ordering::Less } else { core::cmp::Ordering::Greater })
            .unwrap_or_else(|i| i);
        self.clips.insert(index, clip);
    }
}

/// Whether any of `tracks` is soloed, which silences every track that isn't.
///
/// Complexity: O(t) in tracks.
pub fn any_solo<'a>(tracks: impl IntoIterator<Item = &'a std::sync::Arc<PlaylistTrack>>) -> bool {
    tracks.into_iter().any(|t| t.solo)
}

/// A clip on a playlist track.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    /// The clip's id.
    pub id: ClipId,
    /// Where it starts.
    pub start: Tick,
    /// How long it lasts; always positive.
    pub len: Tick,
    /// How far into its source the clip starts.
    #[serde(default)]
    pub offset: ClipOffset,
    /// Whether the clip is muted, separately from its track.
    #[serde(default)]
    pub muted: bool,
    /// A color that overrides the source's.
    #[serde(default)]
    pub color: Option<Color>,
    /// What the clip plays.
    pub source: ClipSource,
    /// Fades, for audio clips.
    #[serde(default)]
    pub fades: Option<Fades>,
}

impl Clip {
    /// Where it ends.
    pub fn end(&self) -> Tick {
        self.start + self.len
    }
}

/// How far into its source a clip starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum ClipOffset {
    /// In ticks, following tempo.
    Ticks(Tick),
    /// In frames, for audio locked to time.
    Frames(Frames),
}

impl Default for ClipOffset {
    fn default() -> Self {
        Self::Ticks(Tick::ZERO)
    }
}

/// What a clip plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "id")]
pub enum ClipSource {
    /// A pattern.
    Pattern(PatternId),
    /// The sample of an audio clip channel.
    Audio(ChannelId),
    /// The lane of an automation clip channel.
    Automation(ChannelId),
}

/// Fade in and fade out of an audio clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fades {
    /// Fade-in length.
    pub fade_in: Tick,
    /// Fade-out length.
    pub fade_out: Tick,
}
