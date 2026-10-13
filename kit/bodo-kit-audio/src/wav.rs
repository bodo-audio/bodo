//! Writing WAV files: 16-bit and 24-bit integer, and 32-bit float.
//!
//! Integer formats round to the nearest step and clamp at full scale. Dither belongs to the export stage before
//! this writer (project.md section 6.8).

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use bodo_kit_core::SampleRate;

use crate::AudioError;

/// The sample format of a WAV file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WavFormat {
    /// 16-bit integer.
    Int16,
    /// 24-bit integer.
    Int24,
    /// 32-bit float, the default: it keeps every sample exactly.
    #[default]
    Float32,
}

/// Writes a WAV file block by block.
pub struct WavWriter {
    writer: hound::WavWriter<BufWriter<File>>,
    format: WavFormat,
    channels: usize,
}

impl std::fmt::Debug for WavWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WavWriter").field("format", &self.format).field("channels", &self.channels).finish_non_exhaustive()
    }
}

impl WavWriter {
    /// Creates `path`, replacing any file there, for `channels` channels at `rate`.
    ///
    /// Does I/O.
    pub fn create(path: &Path, rate: SampleRate, channels: usize, format: WavFormat) -> Result<Self, AudioError> {
        let channel_count = u16::try_from(channels).map_err(|_| AudioError::Unsupported(format!("{channels} channels")))?;
        let (bits_per_sample, sample_format) = match format {
            WavFormat::Int16 => (16, hound::SampleFormat::Int),
            WavFormat::Int24 => (24, hound::SampleFormat::Int),
            WavFormat::Float32 => (32, hound::SampleFormat::Float),
        };
        let spec = hound::WavSpec { channels: channel_count, sample_rate: rate.hz(), bits_per_sample, sample_format };
        let writer = hound::WavWriter::create(path, spec).map_err(|err| wav_error(path, err))?;
        Ok(Self { writer, format, channels })
    }

    /// Appends `frames` frames from planar `channels`, which must hold at least that many samples each. Missing
    /// channels are written as silence.
    ///
    /// Does I/O. Complexity: O(channels * frames).
    pub fn write(&mut self, channels: &[&[f32]], frames: usize) -> Result<(), AudioError> {
        for frame in 0..frames {
            for channel in 0..self.channels {
                let sample = channels.get(channel).and_then(|c| c.get(frame)).copied().unwrap_or(0.0);
                let result = match self.format {
                    WavFormat::Float32 => self.writer.write_sample(sample),
                    WavFormat::Int16 => self.writer.write_sample(quantize(sample, 16) as i16),
                    WavFormat::Int24 => self.writer.write_sample(quantize(sample, 24)),
                };
                result.map_err(|err| AudioError::Wav(err.to_string()))?;
            }
        }
        Ok(())
    }

    /// Writes the header lengths and closes the file.
    ///
    /// Does I/O.
    pub fn finish(self) -> Result<(), AudioError> {
        self.writer.finalize().map_err(|err| AudioError::Wav(err.to_string()))
    }
}

/// Rounds `sample` to a signed integer of `bits` bits, clamped to full scale.
fn quantize(sample: f32, bits: u32) -> i32 {
    let scale = (1i64 << (bits - 1)) as f64;
    let max = scale - 1.0;
    (f64::from(sample) * scale).round().clamp(-scale, max) as i32
}

fn wav_error(path: &Path, err: hound::Error) -> AudioError {
    match err {
        hound::Error::IoError(source) => AudioError::Io { path: path.to_path_buf(), source },
        other => AudioError::Wav(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizes_with_rounding_and_clamping() {
        assert_eq!(quantize(0.0, 16), 0);
        assert_eq!(quantize(1.0, 16), 32767);
        assert_eq!(quantize(-1.0, 16), -32768);
        assert_eq!(quantize(2.0, 16), 32767);
        assert_eq!(quantize(0.5, 24), 4_194_304);
        assert_eq!(quantize(1.4 / 32768.0, 16), 1);
    }
}
