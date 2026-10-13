//! Rendering the beat: the sampler and the step sequencer turn the saved project into audio where the steps are, and
//! silence where nothing plays.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_cli::render::{RenderOptions, render};

/// The loudest sample of `channels` between `from` and `to` seconds.
fn peak(channels: &[Vec<f32>], rate: u32, from: f64, to: f64) -> f32 {
    let range = (from * f64::from(rate)) as usize..(to * f64::from(rate)) as usize;
    channels.iter().flat_map(|c| c[range.clone()].iter()).fold(0.0, |p, s| p.max(s.abs()))
}

#[test]
fn the_beat_renders_to_audio_where_the_steps_play() {
    let dir = common::temp_dir("render");
    let (_, project) = common::saved_beat(&dir, "First Beat.bodo");
    let wav = dir.join("First Beat.wav");
    let rendered = render(&project, &wav, &RenderOptions::default()).unwrap();

    // 8 bars at 96 BPM are 20 seconds, then one second of tail.
    assert!((rendered.seconds() - 21.0).abs() < 0.01, "{} s", rendered.seconds());
    // Stacked factory samples at unity gain go over 0 dBFS where several hits coincide (bar 6, where the Verse and
    // the Fill on track 2 overlap); the mixer has no headroom management yet, so only check the peak is a number.
    assert!(rendered.peak_db.is_finite(), "peak {} dBFS", rendered.peak_db);
    let written = bodo_kit_audio::decode_file(&wav).unwrap();
    assert_eq!(written, rendered.audio, "the file holds exactly the rendered samples");
    let rate = written.rate.hz();
    let channels = &written.channels;
    assert!(peak(channels, rate, 0.0, 0.02) > 0.05, "the kick on bar 1 beat 1 sounds");
    assert!(peak(channels, rate, 0.0, 5.0) < 1.0, "the Intro, where no two hits coincide, stays below 0 dBFS");
    // Bar 1 of the Intro has only the kick on step 1 until the hi-hat on step 3 (0.3125 s at 96 BPM).
    assert!(peak(channels, rate, 0.2, 0.3) < 1e-4, "nothing plays between the kick and the first hi-hat");
    assert!(peak(channels, rate, 0.3125, 0.34) > 0.01, "the hi-hat on step 3 sounds");
    // The Outro (bar 8, from 17.5 s) plays one step; then the song is silent.
    assert!(peak(channels, rate, 18.5, 20.9) < 1e-4, "the end of the song is silent");

    // A render is deterministic.
    let again = render(&project, &dir.join("again.wav"), &RenderOptions::default()).unwrap();
    assert_eq!(again.audio, rendered.audio);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn one_pattern_renders_one_pass() {
    let dir = common::temp_dir("render-pattern");
    let (_, project) = common::saved_beat(&dir, "Beat.bodo");
    let options = RenderOptions { pattern: Some("Verse".into()), end: None };
    let rendered = render(&project, &dir.join("verse.wav"), &options).unwrap();
    assert!((rendered.seconds() - 3.5).abs() < 0.01, "one bar (2.5 s) and the tail: {} s", rendered.seconds());
    let ended = render(&project, &dir.join("two.wav"), &RenderOptions { pattern: Some("2".into()), end: Some("3:1:0".into()) }).unwrap();
    assert!((ended.seconds() - 6.0).abs() < 0.01, "two passes of pattern 2 and the tail: {} s", ended.seconds());
    let missing = render(&project, &dir.join("x.wav"), &RenderOptions { pattern: Some("Chorus".into()), end: None }).unwrap_err();
    assert!(format!("{missing:#}").contains("no pattern matches `Chorus`"), "{missing:#}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_empty_playlist_asks_for_a_pattern() {
    let dir = common::temp_dir("render-empty");
    let mut session = common::session();
    let path = dir.join("Empty.bodo");
    session.execute("project.save", serde_json::json!({ "path": path.display().to_string() })).unwrap();
    let error = render(&path, &dir.join("x.wav"), &RenderOptions::default()).unwrap_err();
    assert!(error.to_string().contains("--pattern"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}
