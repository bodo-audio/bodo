//! Samplers that keep their sample on disk (project.md section 10.2, ADR 0016): long samples stream through the
//! kit's disk thread instead of loading into memory.
//!
//! A note can't wait for the disk, so the first second from each place a note can start (the start offset, and the
//! loop start for the jump back) stays in memory as a *head*. A voice plays its head while its own stream reader,
//! sought to the head's end when the note starts, fills a small window behind it; after the head it reads that
//! window. When the disk falls behind the voice plays silence and the stream counts an underrun.
//!
//! Streaming needs the sample as the file holds it: rendered effects, a reverse, a stretch, slices, regions and
//! ping-pong loops need the sample in memory, and so do offline renders, which must never wait for the disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_audio::{AudioData, AudioError, Decoder, DiskThread};
use bodo_kit_core::SampleRate;
use bodo_model::channel::{LoopMode, SamplerParams, SamplerStretchMode};

/// How much of the sample stays in memory from each place a note can start.
pub const HEAD_SECONDS: f64 = 1.0;

/// A sample a sampler streams: where it is, how long, and the heads it keeps in memory.
#[derive(Debug)]
pub struct DiskSample {
    /// The file.
    pub path: PathBuf,
    /// Its own rate.
    pub rate: SampleRate,
    /// Its length in frames.
    pub frames: u64,
    /// Its channels.
    pub channels: usize,
    /// The audio from each place a note can start, by its first frame, sorted.
    pub heads: Vec<(u64, Arc<AudioData>)>,
    /// The thread that reads it.
    pub thread: Arc<DiskThread>,
}

impl DiskSample {
    /// The head that starts at `frame`.
    ///
    /// Complexity: O(log h) in heads.
    pub fn head_at(&self, frame: u64) -> Option<(u64, &AudioData)> {
        let index = self.heads.binary_search_by_key(&frame, |(start, _)| *start).ok()?;
        self.heads.get(index).map(|(start, data)| (*start, &**data))
    }
}

/// Whether `params` play the sample as the file holds it, forward, so it can stream.
pub fn streamable(params: &SamplerParams) -> bool {
    params.keep_on_disk
        && params.effects.is_identity()
        && !params.reverse
        && (params.stretch.mode == SamplerStretchMode::Resample || params.stretch.is_identity())
        && !params.load_regions
        && !params.load_slices
        && params.loop_mode != LoopMode::PingPong
}

/// Frames each head keeps before the place a note starts, for the interpolation that reads behind the position.
const LEAD: u64 = 4;

/// Where the heads of `params` begin in a sample `frames` long: a little before the start offset, and before the loop
/// start when it loops.
pub fn starts(params: &SamplerParams, frames: u64) -> Vec<u64> {
    let len = frames as f64;
    let (start, end) = (f64::from(params.start.clamp(0.0, 1.0)), f64::from(params.end.clamp(0.0, 1.0)));
    let note = ((start + f64::from(params.start_offset.clamp(0.0, 1.0)) * (end - start)) * len) as u64;
    let mut out = vec![note.saturating_sub(LEAD)];
    if params.loop_mode != LoopMode::Off {
        out.push(((f64::from(params.loop_start.clamp(0.0, 1.0)) * len).max(start * len) as u64).saturating_sub(LEAD));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Reads `frames` frames of `path` from `start` on.
///
/// Does I/O. Complexity: O(frames).
fn read_head(path: &Path, start: u64, frames: usize, channels: usize, rate: SampleRate) -> Result<AudioData, AudioError> {
    let mut decoder = Decoder::open(path)?;
    if start > 0 {
        decoder.seek(start)?;
    }
    let mut out = vec![Vec::with_capacity(frames + 4096); channels];
    while out.first().map_or(0, Vec::len) < frames {
        if decoder.decode_next(&mut out)?.is_none() {
            break;
        }
    }
    for channel in &mut out {
        channel.truncate(frames);
    }
    Ok(AudioData { rate, channels: out })
}

/// Streamed samples by file and the places notes start, and the disk thread they share.
#[derive(Debug, Default)]
pub(crate) struct DiskCache {
    entries: HashMap<(PathBuf, Vec<u64>), Arc<DiskSample>>,
    thread: Option<Arc<DiskThread>>,
}

impl DiskCache {
    /// The streamed sample at `path` for `params`, reading its heads the first time. `None` when the file's length is
    /// unknown or it can't be read, so the sampler loads it into memory instead.
    ///
    /// Does I/O on first use: the heads and the file's header.
    pub(crate) fn get(&mut self, path: &Path, params: &SamplerParams) -> Option<Arc<DiskSample>> {
        let info = Decoder::open(path).ok()?.info();
        let frames = info.frames.filter(|f| *f > 0)?;
        let starts = starts(params, frames);
        let key = (path.to_path_buf(), starts.clone());
        if let Some(sample) = self.entries.get(&key) {
            return Some(Arc::clone(sample));
        }
        let thread = match &self.thread {
            Some(thread) => Arc::clone(thread),
            None => {
                let thread = Arc::new(DiskThread::spawn().map_err(|err| tracing::warn!(%err, "no disk thread")).ok()?);
                self.thread = Some(Arc::clone(&thread));
                thread
            }
        };
        let head = (HEAD_SECONDS * f64::from(info.rate.hz())) as usize;
        let mut heads = Vec::with_capacity(starts.len());
        for start in starts {
            match read_head(path, start, head, info.channels, info.rate) {
                Ok(data) => heads.push((start, Arc::new(data))),
                Err(err) => {
                    tracing::warn!(%err, path = %path.display(), "could not read a sample's head");
                    return None;
                }
            }
        }
        let sample = Arc::new(DiskSample { path: path.to_path_buf(), rate: info.rate, frames, channels: info.channels, heads, thread });
        self.entries.insert(key, Arc::clone(&sample));
        Some(sample)
    }

    /// Forgets streamed samples nothing plays any more.
    pub(crate) fn trim(&mut self) {
        self.entries.retain(|_, sample| Arc::strong_count(sample) > 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_samples_played_as_the_file_holds_them_stream() {
        let on = SamplerParams { keep_on_disk: true, ..SamplerParams::default() };
        assert!(streamable(&on));
        assert!(!streamable(&SamplerParams::default()), "not asked to");
        assert!(!streamable(&SamplerParams { reverse: true, ..on.clone() }));
        assert!(!streamable(&SamplerParams { load_slices: true, ..on.clone() }));
        assert!(!streamable(&SamplerParams { loop_mode: LoopMode::PingPong, ..on.clone() }));
        let mut stretched = on.clone();
        stretched.stretch.mode = SamplerStretchMode::Stretch;
        stretched.stretch.multiplier = 2.0;
        assert!(!streamable(&stretched));
    }

    #[test]
    fn notes_start_at_the_offset_and_loops_at_the_loop_start() {
        let params = SamplerParams { start: 0.5, start_offset: 0.5, loop_mode: LoopMode::Forward, loop_start: 0.25, ..SamplerParams::default() };
        // The played part is 500 to 1000; half way in is 750; the loop start comes no earlier than the start, 500.
        // Each head begins a few frames before.
        assert_eq!(starts(&params, 1000), [496, 746]);
    }
}
