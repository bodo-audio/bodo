//! Sony Wave64 (`.w64`): WAV with 64-bit sizes, so a single recording can pass WAV's 4 GB limit.
//!
//! A Wave64 file is a `riff` chunk holding the `wave` form, a `fmt ` chunk with the usual WAVEFORMATEX, and a `data`
//! chunk. Chunks are named by 16-byte GUIDs, their sizes are 64-bit and include the 24-byte chunk header, and every
//! chunk starts on an 8-byte boundary. [`W64Writer`] writes 32-bit float or 24-bit integer files block by block and
//! fills in the sizes when it finishes; [`W64Reader`] reads 16, 24 and 32-bit integer and 32-bit float files.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use bodo_kit_core::SampleRate;

use crate::AudioError;

const RIFF: [u8; 16] = [0x72, 0x69, 0x66, 0x66, 0x2E, 0x91, 0xCF, 0x11, 0xA5, 0xD6, 0x28, 0xDB, 0x04, 0xC1, 0x00, 0x00];
const WAVE: [u8; 16] = [0x77, 0x61, 0x76, 0x65, 0xF3, 0xAC, 0xD3, 0x11, 0x8C, 0xD1, 0x00, 0xC0, 0x4F, 0x8E, 0xDB, 0x8A];
const FMT: [u8; 16] = [0x66, 0x6D, 0x74, 0x20, 0xF3, 0xAC, 0xD3, 0x11, 0x8C, 0xD1, 0x00, 0xC0, 0x4F, 0x8E, 0xDB, 0x8A];
const DATA: [u8; 16] = [0x64, 0x61, 0x74, 0x61, 0xF3, 0xAC, 0xD3, 0x11, 0x8C, 0xD1, 0x00, 0xC0, 0x4F, 0x8E, 0xDB, 0x8A];

/// The bytes before the samples: the riff header with the form, the fmt chunk, and the data chunk's header.
const HEADER: u64 = 24 + 16 + (24 + 16) + 24;

/// The sample format of a Wave64 file Bodo writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum W64Format {
    /// 24-bit integer.
    Int24,
    /// 32-bit float, the default.
    #[default]
    Float32,
}

impl W64Format {
    fn bits(self) -> u16 {
        match self {
            Self::Int24 => 24,
            Self::Float32 => 32,
        }
    }
}

/// Writes a Wave64 file block by block.
#[derive(Debug)]
pub struct W64Writer {
    file: BufWriter<File>,
    path: PathBuf,
    format: W64Format,
    channels: usize,
    frames: u64,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> AudioError + '_ {
    move |source| AudioError::Io { path: path.to_path_buf(), source }
}

impl W64Writer {
    /// Creates `path`, replacing any file there, for `channels` channels at `rate`.
    ///
    /// Does I/O.
    pub fn create(path: &Path, rate: SampleRate, channels: usize, format: W64Format) -> Result<Self, AudioError> {
        let channel_count = u16::try_from(channels).ok().filter(|&c| c > 0).ok_or_else(|| AudioError::Unsupported(format!("{channels} channels")))?;
        let mut file = BufWriter::new(File::create(path).map_err(io(path))?);
        let bytes = format.bits() / 8;
        let block_align = channel_count * bytes;
        let mut header = Vec::with_capacity(HEADER as usize);
        header.extend_from_slice(&RIFF);
        header.extend_from_slice(&HEADER.to_le_bytes());
        header.extend_from_slice(&WAVE);
        header.extend_from_slice(&FMT);
        header.extend_from_slice(&(24u64 + 16).to_le_bytes());
        let tag: u16 = if format == W64Format::Float32 { 3 } else { 1 };
        header.extend_from_slice(&tag.to_le_bytes());
        header.extend_from_slice(&channel_count.to_le_bytes());
        header.extend_from_slice(&rate.hz().to_le_bytes());
        header.extend_from_slice(&(rate.hz() * u32::from(block_align)).to_le_bytes());
        header.extend_from_slice(&block_align.to_le_bytes());
        header.extend_from_slice(&format.bits().to_le_bytes());
        header.extend_from_slice(&DATA);
        header.extend_from_slice(&24u64.to_le_bytes());
        file.write_all(&header).map_err(io(path))?;
        Ok(Self { file, path: path.to_path_buf(), format, channels, frames: 0 })
    }

    /// Appends `frames` frames from planar `channels`, which must hold at least that many samples each. Missing
    /// channels are written as silence.
    ///
    /// Does I/O. Complexity: O(channels * frames).
    pub fn write(&mut self, channels: &[&[f32]], frames: usize) -> Result<(), AudioError> {
        let mut bytes = Vec::with_capacity(frames * self.channels * 4);
        for frame in 0..frames {
            for channel in 0..self.channels {
                let sample = channels.get(channel).and_then(|c| c.get(frame)).copied().unwrap_or(0.0);
                match self.format {
                    W64Format::Float32 => bytes.extend_from_slice(&sample.to_le_bytes()),
                    W64Format::Int24 => {
                        let value = (f64::from(sample) * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                        bytes.extend_from_slice(&value.to_le_bytes()[..3]);
                    }
                }
            }
        }
        self.file.write_all(&bytes).map_err(io(&self.path))?;
        self.frames += frames as u64;
        Ok(())
    }

    /// Pads the data to 8 bytes, writes the sizes and closes the file.
    ///
    /// Does I/O.
    pub fn finish(mut self) -> Result<(), AudioError> {
        let data = self.frames * self.channels as u64 * u64::from(self.format.bits() / 8);
        let padding = (8 - data % 8) % 8;
        self.file.write_all(&vec![0u8; padding as usize]).map_err(io(&self.path))?;
        let total = HEADER + data + padding;
        self.file.seek(SeekFrom::Start(16)).map_err(io(&self.path))?;
        self.file.write_all(&total.to_le_bytes()).map_err(io(&self.path))?;
        self.file.seek(SeekFrom::Start(HEADER - 8)).map_err(io(&self.path))?;
        self.file.write_all(&(24 + data).to_le_bytes()).map_err(io(&self.path))?;
        self.file.flush().map_err(io(&self.path))
    }
}

/// Reads a Wave64 file's samples.
#[derive(Debug)]
pub struct W64Reader {
    file: BufReader<File>,
    path: PathBuf,
    rate: SampleRate,
    channels: usize,
    /// Bits per sample, and whether samples are floats.
    bits: u16,
    float: bool,
    /// Frames in the data chunk, and frames read so far.
    frames: u64,
    position: u64,
    /// Where the data chunk's samples start.
    data_start: u64,
}

/// Whether `path` names a Wave64 file, by its extension.
pub fn is_w64(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("w64"))
}

impl W64Reader {
    /// Opens `path` and reads its header.
    ///
    /// Does I/O.
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        let mut file = BufReader::new(File::open(path).map_err(io(path))?);
        let bad = |what: &str| AudioError::Unsupported(format!("{}: {what}", path.display()));
        let mut guid = [0u8; 16];
        let mut size = [0u8; 8];
        file.read_exact(&mut guid).map_err(io(path))?;
        file.read_exact(&mut size).map_err(io(path))?;
        let mut form = [0u8; 16];
        file.read_exact(&mut form).map_err(io(path))?;
        if guid != RIFF || form != WAVE {
            return Err(bad("not a Wave64 file"));
        }
        let mut offset = 40u64;
        let mut format: Option<(u16, u16, u32, u16)> = None;
        loop {
            if file.read_exact(&mut guid).is_err() {
                return Err(bad("no data chunk"));
            }
            file.read_exact(&mut size).map_err(io(path))?;
            let chunk = u64::from_le_bytes(size);
            if chunk < 24 {
                return Err(bad("a chunk is too short"));
            }
            let body = chunk - 24;
            if guid == FMT {
                let mut fmt = [0u8; 16];
                file.read_exact(&mut fmt).map_err(io(path))?;
                let read = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
                let rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
                format = Some((read(0), read(2), rate, read(14)));
                let rest = body.saturating_sub(16) + (8 - chunk % 8) % 8;
                file.seek(SeekFrom::Current(rest as i64)).map_err(io(path))?;
            } else if guid == DATA {
                let (tag, channels, rate, bits) = format.ok_or_else(|| bad("the data comes before the format"))?;
                let float = match (tag, bits) {
                    (3, 32) => true,
                    (1 | 0xFFFE, 16 | 24 | 32) => false,
                    _ => return Err(bad("unsupported sample format")),
                };
                let rate = SampleRate::new(rate).ok_or_else(|| bad("bad sample rate"))?;
                let channels = usize::from(channels).max(1);
                let frames = body / (channels as u64 * u64::from(bits / 8));
                return Ok(Self { file, path: path.to_path_buf(), rate, channels, bits, float, frames, position: 0, data_start: offset + 24 });
            } else {
                file.seek(SeekFrom::Current((body + (8 - chunk % 8) % 8) as i64)).map_err(io(path))?;
            }
            offset += chunk + (8 - chunk % 8) % 8;
        }
    }

    /// The sample rate.
    pub fn rate(&self) -> SampleRate {
        self.rate
    }

    /// The number of channels.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The length in frames.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Reads up to `max` frames and appends them to `out`, one `Vec` per channel. Returns the frames read, zero at the
    /// end.
    ///
    /// Does I/O. Complexity: O(n) in the frames read.
    pub fn read(&mut self, out: &mut [Vec<f32>], max: usize) -> Result<usize, AudioError> {
        let frames = (max as u64).min(self.frames - self.position) as usize;
        let width = usize::from(self.bits / 8);
        let mut bytes = vec![0u8; frames * self.channels * width];
        self.file.read_exact(&mut bytes).map_err(io(&self.path))?;
        for (i, sample) in bytes.chunks_exact(width).enumerate() {
            let value = match (self.float, width) {
                (true, _) => f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]),
                (false, 2) => f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0,
                (false, 3) => (i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8) as f32 / 8_388_608.0,
                (false, _) => i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32 / 2_147_483_648.0,
            };
            if let Some(channel) = out.get_mut(i % self.channels) {
                channel.push(value);
            }
        }
        self.position += frames as u64;
        Ok(frames)
    }

    /// Moves to `frame`, clamped to the length. Returns the frame reached.
    ///
    /// Does I/O.
    pub fn seek(&mut self, frame: u64) -> Result<u64, AudioError> {
        let frame = frame.min(self.frames);
        let offset = self.data_start + frame * self.channels as u64 * u64::from(self.bits / 8);
        self.file.seek(SeekFrom::Start(offset)).map_err(io(&self.path))?;
        self.position = frame;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_and_24_bit_files_read_back() {
        let dir = std::env::temp_dir().join(format!("bodo-w64-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let left: Vec<f32> = (0..1001).map(|i| ((i as f32) * 0.01).sin() * 0.9).collect();
        let right: Vec<f32> = left.iter().map(|s| -s * 0.5).collect();
        for format in [W64Format::Float32, W64Format::Int24] {
            let path = dir.join(format!("{format:?}.w64"));
            let mut writer = W64Writer::create(&path, SampleRate::HZ_48000, 2, format).unwrap();
            writer.write(&[&left[..500], &right[..500]], 500).unwrap();
            writer.write(&[&left[500..], &right[500..]], 501).unwrap();
            writer.finish().unwrap();
            let size = std::fs::metadata(&path).unwrap().len();
            assert_eq!(size % 8, 0, "chunks end on 8 bytes");
            let mut reader = W64Reader::open(&path).unwrap();
            assert_eq!((reader.rate(), reader.channels(), reader.frames()), (SampleRate::HZ_48000, 2, 1001));
            let mut out = vec![Vec::new(), Vec::new()];
            while reader.read(&mut out, 300).unwrap() > 0 {}
            let tolerance = if format == W64Format::Float32 { 0.0 } else { 1.0 / 8_000_000.0 };
            for (a, b) in out[0].iter().zip(&left).chain(out[1].iter().zip(&right)) {
                assert!((a - b).abs() <= tolerance, "{format:?}: {a} vs {b}");
            }
            reader.seek(1000).unwrap();
            let mut last = vec![Vec::new(), Vec::new()];
            assert_eq!(reader.read(&mut last, 10).unwrap(), 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
