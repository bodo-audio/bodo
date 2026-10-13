//! Checks that playing audio clips never allocates on the audio side, also while a clip moves and the song index
//! switches. Debug builds only; the RealtimeSanitizer job covers every build.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_core::Tick;
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, PlayMode, engine};
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::playlist::{ClipOffset, ClipSource, Fades};
use bodo_model::*;
use common::{Builder, temp_dir, write_tone};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn audio_clips_play_without_allocating() {
    let dir = temp_dir("rt-audio");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let track = TrackId::from_u128(b.id());
    let mut playlist = PlaylistTrack::new(track, "Track", Color::GRAY);
    for (n, mode) in [StretchMode::None, StretchMode::Resample, StretchMode::Stretch].into_iter().enumerate() {
        let id = ChannelId::from_u128(b.id());
        let kind = ChannelKind::AudioClip { sample, stretch: mode, tempo: Some(100.0) };
        let channel = Channel {
            id,
            name: format!("Audio {n}"),
            color: Color::GRAY,
            kind,
            target: Some(b.insert(n)),
            volume: 0.8,
            pan: -0.3,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
        };
        b.project.channels.insert(id, Arc::new(channel));
        b.project.rack_order.push_back(id);
        playlist.insert_clip(Clip {
            id: ClipId::from_u128(b.id()),
            start: Tick(n as i64 * 960),
            len: Tick(3840),
            offset: ClipOffset::Ticks(Tick(120)),
            muted: false,
            color: None,
            source: ClipSource::Audio(id),
            fades: Some(Fades { fade_in: Tick(240), fade_out: Tick(480) }),
        });
    }
    b.project.tracks.insert(track, Arc::new(playlist));
    b.project.track_order.push_back(track);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
    control.set_mode(PlayMode::Song);
    control.play();
    let mut buffer = AudioBuffer::new(2, 128);
    let mut render = |audio: &mut bodo_mix::AudioEngine| {
        reset_violation_count();
        assert_no_alloc(|| {
            for _ in 0..300 {
                audio.render(128, &mut buffer);
            }
        });
        assert_eq!(violation_count(), 0);
    };
    render(&mut audio);

    // A clip moves while playing: a new song index arrives.
    let mut moved = project.clone();
    let mut playlist = (*moved.tracks[&track]).clone();
    if let Some(clip) = playlist.clips.get_mut(0) {
        clip.start = Tick(480);
    }
    moved.tracks.insert(track, Arc::new(playlist));
    control.set_project(&moved).unwrap();
    render(&mut audio);
    control.seek(Tick(1000));
    render(&mut audio);
    control.stop();
    render(&mut audio);
    assert_eq!(audio.faults(), 0);
}
