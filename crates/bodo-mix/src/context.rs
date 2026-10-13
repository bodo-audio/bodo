//! The per-block context every graph node can read: rate, transport, the device's input, the note events of the
//! block, and the audio clip segments it plays.

use bodo_kit_core::{PPQ, SampleRate, Seconds, TempoMap, Tick, rt_safe};
use bodo_kit_dsp::AudioBuffer;

use crate::automation::ParamValues;
use crate::segments::Segments;

/// The most note events one block can carry. More are dropped and counted.
pub const MAX_EVENTS: usize = 8192;

/// The most device input channels a block carries; inserts can take input from any of them.
pub const MAX_INPUTS: usize = 32;

/// What a note event does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    /// Releases every voice of the channel, as when the transport stops or jumps.
    AllOff,
    /// Releases one voice.
    Off,
    /// Starts a voice.
    On,
}

/// A note event for one channel, at a frame of the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteEvent {
    /// The frame within the block.
    pub frame: u32,
    /// What it does.
    pub kind: EventKind,
    /// The MIDI key.
    pub key: u8,
    /// Velocity, 0 to 1.
    pub velocity: f32,
    /// Which voice it is: the note's id plus the clip it was played from, so the same note playing from two clips
    /// at once makes two voices.
    pub voice: u64,
}

impl NoteEvent {
    /// The order events are applied in: by frame, offs before ons, then by key and voice.
    fn order(&self) -> (u32, EventKind, u8, u64) {
        (self.frame, self.kind, self.key, self.voice)
    }
}

/// The context of one block.
#[derive(Debug)]
pub struct BlockContext {
    /// The sample rate.
    pub rate: SampleRate,
    /// Frames in the block.
    pub frames: usize,
    /// Whether the transport is playing.
    pub playing: bool,
    /// The transport's frame at the start of the block.
    pub frame: i64,
    /// The song position at the start of the block, in ticks.
    pub tick: f64,
    /// How many ticks a frame lasts at the tempo there; zero until the engine sets a position.
    pub ticks_per_frame: f64,
    /// The device's input for the block, [`MAX_INPUTS`] channels; `input_channels` of them are real.
    input: AudioBuffer,
    /// How many of `input`'s channels the device filled this block.
    input_channels: usize,
    /// Note events of the block, sorted by channel key and then applied order. Capacity [`MAX_EVENTS`].
    events: Vec<(u64, NoteEvent)>,
    /// One entry per channel with events: its key and its range in `events`. Sorted by key.
    directory: Vec<(u64, u32, u32)>,
    /// Events dropped because the block was full, since the engine started.
    pub dropped_events: u64,
    /// The audio clip segments of the block.
    pub audio: Segments,
    /// The values automation set, which processors read in place of their live settings. Unlike the events they
    /// persist from block to block.
    pub params: ParamValues,
}

impl BlockContext {
    /// An empty context with room for [`MAX_EVENTS`] events.
    ///
    /// Allocates.
    pub fn new(rate: SampleRate) -> Self {
        Self {
            rate,
            frames: 0,
            playing: false,
            frame: 0,
            tick: 0.0,
            ticks_per_frame: 0.0,
            input: AudioBuffer::for_blocks(MAX_INPUTS),
            input_channels: 0,
            events: Vec::with_capacity(MAX_EVENTS),
            directory: Vec::with_capacity(MAX_EVENTS),
            dropped_events: 0,
            audio: Segments::new(),
            params: ParamValues::new(),
        }
    }

    /// Copies `frames` frames of `source` from frame `offset` as the block's input, or makes the input silent.
    ///
    /// RT-safe. Complexity: O(channels * frames).
    #[rt_safe]
    pub fn set_input(&mut self, source: Option<&AudioBuffer>, offset: usize, frames: usize) {
        self.input.set_frames(frames);
        self.input_channels = source.map_or(0, |s| s.channels().min(MAX_INPUTS));
        for (index, channel) in self.input.channels_mut().enumerate().take(self.input_channels) {
            let from = source.map(|s| s.channel(index)).unwrap_or(&[]);
            let available = from.len().saturating_sub(offset).min(channel.len());
            channel[..available].copy_from_slice(&from[offset..offset + available]);
            channel[available..].fill(0.0);
        }
    }

    /// Sets the song position of the block to the transport's `frame` on `tempo`: its tick and the ticks per frame.
    ///
    /// RT-safe. Complexity: O(log m) in tempo points.
    #[rt_safe]
    pub fn set_position(&mut self, tempo: &TempoMap, frame: i64) {
        let hz = f64::from(self.rate.hz());
        let tick = tempo.ticks_at(Seconds(frame as f64 / hz));
        let bpm = tempo.bpm_at(Tick(tick.floor() as i64));
        self.tick = tick;
        self.ticks_per_frame = bpm / 60.0 * PPQ as f64 / hz;
    }

    /// Device input channel `index` for the block, or an empty slice when the device has no such input.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn input(&self, index: usize) -> &[f32] {
        if index < self.input_channels { self.input.channel(index) } else { &[] }
    }

    /// Clears the events for a new block.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn clear(&mut self) {
        self.events.clear();
        self.directory.clear();
        self.audio.clear();
    }

    /// Adds an event for the channel with `key`. When the block is full the event is dropped and counted.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn push(&mut self, key: u64, event: NoteEvent) {
        if self.events.len() < MAX_EVENTS {
            self.events.push((key, event));
        } else {
            self.dropped_events += 1;
        }
    }

    /// Sorts the events and builds the directory. Call once after the last push of a block.
    ///
    /// RT-safe. Complexity: O(k log k) in the number of events.
    #[rt_safe]
    pub fn finish(&mut self) {
        self.events.sort_unstable_by_key(|(key, event)| (*key, event.order()));
        let mut start = 0;
        while start < self.events.len() {
            let key = self.events[start].0;
            let end = start + self.events[start..].iter().take_while(|(k, _)| *k == key).count();
            self.directory.push((key, start as u32, end as u32));
            start = end;
        }
        self.audio.finish();
    }

    /// The events of the channel with `key`, in the order to apply them.
    ///
    /// RT-safe. Complexity: O(log c) in the channels with events this block.
    #[rt_safe]
    pub fn events_for(&self, key: u64) -> impl Iterator<Item = &NoteEvent> {
        let range = match self.directory.binary_search_by_key(&key, |&(k, _, _)| k) {
            Ok(index) => self.directory.get(index).map_or(0..0, |&(_, start, end)| start as usize..end as usize),
            Err(_) => 0..0,
        };
        self.events.get(range).unwrap_or(&[]).iter().map(|(_, event)| event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(frame: u32, kind: EventKind, key: u8) -> NoteEvent {
        NoteEvent { frame, kind, key, velocity: 1.0, voice: u64::from(key) }
    }

    #[test]
    fn events_are_grouped_by_channel_and_ordered() {
        let mut context = BlockContext::new(SampleRate::HZ_48000);
        context.push(7, event(10, EventKind::On, 60));
        context.push(3, event(5, EventKind::On, 62));
        context.push(7, event(10, EventKind::Off, 61));
        context.push(7, event(2, EventKind::On, 64));
        context.finish();
        let seven: Vec<(u32, EventKind)> = context.events_for(7).map(|e| (e.frame, e.kind)).collect();
        assert_eq!(seven, [(2, EventKind::On), (10, EventKind::Off), (10, EventKind::On)]);
        assert_eq!(context.events_for(3).count(), 1);
        assert_eq!(context.events_for(99).count(), 0);
    }

    #[test]
    fn a_full_block_drops_and_counts() {
        let mut context = BlockContext::new(SampleRate::HZ_48000);
        for i in 0..MAX_EVENTS + 5 {
            context.push(1, event(i as u32, EventKind::On, 60));
        }
        assert_eq!(context.dropped_events, 5);
    }
}
