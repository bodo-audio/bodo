//! The Bodo render engine (project.md section 6).
//!
//! - [`mod@engine`]: builds the two sides of an engine for a project: [`Engine`] on the control thread and
//!   [`AudioEngine`] on the audio thread, which any `bodo-kit-device` stream can run.
//! - [`render_offline`]: the same engine, faster than real time and deterministic, for export.
//! - [`sequence`]: the sequencer and transport, with swing and the Song mode loop.
//! - [`automation`]: automation clips and pattern lanes, played once per block.
//! - [`nodes`]: the graph's processors: the built-in sampler and the mixer strip.
//! - [`input`]: notes played live, from MIDI inputs and the computer keyboard; [`midi_record`]: recording them.
//!
//! M1 covers sampler channels, inserts and the master, audio routes, the sequencer with pattern and song modes, and
//! offline rendering. Plugins, FX slots, sidechains, taps, solo and audio clips arrive in later milestones.

#![forbid(unsafe_code)]

pub mod audio_clips;
pub mod automation;
mod compile;
pub mod context;
pub mod disk_sample;
pub mod engine;
pub mod fx;
pub mod input;
pub mod keys;
pub mod layers;
pub mod live;
pub mod meter;
mod metronome;
pub mod midi_out;
pub mod midi_record;
pub mod nodes;
pub mod offline;
pub mod preview;
pub mod record;
pub mod sample;
pub mod sampler_source;
pub mod segments;
pub mod sequence;
pub mod stretch;

pub use compile::{MixLatency, audible_set, mix_latency};
pub use engine::{AudioEngine, Engine, EngineConfig, EngineStatus, InsertLatency, engine};
pub use input::{LiveInput, MidiRouter};
pub use meter::{MeterReading, SCOPE_FRAMES};
pub use midi_record::{CapturedTake, RecordedNote, TakeTarget};
pub use offline::{OfflineOptions, Stem, render_offline, render_stems};
pub use preview::{Preview, PreviewPlayer, preview};
pub use record::{Compensation, RecordError, Take};
pub use sequence::PlayMode;

/// Why the engine could not start or update.
#[derive(Debug, thiserror::Error)]
pub enum MixError {
    /// The graph could not be compiled.
    #[error(transparent)]
    Compile(#[from] bodo_kit_graph::CompileError),
    /// The RT workers could not start.
    #[error(transparent)]
    Pool(#[from] bodo_kit_core::pool::RtPoolError),
    /// The graph reported internal faults while rendering.
    #[error("the render hit {0} internal fault(s)")]
    Faults(u64),
    /// Recording could not start or finish.
    #[error(transparent)]
    Record(#[from] record::RecordError),
}
