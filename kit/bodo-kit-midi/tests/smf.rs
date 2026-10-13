//! Standard MIDI File reading and writing.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use bodo_kit_midi::smf::{EventKind, Format, MetaEvent, SmpteFps, SmpteTime, Timing, Track, TrackEvent};
use bodo_kit_midi::{MidiMessage, Smf, SmfError, U4, U7, U14};
use proptest::prelude::*;

fn u7(value: u8) -> U7 {
    U7::new(value).unwrap()
}

fn channel(channel: u8, message: MidiMessage) -> EventKind {
    EventKind::Channel { channel: U4::new(channel).unwrap(), message }
}

fn at(tick: u64, kind: EventKind) -> TrackEvent {
    TrackEvent { tick, kind }
}

/// A format 1 file at 480 PPQ: a tempo track, and a note track that uses running status and has a stray event
/// after its End of Track.
const FILE: &[u8] = &[
    b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 1, 0, 2, 0x01, 0xE0, //
    b'M', b'T', b'r', b'k', 0, 0, 0, 0x14, //
    0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tempo 500000 us per quarter
    0x00, 0xFF, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08, // 4/4
    0x83, 0x60, 0xFF, 0x2F, 0x00, // End of Track at 480
    b'M', b'T', b'r', b'k', 0, 0, 0, 0x2A, //
    0x00, 0xFF, 0x03, 0x05, b'P', b'i', b'a', b'n', b'o', // track name
    0x00, 0x90, 0x3C, 0x64, // note on C4
    0x60, 0x40, 0x64, // running status: note on E4 at 96
    0x82, 0x20, 0x80, 0x3C, 0x00, // note off C4 at 384
    0x00, 0x40, 0x00, // running status: note off E4
    0x00, 0xE1, 0x00, 0x40, // pitch bend center on channel 2
    0x00, 0xF0, 0x03, 0x7E, 0x7F, 0xF7, // sysex
    0x00, 0xFF, 0x2F, 0x00, // End of Track at 384
    0x00, 0x90, 0x3C, 0x64, // after End of Track: ignored
];

fn expected_file() -> Smf {
    Smf {
        format: Format::Parallel,
        timing: Timing::Metrical { ticks_per_quarter: 480 },
        tracks: vec![
            Track {
                events: vec![
                    at(0, EventKind::Meta(MetaEvent::Tempo(500_000))),
                    at(
                        0,
                        EventKind::Meta(MetaEvent::TimeSignature {
                            numerator: 4,
                            denominator_pow2: 2,
                            clocks_per_click: 24,
                            thirty_seconds_per_quarter: 8,
                        }),
                    ),
                ],
                end: 480,
            },
            Track {
                events: vec![
                    at(0, EventKind::Meta(MetaEvent::TrackName(b"Piano".to_vec()))),
                    at(0, channel(0, MidiMessage::NoteOn { key: u7(60), velocity: u7(100) })),
                    at(96, channel(0, MidiMessage::NoteOn { key: u7(64), velocity: u7(100) })),
                    at(384, channel(0, MidiMessage::NoteOff { key: u7(60), velocity: u7(0) })),
                    at(384, channel(0, MidiMessage::NoteOff { key: u7(64), velocity: u7(0) })),
                    at(384, channel(1, MidiMessage::PitchBend { value: U14::CENTER })),
                    at(384, EventKind::SysEx(vec![0x7E, 0x7F, 0xF7])),
                ],
                end: 384,
            },
        ],
    }
}

#[test]
fn parses_a_known_file() {
    assert_eq!(Smf::parse(FILE).unwrap(), expected_file());
}

#[test]
fn writes_the_known_file_back_byte_for_byte() {
    // Everything up to the second End of Track, with the second track's length shortened by the dropped event.
    let mut expected = FILE[..FILE.len() - 4].to_vec();
    // The header (14 bytes) and the first track (8 + 0x14 bytes) come first; the length's low byte is 7 bytes into
    // the second track's chunk header.
    expected[14 + 8 + 0x14 + 7] = 0x26;
    assert_eq!(expected_file().to_bytes().unwrap(), expected);
}

#[test]
fn end_of_track_never_precedes_the_last_event() {
    let smf = Smf {
        format: Format::SingleTrack,
        timing: Timing::Metrical { ticks_per_quarter: 96 },
        tracks: vec![Track { events: vec![at(500, EventKind::Meta(MetaEvent::Marker(b"A".to_vec())))], end: 10 }],
    };
    assert_eq!(Smf::parse(&smf.to_bytes().unwrap()).unwrap().tracks[0].end, 500);
}

#[test]
fn rejects_files_that_would_not_read_back() {
    let one_track = |events: Vec<TrackEvent>| Smf {
        format: Format::SingleTrack,
        timing: Timing::Metrical { ticks_per_quarter: 960 },
        tracks: vec![Track { events, end: 0 }],
    };
    let invalid = |smf: Smf| match smf.to_bytes() {
        Err(SmfError::InvalidEvent { reason, .. }) => reason,
        other => panic!("expected an invalid event, got {other:?}"),
    };

    let marker = || EventKind::Meta(MetaEvent::Marker(Vec::new()));
    assert_eq!(invalid(one_track(vec![at(10, marker()), at(5, marker())])), "events are not in tick order");
    assert_eq!(invalid(one_track(vec![at(0x1000_0000, marker())])), "the gap to the previous event is too large for SMF");
    assert_eq!(invalid(one_track(vec![at(0, EventKind::Meta(MetaEvent::Tempo(1 << 24)))])), "tempo does not fit in 24 bits");
    let bad_smpte = SmpteTime { hour: 24, minute: 0, second: 0, frame: 0, subframe: 0, fps: SmpteFps::Fps25 };
    assert_eq!(invalid(one_track(vec![at(0, EventKind::Meta(MetaEvent::SmpteOffset(bad_smpte)))])), "SMPTE offset is not a valid time");
    let end_of_track = EventKind::Meta(MetaEvent::Unknown { kind: 0x2F, data: Vec::new() });
    assert_eq!(invalid(one_track(vec![at(0, end_of_track)])), "End of Track is implicit; set Track::end instead");

    let two_tracks = Smf { tracks: vec![Track::default(), Track::default()], ..one_track(vec![]) };
    assert_eq!(two_tracks.to_bytes(), Err(SmfError::SingleTrackCount(2)));
    let no_resolution = Smf { timing: Timing::Metrical { ticks_per_quarter: 0 }, ..one_track(vec![]) };
    assert!(matches!(no_resolution.to_bytes(), Err(SmfError::InvalidTiming(_))));
}

#[test]
fn rejects_an_smpte_rate_that_would_overflow_the_reader() {
    // Format 0, one track, division 0x80 0x10: frame-rate byte -128.
    let header = [b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, 0x80, 0x10, b'M', b'T', b'r', b'k', 0, 0, 0, 4, 0, 0xFF, 0x2F, 0];
    assert!(matches!(Smf::parse(&header), Err(SmfError::Parse(_))));
}

#[test]
fn rejects_bytes_that_are_not_midi() {
    assert!(matches!(Smf::parse(b"RIFF\0\0\0\0WAVE"), Err(SmfError::Parse(_))));
    assert!(matches!(Smf::parse(&[]), Err(SmfError::Parse(_))));
}

fn u7_strategy() -> impl Strategy<Value = U7> {
    (0..=127u8).prop_map(u7)
}

fn message() -> impl Strategy<Value = MidiMessage> {
    prop_oneof![
        (u7_strategy(), u7_strategy()).prop_map(|(key, velocity)| MidiMessage::NoteOff { key, velocity }),
        (u7_strategy(), u7_strategy()).prop_map(|(key, velocity)| MidiMessage::NoteOn { key, velocity }),
        (u7_strategy(), u7_strategy()).prop_map(|(key, pressure)| MidiMessage::PolyPressure { key, pressure }),
        (u7_strategy(), u7_strategy()).prop_map(|(controller, value)| MidiMessage::ControlChange { controller, value }),
        u7_strategy().prop_map(|program| MidiMessage::ProgramChange { program }),
        u7_strategy().prop_map(|pressure| MidiMessage::ChannelPressure { pressure }),
        (0..=0x3FFFu16).prop_map(|value| MidiMessage::PitchBend { value: U14::new(value).unwrap() }),
    ]
}

fn fps() -> impl Strategy<Value = SmpteFps> {
    prop_oneof![Just(SmpteFps::Fps24), Just(SmpteFps::Fps25), Just(SmpteFps::Fps30Drop), Just(SmpteFps::Fps30)]
}

fn smpte_time() -> impl Strategy<Value = SmpteTime> {
    (0..24u8, 0..60u8, 0..60u8, 0..24u8, 0..100u8, fps()).prop_map(|(hour, minute, second, frame, subframe, fps)| SmpteTime {
        hour,
        minute,
        second,
        frame,
        subframe,
        fps,
    })
}

fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..24)
}

/// Meta event types the reader maps to their own variants; `Unknown` must avoid them to read back as `Unknown`.
const KNOWN_META: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x20, 0x21, 0x2F, 0x51, 0x54, 0x58, 0x59, 0x7F];

fn meta() -> impl Strategy<Value = MetaEvent> {
    prop_oneof![
        any::<Option<u16>>().prop_map(MetaEvent::SequenceNumber),
        bytes().prop_map(MetaEvent::Text),
        bytes().prop_map(MetaEvent::Copyright),
        bytes().prop_map(MetaEvent::TrackName),
        bytes().prop_map(MetaEvent::InstrumentName),
        bytes().prop_map(MetaEvent::Lyric),
        bytes().prop_map(MetaEvent::Marker),
        bytes().prop_map(MetaEvent::CuePoint),
        bytes().prop_map(MetaEvent::ProgramName),
        bytes().prop_map(MetaEvent::DeviceName),
        (0..16u8).prop_map(|c| MetaEvent::ChannelPrefix(U4::new(c).unwrap())),
        u7_strategy().prop_map(MetaEvent::Port),
        (0..=0xFF_FFFFu32).prop_map(MetaEvent::Tempo),
        smpte_time().prop_map(MetaEvent::SmpteOffset),
        any::<[u8; 4]>().prop_map(|[numerator, denominator_pow2, clocks_per_click, thirty_seconds_per_quarter]| {
            MetaEvent::TimeSignature { numerator, denominator_pow2, clocks_per_click, thirty_seconds_per_quarter }
        }),
        (any::<i8>(), any::<bool>()).prop_map(|(sharps, minor)| MetaEvent::KeySignature { sharps, minor }),
        bytes().prop_map(MetaEvent::SequencerSpecific),
        (any::<u8>().prop_filter("known meta type", |k| !KNOWN_META.contains(k)), bytes()).prop_map(|(kind, data)| MetaEvent::Unknown { kind, data }),
    ]
}

fn event_kind() -> impl Strategy<Value = EventKind> {
    prop_oneof![
        6 => (0..16u8, message()).prop_map(|(c, message)| channel(c, message)),
        1 => bytes().prop_map(EventKind::SysEx),
        1 => bytes().prop_map(EventKind::Escape),
        3 => meta().prop_map(EventKind::Meta),
    ]
}

fn track() -> impl Strategy<Value = Track> {
    let delta = prop_oneof![8 => 0..1000u64, 1 => 0..=0x0FFF_FFFFu64];
    (prop::collection::vec((delta, event_kind()), 0..40), 0..1000u64).prop_map(|(events, extra)| {
        let mut tick = 0;
        let events = events
            .into_iter()
            .map(|(delta, kind)| {
                tick += delta;
                TrackEvent { tick, kind }
            })
            .collect();
        Track { events, end: tick + extra }
    })
}

fn smf() -> impl Strategy<Value = Smf> {
    let timing = prop_oneof![
        (1..=0x7FFFu16).prop_map(|ticks_per_quarter| Timing::Metrical { ticks_per_quarter }),
        (fps(), 1..=255u8).prop_map(|(fps, ticks_per_frame)| Timing::Timecode { fps, ticks_per_frame }),
    ];
    let format = prop_oneof![Just(Format::SingleTrack), Just(Format::Parallel), Just(Format::Sequential)];
    (format, timing, prop::collection::vec(track(), 1..5)).prop_map(|(format, timing, mut tracks)| {
        if format == Format::SingleTrack {
            tracks.truncate(1);
        }
        Smf { format, timing, tracks }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn written_files_read_back_identically(smf in smf()) {
        let bytes = smf.to_bytes().unwrap();
        prop_assert_eq!(Smf::parse(&bytes).unwrap(), smf);
    }

    #[test]
    fn parsing_arbitrary_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = Smf::parse(&bytes);
    }

    #[test]
    fn parsing_corrupted_files_never_panics(smf in smf(), flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = smf.to_bytes().unwrap();
        for (index, value) in flips {
            let i = index.index(bytes.len());
            bytes[i] = value;
        }
        let _ = Smf::parse(&bytes);
    }
}
