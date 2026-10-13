//! What the session knows besides the project: the editor's selection, the transport, and the events the frontend
//! acts on.
//!
//! None of this is part of the document. Selection never enters the undo history (project.md section 10.3), and the
//! transport belongs to whoever plays the project.

use std::collections::BTreeSet;
use std::path::PathBuf;

use bodo_kit_core::Tick;
use bodo_model::{ChannelId, ClipId, InsertId, NoteId, PatternId, Project, TrackId};

/// The editor's current pattern, channel and selection. Commands with empty params act on these.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorState {
    /// The current pattern: what the channel rack and piano roll show, and what Pattern mode plays.
    pub pattern: Option<PatternId>,
    /// The channel the piano roll edits.
    pub channel: Option<ChannelId>,
    /// Selected channels in the channel rack.
    pub channels: BTreeSet<ChannelId>,
    /// Selected clips on the playlist.
    pub clips: BTreeSet<ClipId>,
    /// Selected notes of the current pattern.
    pub notes: BTreeSet<NoteId>,
    /// The selected mixer insert.
    pub insert: Option<InsertId>,
    /// The selected playlist track.
    pub track: Option<TrackId>,
}

impl EditorState {
    /// Drops everything that no longer exists in `project`, and picks a current pattern if the old one is gone.
    ///
    /// Complexity: O(s log n) in selected items, plus O(n) in notes of the current pattern.
    pub fn sanitize(&mut self, project: &Project) {
        if self.pattern.is_none_or(|p| !project.patterns.contains_key(&p)) {
            self.pattern = project.patterns.keys().next().copied();
            self.notes.clear();
        }
        if self.channel.is_some_and(|c| !project.channels.contains_key(&c)) {
            self.channel = None;
        }
        self.channels.retain(|c| project.channels.contains_key(c));
        if !self.clips.is_empty() {
            let live: BTreeSet<ClipId> = project.tracks.values().flat_map(|t| t.clips.iter().map(|c| c.id)).collect();
            self.clips.retain(|c| live.contains(c));
        }
        if !self.notes.is_empty() {
            let live: BTreeSet<NoteId> = self
                .pattern
                .and_then(|p| project.patterns.get(&p))
                .map(|p| p.notes.values().flat_map(|seq| seq.notes().iter().map(|n| n.id)).collect())
                .unwrap_or_default();
            self.notes.retain(|n| live.contains(n));
        }
        if self.insert.is_some_and(|i| project.mixer.insert(i).is_none()) {
            self.insert = None;
        }
        if self.track.is_some_and(|t| !project.tracks.contains_key(&t)) {
            self.track = None;
        }
    }
}

/// What the transport plays (project.md section 6.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PlayMode {
    /// Loops the current pattern.
    #[default]
    Pattern,
    /// Plays the playlist.
    Song,
}

/// The transport as the session sees it. The frontend that plays audio keeps it in step with the audio engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportState {
    /// Whether it is playing.
    pub playing: bool,
    /// What it plays.
    pub mode: PlayMode,
    /// The playhead.
    pub position: Tick,
}

/// Something the frontend has to act on after a command. Read them with `Session::take_events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Start playing from the transport's position.
    Play,
    /// Stop playing and release every note.
    Stop,
    /// Move the playhead.
    Seek(Tick),
    /// The project was replaced by a new or opened one: rebuild everything from it.
    ProjectReplaced,
    /// The project was saved to this file. Relative sample paths now start at its folder.
    Saved(PathBuf),
}
