//! The Sampler window's waveform: the sample as the sampler plays it, with the precomputed effects, the reverse and
//! any stretch rendered in by the engine's own code ([`bodo_mix::sampler_source::derive`]), and its slices or regions.
//! Each preview renders on a thread of its own, once per file and shape, so drawing never waits for the disk.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bodo_kit_audio::ChannelPeaks;
use bodo_model::ChannelId;
use bodo_model::channel::SamplerParams;

/// A rendered preview: the peaks of every channel, the length, and the zones as fractions of the length.
#[derive(Debug)]
pub(crate) struct Preview {
    channels: Vec<ChannelPeaks>,
    /// Frames in the played sample.
    pub(crate) frames: usize,
    /// Slices or regions, as start and end fractions.
    pub(crate) zones: Vec<(f32, f32)>,
}

impl Preview {
    /// The lowest and highest sample of any channel between fractions `from` and `to` of the sample.
    ///
    /// Complexity: O(c log n) in channels and frames.
    pub(crate) fn range(&self, from: f32, to: f32) -> Option<(f32, f32)> {
        let frames = self.frames as f32;
        let (start, end) = ((from.max(0.0) * frames) as usize, ((to * frames).ceil() as usize).max((from * frames) as usize + 1));
        self.channels.iter().filter_map(|c| c.range(start, end)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }
}

/// A finished render: the channel, the shape it was made for, and the preview if the file could be read.
type Done = (ChannelId, String, Option<Arc<Preview>>);

/// Every open sampler window's preview.
#[derive(Debug)]
pub(crate) struct Previews {
    /// By channel: the shape rendered or being rendered, and the result once there is one.
    states: HashMap<ChannelId, (String, Option<Arc<Preview>>, bool)>,
    sender: Sender<Done>,
    receiver: Receiver<Done>,
}

impl Default for Previews {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { states: HashMap::new(), sender, receiver }
    }
}

impl Previews {
    /// The preview of channel `channel` playing the file at `path` with `params` at `bpm`. While a new shape renders,
    /// the last one stays on screen.
    ///
    /// Complexity: O(1) on average; rendering happens on its own thread.
    pub(crate) fn get(&mut self, ctx: &egui::Context, channel: ChannelId, path: &Path, params: &SamplerParams, bpm: f64) -> Option<Arc<Preview>> {
        self.collect();
        let shape = shape(path, params, bpm);
        if let Some((current, preview, _)) = self.states.get(&channel)
            && *current == shape
        {
            return preview.clone();
        }
        let last = self.states.get(&channel).and_then(|(_, preview, _)| preview.clone());
        self.states.insert(channel, (shape.clone(), last.clone(), true));
        let (sender, ctx, file, params) = (self.sender.clone(), ctx.clone(), path.to_path_buf(), params.clone());
        let spawned = std::thread::Builder::new().name(String::from("bodo-sampler-preview")).spawn(move || {
            let preview = render(&file, &params, bpm);
            let _ = sender.send((channel, shape, preview));
            ctx.request_repaint();
        });
        if spawned.is_err()
            && let Some(state) = self.states.get_mut(&channel)
        {
            state.2 = false;
        }
        last
    }

    /// Takes in the renders that finished; a render for a shape the channel no longer has is dropped.
    fn collect(&mut self) {
        while let Ok((channel, shape, preview)) = self.receiver.try_recv() {
            if let Some(state) = self.states.get_mut(&channel)
                && state.0 == shape
            {
                *state = (shape, preview, false);
            }
        }
    }

    /// How many previews are still rendering.
    ///
    /// Complexity: O(w) in windows.
    pub(crate) fn pending(&mut self) -> usize {
        self.collect();
        self.states.values().filter(|(_, _, loading)| *loading).count()
    }
}

/// What decides how the preview looks: the file and the settings that shape the played sample.
fn shape(path: &Path, params: &SamplerParams, bpm: f64) -> String {
    format!(
        "{}|{:?}|{}|{:?}|{:?}|{}|{}|{}|{}|{}|{}",
        path.display(),
        params.effects,
        params.reverse,
        params.stretch,
        params.loop_mode,
        params.loop_start,
        params.loop_end,
        params.load_regions,
        params.load_slices,
        params.start,
        params.end,
    ) + &format!("|{}", bpm.to_bits())
}

/// Decodes `file` and renders it as `params` play it.
fn render(file: &Path, params: &SamplerParams, bpm: f64) -> Option<Arc<Preview>> {
    let data = match bodo_kit_audio::decode_file(file) {
        Ok(data) => data,
        Err(error) => {
            tracing::warn!(%error, file = %file.display(), "no waveform for a sampler");
            return None;
        }
    };
    let markers = if params.load_regions || params.load_slices { bodo_kit_audio::read_markers(file).unwrap_or_default() } else { Default::default() };
    let (played, zones) = bodo_mix::sampler_source::derive(&data, &markers, params, bpm);
    let frames = played.frames().max(1);
    let zones = zones.iter().map(|(s, e)| ((*s / frames as f64) as f32, (*e / frames as f64) as f32)).collect();
    Some(Arc::new(Preview { channels: played.channels.iter().map(|c| ChannelPeaks::build(c)).collect(), frames, zones }))
}
