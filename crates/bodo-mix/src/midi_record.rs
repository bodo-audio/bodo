//! MIDI recording (project.md sections 6.2 and 10.2).
//!
//! While the transport records, the audio thread captures every live note on and off with the timeline frame of the
//! block it arrived in, and marks where the take starts and ends. On the control side, [`place`] turns a finished
//! take into notes for the current pattern: in Pattern mode at pattern-local positions, wrapping into the loop; in
//! Song mode relative to the start of the bar where recording began. A note still held when recording stops ends
//! there.

use std::collections::HashMap;

use bodo_kit_core::{SampleRate, Seconds, Tick};
use bodo_model::pattern::PatternLen;
use bodo_model::{ChannelId, PatternId, Project};

use crate::keys;

/// How many captured events wait for the control side at most. A full queue drops new events and counts them.
pub const CAPTURE_CAPACITY: usize = 16_384;

/// What the audio thread captures while recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Capture {
    /// Recording began at this timeline frame, after any count-in.
    Start { frame: i64 },
    /// A live note at this timeline frame.
    Note(CapturedNote),
    /// Recording ended at this timeline frame.
    End { frame: i64 },
}

/// A live note on or off, captured while recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturedNote {
    /// The channel's event key.
    pub channel: u64,
    /// The MIDI key.
    pub key: u8,
    /// Velocity, 0 to 1.
    pub velocity: f32,
    /// Down or up.
    pub on: bool,
    /// The timeline frame of the block it arrived in.
    pub frame: i64,
}

/// A finished take, in timeline frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapturedTake {
    /// Where recording began.
    pub start: i64,
    /// Where recording ended.
    pub end: i64,
    /// The notes on and off, in the order they arrived.
    pub notes: Vec<CapturedNote>,
}

/// Where a take's notes go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TakeTarget {
    /// Pattern mode: into this pattern, wrapping into its loop.
    Pattern(PatternId),
    /// Song mode: into this pattern, relative to the start of the bar where recording began.
    Song(PatternId),
}

impl TakeTarget {
    /// The pattern the notes go into.
    pub fn pattern(self) -> PatternId {
        match self {
            Self::Pattern(p) | Self::Song(p) => p,
        }
    }
}

/// A recorded note, ready to add to a pattern.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedNote {
    /// The channel that played it.
    pub channel: ChannelId,
    /// The MIDI key.
    pub key: u8,
    /// Where it starts in the pattern.
    pub start: Tick,
    /// How long it lasts; at least one tick.
    pub len: Tick,
    /// Velocity, 0 to 1.
    pub velocity: f32,
}

/// How long pattern `id` loops: its fixed length, or its notes rounded up to whole bars and at least one bar. The
/// sequencer loops by the same rule.
///
/// Complexity: O(c log n) in the pattern's channels and notes.
fn pattern_length(project: &Project, id: PatternId) -> i64 {
    let bar = project.meter.signature_at(Tick::ZERO).bar_ticks().max(1);
    match project.patterns.get(&id).map(|p| (p.length, p.content_end())) {
        Some((PatternLen::Fixed(len), _)) => len.0.max(1),
        Some((PatternLen::Auto, end)) => (end.0.max(1) + bar - 1) / bar * bar,
        None => bar,
    }
}

/// The notes of `take`, recorded at `rate`, placed for `target` in `project`. Notes of channels that no longer exist
/// are left out. The result is sorted by channel, start and key.
///
/// Complexity: O(e log e) in captured events.
pub fn place(take: &CapturedTake, project: &Project, rate: SampleRate, target: TakeTarget) -> Vec<RecordedNote> {
    let hz = f64::from(rate.hz());
    let tick = |frame: i64| project.tempo.ticks_at(Seconds(frame as f64 / hz)).round() as i64;
    let channels: HashMap<u64, ChannelId> = project.channels.keys().map(|id| (keys::channel(*id), *id)).collect();
    let (origin, wrap) = match target {
        TakeTarget::Pattern(pattern) => (0, Some(pattern_length(project, pattern))),
        TakeTarget::Song(_) => {
            let at = Tick(tick(take.start));
            (project.meter.bar_start(project.meter.position(at).bar).0, None)
        }
    };
    let mut held: HashMap<(u64, u8), (i64, f32)> = HashMap::new();
    let mut spans: Vec<(u64, u8, i64, i64, f32)> = Vec::new();
    for note in &take.notes {
        let at = tick(note.frame.clamp(take.start, take.end.max(take.start)));
        let id = (note.channel, note.key);
        if let Some((start, velocity)) = held.remove(&id) {
            spans.push((note.channel, note.key, start, at, velocity));
        }
        if note.on {
            held.insert(id, (at, note.velocity));
        }
    }
    let end = tick(take.end.max(take.start));
    spans.extend(held.into_iter().map(|((channel, key), (start, velocity))| (channel, key, start, end, velocity)));
    let mut notes: Vec<RecordedNote> = spans
        .into_iter()
        .filter_map(|(channel, key, start, stop, velocity)| {
            let channel = *channels.get(&channel)?;
            let len = Tick((stop - start).max(1));
            let start = match wrap {
                Some(length) => (start - origin).rem_euclid(length.max(1)),
                None => (start - origin).max(0),
            };
            Some(RecordedNote { channel, key, start: Tick(start), len, velocity })
        })
        .collect();
    notes.sort_by_key(|n| (n.channel, n.start, n.key));
    notes
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bodo_model::channel::{ChannelKind, SamplerParams};
    use bodo_model::{Channel, Color, Pattern};

    use super::*;

    const RATE: SampleRate = SampleRate::HZ_48000;
    /// Frames per tick at 120 BPM and 48 kHz: half a second per beat, 960 ticks per beat.
    const FRAMES_PER_TICK: f64 = 24_000.0 / 960.0;

    fn frame(tick: i64) -> i64 {
        (tick as f64 * FRAMES_PER_TICK) as i64
    }

    fn project() -> (Project, ChannelId, PatternId) {
        let mut project = Project::new(120.0).unwrap();
        let channel = ChannelId::from_u128(7);
        project.channels.insert(
            channel,
            Arc::new(Channel {
                id: channel,
                name: String::from("Keys"),
                color: Color::GRAY,
                kind: ChannelKind::Sampler(Arc::new(SamplerParams::default())),
                target: None,
                volume: 1.0,
                pan: 0.0,
                pitch_cents: 0.0,
                swing: 0.0,
                muted: false,
                midi_in: None,
            }),
        );
        project.rack_order.push_back(channel);
        let pattern = PatternId::from_u128(9);
        project.patterns.insert(pattern, Arc::new(Pattern::new(pattern, "P", Color::GRAY)));
        (project, channel, pattern)
    }

    fn note(channel: ChannelId, key: u8, on: bool, tick: i64) -> CapturedNote {
        CapturedNote { channel: keys::channel(channel), key, velocity: if on { 0.5 } else { 0.0 }, on, frame: frame(tick) }
    }

    #[test]
    fn pattern_mode_wraps_notes_into_the_loop() {
        let (project, channel, pattern) = project();
        // An empty pattern loops one bar (3840 ticks). Recording ran from tick 0 into the second loop.
        let take = CapturedTake {
            start: 0,
            end: frame(7680),
            notes: vec![
                note(channel, 60, true, 960),
                note(channel, 60, false, 1440),
                note(channel, 62, true, 3840 + 480),
                note(channel, 62, false, 3840 + 960),
            ],
        };
        let notes = place(&take, &project, RATE, TakeTarget::Pattern(pattern));
        let shape: Vec<(u8, i64, i64)> = notes.iter().map(|n| (n.key, n.start.0, n.len.0)).collect();
        assert_eq!(shape, [(62, 480, 480), (60, 960, 480)]);
        assert!(notes.iter().all(|n| n.channel == channel && (n.velocity - 0.5).abs() < 1e-6));
    }

    #[test]
    fn song_mode_starts_at_the_bar_recording_began_in_and_held_notes_end_at_the_stop() {
        let (project, channel, pattern) = project();
        // Recording began in bar 3 (from tick 7680), part-way through it.
        let take = CapturedTake {
            start: frame(7680 + 1000),
            end: frame(7680 + 3000),
            notes: vec![note(channel, 64, true, 7680 + 1920), note(channel, 67, true, 7680 + 2400)],
        };
        let notes = place(&take, &project, RATE, TakeTarget::Song(pattern));
        let shape: Vec<(u8, i64, i64)> = notes.iter().map(|n| (n.key, n.start.0, n.len.0)).collect();
        assert_eq!(shape, [(64, 1920, 1080), (67, 2400, 600)]);
    }

    #[test]
    fn a_repeated_note_on_closes_the_held_one_and_unknown_channels_are_left_out() {
        let (project, channel, pattern) = project();
        let stranger = ChannelId::from_u128(99);
        let take = CapturedTake {
            start: 0,
            end: frame(3840),
            notes: vec![note(channel, 60, true, 0), note(channel, 60, true, 480), note(channel, 60, false, 720), note(stranger, 61, true, 10)],
        };
        let notes = place(&take, &project, RATE, TakeTarget::Pattern(pattern));
        let shape: Vec<(i64, i64)> = notes.iter().map(|n| (n.start.0, n.len.0)).collect();
        assert_eq!(shape, [(0, 480), (480, 240)]);
    }
}
