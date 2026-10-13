//! Bodo's project files (project.md section 18.1, ADR 0007). The format is specified in `docs/formats/bodo.md`.
//!
//! - [`save`] and [`load`] write and read a project in either container: a `.bodo` zip file, or the same layout
//!   unpacked in a `.bodoproj` folder that can live in git. Saves are atomic and keep rotating backups.
//! - [`to_json`] and [`from_json`] convert a project to and from `project.json`, the readable heart of both
//!   containers, with forward migrations for older format versions.
//! - [`convert`] packs a folder into a `.bodo` file or unpacks one, keeping every entry.
//! - [`mod@paths`] holds the rules for sample paths, and [`hash_file`] the BLAKE3 hash that finds moved samples.
//! - [`midi`] imports and exports Standard MIDI Files.
//!
//! Plugin states never go into `project.json`: each one is a binary blob in `plugins/<instance>.state`.

#![forbid(unsafe_code)]

mod atomic;
mod backup;
mod container;
mod error;
mod json;
pub mod midi;
pub mod paths;

pub use backup::backups;
pub use container::{DEFAULT_BACKUPS, SaveOptions, convert, load, save};
pub use error::IoError;
pub use json::{FORMAT, FORMAT_VERSION, from_json, to_json};
pub use midi::{ImportedMidi, ImportedNote, ImportedPart, export_arrangement, export_pattern, import_smf, read_midi_file, write_midi_file};
pub use paths::{FACTORY_PREFIX, hash_file, project_folder, resolve_sample_path, sample_path};
