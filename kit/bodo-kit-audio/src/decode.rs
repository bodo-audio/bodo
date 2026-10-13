//! Decoding audio files with Symphonia: WAV, AIFF, FLAC, MP3, AAC and ALAC in MP4/M4A, and Ogg Vorbis.

use std::fs::File;
use std::path::Path;

use bodo_kit_core::SampleRate;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

use crate::{AudioData, AudioError};

/// What a decoder knows about its file before decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    /// The sample rate.
    pub rate: SampleRate,
    /// The number of channels.
    pub channels: usize,
    /// The length in frames, when the container states it.
    pub frames: Option<u64>,
}

/// Decodes a file packet by packet, and can seek.
pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    info: AudioInfo,
    /// Frames still to drop from the next decoded packets, after an accurate seek landed early.
    skip: u64,
    scratch: Vec<Vec<f32>>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder").field("info", &self.info).finish_non_exhaustive()
    }
}

impl Decoder {
    /// Opens `path` and prepares to decode its first audio track.
    ///
    /// Does I/O.
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        let file = File::open(path).map_err(|source| AudioError::Io { path: path.to_path_buf(), source })?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
            .map_err(|err| AudioError::Unsupported(err.to_string()))?;
        let track = format.default_track(TrackType::Audio).ok_or(AudioError::NoAudioTrack)?;
        let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or(AudioError::NoAudioTrack)?;
        let rate = params.sample_rate.and_then(SampleRate::new).ok_or_else(|| AudioError::Unsupported("unknown sample rate".into()))?;
        let channels =
            params.channels.as_ref().map(|c| c.count()).filter(|&c| c > 0).ok_or_else(|| AudioError::Unsupported("unknown channel layout".into()))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|err| AudioError::Unsupported(err.to_string()))?;
        let info = AudioInfo { rate, channels, frames: track.num_frames };
        Ok(Self { track_id: track.id, format, decoder, info, skip: 0, scratch: Vec::new() })
    }

    /// The file's rate, channels and length.
    pub fn info(&self) -> AudioInfo {
        self.info
    }

    /// Decodes the next packet and appends its frames to `out`, one `Vec` per channel. Returns the number of frames
    /// appended, or `None` at the end of the file. Damaged packets are skipped.
    ///
    /// Does I/O. Complexity: O(n) in the packet's frames.
    pub fn decode_next(&mut self, out: &mut [Vec<f32>]) -> Result<Option<usize>, AudioError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(SymphoniaError::IoError(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(err) => return Err(AudioError::Decode(err.to_string())),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::DecodeError(err)) => {
                    tracing::debug!(%err, "skipping a damaged packet");
                    continue;
                }
                Err(err) => return Err(AudioError::Decode(err.to_string())),
            };
            decoded.copy_to_vecs_planar::<f32>(&mut self.scratch);
            let frames = self.scratch.first().map_or(0, Vec::len);
            let skip = (self.skip.min(frames as u64)) as usize;
            self.skip -= skip as u64;
            for (out, decoded) in out.iter_mut().zip(&self.scratch) {
                out.extend_from_slice(&decoded[skip..]);
            }
            if frames > skip {
                return Ok(Some(frames - skip));
            }
        }
    }

    /// Moves to `frame`, so the next decoded sample is that frame. Returns the frame reached, which is `frame` unless
    /// the file is shorter.
    ///
    /// Does I/O.
    pub fn seek(&mut self, frame: u64) -> Result<u64, AudioError> {
        let ts = Timestamp::new(i64::try_from(frame).unwrap_or(i64::MAX));
        let seeked = self
            .format
            .seek(SeekMode::Accurate, SeekTo::Timestamp { ts, track_id: self.track_id })
            .map_err(|err| AudioError::Decode(err.to_string()))?;
        self.decoder.reset();
        let required = u64::try_from(seeked.required_ts.get()).unwrap_or(0);
        let actual = u64::try_from(seeked.actual_ts.get()).unwrap_or(0);
        self.skip = required.saturating_sub(actual);
        Ok(required)
    }
}

/// Decodes all of `path` into memory.
///
/// Does I/O. Complexity: O(n) in the file's frames.
pub fn decode_file(path: &Path) -> Result<AudioData, AudioError> {
    let mut decoder = Decoder::open(path)?;
    let info = decoder.info();
    let capacity = info.frames.map_or(0, |f| usize::try_from(f).unwrap_or(0));
    let mut channels: Vec<Vec<f32>> = (0..info.channels).map(|_| Vec::with_capacity(capacity)).collect();
    while decoder.decode_next(&mut channels)?.is_some() {}
    Ok(AudioData { rate: info.rate, channels })
}
