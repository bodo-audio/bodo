//! The Bodo project document (project.md section 8).
//!
//! - [`project`]: [`Project`], the root, with its settings, tempo and meter maps.
//! - [`channel`], [`pattern`], [`playlist`], [`automation`], [`mixer`]: the entities.
//! - [`id`]: typed ULID ids, printed as `ch_01J...` and the like.
//! - [`validate`]: [`Project::validate`], every invariant of section 8.4.
//!
//! Everything is an immutable snapshot built from `imbl` persistent collections with `Arc` around each entity, so an
//! edit copies only the path to what changed (ADR 0006). The model knows nothing about audio or the UI.

#![forbid(unsafe_code)]

pub mod automation;
pub mod channel;
pub mod color;
pub mod id;
pub mod meter;
pub mod mixer;
pub mod pattern;
pub mod playlist;
pub mod project;
pub mod validate;

pub use channel::{Channel, ChannelKind, SamplerParams};
pub use color::Color;
pub use id::{ChannelId, ClipId, InsertId, NoteId, PatternId, PluginInstanceId, ProjectId, SampleId, TrackId};
pub use meter::MeterMap;
pub use mixer::{FX_SLOTS, Mixer, MixerInsert, Route, RouteKey, RouteSource};
pub use pattern::{Note, NoteSeq, Pattern};
pub use playlist::{Clip, ClipSource, PlaylistTrack};
pub use project::{FACTORY_PREFIX, Project, SampleRef, SampleRoots, Settings};
pub use validate::{ValidationError, Violation};
