//! Reading and writing Standard MIDI Files (SMF), formats 0, 1 and 2.
//!
//! [`Smf::parse`] reads a file into owned, absolute-time tracks, and [`Smf::to_bytes`] writes one back. Event times
//! are absolute ticks in the file's own resolution ([`Timing`]), not Bodo ticks.
//!
//! The End of Track meta event is implicit: parsing stores its time in [`Track::end`] and drops any events after
//! it, and writing appends it at `end` or at the last event, whichever is later.
//!
//! Parsing is lenient, as MIDI files in the wild often are slightly broken: a track that fails to parse midway keeps
//! the events read so far, and unknown chunks are skipped. Writing is strict and rejects anything that would not read
//! back the same way.

use crate::message::{MidiMessage, U4, U7, U14};
use midly::num::{u4, u7, u14, u15, u24, u28};

/// How the tracks of a file relate to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// Format 0: exactly one track.
    SingleTrack,
    /// Format 1: tracks that play together. The first one usually carries the tempo map.
    Parallel,
    /// Format 2: independent tracks that play one after another, such as separate patterns.
    Sequential,
}

/// Frame rates for SMPTE timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SmpteFps {
    /// 24 frames per second.
    Fps24,
    /// 25 frames per second.
    Fps25,
    /// 30 drop-frame, which runs at 29.97 frames per second.
    Fps30Drop,
    /// 30 frames per second.
    Fps30,
}

/// What a tick means in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Timing {
    /// Ticks are fractions of a quarter note; their length in seconds follows the tempo events.
    Metrical {
        /// Ticks per quarter note, 1 to 32767.
        ticks_per_quarter: u16,
    },
    /// Ticks are fractions of an SMPTE frame, independent of tempo.
    Timecode {
        /// The frame rate.
        fps: SmpteFps,
        /// Ticks per frame, at least 1.
        ticks_per_frame: u8,
    },
}

/// An SMPTE time of day. Valid when `hour < 24`, `minute < 60`, `second < 60`, `frame` is below the frame rate and
/// `subframe < 100`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SmpteTime {
    /// Hours, 0 to 23.
    pub hour: u8,
    /// Minutes, 0 to 59.
    pub minute: u8,
    /// Seconds, 0 to 59.
    pub second: u8,
    /// Frames, below the frame rate.
    pub frame: u8,
    /// Hundredths of a frame, 0 to 99.
    pub subframe: u8,
    /// The frame rate.
    pub fps: SmpteFps,
}

/// A meta event: information about the track rather than sound.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MetaEvent {
    /// The sequence number, or `None` to use the track's position.
    SequenceNumber(Option<u16>),
    /// Free text. Text events are bytes; files rarely say which encoding they use.
    Text(Vec<u8>),
    /// A copyright notice.
    Copyright(Vec<u8>),
    /// The track or sequence name.
    TrackName(Vec<u8>),
    /// The instrument name.
    InstrumentName(Vec<u8>),
    /// A lyric syllable.
    Lyric(Vec<u8>),
    /// A marker, such as a section name.
    Marker(Vec<u8>),
    /// A cue point.
    CuePoint(Vec<u8>),
    /// The program name.
    ProgramName(Vec<u8>),
    /// The device name.
    DeviceName(Vec<u8>),
    /// The MIDI channel that following meta and sysex events refer to.
    ChannelPrefix(U4),
    /// The MIDI port the track plays on.
    Port(U7),
    /// Tempo in microseconds per quarter note, at most 2^24 - 1.
    Tempo(u32),
    /// The SMPTE time at which the track starts.
    SmpteOffset(SmpteTime),
    /// A time signature.
    TimeSignature {
        /// Beats per bar.
        numerator: u8,
        /// The beat unit as a power of two: 2 means quarter notes, 3 eighth notes.
        denominator_pow2: u8,
        /// MIDI clocks per metronome click.
        clocks_per_click: u8,
        /// Notated 32nd notes per MIDI quarter note, normally 8.
        thirty_seconds_per_quarter: u8,
    },
    /// A key signature.
    KeySignature {
        /// Sharps when positive, flats when negative.
        sharps: i8,
        /// Minor rather than major.
        minor: bool,
    },
    /// Data for a specific sequencer.
    SequencerSpecific(Vec<u8>),
    /// A meta event this reader doesn't know, or a known one with malformed data.
    Unknown {
        /// The meta event type byte.
        kind: u8,
        /// The payload.
        data: Vec<u8>,
    },
}

/// What happens at a point in a track.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A channel voice message.
    Channel {
        /// The MIDI channel, 0 to 15.
        channel: U4,
        /// The message.
        message: MidiMessage,
    },
    /// A system exclusive message, without the leading `0xF0`. Complete messages end with `0xF7`.
    SysEx(Vec<u8>),
    /// An escape: raw bytes sent as they are, or the continuation of a split sysex message.
    Escape(Vec<u8>),
    /// A meta event.
    Meta(MetaEvent),
}

/// An event at an absolute tick.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrackEvent {
    /// Ticks since the start of the track, in the file's resolution.
    pub tick: u64,
    /// The event.
    pub kind: EventKind,
}

/// One track: events sorted by tick, and the time at which the track ends.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Track {
    /// The events, in non-decreasing tick order. Events at the same tick keep their order.
    pub events: Vec<TrackEvent>,
    /// The tick of the End of Track event. Writing uses the later of this and the last event's tick.
    pub end: u64,
}

/// A Standard MIDI File.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Smf {
    /// How the tracks relate.
    pub format: Format,
    /// What a tick means.
    pub timing: Timing,
    /// The tracks.
    pub tracks: Vec<Track>,
}

/// Why a file could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SmfError {
    /// The bytes are not a readable MIDI file.
    #[error("not a readable MIDI file: {0}")]
    Parse(String),
    /// The header's timing has zero resolution, or more ticks per quarter than 32767.
    #[error("invalid timing {0:?}")]
    InvalidTiming(Timing),
    /// A format 0 file must have exactly one track.
    #[error("a single-track MIDI file must have exactly one track, not {0}")]
    SingleTrackCount(usize),
    /// An event can't be written.
    #[error("track {track}, event {index}: {reason}")]
    InvalidEvent {
        /// Index of the track.
        track: usize,
        /// Index of the event in the track. The End of Track event has index `events.len()`.
        index: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// The encoder rejected the file, for example because it has more than 65535 tracks.
    #[error("could not encode the MIDI file: {0}")]
    Encode(&'static str),
}

/// Largest gap between consecutive events that SMF can store.
const MAX_DELTA: u64 = 0x0FFF_FFFF;
/// Largest payload of a sysex, escape or meta event.
const MAX_DATA_LEN: usize = 0x0FFF_FFFF;
/// The End of Track meta event type, which may not be written as an unknown event.
const END_OF_TRACK: u8 = 0x2F;

impl Smf {
    /// Reads a Standard MIDI File.
    ///
    /// Complexity: O(n) in the size of the file.
    pub fn parse(bytes: &[u8]) -> Result<Self, SmfError> {
        check_header(bytes)?;
        // MIDI files come from anywhere, and the reader is a third-party crate: a crash inside it on a damaged file
        // must become an error, never take Bodo down.
        let smf = std::panic::catch_unwind(|| midly::Smf::parse(bytes))
            .map_err(|_| SmfError::Parse(String::from("the MIDI reader failed on this file")))?
            .map_err(|err| SmfError::Parse(err.to_string()))?;
        let format = match smf.header.format {
            midly::Format::SingleTrack => Format::SingleTrack,
            midly::Format::Parallel => Format::Parallel,
            midly::Format::Sequential => Format::Sequential,
        };
        let timing = match smf.header.timing {
            midly::Timing::Metrical(ticks) => Timing::Metrical { ticks_per_quarter: ticks.as_int() },
            midly::Timing::Timecode(fps, ticks_per_frame) => Timing::Timecode { fps: fps_from_midly(fps), ticks_per_frame },
        };
        if !timing_is_valid(timing) {
            return Err(SmfError::InvalidTiming(timing));
        }
        let tracks = smf.tracks.iter().map(|events| track_from_midly(events)).collect();
        Ok(Self { format, timing, tracks })
    }

    /// Writes the file.
    ///
    /// Complexity: O(n) in the number of events plus the size of their payloads.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SmfError> {
        if !timing_is_valid(self.timing) {
            return Err(SmfError::InvalidTiming(self.timing));
        }
        if self.format == Format::SingleTrack && self.tracks.len() != 1 {
            return Err(SmfError::SingleTrackCount(self.tracks.len()));
        }
        let format = match self.format {
            Format::SingleTrack => midly::Format::SingleTrack,
            Format::Parallel => midly::Format::Parallel,
            Format::Sequential => midly::Format::Sequential,
        };
        let timing = match self.timing {
            Timing::Metrical { ticks_per_quarter } => midly::Timing::Metrical(u15::new(ticks_per_quarter)),
            Timing::Timecode { fps, ticks_per_frame } => midly::Timing::Timecode(fps_to_midly(fps), ticks_per_frame),
        };
        let tracks = self.tracks.iter().enumerate().map(|(index, track)| track_to_midly(index, track)).collect::<Result<Vec<_>, _>>()?;
        let smf = midly::Smf { header: midly::Header::new(format, timing), tracks };
        let mut bytes = Vec::new();
        smf.write(&mut bytes).map_err(SmfError::Encode)?;
        Ok(bytes)
    }
}

/// Rejects headers the reader can't handle safely. An SMPTE division whose frame-rate byte is -128 overflows the
/// negation midly 0.5.3 applies to it.
fn check_header(bytes: &[u8]) -> Result<(), SmfError> {
    if let (Some(b"MThd"), Some(&[high, _])) = (bytes.get(0..4), bytes.get(12..14))
        && high == 0x80
    {
        return Err(SmfError::Parse(String::from("invalid SMPTE frame rate")));
    }
    Ok(())
}

fn timing_is_valid(timing: Timing) -> bool {
    match timing {
        Timing::Metrical { ticks_per_quarter } => (1..=0x7FFF).contains(&ticks_per_quarter),
        Timing::Timecode { ticks_per_frame, .. } => ticks_per_frame > 0,
    }
}

fn fps_from_midly(fps: midly::Fps) -> SmpteFps {
    match fps {
        midly::Fps::Fps24 => SmpteFps::Fps24,
        midly::Fps::Fps25 => SmpteFps::Fps25,
        midly::Fps::Fps29 => SmpteFps::Fps30Drop,
        midly::Fps::Fps30 => SmpteFps::Fps30,
    }
}

fn fps_to_midly(fps: SmpteFps) -> midly::Fps {
    match fps {
        SmpteFps::Fps24 => midly::Fps::Fps24,
        SmpteFps::Fps25 => midly::Fps::Fps25,
        SmpteFps::Fps30Drop => midly::Fps::Fps29,
        SmpteFps::Fps30 => midly::Fps::Fps30,
    }
}

fn track_from_midly(events: &[midly::TrackEvent<'_>]) -> Track {
    let mut track = Track { events: Vec::with_capacity(events.len()), end: 0 };
    let mut tick = 0u64;
    for event in events {
        tick = tick.saturating_add(u64::from(event.delta.as_int()));
        let kind = match event.kind {
            midly::TrackEventKind::Midi { channel, message } => {
                EventKind::Channel { channel: U4::from_masked(channel.as_int()), message: message_from_midly(message) }
            }
            midly::TrackEventKind::SysEx(data) => EventKind::SysEx(data.to_vec()),
            midly::TrackEventKind::Escape(data) => EventKind::Escape(data.to_vec()),
            midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack) => break,
            midly::TrackEventKind::Meta(meta) => EventKind::Meta(meta_from_midly(meta)),
        };
        track.events.push(TrackEvent { tick, kind });
    }
    track.end = tick;
    track
}

fn track_to_midly(track_index: usize, track: &Track) -> Result<Vec<midly::TrackEvent<'_>>, SmfError> {
    let invalid = |index, reason| SmfError::InvalidEvent { track: track_index, index, reason };
    let mut events = Vec::with_capacity(track.events.len() + 1);
    let mut previous = 0u64;
    for (index, event) in track.events.iter().enumerate() {
        let delta = event.tick.checked_sub(previous).ok_or(invalid(index, "events are not in tick order"))?;
        if delta > MAX_DELTA {
            return Err(invalid(index, "the gap to the previous event is too large for SMF"));
        }
        previous = event.tick;
        let kind = match &event.kind {
            EventKind::Channel { channel, message } => {
                midly::TrackEventKind::Midi { channel: u4::new(channel.get()), message: message_to_midly(*message) }
            }
            EventKind::SysEx(data) => midly::TrackEventKind::SysEx(checked_data(data).ok_or(invalid(index, "sysex data is too long"))?),
            EventKind::Escape(data) => midly::TrackEventKind::Escape(checked_data(data).ok_or(invalid(index, "escape data is too long"))?),
            EventKind::Meta(meta) => midly::TrackEventKind::Meta(meta_to_midly(meta).map_err(|reason| invalid(index, reason))?),
        };
        events.push(midly::TrackEvent { delta: u28::new(delta as u32), kind });
    }
    let end_delta = track.end.saturating_sub(previous);
    if end_delta > MAX_DELTA {
        return Err(invalid(track.events.len(), "the end of the track is too far after the last event for SMF"));
    }
    events.push(midly::TrackEvent { delta: u28::new(end_delta as u32), kind: midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack) });
    Ok(events)
}

fn checked_data(data: &[u8]) -> Option<&[u8]> {
    (data.len() <= MAX_DATA_LEN).then_some(data)
}

fn message_from_midly(message: midly::MidiMessage) -> MidiMessage {
    let u7 = |value: u7| U7::from_masked(value.as_int());
    match message {
        midly::MidiMessage::NoteOff { key, vel } => MidiMessage::NoteOff { key: u7(key), velocity: u7(vel) },
        midly::MidiMessage::NoteOn { key, vel } => MidiMessage::NoteOn { key: u7(key), velocity: u7(vel) },
        midly::MidiMessage::Aftertouch { key, vel } => MidiMessage::PolyPressure { key: u7(key), pressure: u7(vel) },
        midly::MidiMessage::Controller { controller, value } => MidiMessage::ControlChange { controller: u7(controller), value: u7(value) },
        midly::MidiMessage::ProgramChange { program } => MidiMessage::ProgramChange { program: u7(program) },
        midly::MidiMessage::ChannelAftertouch { vel } => MidiMessage::ChannelPressure { pressure: u7(vel) },
        midly::MidiMessage::PitchBend { bend } => MidiMessage::PitchBend { value: U14::from_masked(bend.0.as_int()) },
    }
}

fn message_to_midly(message: MidiMessage) -> midly::MidiMessage {
    let u7 = |value: U7| u7::new(value.get());
    match message {
        MidiMessage::NoteOff { key, velocity } => midly::MidiMessage::NoteOff { key: u7(key), vel: u7(velocity) },
        MidiMessage::NoteOn { key, velocity } => midly::MidiMessage::NoteOn { key: u7(key), vel: u7(velocity) },
        MidiMessage::PolyPressure { key, pressure } => midly::MidiMessage::Aftertouch { key: u7(key), vel: u7(pressure) },
        MidiMessage::ControlChange { controller, value } => midly::MidiMessage::Controller { controller: u7(controller), value: u7(value) },
        MidiMessage::ProgramChange { program } => midly::MidiMessage::ProgramChange { program: u7(program) },
        MidiMessage::ChannelPressure { pressure } => midly::MidiMessage::ChannelAftertouch { vel: u7(pressure) },
        MidiMessage::PitchBend { value } => midly::MidiMessage::PitchBend { bend: midly::PitchBend(u14::new(value.get())) },
    }
}

fn meta_from_midly(meta: midly::MetaMessage<'_>) -> MetaEvent {
    match meta {
        midly::MetaMessage::TrackNumber(number) => MetaEvent::SequenceNumber(number),
        midly::MetaMessage::Text(text) => MetaEvent::Text(text.to_vec()),
        midly::MetaMessage::Copyright(text) => MetaEvent::Copyright(text.to_vec()),
        midly::MetaMessage::TrackName(text) => MetaEvent::TrackName(text.to_vec()),
        midly::MetaMessage::InstrumentName(text) => MetaEvent::InstrumentName(text.to_vec()),
        midly::MetaMessage::Lyric(text) => MetaEvent::Lyric(text.to_vec()),
        midly::MetaMessage::Marker(text) => MetaEvent::Marker(text.to_vec()),
        midly::MetaMessage::CuePoint(text) => MetaEvent::CuePoint(text.to_vec()),
        midly::MetaMessage::ProgramName(text) => MetaEvent::ProgramName(text.to_vec()),
        midly::MetaMessage::DeviceName(text) => MetaEvent::DeviceName(text.to_vec()),
        midly::MetaMessage::MidiChannel(channel) => MetaEvent::ChannelPrefix(U4::from_masked(channel.as_int())),
        midly::MetaMessage::MidiPort(port) => MetaEvent::Port(U7::from_masked(port.as_int())),
        // Handled by the caller, which ends the track there. Kept as an unknown event so this match stays total.
        midly::MetaMessage::EndOfTrack => MetaEvent::Unknown { kind: END_OF_TRACK, data: Vec::new() },
        midly::MetaMessage::Tempo(micros) => MetaEvent::Tempo(micros.as_int()),
        midly::MetaMessage::SmpteOffset(time) => MetaEvent::SmpteOffset(SmpteTime {
            hour: time.hour(),
            minute: time.minute(),
            second: time.second(),
            frame: time.frame(),
            subframe: time.subframe(),
            fps: fps_from_midly(time.fps()),
        }),
        midly::MetaMessage::TimeSignature(numerator, denominator_pow2, clocks_per_click, thirty_seconds_per_quarter) => {
            MetaEvent::TimeSignature { numerator, denominator_pow2, clocks_per_click, thirty_seconds_per_quarter }
        }
        midly::MetaMessage::KeySignature(sharps, minor) => MetaEvent::KeySignature { sharps, minor },
        midly::MetaMessage::SequencerSpecific(data) => MetaEvent::SequencerSpecific(data.to_vec()),
        midly::MetaMessage::Unknown(kind, data) => MetaEvent::Unknown { kind, data: data.to_vec() },
    }
}

fn meta_data(data: &[u8]) -> Result<&[u8], &'static str> {
    checked_data(data).ok_or("meta event data is too long")
}

fn meta_to_midly(meta: &MetaEvent) -> Result<midly::MetaMessage<'_>, &'static str> {
    Ok(match meta {
        MetaEvent::SequenceNumber(number) => midly::MetaMessage::TrackNumber(*number),
        MetaEvent::Text(data) => midly::MetaMessage::Text(meta_data(data)?),
        MetaEvent::Copyright(data) => midly::MetaMessage::Copyright(meta_data(data)?),
        MetaEvent::TrackName(data) => midly::MetaMessage::TrackName(meta_data(data)?),
        MetaEvent::InstrumentName(data) => midly::MetaMessage::InstrumentName(meta_data(data)?),
        MetaEvent::Lyric(data) => midly::MetaMessage::Lyric(meta_data(data)?),
        MetaEvent::Marker(data) => midly::MetaMessage::Marker(meta_data(data)?),
        MetaEvent::CuePoint(data) => midly::MetaMessage::CuePoint(meta_data(data)?),
        MetaEvent::ProgramName(data) => midly::MetaMessage::ProgramName(meta_data(data)?),
        MetaEvent::DeviceName(data) => midly::MetaMessage::DeviceName(meta_data(data)?),
        MetaEvent::ChannelPrefix(channel) => midly::MetaMessage::MidiChannel(u4::new(channel.get())),
        MetaEvent::Port(port) => midly::MetaMessage::MidiPort(u7::new(port.get())),
        MetaEvent::Tempo(micros) => midly::MetaMessage::Tempo(u24::try_from(*micros).ok_or("tempo does not fit in 24 bits")?),
        MetaEvent::SmpteOffset(time) => midly::MetaMessage::SmpteOffset(
            midly::SmpteTime::new(time.hour, time.minute, time.second, time.frame, time.subframe, fps_to_midly(time.fps))
                .ok_or("SMPTE offset is not a valid time")?,
        ),
        MetaEvent::TimeSignature { numerator, denominator_pow2, clocks_per_click, thirty_seconds_per_quarter } => {
            midly::MetaMessage::TimeSignature(*numerator, *denominator_pow2, *clocks_per_click, *thirty_seconds_per_quarter)
        }
        MetaEvent::KeySignature { sharps, minor } => midly::MetaMessage::KeySignature(*sharps, *minor),
        MetaEvent::SequencerSpecific(data) => midly::MetaMessage::SequencerSpecific(meta_data(data)?),
        MetaEvent::Unknown { kind: END_OF_TRACK, .. } => return Err("End of Track is implicit; set Track::end instead"),
        MetaEvent::Unknown { kind, data } => midly::MetaMessage::Unknown(*kind, meta_data(data)?),
    })
}
