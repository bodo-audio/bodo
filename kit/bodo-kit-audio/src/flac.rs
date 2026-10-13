//! Writing FLAC files, block by block, so a recording can stream to disk losslessly at about half the size of WAV.
//!
//! The encoder is deliberately small: 24-bit samples, frames of 4096 samples, independent channels, and for each
//! subframe the best of the fixed predictors of order 0 to 4 (or the samples verbatim), with its residual Rice coded
//! in up to 16 partitions.
//! That reaches most of the compression of a full encoder on recorded audio. The STREAMINFO block is written first
//! and filled in with the length and frame sizes when the writer finishes; its MD5 signature is left zero, which FLAC
//! defines as "not computed".

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use bodo_kit_core::SampleRate;

use crate::AudioError;

/// Samples per channel in each frame.
const BLOCK: usize = 4096;
/// Bits per sample.
const BITS: u32 = 24;

/// Writes a 24-bit FLAC file block by block.
#[derive(Debug)]
pub struct FlacWriter {
    file: BufWriter<File>,
    path: PathBuf,
    rate: SampleRate,
    channels: usize,
    /// Samples waiting for a full frame, one `Vec` per channel.
    pending: Vec<Vec<i32>>,
    frames: u64,
    frame_number: u64,
    min_frame: u32,
    max_frame: u32,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> AudioError + '_ {
    move |source| AudioError::Io { path: path.to_path_buf(), source }
}

impl FlacWriter {
    /// Creates `path`, replacing any file there, for `channels` channels (1 to 8) at `rate`.
    ///
    /// Does I/O.
    pub fn create(path: &Path, rate: SampleRate, channels: usize) -> Result<Self, AudioError> {
        if !(1..=8).contains(&channels) {
            return Err(AudioError::Unsupported(format!("FLAC with {channels} channels")));
        }
        let mut file = BufWriter::new(File::create(path).map_err(io(path))?);
        file.write_all(b"fLaC").map_err(io(path))?;
        // The STREAMINFO block, the last metadata block; finish() fills in the parts known only at the end.
        file.write_all(&[0x80, 0, 0, 34]).map_err(io(path))?;
        file.write_all(&[0u8; 34]).map_err(io(path))?;
        let writer = Self {
            file,
            path: path.to_path_buf(),
            rate,
            channels,
            pending: vec![Vec::with_capacity(BLOCK); channels],
            frames: 0,
            frame_number: 0,
            min_frame: u32::MAX,
            max_frame: 0,
        };
        Ok(writer)
    }

    /// Appends `frames` frames from planar `channels`, which must hold at least that many samples each. Missing
    /// channels are written as silence; samples are rounded to 24 bits and clamped at full scale.
    ///
    /// Does I/O. Complexity: O(channels * frames).
    pub fn write(&mut self, channels: &[&[f32]], frames: usize) -> Result<(), AudioError> {
        let mut done = 0;
        while done < frames {
            let take = (BLOCK - self.pending[0].len()).min(frames - done);
            for (index, pending) in self.pending.iter_mut().enumerate() {
                let source = channels.get(index).copied().unwrap_or(&[]);
                for frame in done..done + take {
                    let sample = source.get(frame).copied().unwrap_or(0.0);
                    pending.push((f64::from(sample) * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32);
                }
            }
            done += take;
            if self.pending[0].len() == BLOCK {
                self.flush_frame()?;
            }
        }
        Ok(())
    }

    /// Encodes the pending samples as one frame.
    fn flush_frame(&mut self) -> Result<(), AudioError> {
        let samples = self.pending[0].len();
        if samples == 0 {
            return Ok(());
        }
        let mut bits = BitWriter::default();
        // Frame header: sync, fixed block size, a 16-bit block size at the end of the header, the rate from
        // STREAMINFO, independent channels, 24 bits per sample.
        bits.put(0b11_1111_1111_1110, 14);
        bits.put(0, 1);
        bits.put(0, 1);
        bits.put(0b0111, 4);
        bits.put(0b0000, 4);
        bits.put((self.channels - 1) as u64, 4);
        bits.put(0b110, 3);
        bits.put(0, 1);
        put_utf8(&mut bits, self.frame_number);
        bits.put((samples - 1) as u64, 16);
        let header_crc = crc8(&bits.bytes);
        bits.put(u64::from(header_crc), 8);
        for channel in &self.pending {
            encode_subframe(&mut bits, channel);
        }
        bits.align();
        let crc = crc16(&bits.bytes);
        bits.put(u64::from(crc), 16);
        let bytes = bits.bytes;
        self.file.write_all(&bytes).map_err(io(&self.path))?;
        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        self.min_frame = self.min_frame.min(size);
        self.max_frame = self.max_frame.max(size);
        self.frames += samples as u64;
        self.frame_number += 1;
        for pending in &mut self.pending {
            pending.clear();
        }
        Ok(())
    }

    /// Encodes the last, shorter frame, fills in STREAMINFO and closes the file.
    ///
    /// Does I/O.
    pub fn finish(mut self) -> Result<(), AudioError> {
        self.flush_frame()?;
        let mut info = BitWriter::default();
        info.put(BLOCK as u64, 16);
        info.put(BLOCK as u64, 16);
        info.put(if self.min_frame == u32::MAX { 0 } else { u64::from(self.min_frame) }, 24);
        info.put(u64::from(self.max_frame), 24);
        info.put(u64::from(self.rate.hz()), 20);
        info.put((self.channels - 1) as u64, 3);
        info.put(u64::from(BITS - 1), 5);
        info.put(self.frames, 36);
        info.put(0, 64);
        info.put(0, 64);
        self.file.seek(SeekFrom::Start(8)).map_err(io(&self.path))?;
        self.file.write_all(&info.bytes).map_err(io(&self.path))?;
        self.file.flush().map_err(io(&self.path))
    }
}

/// Packs bits most significant first.
#[derive(Debug, Default)]
struct BitWriter {
    bytes: Vec<u8>,
    current: u64,
    filled: u32,
}

impl BitWriter {
    /// Appends the low `count` bits of `value`, `count` at most 57.
    fn put(&mut self, value: u64, count: u32) {
        if count > 32 {
            self.put(value >> 32, count - 32);
            self.put(value & 0xFFFF_FFFF, 32);
            return;
        }
        let mask = if count == 64 { u64::MAX } else { (1u64 << count) - 1 };
        self.current = (self.current << count) | (value & mask);
        self.filled += count;
        while self.filled >= 8 {
            self.filled -= 8;
            self.bytes.push((self.current >> self.filled) as u8);
        }
        self.current &= (1u64 << self.filled) - 1;
    }

    /// Writes `zeros` zero bits and a one: the unary part of a Rice code.
    fn unary(&mut self, zeros: u32) {
        let mut left = zeros;
        while left >= 32 {
            self.put(0, 32);
            left -= 32;
        }
        self.put(1, left + 1);
    }

    /// Pads with zero bits to a byte boundary.
    fn align(&mut self) {
        if self.filled > 0 {
            self.put(0, 8 - self.filled);
        }
    }
}

/// Writes `value` in FLAC's UTF-8-like variable-length code, as frame numbers are.
fn put_utf8(bits: &mut BitWriter, value: u64) {
    if value < 0x80 {
        bits.put(value, 8);
        return;
    }
    let mut count = 2;
    while count < 7 && value >= 1u64 << (5 * count + 1) {
        count += 1;
    }
    let lead = (0xFF00u64 >> count) & 0xFF;
    bits.put(lead | (value >> (6 * (count - 1))), 8);
    for i in (0..count - 1).rev() {
        bits.put(0x80 | ((value >> (6 * i)) & 0x3F), 8);
    }
}

/// The residual of the fixed predictor of `order` (0 to 4) after its warm-up samples.
fn residual(samples: &[i32], order: usize) -> impl Iterator<Item = i64> + '_ {
    samples.windows(order + 1).map(move |w| {
        let s = |k: usize| i64::from(w[order - k]);
        match order {
            0 => s(0),
            1 => s(0) - s(1),
            2 => s(0) - 2 * s(1) + s(2),
            3 => s(0) - 3 * s(1) + 3 * s(2) - s(3),
            _ => s(0) - 4 * s(1) + 6 * s(2) - 4 * s(3) + s(4),
        }
    })
}

/// The Rice parameter and bit count that code `values` best. Partitions use 5-bit parameters (FLAC's second Rice
/// method), so any parameter up to 30 is allowed.
fn rice_cost(values: &[u64]) -> (u32, u64) {
    let mut best = (0, u64::MAX);
    for k in 0..=30u32 {
        let cost: u64 = values.iter().map(|&u| (u >> k) + 1 + u64::from(k)).sum();
        if cost < best.1 {
            best = (k, cost);
        }
    }
    best
}

/// Rice partitions: each one's parameter and its range of residuals.
type Partitions = Vec<(u32, usize, usize)>;

/// A way to code a subframe and what it costs in bits.
struct Plan {
    order: usize,
    partition_order: u32,
    partitions: Partitions,
    bits: u64,
}

/// Encodes one channel of a frame as the cheapest fixed-predictor subframe, or verbatim when that is smaller.
fn encode_subframe(bits: &mut BitWriter, samples: &[i32]) {
    let zigzag = |r: i64| ((r << 1) ^ (r >> 63)) as u64;
    let max_order = 4.min(samples.len().saturating_sub(1));
    let mut best: Option<Plan> = None;
    for order in 0..=max_order {
        let residuals: Vec<u64> = residual(samples, order).map(zigzag).collect();
        // Rice partitions: the split into up to 16 equal partitions that codes cheapest, the first partition
        // shortened by the warm-up samples.
        let mut best_partitioned: Option<(u32, Partitions, u64)> = None;
        for partition_order in 0..=4u32 {
            let parts = 1usize << partition_order;
            if !samples.len().is_multiple_of(parts) || samples.len() / parts <= order {
                break;
            }
            let length = samples.len() / parts;
            let mut ranges = Vec::with_capacity(parts);
            let mut cost = 0u64;
            let mut start = 0usize;
            for part in 0..parts {
                let count = if part == 0 { length - order } else { length };
                let (k, bits_needed) = rice_cost(&residuals[start..start + count]);
                ranges.push((k, start, start + count));
                cost += bits_needed + 5;
                start += count;
            }
            if best_partitioned.as_ref().is_none_or(|b| cost < b.2) {
                best_partitioned = Some((partition_order, ranges, cost));
            }
        }
        let Some((partition_order, partitions, cost)) = best_partitioned else { continue };
        let total = cost + order as u64 * u64::from(BITS);
        if best.as_ref().is_none_or(|b| total < b.bits) {
            best = Some(Plan { order, partition_order, partitions, bits: total });
        }
    }
    let verbatim = samples.len() as u64 * u64::from(BITS);
    match best {
        Some(Plan { order, partition_order, partitions: ranges, bits: total }) if total < verbatim => {
            let residuals: Vec<u64> = residual(samples, order).map(zigzag).collect();
            bits.put(0, 1);
            bits.put(0b001000 | order as u64, 6);
            bits.put(0, 1);
            for &warm in &samples[..order] {
                bits.put(u64::from(warm as u32) & 0xFF_FFFF, BITS);
            }
            bits.put(0b01, 2);
            bits.put(u64::from(partition_order), 4);
            for (k, start, end) in ranges {
                bits.put(u64::from(k), 5);
                for &u in &residuals[start..end] {
                    bits.unary((u >> k) as u32);
                    if k > 0 {
                        bits.put(u & ((1u64 << k) - 1), k);
                    }
                }
            }
        }
        _ => {
            bits.put(0, 1);
            bits.put(0b000001, 6);
            bits.put(0, 1);
            for &sample in samples {
                bits.put(u64::from(sample as u32) & 0xFF_FFFF, BITS);
            }
        }
    }
}

/// CRC-8 with polynomial x^8 + x^2 + x + 1, as FLAC frame headers use.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

/// CRC-16 with polynomial x^16 + x^15 + x^2 + 1, as FLAC frames use.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flac_file_decodes_back_to_the_same_24_bit_samples() {
        let dir = std::env::temp_dir().join(format!("bodo-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("take.flac");
        // A tone, some noise and a jump, over more than two frames and a short last one.
        let frames = 2 * BLOCK + 777;
        let mut seed = 1u32;
        let left: Vec<f32> = (0..frames)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (i as f32 * 0.03).sin() * 0.7 + (seed >> 9) as f32 / 8_388_608.0 * 0.01 - 0.005
            })
            .collect();
        let right: Vec<f32> = (0..frames).map(|i| if i % 1000 < 500 { 0.5 } else { -0.25 }).collect();
        let mut writer = FlacWriter::create(&path, SampleRate::HZ_48000, 2).unwrap();
        writer.write(&[&left[..3000], &right[..3000]], 3000).unwrap();
        writer.write(&[&left[3000..], &right[3000..]], frames - 3000).unwrap();
        writer.finish().unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size < (frames * 2 * 3) as u64, "smaller than 24-bit WAV data: {size}");
        let audio = crate::decode_file(&path).unwrap();
        assert_eq!((audio.rate, audio.channel_count(), audio.frames()), (SampleRate::HZ_48000, 2, frames));
        let step = 1.0 / 8_388_608.0;
        for (decoded, original) in [(&audio.channels[0], &left), (&audio.channels[1], &right)] {
            for (a, b) in decoded.iter().zip(original.iter()) {
                assert!((a - b).abs() <= step, "{a} vs {b}");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
