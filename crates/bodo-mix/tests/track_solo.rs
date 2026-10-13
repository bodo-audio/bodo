//! Soloing playlist tracks (project.md section 10.3): while any track is soloed, only soloed tracks play, and a muted
//! track stays silent even when soloed.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::sync::Arc;

use bodo_kit_core::{Seconds, Tick};
use bodo_mix::{OfflineOptions, render_offline};
use bodo_model::channel::SamplerParams;
use bodo_model::playlist::PlaylistTrack;
use bodo_model::*;
use common::{Builder, temp_dir, write_impulse};

/// An impulse at 0 on the first track and one at a quarter note on a second track, with the tracks' solo and mute
/// set as given.
fn two_tracks(first: (bool, bool), second: (bool, bool)) -> Project {
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let insert = b.insert(0);
    let channel = b.sampler(sample, Some(insert), SamplerParams::default());
    let early = b.pattern(Some(3840), &[(channel, 60, 0, 240)]);
    let late = b.pattern(Some(3840), &[(channel, 60, 960, 240)]);
    b.clip(early, 0, 3840, 0);
    b.clip(late, 0, 3840, 0);
    let mut project = b.build();
    // Move the second clip onto a track of its own.
    let first_id = project.track_order[0];
    let mut track = (*project.tracks[&first_id]).clone();
    let moved = track.clips.iter().position(|c| c.source == bodo_model::playlist::ClipSource::Pattern(late)).unwrap();
    let clip = track.clips.remove(moved);
    (track.solo, track.muted) = first;
    project.tracks.insert(first_id, Arc::new(track));
    let second_id = TrackId::from_u128(9_999);
    let mut other = PlaylistTrack::new(second_id, "Second", Color::GRAY);
    other.insert_clip(clip);
    (other.solo, other.muted) = second;
    project.tracks.insert(second_id, Arc::new(other));
    project.track_order.push_back(second_id);
    project.validate().unwrap();
    project
}

fn onsets(project: &Project) -> Vec<usize> {
    let dir = temp_dir("track-solo");
    write_impulse(&dir);
    let out = render_offline(project, &SampleRoots::new(&dir), &OfflineOptions { end: Tick(3840), tail: Seconds(0.0), ..OfflineOptions::default() })
        .unwrap()
        .channels;
    out[0].iter().enumerate().filter(|(_, s)| s.abs() > 1e-9).map(|(i, _)| i).collect()
}

#[test]
fn soloed_tracks_play_alone() {
    assert_eq!(onsets(&two_tracks((false, false), (false, false))), [0, 24_000], "both play without solo");
    assert_eq!(onsets(&two_tracks((false, false), (true, false))), [24_000], "only the soloed track");
    assert_eq!(onsets(&two_tracks((true, false), (true, false))), [0, 24_000], "both soloed");
    assert!(onsets(&two_tracks((false, false), (true, true))).is_empty(), "a muted soloed track stays silent, and silences the rest");
}
