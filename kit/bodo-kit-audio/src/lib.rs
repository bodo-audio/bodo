//! Audio files for Bodo Kit.
//!
//! - [`decode`]: reading WAV, AIFF, FLAC, MP3, AAC, ALAC and Ogg Vorbis into memory, or packet by packet.
//! - [`mod@resample`]: converting to the project's sample rate on load.
//! - [`wav`]: writing WAV files.
//! - [`peaks`]: the peaks pyramid for drawing waveforms.
//! - [`markers`]: cue points, regions and loops stored in WAV files.
//! - [`edit`]: edits rendered into decoded audio, such as fades, normalizing and loop crossfades.
//! - [`stream`]: streaming long files from disk to the audio thread.
//!
//! Everything here except [`stream::StreamReader`] runs off the audio thread.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use bodo_kit_core::SampleRate;

pub mod decode;
pub mod edit;
pub mod markers;
pub mod peaks;
pub mod resample;
pub mod stream;
pub mod wav;

pub use decode::{AudioInfo, Decoder, decode_file};
pub use markers::{SampleMarkers, read_markers};
pub use peaks::ChannelPeaks;
pub use resample::resample;
pub use stream::{DiskThread, StreamConfig, StreamReader};
pub use wav::{WavFormat, WavWriter};

/// Decoded audio in memory: one `Vec` per channel, all the same length.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioData {
    /// The sample rate.
    pub rate: SampleRate,
    /// The samples, one `Vec` per channel.
    pub channels: Vec<Vec<f32>>,
}

impl AudioData {
    /// The length in frames.
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// The number of channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

/// Why an audio file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The file could not be opened, read or written.
    #[error("{}: {source}", .path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The format or codec isn't supported, or the file is not audio.
    #[error("unsupported audio: {0}")]
    Unsupported(String),
    /// The file has no audio track.
    #[error("the file has no audio track")]
    NoAudioTrack,
    /// The data is damaged beyond recovery.
    #[error("could not decode the audio: {0}")]
    Decode(String),
    /// Writing a WAV file failed.
    #[error("could not write the WAV file: {0}")]
    Wav(String),
    /// Resampling failed.
    #[error("could not resample: {0}")]
    Resample(String),
}
