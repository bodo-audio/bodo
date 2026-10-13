//! Waveform peaks for audio clips (project.md section 6.7). Each sample is decoded once, on a thread of its own, into
//! the kit's peaks pyramid and kept while the playlist lives, so drawing a waveform never touches the disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bodo_kit_audio::ChannelPeaks;
use bodo_model::SampleId;

/// The peaks of one sample, for every channel.
#[derive(Debug)]
pub(crate) struct Waveform {
    /// The file's sample rate.
    rate: f64,
    /// Its length in frames.
    frames: usize,
    channels: Vec<ChannelPeaks>,
}

impl Waveform {
    /// The peaks of `channels`, recorded at `rate` frames per second.
    ///
    /// Complexity: O(n) in samples.
    pub(crate) fn new(rate: f64, channels: &[Vec<f32>]) -> Self {
        let channels: Vec<ChannelPeaks> = channels.iter().map(|c| ChannelPeaks::build(c)).collect();
        let frames = channels.iter().map(ChannelPeaks::frames).max().unwrap_or(0);
        Self { rate: rate.max(1.0), frames, channels }
    }

    /// How long the sample lasts.
    pub(crate) fn seconds(&self) -> f64 {
        self.frames as f64 / self.rate
    }

    /// The lowest and highest sample of any channel between `start` and `end` seconds into the file, or `None` outside
    /// it.
    ///
    /// Complexity: O(c log n) in channels and frames.
    pub(crate) fn range(&self, start: f64, end: f64) -> Option<(f32, f32)> {
        let from = (start.max(0.0) * self.rate).floor() as usize;
        let to = ((end * self.rate).ceil().max(0.0) as usize).max(from + 1);
        self.channels.iter().filter_map(|c| c.range(from, to)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }
}

/// Where a sample's peaks are.
#[derive(Debug)]
enum State {
    /// Being read.
    Loading,
    /// Ready to draw.
    Ready(Arc<Waveform>),
    /// The file could not be read.
    Missing,
}

/// A finished read: the sample, its file, and its peaks if the file could be decoded.
type Done = (SampleId, PathBuf, Option<Arc<Waveform>>);

/// Every sample's peaks, read on demand.
#[derive(Debug)]
pub(crate) struct PeaksCache {
    /// By sample: the file read and where its peaks are.
    states: HashMap<SampleId, (PathBuf, State)>,
    sender: Sender<Done>,
    receiver: Receiver<Done>,
}

impl Default for PeaksCache {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { states: HashMap::new(), sender, receiver }
    }
}

impl PeaksCache {
    /// The peaks of `sample`, stored at `path`, once they are ready. The first time a sample (or a new file for it) is
    /// asked for, a thread starts reading it and asks `ctx` to draw again when it is done.
    ///
    /// Complexity: O(1) on average; reading happens on its own thread.
    pub(crate) fn get(&mut self, ctx: &egui::Context, sample: SampleId, path: &Path) -> Option<Arc<Waveform>> {
        self.collect();
        match self.states.get(&sample) {
            Some((file, State::Ready(waveform))) if file == path => return Some(Arc::clone(waveform)),
            Some((file, _)) if file == path => return None,
            _ => {}
        }
        self.states.insert(sample, (path.to_path_buf(), State::Loading));
        let (sender, ctx, file) = (self.sender.clone(), ctx.clone(), path.to_path_buf());
        let read = std::thread::Builder::new().name(String::from("bodo-peaks")).spawn(move || {
            let waveform = match bodo_kit_audio::decode_file(&file) {
                Ok(data) => Some(Arc::new(Waveform::new(f64::from(data.rate.hz()), &data.channels))),
                Err(error) => {
                    tracing::warn!(%error, file = %file.display(), "no waveform for a sample");
                    None
                }
            };
            let _ = sender.send((sample, file, waveform));
            ctx.request_repaint();
        });
        if read.is_err() {
            self.states.insert(sample, (path.to_path_buf(), State::Missing));
        }
        None
    }

    /// Takes in the reads that finished.
    fn collect(&mut self) {
        while let Ok((sample, file, waveform)) = self.receiver.try_recv() {
            // A read for a file the sample no longer uses is dropped.
            if self.states.get(&sample).is_some_and(|(current, _)| *current == file) {
                let state = waveform.map_or(State::Missing, State::Ready);
                self.states.insert(sample, (file, state));
            }
        }
    }

    /// How many samples are still being read.
    ///
    /// Complexity: O(s) in samples asked for.
    pub(crate) fn pending(&mut self) -> usize {
        self.collect();
        self.states.values().filter(|(_, state)| matches!(state, State::Loading)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_cover_every_channel_in_seconds() {
        let left: Vec<f32> = (0..1000).map(|i| if i == 500 { 0.8 } else { 0.0 }).collect();
        let right: Vec<f32> = (0..1000).map(|i| if i == 900 { -0.5 } else { 0.0 }).collect();
        let waveform = Waveform::new(100.0, &[left, right]);
        assert_eq!(waveform.seconds(), 10.0);
        assert_eq!(waveform.range(0.0, 10.0), Some((-0.5, 0.8)));
        assert_eq!(waveform.range(0.0, 4.0), Some((0.0, 0.0)));
        assert_eq!(waveform.range(4.9, 5.1), Some((0.0, 0.8)));
        assert_eq!(waveform.range(10.5, 11.0), None, "past the end");
    }
}
