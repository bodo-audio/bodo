//! Standard MIDI File import and export: times in any division land on Bodo ticks, notes pair up the way players
//! expect, and what Bodo writes it reads back the same.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

use std::sync::Arc;

use bodo_io::{IoError, export_arrangement, export_pattern, import_smf, midi};
use bodo_kit_core::{TempoMap, Tick};
use bodo_kit_midi::smf::{EventKind, Format, MetaEvent, SmpteFps, Timing, Track, TrackEvent};
use bodo_kit_midi::{MidiMessage, Smf, U4, U7};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::pattern::NoteSeq;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::*;

fn on(tick: u64, channel: u8, key: u8, velocity: u8) -> TrackEvent {
    TrackEvent {
        tick,
        kind: EventKind::Channel {
            channel: U4::new(channel).unwrap(),
            message: MidiMessage::NoteOn { key: U7::new(key).unwrap(), velocity: U7::new(velocity).unwrap() },
        },
    }
}

fn off(tick: u64, channel: u8, key: u8, velocity: u8) -> TrackEvent {
    TrackEvent {
        tick,
        kind: EventKind::Channel {
            channel: U4::new(channel).unwrap(),
            message: MidiMessage::NoteOff { key: U7::new(key).unwrap(), velocity: U7::new(velocity).unwrap() },
        },
    }
}

fn meta(tick: u64, meta: MetaEvent) -> TrackEvent {
    TrackEvent { tick, kind: EventKind::Meta(meta) }
}

/// A file with `tracks`, each track's events put in tick order first (stably, so same-tick events keep their order).
fn file(timing: Timing, mut tracks: Vec<Track>) -> Vec<u8> {
    for track in &mut tracks {
        track.events.sort_by_key(|e| e.tick);
    }
    Smf { format: if tracks.len() == 1 { Format::SingleTrack } else { Format::Parallel }, timing, tracks }.to_bytes().unwrap()
}

fn starts_and_lengths(part: &bodo_io::ImportedPart) -> Vec<(u8, i64, i64)> {
    part.notes.iter().map(|n| (n.key, n.start.0, n.len.0)).collect()
}

#[test]
fn a_480_division_file_imports_exactly() {
    let conductor = Track {
        events: vec![
            meta(0, MetaEvent::Tempo(600_000)),
            meta(0, MetaEvent::TimeSignature { numerator: 3, denominator_pow2: 2, clocks_per_click: 24, thirty_seconds_per_quarter: 8 }),
            meta(1920, MetaEvent::Tempo(428_571)),
        ],
        end: 1920,
    };
    let piano = Track {
        events: vec![
            meta(0, MetaEvent::TrackName(b"Piano".to_vec())),
            TrackEvent {
                tick: 0,
                kind: EventKind::Channel { channel: U4::MIN, message: MidiMessage::ProgramChange { program: U7::new(5).unwrap() } },
            },
            on(0, 0, 60, 100),
            off(480, 0, 60, 40),
            on(480, 0, 64, 64),
            on(720, 0, 64, 0),
            // Two overlapping notes on one key pair up first in, first out.
            on(960, 0, 67, 90),
            on(1200, 0, 67, 80),
            off(1440, 0, 67, 0),
            off(1680, 0, 67, 0),
            // A note that never ends lasts until the track does.
            on(1800, 0, 72, 127),
        ],
        end: 2400,
    };
    let imported = import_smf(&file(Timing::Metrical { ticks_per_quarter: 480 }, vec![conductor, piano])).unwrap();
    assert_eq!(imported.tempo.len(), 2);
    assert_eq!((imported.tempo[0].at, imported.tempo[0].bpm), (Tick(0), 100.0));
    assert_eq!(imported.tempo[1].at, Tick(3840));
    assert_eq!(imported.tempo[1].bpm, 140.0);
    assert_eq!(imported.meter, vec![bodo_model::meter::MeterChange { bar: 0, numerator: 3, denominator: 4 }]);
    let [part] = imported.parts.as_slice() else { panic!("{:?}", imported.parts) };
    assert_eq!((part.name.as_str(), part.channel, part.program, part.track), ("Piano", 0, Some(5), 1));
    assert_eq!(starts_and_lengths(part), [(60, 0, 960), (64, 960, 480), (67, 1920, 960), (67, 2400, 960), (72, 3600, 1200)]);
    assert_eq!(part.notes[0].velocity, midi::velocity_to_16(100));
    assert_eq!(part.notes[0].release_velocity, midi::velocity_to_16(40));
    assert_eq!(part.notes[1].release_velocity, midi::velocity_to_16(64), "a note-on of velocity 0 releases at 64");
    assert_eq!(imported.length, Tick(4800));
    assert_eq!(imported.note_count(), 5);
}

#[test]
fn a_96_division_file_with_several_channels_imports_exactly() {
    let drums = Track {
        events: vec![meta(0, MetaEvent::InstrumentName(b"Kit".to_vec())), on(0, 9, 36, 110), off(24, 9, 36, 0), on(48, 0, 40, 50), off(72, 0, 40, 0)],
        end: 96,
    };
    let imported = import_smf(&file(Timing::Metrical { ticks_per_quarter: 96 }, vec![drums])).unwrap();
    let names: Vec<(&str, u8)> = imported.parts.iter().map(|p| (p.name.as_str(), p.channel)).collect();
    assert_eq!(names, [("Kit (channel 1)", 0), ("Kit (channel 10)", 9)], "one part per channel, named after the instrument");
    assert_eq!(starts_and_lengths(&imported.parts[1]), [(36, 0, 240)]);
    assert_eq!(starts_and_lengths(&imported.parts[0]), [(40, 480, 240)]);
    assert_eq!(imported.tempo, vec![bodo_kit_core::TempoPoint::new(Tick(0), 120.0, bodo_kit_core::TempoCurve::Jump)]);
}

#[test]
fn smpte_files_follow_the_tempo_in_seconds() {
    // 25 frames of 40 ticks: 1000 ticks per second. 120 BPM for one second, then 60 BPM.
    let track = Track {
        events: vec![
            meta(0, MetaEvent::Tempo(500_000)),
            meta(1000, MetaEvent::Tempo(1_000_000)),
            on(500, 0, 60, 100),
            off(1000, 0, 60, 0),
            on(2000, 0, 62, 100),
            off(3000, 0, 62, 0),
        ],
        end: 3000,
    };
    let imported = import_smf(&file(Timing::Timecode { fps: SmpteFps::Fps25, ticks_per_frame: 40 }, vec![track])).unwrap();
    assert_eq!(imported.tempo.iter().map(|p| (p.at.0, p.bpm)).collect::<Vec<_>>(), [(0, 120.0), (1920, 60.0)]);
    assert_eq!(starts_and_lengths(&imported.parts[0]), [(60, 960, 960), (62, 2880, 960)]);
}

#[test]
fn time_signature_changes_land_on_bars() {
    let track = Track {
        events: vec![
            meta(0, MetaEvent::TimeSignature { numerator: 4, denominator_pow2: 2, clocks_per_click: 24, thirty_seconds_per_quarter: 8 }),
            meta(3840, MetaEvent::TimeSignature { numerator: 6, denominator_pow2: 3, clocks_per_click: 12, thirty_seconds_per_quarter: 8 }),
            meta(
                3840 + 2880 * 2,
                MetaEvent::TimeSignature { numerator: 7, denominator_pow2: 3, clocks_per_click: 12, thirty_seconds_per_quarter: 8 },
            ),
            on(0, 0, 60, 100),
            off(10, 0, 60, 0),
        ],
        end: 20000,
    };
    let imported = import_smf(&file(Timing::Metrical { ticks_per_quarter: 960 }, vec![track])).unwrap();
    let bars: Vec<(u32, u8, u8)> = imported.meter.iter().map(|m| (m.bar, m.numerator, m.denominator)).collect();
    assert_eq!(bars, [(0, 4, 4), (1, 6, 8), (3, 7, 8)]);
    assert!(bodo_model::MeterMap::new(imported.meter).is_ok());
}

#[test]
fn broken_files_are_refused_with_a_clear_error() {
    let error = import_smf(b"MThd nonsense").unwrap_err();
    assert!(matches!(error, IoError::Midi { .. }), "{error}");
    assert!(error.to_string().contains("MIDI"), "{error}");
    let missing = bodo_io::read_midi_file(std::path::Path::new("/no/such/file.mid")).unwrap_err();
    assert!(missing.to_string().contains("/no/such/file.mid"), "{missing}");
}

/// A project with two sampler channels and one pattern, at 120 BPM.
fn project() -> (Project, PatternId, ChannelId, ChannelId) {
    let mut project = Project::new(120.0).unwrap();
    let (kick, bass) = (ChannelId::from_u128(1), ChannelId::from_u128(2));
    for (id, name) in [(kick, "Kick"), (bass, "Bass")] {
        let channel = Channel {
            id,
            name: name.into(),
            color: Color::GRAY,
            kind: ChannelKind::Sampler(Arc::new(SamplerParams::default())),
            target: None,
            volume: 1.0,
            pan: 0.0,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
            group: None,
        };
        project.channels.insert(id, Arc::new(channel));
        project.rack_order.push_back(id);
    }
    let id = PatternId::from_u128(3);
    let mut pattern = Pattern::new(id, "Groove", Color::GRAY);
    let note = |n: u32, key: u8, start: i64, len: i64, velocity: u8| {
        let mut note = Note::new(NoteId(n), key, Tick(start), Tick(len));
        note.velocity = midi::velocity_to_16(velocity);
        note.release_velocity = midi::velocity_to_16(velocity / 2);
        note
    };
    pattern.notes.insert(kick, NoteSeq::new([note(0, 36, 0, 240, 120), note(1, 36, 1920, 240, 100)]));
    let mut muted = note(4, 40, 960, 240, 90);
    muted.muted = true;
    pattern.notes.insert(bass, NoteSeq::new([note(2, 28, 0, 960, 80), note(3, 31, 3360, 480, 70), muted]));
    pattern.next_note_id = 5;
    project.patterns.insert(id, Arc::new(pattern));
    (project, id, kick, bass)
}

#[test]
fn an_exported_pattern_imports_with_the_same_notes() {
    let (project, id, _, _) = project();
    let smf = export_pattern(&project, id).unwrap();
    assert_eq!(smf.timing, Timing::Metrical { ticks_per_quarter: 960 });
    let imported = import_smf(&smf.to_bytes().unwrap()).unwrap();
    let names: Vec<&str> = imported.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Kick", "Bass"]);
    let pattern = &project.patterns[&id];
    for (part, channel) in imported.parts.iter().zip([ChannelId::from_u128(1), ChannelId::from_u128(2)]) {
        let original: Vec<(u8, i64, i64, u16, u16)> = pattern.notes[&channel]
            .notes()
            .iter()
            .filter(|n| !n.muted)
            .map(|n| (n.key, n.start.0, n.len.0, n.velocity, n.release_velocity))
            .collect();
        let back: Vec<(u8, i64, i64, u16, u16)> = part.notes.iter().map(|n| (n.key, n.start.0, n.len.0, n.velocity, n.release_velocity)).collect();
        assert_eq!(back, original, "{}", part.name);
    }
    assert_eq!(imported.tempo[0].bpm, 120.0);
    assert_eq!(imported.length, Tick(3840), "the file ends with the pattern");
}

#[test]
fn an_exported_arrangement_plays_like_the_sequencer() {
    let (mut project, id, _, _) = project();
    let track = TrackId::from_u128(10);
    let mut lane = PlaylistTrack::new(track, "Track 1", Color::GRAY);
    let clip = |n: u128, start: i64, len: i64, offset: i64, muted: bool| Clip {
        id: ClipId::from_u128(n),
        start: Tick(start),
        len: Tick(len),
        offset: ClipOffset::Ticks(Tick(offset)),
        muted,
        color: None,
        source: ClipSource::Pattern(id),
        fades: None,
    };
    // Two bars from bar 2: the one-bar pattern plays twice.
    lane.insert_clip(clip(11, 3840, 7680, 0, false));
    // Half a bar starting half way into the pattern, at bar 5.
    lane.insert_clip(clip(12, 15360, 1920, 1920, false));
    // A muted clip plays nothing.
    lane.insert_clip(clip(13, 30000, 3840, 0, true));
    project.tracks.insert(track, Arc::new(lane));
    project.track_order.push_back(track);
    project.tempo = Arc::new(TempoMap::constant(90.0).unwrap());
    project.markers.push_back(bodo_model::project::Marker { at: Tick(3840), name: "Verse".into(), color: None });

    let smf = export_arrangement(&project);
    let imported = import_smf(&smf.to_bytes().unwrap()).unwrap();
    assert_eq!(imported.tempo[0].bpm, 90.0);
    let kick = starts_and_lengths(&imported.parts[0]);
    assert_eq!(kick, [(36, 3840, 240), (36, 5760, 240), (36, 7680, 240), (36, 9600, 240), (36, 15360, 240)]);
    let bass = starts_and_lengths(&imported.parts[1]);
    // The half-bar clip starts half way into the pattern, so it plays only the note at 3360, cut at the clip's end.
    assert_eq!(bass, [(28, 3840, 960), (31, 7200, 480), (28, 7680, 960), (31, 11040, 480), (31, 16800, 480)]);
    let markers: Vec<u64> =
        smf.tracks[0].events.iter().filter_map(|e| matches!(e.kind, EventKind::Meta(MetaEvent::Marker(_))).then_some(e.tick)).collect();
    assert_eq!(markers, [3840]);
}
