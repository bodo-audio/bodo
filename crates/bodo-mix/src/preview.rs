//! Previewing sounds from the Browser (project.md section 14.1, ADR 0029).
//!
//! Clicking a sound plays it on its own, after the master, at its own pitch, whether or not the song plays.
//! [`preview`] makes a pair:
//!
//! - [`Preview`], on the control side, decodes the file on a background thread (at most its first
//!   [`PREVIEW_SECONDS`]), converts it to the engine's rate and hands it to the audio side through
//!   [`bodo_kit_core::handoff`], which also brings the old sound back to be freed off the audio thread;
//! - [`PreviewPlayer`], on the audio side, adds the sound to the output after the engine has rendered it.
//!
//! A new sound, or stopping, never cuts the old one: the last few milliseconds of the old sound are copied into the
//! new one's preallocated tail and faded out, and every sound fades in, so previews never click.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use bodo_kit_audio::{AudioData, AudioError, Decoder};
use bodo_kit_core::handoff::{self, Receiver, Sender};
use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::AudioBuffer;

/// The longest stretch of a file a preview plays.
pub const PREVIEW_SECONDS: f64 = 30.0;
/// How loud previews play: -3 dB, to leave the song some room.
const GAIN: f32 = 0.707_946;
/// How long fades in and out take.
const FADE_SECONDS: f64 = 0.005;

/// A sound ready to play, or silence.
#[derive(Debug)]
pub struct PreviewSound {
    /// Which request it answers; 0 for silence.
    id: u64,
    left: Vec<f32>,
    right: Vec<f32>,
    position: usize,
    /// Frames of the fade in, and of the tail.
    fade: usize,
    /// The end of the sound this one replaced, fading out. Allocated when the sound is made, `fade` frames long.
    tail_left: Vec<f32>,
    tail_right: Vec<f32>,
    tail_len: usize,
    tail_position: usize,
}

impl PreviewSound {
    /// Silence that still fades out what it replaces.
    fn silent(fade: usize) -> Self {
        Self::new(0, Vec::new(), Vec::new(), fade)
    }

    fn new(id: u64, left: Vec<f32>, right: Vec<f32>, fade: usize) -> Self {
        Self { id, left, right, position: 0, fade, tail_left: vec![0.0; fade], tail_right: vec![0.0; fade], tail_len: 0, tail_position: 0 }
    }

    /// How long it is, in frames.
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    fn remaining(&self) -> usize {
        self.left.len().saturating_sub(self.position)
    }

    /// The fade-in gain at frame `position`.
    fn fade_in(&self, position: usize) -> f32 {
        if self.fade == 0 { 1.0 } else { ((position + 1) as f32 / self.fade as f32).min(1.0) }
    }
}

/// Copies the next frames of `old` into the tail of `new`, fading out, so `new` can finish what `old` was playing.
///
/// RT-safe: it writes into buffers allocated when `new` was made. Complexity: O(fade).
#[rt_safe]
fn hand_over(old: &mut PreviewSound, new: &mut PreviewSound) {
    let n = old.remaining().min(new.tail_left.len()).min(new.tail_right.len());
    for k in 0..n {
        let at = old.position + k;
        let gain = old.fade_in(at) * (1.0 - k as f32 / n as f32);
        new.tail_left[k] = old.left[at] * gain;
        new.tail_right[k] = old.right[at] * gain;
    }
    new.tail_len = n;
    new.tail_position = 0;
}

/// What both sides share: the id of the sound playing, or 0.
#[derive(Debug, Default)]
struct Shared {
    playing: AtomicU64,
}

/// The audio side of a preview: adds the sound to the output.
#[derive(Debug)]
pub struct PreviewPlayer {
    receiver: Receiver<Box<PreviewSound>>,
    current: Box<PreviewSound>,
    shared: Arc<Shared>,
}

impl PreviewPlayer {
    /// Takes the newest sound sent, then adds `frames` frames of it to `output`: to its first two channels, or mixed
    /// to mono on a one-channel output.
    ///
    /// RT-safe. Complexity: O(frames).
    #[rt_safe]
    pub fn mix_into(&mut self, output: &mut AudioBuffer, frames: usize) {
        self.receiver.update_with(&mut self.current, |old, new| hand_over(old, new));
        let sound = &mut *self.current;
        let frames = frames.min(output.frames());
        let mut channels = output.channels_mut();
        let left = channels.next();
        let right = channels.next();
        let Some(left) = left else { return };
        let mut right = right;
        let quiet = sound.tail_position >= sound.tail_len && sound.position >= sound.left.len();
        if !quiet {
            for i in 0..frames {
                let (mut l, mut r) = (0.0, 0.0);
                if sound.tail_position < sound.tail_len {
                    l += sound.tail_left[sound.tail_position];
                    r += sound.tail_right[sound.tail_position];
                    sound.tail_position += 1;
                }
                if sound.position < sound.left.len() {
                    let gain = sound.fade_in(sound.position);
                    l += sound.left[sound.position] * gain;
                    r += sound.right[sound.position] * gain;
                    sound.position += 1;
                }
                match right.as_deref_mut() {
                    Some(right) => {
                        left[i] += l * GAIN;
                        right[i] += r * GAIN;
                    }
                    None => left[i] += (l + r) * 0.5 * GAIN,
                }
            }
        }
        let playing = if sound.position < sound.left.len() { sound.id } else { 0 };
        self.shared.playing.store(playing, Ordering::Relaxed);
    }
}

/// A decoded sound, or why it couldn't be read, from the decoding thread.
type Decoded = Result<Box<PreviewSound>, String>;

/// The control side of a preview: starts and stops sounds.
#[derive(Debug)]
pub struct Preview {
    sender: Sender<Box<PreviewSound>>,
    shared: Arc<Shared>,
    rate: SampleRate,
    fade: usize,
    next_id: u64,
    /// The sound asked for last, by id and file.
    wanted: Option<(u64, PathBuf)>,
    loading: Option<(u64, mpsc::Receiver<Decoded>)>,
    /// A sound decoded while the handoff was full, sent on the next poll.
    pending: Option<Box<PreviewSound>>,
}

/// Makes a preview for an engine running at `rate`: the control side and the audio side.
///
/// Allocates. Complexity: O(1).
pub fn preview(rate: SampleRate) -> (Preview, PreviewPlayer) {
    let fade = (f64::from(rate.hz()) * FADE_SECONDS).round() as usize;
    let (sender, receiver) = handoff::handoff(4);
    let shared = Arc::new(Shared::default());
    let player = PreviewPlayer { receiver, current: Box::new(PreviewSound::silent(fade)), shared: Arc::clone(&shared) };
    let preview = Preview { sender, shared, rate, fade, next_id: 1, wanted: None, loading: None, pending: None };
    (preview, player)
}

impl Preview {
    /// Starts previewing the audio file at `path`, replacing any sound previewing now. Decoding happens on a
    /// background thread; [`Preview::poll`] sends the sound to the audio side once it is ready. Returns the request's
    /// id.
    pub fn play(&mut self, path: &Path) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = mpsc::channel();
        let (file, rate, fade) = (path.to_path_buf(), self.rate, self.fade);
        let spawned = std::thread::Builder::new().name(String::from("bodo-preview")).spawn(move || {
            let sound = decode(&file, rate, fade, id).map(Box::new).map_err(|e| e.to_string());
            let _ = tx.send(sound);
        });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start decoding a preview");
            return id;
        }
        self.wanted = Some((id, path.to_path_buf()));
        self.loading = Some((id, rx));
        id
    }

    /// Stops the preview, fading it out.
    pub fn stop(&mut self) {
        self.wanted = None;
        self.loading = None;
        self.pending = None;
        self.send(Box::new(PreviewSound::silent(self.fade)));
    }

    fn send(&mut self, sound: Box<PreviewSound>) {
        self.sender.collect();
        if let Err(full) = self.sender.send(sound) {
            self.pending = Some(full.0);
        }
    }

    /// Sends a sound that finished decoding to the audio side, and frees the sounds it is done with. Returns the
    /// error when the file asked for could not be read. Call it regularly, such as once per frame.
    ///
    /// Complexity: O(1), plus freeing old sounds.
    pub fn poll(&mut self) -> Option<String> {
        self.sender.collect();
        if let Some(sound) = self.pending.take() {
            self.send(sound);
        }
        let (id, rx) = self.loading.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(sound)) => {
                let current = self.wanted.as_ref().is_some_and(|(wanted, _)| wanted == id);
                self.loading = None;
                if current {
                    self.send(sound);
                }
                None
            }
            Ok(Err(error)) => {
                self.loading = None;
                self.wanted = None;
                Some(error)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.loading = None;
                self.wanted = None;
                None
            }
        }
    }

    /// The file previewing now: being decoded, or playing.
    pub fn current(&self) -> Option<&Path> {
        let (id, path) = self.wanted.as_ref()?;
        let loading = self.loading.as_ref().is_some_and(|(l, _)| l == id) || self.pending.as_ref().is_some_and(|s| s.id == *id);
        (loading || self.shared.playing.load(Ordering::Relaxed) == *id).then_some(path.as_path())
    }
}

/// Decodes the start of the file at `path` for a preview: at most [`PREVIEW_SECONDS`] of it, converted to `rate`, as
/// stereo (a mono file plays on both sides; a file with more channels plays its first two).
///
/// Does I/O and allocates. Complexity: O(frames).
pub fn decode(path: &Path, rate: SampleRate, fade: usize, id: u64) -> Result<PreviewSound, AudioError> {
    let mut decoder = Decoder::open(path)?;
    let info = decoder.info();
    let limit = (f64::from(info.rate.hz()) * PREVIEW_SECONDS) as usize;
    let mut channels = vec![Vec::new(); info.channels.max(1)];
    while channels.first().map_or(0, Vec::len) < limit {
        if decoder.decode_next(&mut channels)?.is_none() {
            break;
        }
    }
    for channel in &mut channels {
        channel.truncate(limit);
    }
    let mut audio = AudioData { rate: info.rate, channels };
    if info.rate != rate {
        audio = bodo_kit_audio::resample(audio, rate)?;
    }
    let mut channels = audio.channels.into_iter();
    let left = channels.next().unwrap_or_default();
    let right = channels.next().unwrap_or_else(|| left.clone());
    Ok(PreviewSound::new(id, left, right, fade))
}
