//! The project document (project.md section 8.3).
//!
//! A [`Project`] is an immutable snapshot built from persistent collections: maps and vectors that share structure
//! between versions. Changing one note copies only the path to it, so a snapshot costs a handful of pointers and undo
//! is a stack of snapshots (ADR 0006).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_core::{SampleRate, TempoMap, Tick};
use bodo_kit_dsp::pan::PanLaw;
use imbl::{OrdMap, Vector};

use crate::channel::Channel;
use crate::color::Color;
use crate::id::{ChannelId, InsertId, PatternId, ProjectId, SampleId, TrackId};
use crate::meter::MeterMap;
use crate::mixer::Mixer;
use crate::pattern::Pattern;
use crate::playlist::PlaylistTrack;

/// A whole project.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// The project's id.
    pub id: ProjectId,
    /// Title, author, credits.
    pub meta: Arc<Meta>,
    /// Sample rate, PPQ and other settings.
    pub settings: Arc<Settings>,
    /// Tempo over time.
    pub tempo: Arc<TempoMap>,
    /// Time signatures over time.
    #[serde(default)]
    pub meter: Arc<MeterMap>,
    /// The channels of the channel rack.
    #[serde(default)]
    pub channels: OrdMap<ChannelId, Arc<Channel>>,
    /// Channel rack order.
    #[serde(default)]
    pub rack_order: Vector<ChannelId>,
    /// The patterns.
    #[serde(default)]
    pub patterns: OrdMap<PatternId, Arc<Pattern>>,
    /// The playlist tracks.
    #[serde(default)]
    pub tracks: OrdMap<TrackId, Arc<PlaylistTrack>>,
    /// Playlist track order.
    #[serde(default)]
    pub track_order: Vector<TrackId>,
    /// The mixer.
    pub mixer: Arc<Mixer>,
    /// The sample files the project uses.
    #[serde(default)]
    pub samples: OrdMap<SampleId, Arc<SampleRef>>,
    /// Markers, sorted by time.
    #[serde(default)]
    pub markers: Vector<Marker>,
}

/// Information about the project.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    /// The title.
    pub title: String,
    /// The author.
    pub author: String,
    /// Credits for licensed content the project uses, such as CC-BY samples.
    pub credits: Vec<String>,
    /// When the project was created, as an RFC 3339 timestamp.
    pub created: Option<String>,
}

/// Project-wide settings.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// The sample rate the project renders at.
    pub sample_rate: SampleRate,
    /// Ticks per quarter note. Stored so a later resolution change can migrate files (ADR 0006).
    pub ppq: i64,
    /// The pan law for mono sources (project.md section 9.5).
    pub pan_law: PanLaw,
}

impl Default for Settings {
    fn default() -> Self {
        Self { sample_rate: SampleRate::HZ_48000, ppq: bodo_kit_core::PPQ, pan_law: PanLaw::default() }
    }
}

/// A sample file the project uses.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleRef {
    /// The sample's id.
    pub id: SampleId,
    /// Where it is, relative to the project file when possible.
    pub path: String,
    /// The BLAKE3 hash of its contents, used to find it again if it moves.
    #[serde(default)]
    pub hash: Option<String>,
    /// Where it came from in the marketplace, if it did.
    #[serde(default)]
    pub market: Option<MarketRef>,
}

/// The prefix of sample paths inside Bodo's factory content, such as `factory:Packs/Bodo Drums/Kick.wav`. Such paths
/// stay valid when Bodo is installed somewhere else.
pub const FACTORY_PREFIX: &str = "factory:";

/// The folders that stored sample paths start from (project.md section 18.1).
///
/// A [`SampleRef::path`] is relative to the project's folder, absolute, or starts with [`FACTORY_PREFIX`] and is
/// relative to Bodo's factory content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleRoots {
    /// The project's folder. Relative paths start here.
    pub project: PathBuf,
    /// Bodo's factory content. Paths with [`FACTORY_PREFIX`] start here.
    pub factory: Option<PathBuf>,
}

impl SampleRoots {
    /// Roots for a project in `project`, without factory content.
    pub fn new(project: impl Into<PathBuf>) -> Self {
        Self { project: project.into(), factory: None }
    }

    /// The same roots with factory content in `factory`.
    pub fn with_factory(self, factory: Option<PathBuf>) -> Self {
        Self { factory, ..self }
    }

    /// Where the stored sample path `path` points. A factory path without a factory folder resolves to a path that
    /// doesn't exist, so the sample is reported missing.
    pub fn resolve(&self, path: &str) -> PathBuf {
        match path.strip_prefix(FACTORY_PREFIX) {
            Some(rest) => self.factory.as_deref().unwrap_or_else(|| Path::new("<no factory content>")).join(rest),
            None => self.project.join(path),
        }
    }
}

/// A marketplace item a sample came from (project.md section 15.2).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketRef {
    /// The package id.
    pub package: String,
    /// The package version.
    pub version: String,
    /// The item's path inside the package.
    pub item: String,
}

/// A named position on the timeline.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    /// Where it is.
    pub at: Tick,
    /// Its name.
    pub name: String,
    /// Its color.
    #[serde(default)]
    pub color: Option<Color>,
}

impl Project {
    /// A new, empty project at `tempo` BPM in 4/4: the master and 16 inserts, nothing else.
    ///
    /// Fails only when `tempo` is outside the range a tempo map accepts.
    pub fn new(tempo: f64) -> Result<Self, bodo_kit_core::tempo::TempoMapError> {
        Ok(Self {
            id: ProjectId::new(),
            meta: Arc::new(Meta::default()),
            settings: Arc::new(Settings::default()),
            tempo: Arc::new(TempoMap::constant(tempo)?),
            meter: Arc::new(MeterMap::common_time()),
            channels: OrdMap::new(),
            rack_order: Vector::new(),
            patterns: OrdMap::new(),
            tracks: OrdMap::new(),
            track_order: Vector::new(),
            mixer: Arc::new(Mixer::with_inserts(InsertId::new(), (0..16).map(|_| InsertId::new()))),
            samples: OrdMap::new(),
            markers: Vector::new(),
        })
    }
}
