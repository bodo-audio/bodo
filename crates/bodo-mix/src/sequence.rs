//! The sequencer: which notes start and stop in each block (project.md section 6.6).
//!
//! The control thread flattens the project into a [`SongIndex`]: per track, clips sorted by start; per pattern, notes
//! sorted by start. For a block from tick `t0` to `t1`, the audio thread then finds each track's active clips by
//! binary search, and inside each clip the notes of the pattern-local range by binary search: O(T log c + sum of
//! (log n + k)). A pattern placed 200 times is still one list of notes. A clip longer than its pattern repeats it.
//!
//! Every event lands on the exact frame its tick maps to through the tempo map, so timing is sample accurate and
//! renders are deterministic.

use std::collections::HashMap;

use bodo_kit_core::{Frames, SampleRate, TempoMap, Tick, rt_safe};
use bodo_model::meter::MeterMap;
use bodo_model::pattern::PatternLen;
use bodo_model::playlist::ClipSource;
use bodo_model::{PatternId, Project};

use crate::audio_clips::AudioIndex;
use crate::automation::AutomationIndex;
use crate::context::{BlockContext, EventKind, NoteEvent};
use crate::keys;

/// A note, flattened for the sequencer.
#[derive(Clone, Copy, Debug, PartialEq)]
struct IndexedNote {
    start: i64,
    len: i64,
    key: u8,
    velocity: f32,
    channel: u64,
    id: u32,
}

/// A pattern, flattened: its loop length and its notes from every channel, sorted by start.
#[derive(Clone, Debug, PartialEq)]
struct IndexedPattern {
    length: i64,
    notes: Vec<IndexedNote>,
    max_len: i64,
}

/// A pattern clip, flattened.
#[derive(Clone, Copy, Debug, PartialEq)]
struct IndexedClip {
    start: i64,
    end: i64,
    /// Pattern ticks skipped at the clip's start.
    offset: i64,
    pattern: u32,
    /// Distinguishes this clip's voices from other clips playing the same pattern.
    instance: u32,
}

/// A track, flattened: its pattern clips sorted by start.
#[derive(Clone, Debug, PartialEq)]
struct IndexedTrack {
    clips: Vec<IndexedClip>,
    max_len: i64,
}

/// Everything the sequencer needs, flat and sorted, built off the audio thread.
#[derive(Clone, Debug, PartialEq)]
pub struct SongIndex {
    tempo: TempoMap,
    /// The meter, for the metronome and count-in.
    meter: MeterMap,
    patterns: Vec<IndexedPattern>,
    /// Pattern keys and their index in `patterns`, sorted by key.
    pattern_lookup: Vec<(u64, u32)>,
    tracks: Vec<IndexedTrack>,
    /// Every channel key that has notes, sorted, so a stop can release them all.
    channels: Vec<u64>,
    /// The audio clips.
    audio: AudioIndex,
    /// The automation clips and pattern lanes.
    pub(crate) automation: AutomationIndex,
}

/// The length of one step of the step sequencer: a sixteenth note.
pub const STEP: i64 = bodo_kit_core::PPQ / 4;

/// How long `pattern` loops: its fixed length, or its notes rounded up to whole bars of `bar` ticks.
pub(crate) fn loop_length(pattern: &bodo_model::Pattern, bar: i64) -> i64 {
    match pattern.length {
        PatternLen::Fixed(len) => len.0.max(1),
        PatternLen::Auto => {
            let end = pattern.content_end().0;
            (end.max(1) + bar - 1) / bar * bar
        }
    }
}

/// Swing: how late a note starting at `start` plays on a channel with `swing` (0 to 1). Notes that start on the
/// second sixteenth of an eighth (an odd step) play up to half a step late; the others play on time.
pub fn swing_delay(start: i64, swing: f32) -> i64 {
    if swing <= 0.0 || start.div_euclid(STEP).rem_euclid(2) != 1 {
        return 0;
    }
    (f64::from(swing.min(1.0)) * STEP as f64 / 2.0).round() as i64
}

impl SongIndex {
    /// Flattens `project`. Muted tracks and clips are left out.
    ///
    /// Allocates. Complexity: O(N log N) in the notes, clips and automation points.
    pub fn build(project: &Project) -> Self {
        let bar = project.meter.changes().first().map_or(Tick::from_quarters(4).0, |m| m.bar_ticks());
        let swing: HashMap<u64, f32> = project.channels.values().filter(|c| c.swing > 0.0).map(|c| (keys::channel(c.id), c.swing)).collect();
        let mut index_of: HashMap<PatternId, u32> = HashMap::new();
        let mut patterns = Vec::with_capacity(project.patterns.len());
        let mut pattern_lookup = Vec::with_capacity(project.patterns.len());
        let mut channels = Vec::new();
        for (id, pattern) in &project.patterns {
            let mut notes: Vec<IndexedNote> = pattern
                .notes
                .iter()
                .flat_map(|(channel, seq)| {
                    let channel = keys::channel(*channel);
                    let swing = swing.get(&channel).copied().unwrap_or(0.0);
                    seq.notes().iter().filter(|n| !n.muted).map(move |n| IndexedNote {
                        start: n.start.0 + swing_delay(n.start.0, swing),
                        len: n.len.0,
                        key: n.key,
                        velocity: f32::from(n.velocity) / f32::from(u16::MAX),
                        channel,
                        id: n.id.0,
                    })
                })
                .collect();
            notes.sort_by_key(|n| (n.start, n.channel, n.key, n.id));
            channels.extend(notes.iter().map(|n| n.channel));
            let length = loop_length(pattern, bar);
            let max_len = notes.iter().map(|n| n.len).max().unwrap_or(0);
            index_of.insert(*id, patterns.len() as u32);
            pattern_lookup.push((keys::pattern(*id), patterns.len() as u32));
            patterns.push(IndexedPattern { length, notes, max_len });
        }
        channels.sort_unstable();
        channels.dedup();
        pattern_lookup.sort_unstable();
        let mut instance = 0u32;
        let tracks = project
            .track_order
            .iter()
            .filter_map(|id| project.tracks.get(id))
            .filter(|track| !track.muted)
            .map(|track| {
                let clips: Vec<IndexedClip> = track
                    .clips
                    .iter()
                    .filter(|clip| !clip.muted)
                    .filter_map(|clip| {
                        let ClipSource::Pattern(pattern) = clip.source else { return None };
                        let offset = match clip.offset {
                            bodo_model::playlist::ClipOffset::Ticks(t) => t.0,
                            bodo_model::playlist::ClipOffset::Frames(_) => 0,
                        };
                        instance += 1;
                        Some(IndexedClip { start: clip.start.0, end: clip.end().0, offset, pattern: *index_of.get(&pattern)?, instance })
                    })
                    .collect();
                let max_len = clips.iter().map(|c| c.end - c.start).max().unwrap_or(0);
                IndexedTrack { clips, max_len }
            })
            .collect();
        let automation = AutomationIndex::build(project, |pattern| loop_length(pattern, bar));
        Self {
            tempo: (*project.tempo).clone(),
            meter: (*project.meter).clone(),
            patterns,
            pattern_lookup,
            tracks,
            channels,
            audio: AudioIndex::build(project),
            automation,
        }
    }

    /// The tempo map the index was built with.
    pub fn tempo(&self) -> &TempoMap {
        &self.tempo
    }

    /// The meter map the index was built with.
    pub fn meter(&self) -> &MeterMap {
        &self.meter
    }

    /// The index of the pattern with `key`. O(log p).
    fn pattern_index(&self, key: u64) -> Option<usize> {
        let found = self.pattern_lookup.binary_search_by_key(&key, |&(k, _)| k).ok()?;
        self.pattern_lookup.get(found).map(|&(_, index)| index as usize)
    }
}

/// What the transport plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PlayMode {
    /// The playlist.
    #[default]
    Song,
    /// One pattern, looping (project.md section 6.6).
    Pattern(PatternId),
}

/// The transport's state on the audio thread.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transport {
    /// Whether it is playing.
    pub playing: bool,
    /// The playhead, in frames from tick 0.
    pub frame: i64,
    /// What it plays.
    pub mode: PlayMode,
}

/// Adds the note events of the block starting at `transport.frame` and lasting `frames` frames.
///
/// RT-safe. Complexity: O(T log c + sum of (log n + k)).
#[rt_safe]
pub fn emit(song: &SongIndex, transport: &Transport, frames: usize, rate: SampleRate, context: &mut BlockContext) {
    let hz = f64::from(rate.hz());
    let (first, end) = (transport.frame, transport.frame + frames as i64);
    // The tick span is only a coarse filter, widened by a frame each way; each event is then placed by its exact
    // frame, so the result never depends on where blocks begin.
    let window = Window {
        t0: song.tempo.ticks_at(bodo_kit_core::Seconds((first - 1) as f64 / hz)),
        t1: song.tempo.ticks_at(bodo_kit_core::Seconds((end + 1) as f64 / hz)),
        first,
        end,
        hz,
    };
    match transport.mode {
        PlayMode::Song => {
            for track in &song.tracks {
                // Clips whose start is in [t0 - max_len, t1) may overlap the window.
                let earliest = window.t0 - track.max_len as f64;
                let from = track.clips.partition_point(|c| (c.start as f64) <= earliest);
                for clip in track.clips[from..].iter().take_while(|c| (c.start as f64) < window.t1) {
                    if (clip.end as f64) <= window.t0 {
                        continue;
                    }
                    let Some(pattern) = song.patterns.get(clip.pattern as usize) else { continue };
                    let bounds = (clip.start as f64, clip.end as f64);
                    emit_repeats(song, pattern, clip.start - clip.offset, bounds, clip.instance, &window, context);
                }
            }
            song.audio.emit(&song.tempo, first, end, hz, context);
        }
        PlayMode::Pattern(id) => {
            if let Some(pattern) = song.pattern_index(keys::pattern(id)).and_then(|i| song.patterns.get(i)) {
                emit_repeats(song, pattern, 0, (f64::NEG_INFINITY, f64::INFINITY), 0, &window, context);
            }
        }
    }
}

/// The block: a slightly widened span in ticks for searching, and its exact span in frames.
struct Window {
    t0: f64,
    t1: f64,
    first: i64,
    end: i64,
    hz: f64,
}

impl Window {
    /// The offset in this block of the frame `tick` falls on, if it falls in this block.
    fn offset(&self, song: &SongIndex, tick: f64) -> Option<u32> {
        let frame = (song.tempo.seconds_at_ticks(tick).0 * self.hz).round() as i64;
        (frame >= self.first && frame < self.end).then(|| (frame - self.first) as u32)
    }
}

/// Emits the notes of `pattern`, repeating every pattern length from tick `origin`, clipped to `bounds`.
fn emit_repeats(
    song: &SongIndex,
    pattern: &IndexedPattern,
    origin: i64,
    bounds: (f64, f64),
    instance: u32,
    window: &Window,
    context: &mut BlockContext,
) {
    let length = pattern.length.max(1);
    // Repeats that can have a note start or end inside the window.
    let first = ((window.t0 - origin as f64 - pattern.max_len as f64) / length as f64).floor() as i64;
    let last = ((window.t1 - origin as f64) / length as f64).floor() as i64;
    for repeat in first..=last {
        let base = origin + repeat * length;
        let earliest = window.t0 - base as f64 - pattern.max_len as f64;
        let from = pattern.notes.partition_point(|n| (n.start as f64) <= earliest);
        for note in pattern.notes[from..].iter().take_while(|n| ((base + n.start) as f64) < window.t1) {
            if note.start >= length {
                continue;
            }
            let on = (base + note.start) as f64;
            if on < bounds.0 || on >= bounds.1 {
                continue;
            }
            // The clip's end cuts notes short.
            let off = ((base + note.start + note.len) as f64).min(bounds.1);
            let voice = (u64::from(instance) << 40) ^ ((repeat as u64 & 0xFF_FFFF) << 32) ^ u64::from(note.id);
            if let Some(frame) = window.offset(song, on) {
                context.push(note.channel, NoteEvent { frame, kind: EventKind::On, key: note.key, velocity: note.velocity, voice });
            }
            if let Some(frame) = window.offset(song, off) {
                context.push(note.channel, NoteEvent { frame, kind: EventKind::Off, key: note.key, velocity: 0.0, voice });
            }
        }
    }
}

/// Releases every channel's voices at the start of the block, for a stop or a jump.
///
/// RT-safe. Complexity: O(c) in the channels with notes.
#[rt_safe]
pub fn all_notes_off(song: &SongIndex, context: &mut BlockContext) {
    for &channel in &song.channels {
        context.push(channel, NoteEvent { frame: 0, kind: EventKind::AllOff, key: 0, velocity: 0.0, voice: 0 });
    }
}

/// The frame at which `tick` falls, through `song`'s tempo map.
pub fn frame_at(song: &SongIndex, tick: Tick, rate: SampleRate) -> Frames {
    rate.seconds_to_frames(song.tempo.seconds_at(tick))
}
