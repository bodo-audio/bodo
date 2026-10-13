//! Standard MIDI File import and export (project.md section 18.2).
//!
//! - [`import_smf`] reads a file into tempo points, time signatures and one [`ImportedPart`] per track and MIDI channel
//!   that has notes, with times in Bodo ticks (960 per quarter note).
//! - [`export_pattern`] writes one pattern; [`export_arrangement`] writes the whole playlist, every clip expanded the
//!   way the sequencer plays it.
//!
//! Velocities move between MIDI 1.0's 7 bits and Bodo's 16 bits with MIDI 2.0's min-center-max scaling, so every 7-bit
//! value survives a round trip exactly. A note's group (`Note::color`) is its MIDI channel.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use bodo_kit_core::tempo::{MAX_BPM, MIN_BPM};
use bodo_kit_core::{PPQ, Seconds, TempoCurve, TempoMap, TempoPoint, Tick};
use bodo_kit_midi::smf::{EventKind, Format, MetaEvent, SmpteFps, Timing, Track, TrackEvent};
use bodo_kit_midi::{MidiMessage, Smf, U4, U7};
use bodo_model::meter::MeterChange;
use bodo_model::pattern::PatternLen;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{ChannelId, Note, Pattern, PatternId, Project};

use crate::error::IoError;

/// A note read from a MIDI file, in Bodo ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImportedNote {
    /// The MIDI key, 0 to 127.
    pub key: u8,
    /// Where it starts.
    pub start: Tick,
    /// How long it lasts; at least one tick.
    pub len: Tick,
    /// Velocity, 16-bit.
    pub velocity: u16,
    /// Release velocity, 16-bit.
    pub release_velocity: u16,
}

/// The notes of one MIDI channel of one track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedPart {
    /// The track's name (its track name, else its instrument name, else `Track N`), with the channel added when the
    /// track uses more than one.
    pub name: String,
    /// The track's index in the file, from 0.
    pub track: usize,
    /// The MIDI channel, 0 to 15.
    pub channel: u8,
    /// The first program change on the channel, if any.
    pub program: Option<u8>,
    /// The notes, sorted by start and key.
    pub notes: Vec<ImportedNote>,
}

/// What a MIDI file holds, in Bodo's terms.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedMidi {
    /// Tempo points in Bodo ticks; the first is at 0.
    pub tempo: Vec<TempoPoint>,
    /// Time signatures by bar; the first is at bar 0.
    pub meter: Vec<MeterChange>,
    /// The parts with notes, in track and channel order.
    pub parts: Vec<ImportedPart>,
    /// Where the file ends: the latest track end or note end.
    pub length: Tick,
}

impl ImportedMidi {
    /// The number of notes in every part.
    pub fn note_count(&self) -> usize {
        self.parts.iter().map(|p| p.notes.len()).sum()
    }
}

/// A 7-bit velocity as 16 bits, with MIDI 2.0's min-center-max scaling: 0 stays 0, 64 becomes 0x8000 and 127 becomes
/// 0xFFFF, and [`velocity_to_7`] gives every 7-bit value back.
pub fn velocity_to_16(value: u8) -> u16 {
    let value = u32::from(value.min(127));
    if value <= 64 {
        return (value << 9) as u16;
    }
    // Above the center, the bits below the top one repeat to fill the new low bits.
    let repeat = value & 0x3F;
    let mut out = value << 9;
    let mut fill = repeat << 3;
    while fill != 0 {
        out |= fill;
        fill >>= 6;
    }
    out.min(u32::from(u16::MAX)) as u16
}

/// A 16-bit velocity as 7 bits.
pub fn velocity_to_7(value: u16) -> u8 {
    (value >> 9) as u8
}

fn midi_error(message: impl Into<String>) -> IoError {
    IoError::Midi { path: None, message: message.into() }
}

/// Turns file ticks into Bodo ticks.
enum Clock {
    /// Ticks per quarter note.
    Metrical(u16),
    /// SMPTE ticks per second, and the file's tempo map to turn seconds into ticks.
    Timecode(f64, TempoMap),
}

impl Clock {
    fn ticks(&self, file_tick: u64) -> i64 {
        match self {
            Self::Metrical(per_quarter) => {
                let per_quarter = i128::from(*per_quarter.max(&1));
                let scaled = (i128::from(file_tick) * i128::from(PPQ) * 2 + per_quarter) / (per_quarter * 2);
                i64::try_from(scaled).unwrap_or(i64::MAX)
            }
            Self::Timecode(per_second, tempo) => tempo.ticks_at(Seconds(file_tick as f64 / per_second)).round() as i64,
        }
    }
}

fn fps_value(fps: SmpteFps) -> f64 {
    match fps {
        SmpteFps::Fps24 => 24.0,
        SmpteFps::Fps25 => 25.0,
        SmpteFps::Fps30Drop => 30_000.0 / 1001.0,
        SmpteFps::Fps30 => 30.0,
    }
}

/// The tempo `micros_per_quarter` stands for: the simplest BPM, with at most three decimals, that a file stores as the
/// same number of microseconds, so 90 BPM written as 666667 microseconds reads back as 90.
fn bpm_of(micros_per_quarter: u32) -> f64 {
    let micros = micros_per_quarter.max(1);
    let exact = 60_000_000.0 / f64::from(micros);
    let simplest = [1.0, 10.0, 100.0, 1000.0]
        .into_iter()
        .map(|scale| (exact * scale).round() / scale)
        .find(|bpm| *bpm > 0.0 && (60_000_000.0 / bpm).round() == f64::from(micros))
        .unwrap_or(exact);
    simplest.clamp(MIN_BPM, MAX_BPM)
}

/// Tempo points from `(position, bpm)` pairs sorted by position: one per position (the last wins), the first at 0.
fn tempo_points(changes: impl Iterator<Item = (i64, f64)>) -> Vec<TempoPoint> {
    let mut points: Vec<TempoPoint> = Vec::new();
    for (at, bpm) in changes {
        let at = Tick(at.max(0));
        match points.last_mut() {
            Some(last) if last.at == at => last.bpm = bpm,
            Some(last) if (last.bpm - bpm).abs() < 1e-9 => {}
            _ => points.push(TempoPoint::new(at, bpm, TempoCurve::Jump)),
        }
    }
    if points.first().is_none_or(|p| p.at != Tick::ZERO) {
        points.insert(0, TempoPoint::new(Tick::ZERO, 120.0, TempoCurve::Jump));
    }
    points
}

/// The text of a meta event, without control characters, or `None` when it is empty.
fn text(bytes: &[u8]) -> Option<String> {
    let text: String = String::from_utf8_lossy(bytes).chars().filter(|c| !c.is_control()).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Reads a Standard MIDI File (format 0, 1 or 2; tracks of a format 2 file are read as if they played together).
///
/// Complexity: O(n log n) in the events of the file.
pub fn import_smf(bytes: &[u8]) -> Result<ImportedMidi, IoError> {
    let smf = Smf::parse(bytes).map_err(|e| midi_error(e.to_string()))?;
    let mut tempo_events: Vec<(u64, u32)> = smf
        .tracks
        .iter()
        .flat_map(|t| t.events.iter())
        .filter_map(|e| match &e.kind {
            EventKind::Meta(MetaEvent::Tempo(micros)) => Some((e.tick, *micros)),
            _ => None,
        })
        .collect();
    tempo_events.sort_by_key(|(tick, _)| *tick);
    let (clock, tempo) = match smf.timing {
        Timing::Metrical { ticks_per_quarter } => {
            let clock = Clock::Metrical(ticks_per_quarter);
            let tempo = tempo_points(tempo_events.iter().map(|&(tick, micros)| (clock.ticks(tick), bpm_of(micros))));
            (clock, tempo)
        }
        Timing::Timecode { fps, ticks_per_frame } => {
            let per_second = fps_value(fps) * f64::from(ticks_per_frame.max(1));
            // Tempo events sit at times in seconds; lay them out in ticks one segment at a time.
            let (mut at_ticks, mut at_seconds, mut bpm) = (0.0, 0.0, 120.0);
            let mut changes = Vec::with_capacity(tempo_events.len());
            for &(tick, micros) in &tempo_events {
                let seconds = tick as f64 / per_second;
                at_ticks += (seconds - at_seconds) * bpm / 60.0 * PPQ as f64;
                at_seconds = seconds;
                bpm = bpm_of(micros);
                changes.push((at_ticks.round() as i64, bpm));
            }
            let tempo = tempo_points(changes.into_iter());
            let map = TempoMap::new(tempo.clone()).map_err(|e| midi_error(e.to_string()))?;
            (Clock::Timecode(per_second, map), tempo)
        }
    };
    let meter = meter_changes(&smf, &clock);
    let mut parts = Vec::new();
    let mut length = 0;
    for (index, track) in smf.tracks.iter().enumerate() {
        length = length.max(clock.ticks(track.end));
        let found = read_track(index, track, &clock);
        for part in &found {
            length = length.max(part.notes.iter().map(|n| n.start.0 + n.len.0).max().unwrap_or(0));
        }
        parts.extend(found);
    }
    Ok(ImportedMidi { tempo, meter, parts, length: Tick(length) })
}

/// The file's time signatures as changes at bar starts. A change that falls inside a bar moves to the nearest bar.
fn meter_changes(smf: &Smf, clock: &Clock) -> Vec<MeterChange> {
    let mut signatures: Vec<(i64, u8, u8)> = smf
        .tracks
        .iter()
        .flat_map(|t| t.events.iter())
        .filter_map(|e| match &e.kind {
            EventKind::Meta(MetaEvent::TimeSignature { numerator, denominator_pow2, .. })
                if (1..=64).contains(numerator) && *denominator_pow2 <= 5 =>
            {
                Some((clock.ticks(e.tick), *numerator, 1u8 << denominator_pow2))
            }
            _ => None,
        })
        .collect();
    signatures.sort_by_key(|(tick, _, _)| *tick);
    let mut changes = vec![MeterChange { bar: 0, numerator: 4, denominator: 4 }];
    let mut start = 0i64;
    for (tick, numerator, denominator) in signatures {
        let Some(last) = changes.last().copied() else { break };
        let bar_ticks = last.bar_ticks().max(1);
        let bars = ((tick - start) as f64 / bar_ticks as f64).round().max(0.0) as i64;
        let bar = u32::try_from(i64::from(last.bar) + bars).unwrap_or(u32::MAX);
        let change = MeterChange { bar, numerator, denominator };
        if bar == last.bar {
            if let Some(slot) = changes.last_mut() {
                *slot = change;
            }
        } else {
            start += bars * bar_ticks;
            changes.push(change);
        }
    }
    changes
}

/// A track's notes per MIDI channel.
fn read_track(index: usize, track: &Track, clock: &Clock) -> Vec<ImportedPart> {
    let mut name = None;
    let mut instrument = None;
    let mut programs: BTreeMap<u8, u8> = BTreeMap::new();
    let mut open: BTreeMap<(u8, u8), VecDeque<(i64, u8)>> = BTreeMap::new();
    let mut notes: BTreeMap<u8, Vec<ImportedNote>> = BTreeMap::new();
    let close = |notes: &mut BTreeMap<u8, Vec<ImportedNote>>, channel: u8, key: u8, (start, velocity): (i64, u8), end: i64, release: u8| {
        notes.entry(channel).or_default().push(ImportedNote {
            key,
            start: Tick(start),
            len: Tick((end - start).max(1)),
            velocity: velocity_to_16(velocity),
            release_velocity: velocity_to_16(release),
        });
    };
    for event in &track.events {
        match &event.kind {
            EventKind::Meta(MetaEvent::TrackName(bytes)) if name.is_none() => name = text(bytes),
            EventKind::Meta(MetaEvent::InstrumentName(bytes)) if instrument.is_none() => instrument = text(bytes),
            EventKind::Channel { channel, message } => {
                let channel = channel.get();
                let at = clock.ticks(event.tick);
                match *message {
                    MidiMessage::NoteOn { key, velocity } if velocity.get() > 0 => {
                        open.entry((channel, key.get())).or_default().push_back((at, velocity.get()));
                    }
                    MidiMessage::NoteOn { key, .. } => {
                        if let Some(start) = open.get_mut(&(channel, key.get())).and_then(VecDeque::pop_front) {
                            close(&mut notes, channel, key.get(), start, at, 64);
                        }
                    }
                    MidiMessage::NoteOff { key, velocity } => {
                        if let Some(start) = open.get_mut(&(channel, key.get())).and_then(VecDeque::pop_front) {
                            close(&mut notes, channel, key.get(), start, at, velocity.get());
                        }
                    }
                    MidiMessage::ProgramChange { program } => {
                        programs.entry(channel).or_insert(program.get());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // Notes still sounding end with the track.
    let end = clock.ticks(track.end);
    for ((channel, key), starts) in open {
        for start in starts {
            close(&mut notes, channel, key, start, end, 64);
        }
    }
    let base = name.or(instrument).unwrap_or_else(|| format!("Track {}", index + 1));
    let several = notes.len() > 1;
    notes
        .into_iter()
        .map(|(channel, mut list)| {
            list.sort_by_key(|n| (n.start, n.key));
            ImportedPart {
                name: if several { format!("{base} (channel {})", channel + 1) } else { base.clone() },
                track: index,
                channel,
                program: programs.get(&channel).copied(),
                notes: list,
            }
        })
        .collect()
}

/// Reads the MIDI file at `path`.
///
/// Does I/O. Complexity: O(n log n) in the events of the file.
pub fn read_midi_file(path: &Path) -> Result<ImportedMidi, IoError> {
    let bytes = std::fs::read(path).map_err(|e| IoError::io("read", path, e))?;
    import_smf(&bytes).map_err(|e| e.at(path))
}

/// Writes `smf` to `path` atomically.
///
/// Does I/O. Complexity: O(n) in the events.
pub fn write_midi_file(path: &Path, smf: &Smf) -> Result<(), IoError> {
    let bytes = smf.to_bytes().map_err(|e| IoError::Midi { path: Some(path.to_path_buf()), message: e.to_string() })?;
    crate::atomic::write(path, &bytes)
}

/// How long `pattern` plays: its fixed length, or its notes rounded up to whole bars, at least one bar. The sequencer's
/// rule.
fn pattern_length(project: &Project, pattern: &Pattern) -> i64 {
    match pattern.length {
        PatternLen::Fixed(len) => len.0.max(1),
        PatternLen::Auto => {
            let bar = project.meter.changes().first().map_or(PPQ * 4, MeterChange::bar_ticks).max(1);
            (pattern.content_end().0.max(1) + bar - 1) / bar * bar
        }
    }
}

/// A note to write: on at `start`, off at `end`.
#[derive(Clone, Copy, Debug)]
struct Placed {
    start: i64,
    end: i64,
    key: u8,
    channel: u8,
    velocity: u16,
    release: u16,
}

fn event(tick: i64, kind: EventKind) -> TrackEvent {
    TrackEvent { tick: u64::try_from(tick).unwrap_or(0), kind }
}

/// A track of notes named `name`, ending no earlier than `end`. Note-offs come before note-ons at the same tick.
fn note_track(name: &str, notes: &[Placed], end: i64) -> Track {
    let mut events: Vec<(i64, u8, TrackEvent)> = Vec::with_capacity(notes.len() * 2 + 1);
    events.push((0, 0, event(0, EventKind::Meta(MetaEvent::TrackName(name.as_bytes().to_vec())))));
    for note in notes {
        let (key, channel) = (U7::from_masked(note.key), U4::from_masked(note.channel));
        let velocity = U7::from_masked(velocity_to_7(note.velocity).max(1));
        let release = U7::from_masked(velocity_to_7(note.release));
        let on = MidiMessage::NoteOn { key, velocity };
        let off = MidiMessage::NoteOff { key, velocity: release };
        events.push((note.start.max(0), 2, event(note.start, EventKind::Channel { channel, message: on })));
        events.push((note.end.max(0), 1, event(note.end, EventKind::Channel { channel, message: off })));
    }
    events.sort_by_key(|(tick, order, _)| (*tick, *order));
    let last = events.last().map_or(0, |(tick, _, _)| *tick);
    Track { events: events.into_iter().map(|(_, _, e)| e).collect(), end: u64::try_from(end.max(last)).unwrap_or(0) }
}

/// The conductor track: a name, the tempo map (linear ramps written as a step every sixteenth), the time signatures
/// and the project's markers when `markers` is set.
fn conductor(project: &Project, name: &str, end: i64, markers: bool) -> Track {
    let mut events = vec![event(0, EventKind::Meta(MetaEvent::TrackName(name.as_bytes().to_vec())))];
    let mut timed: Vec<TrackEvent> = Vec::new();
    let micros = |bpm: f64| ((60_000_000.0 / bpm).round() as u32).clamp(1, 0x00FF_FFFF);
    let points = project.tempo.points();
    for (index, point) in points.iter().enumerate() {
        timed.push(event(point.at.0, EventKind::Meta(MetaEvent::Tempo(micros(point.bpm)))));
        if point.curve == TempoCurve::Linear
            && let Some(next) = points.get(index + 1)
        {
            let step = PPQ / 4;
            let mut at = point.at.0 + step;
            while at < next.at.0 {
                timed.push(event(at, EventKind::Meta(MetaEvent::Tempo(micros(project.tempo.bpm_at(Tick(at)))))));
                at += step;
            }
        }
    }
    for change in project.meter.changes() {
        let at = project.meter.bar_start(i64::from(change.bar)).0;
        let pow2 = change.denominator.max(1).trailing_zeros() as u8;
        let clocks = (96 / u16::from(change.denominator.max(1))).clamp(1, 255) as u8;
        timed.push(event(
            at,
            EventKind::Meta(MetaEvent::TimeSignature {
                numerator: change.numerator,
                denominator_pow2: pow2,
                clocks_per_click: clocks,
                thirty_seconds_per_quarter: 8,
            }),
        ));
    }
    if markers {
        for marker in &project.markers {
            timed.push(event(marker.at.0, EventKind::Meta(MetaEvent::Marker(marker.name.as_bytes().to_vec()))));
        }
    }
    timed.sort_by_key(|e| e.tick);
    events.extend(timed);
    let last = events.last().map_or(0, |e| e.tick);
    Track { events, end: u64::try_from(end).unwrap_or(0).max(last) }
}

/// Every channel that has notes in `notes`, in channel rack order, then any others by id.
fn channel_order(project: &Project, has_notes: impl Fn(&ChannelId) -> bool) -> Vec<ChannelId> {
    let mut order: Vec<ChannelId> = project.rack_order.iter().copied().filter(|c| has_notes(c)).collect();
    let mut rest: Vec<ChannelId> = project.channels.keys().copied().filter(|c| has_notes(c) && !order.contains(c)).collect();
    order.append(&mut rest);
    order
}

fn channel_name(project: &Project, id: ChannelId) -> String {
    project.channels.get(&id).map_or_else(|| id.to_string(), |c| c.name.clone())
}

fn placed(note: &Note, start: i64, end: i64) -> Placed {
    Placed { start, end, key: note.key, channel: note.color & 0x0F, velocity: note.velocity, release: note.release_velocity }
}

/// One pattern as a type 1 file at 960 ticks per quarter: a conductor track with the tempo and time signatures, then
/// one track per channel with notes, named after the channel. Muted notes are left out.
///
/// Complexity: O(n log n) in the pattern's notes.
pub fn export_pattern(project: &Project, id: PatternId) -> Result<Smf, IoError> {
    let pattern = project.patterns.get(&id).ok_or_else(|| midi_error(format!("there is no pattern {id}")))?;
    let length = pattern_length(project, pattern);
    let mut tracks = vec![conductor(project, &pattern.name, length, false)];
    let order = channel_order(project, |c| pattern.notes.get(c).is_some_and(|seq| seq.notes().iter().any(|n| !n.muted)));
    for channel in order {
        let Some(seq) = pattern.notes.get(&channel) else { continue };
        let notes: Vec<Placed> = seq.notes().iter().filter(|n| !n.muted).map(|n| placed(n, n.start.0, n.end().0)).collect();
        tracks.push(note_track(&channel_name(project, channel), &notes, length));
    }
    Ok(Smf { format: Format::Parallel, timing: Timing::Metrical { ticks_per_quarter: PPQ as u16 }, tracks })
}

/// The whole playlist as a type 1 file: every unmuted pattern clip on every unmuted track expanded as the sequencer
/// plays it (a clip longer than its pattern repeats it, its offset skips into it, and its end cuts notes short), one
/// track per channel, with the tempo map, time signatures and markers. Muted notes are left out.
///
/// Complexity: O(c * r * n) in clips, repeats per clip and notes per pattern, plus sorting.
pub fn export_arrangement(project: &Project) -> Smf {
    let mut by_channel: BTreeMap<ChannelId, Vec<Placed>> = BTreeMap::new();
    let mut end = 0;
    for track in project.track_order.iter().filter_map(|id| project.tracks.get(id)).filter(|t| !t.muted) {
        for clip in track.clips.iter().filter(|c| !c.muted) {
            let ClipSource::Pattern(id) = clip.source else { continue };
            let Some(pattern) = project.patterns.get(&id) else { continue };
            let length = pattern_length(project, pattern);
            let offset = match clip.offset {
                ClipOffset::Ticks(t) => t.0,
                ClipOffset::Frames(_) => 0,
            };
            let origin = clip.start.0 - offset;
            let (from, to) = (clip.start.0, clip.end().0);
            end = end.max(to);
            let first = (from - origin).div_euclid(length) - 1;
            let last = (to - origin).div_euclid(length);
            for (channel, seq) in &pattern.notes {
                for repeat in first..=last {
                    let base = origin + repeat * length;
                    for note in seq.notes().iter().filter(|n| !n.muted && n.start.0 < length) {
                        let on = base + note.start.0;
                        if on < from || on >= to {
                            continue;
                        }
                        let off = (on + note.len.0).min(to);
                        by_channel.entry(*channel).or_default().push(placed(note, on, off));
                    }
                }
            }
        }
    }
    let title = if project.meta.title.is_empty() { String::from("Bodo") } else { project.meta.title.clone() };
    let mut tracks = vec![conductor(project, &title, end, true)];
    for channel in channel_order(project, |c| by_channel.contains_key(c)) {
        let notes = by_channel.remove(&channel).unwrap_or_default();
        tracks.push(note_track(&channel_name(project, channel), &notes, end));
    }
    Smf { format: Format::Parallel, timing: Timing::Metrical { ticks_per_quarter: PPQ as u16 }, tracks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_bit_velocities_round_trip_through_sixteen() {
        assert_eq!(velocity_to_16(0), 0);
        assert_eq!(velocity_to_16(64), 0x8000);
        assert_eq!(velocity_to_16(127), 0xFFFF);
        for v in 0..=127u8 {
            assert_eq!(velocity_to_7(velocity_to_16(v)), v, "{v}");
        }
        let ups: Vec<u16> = (0..=127).map(velocity_to_16).collect();
        assert!(ups.windows(2).all(|w| w[0] < w[1]), "scaling keeps order");
    }

    #[test]
    fn tempos_read_back_as_the_simplest_bpm() {
        assert_eq!(bpm_of(666_667), 90.0);
        assert_eq!(bpm_of(500_000), 120.0);
        assert_eq!(bpm_of(428_571), 140.0);
        assert_eq!(bpm_of(461_538), 130.0);
        assert!((bpm_of(500_001) - 119.99976).abs() < 1e-5, "{}", bpm_of(500_001));
    }

    #[test]
    fn metrical_ticks_scale_to_960() {
        assert_eq!(Clock::Metrical(480).ticks(240), 480);
        assert_eq!(Clock::Metrical(96).ticks(25), 250);
        assert_eq!(Clock::Metrical(1000).ticks(1000), 960);
        assert_eq!(Clock::Metrical(7).ticks(1), 137, "rounded to the nearest tick");
    }
}
