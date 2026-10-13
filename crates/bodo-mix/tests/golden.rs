//! Golden renders of the native effects: each effect, at its defaults and at telling settings, processes the same
//! second of test signal, and the loudness of every 1024-frame window must match `tests/golden/effects.json`.
//!
//! The fingerprint is a tolerance check, not a hash, so the same effect passes on every platform's math library;
//! it still catches any change to what an effect does. After a deliberate change to an effect, regenerate the file
//! with `BODO_BLESS=1 cargo test -p bodo-mix --test golden` and say why in the commit. Bit-exact behavior where it
//! matters (an effect or strip at its defaults passing audio through untouched) is checked exactly elsewhere.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use bodo_kit_core::SampleRate;
use bodo_mix::fx::{EffectBlock, find};

const RATE: SampleRate = SampleRate::HZ_48000;
const FRAMES: usize = 48_000;
const BLOCK: usize = 256;
const WINDOW: usize = 1024;

/// One second of test signal: a saw on each side (different pitches), with a click every quarter second.
fn signal() -> [Vec<f32>; 2] {
    let click = |i: usize| if i.is_multiple_of(12_000) { 0.8 } else { 0.0 };
    let left = (0..FRAMES).map(|i| ((i % 109) as f32 / 109.0) * 0.5 - 0.25 + click(i)).collect();
    let right = (0..FRAMES).map(|i| ((i % 73) as f32 / 73.0) * 0.4 - 0.2 + click(i)).collect();
    [left, right]
}

/// A sidechain key: bursts of a 100 Hz sine for the first 50 ms of every quarter second.
fn key() -> Vec<f32> {
    (0..FRAMES).map(|i| if i % 12_000 < 2_400 { (i as f32 * 2.0 * core::f32::consts::PI * 100.0 / 48_000.0).sin() * 0.9 } else { 0.0 }).collect()
}

/// Runs effect `uid` with parameters `set` (by id, the rest at their defaults) over the test signal, in blocks.
fn run(uid: &str, set: &[(u32, f32)], sidechain: bool) -> [Vec<f32>; 2] {
    let info = find(uid).unwrap();
    let mut params = info.defaults();
    for &(id, value) in set {
        let index = info.index_of(id).unwrap();
        params[index] = info.params[index].clamp(value);
    }
    let mut effect = (info.create)(RATE, BLOCK);
    let [mut left, mut right] = signal();
    let key = key();
    for start in (0..FRAMES).step_by(BLOCK) {
        let end = (start + BLOCK).min(FRAMES);
        let mut block = EffectBlock {
            left: &mut left[start..end],
            right: &mut right[start..end],
            sidechain: sidechain.then(|| (&key[start..end], &key[start..end])),
            params: &params,
            rate: RATE,
            tempo: 120.0,
        };
        effect.process(&mut block);
    }
    [left, right]
}

/// The RMS of each side over every window.
fn fingerprint(audio: &[Vec<f32>; 2]) -> Vec<[f32; 2]> {
    let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
    (0..FRAMES / WINDOW).map(|w| [rms(&audio[0][w * WINDOW..(w + 1) * WINDOW]), rms(&audio[1][w * WINDOW..(w + 1) * WINDOW])]).collect()
}

/// A case: a name, the effect, its settings by parameter id, and whether a sidechain key plays.
type Case = (&'static str, &'static str, Vec<(u32, f32)>, bool);

/// Every case.
fn cases() -> Vec<Case> {
    vec![
        ("utility-gain-pan-width", "bodo.utility", vec![(0, -6.0), (1, 0.3), (2, 150.0)], false),
        ("utility-polarity-mono", "bodo.utility", vec![(3, 1.0), (4, 1.0)], false),
        ("filter-defaults", "bodo.filter", vec![], false),
        ("filter-low-pass-resonant", "bodo.filter", vec![(0, 0.0), (1, 500.0), (2, 70.0)], false),
        ("filter-high-pass", "bodo.filter", vec![(0, 1.0), (1, 3000.0)], false),
        ("filter-band-pass", "bodo.filter", vec![(0, 2.0), (1, 1200.0), (2, 40.0)], false),
        ("delay-defaults", "bodo.delay", vec![], false),
        ("delay-eighth-ping-pong", "bodo.delay", vec![(0, 2.0), (1, 60.0), (2, 50.0), (3, 1.0), (4, 3000.0)], false),
        ("ducker-keyed", "bodo.ducker", vec![(0, 24.0), (1, -40.0), (2, 1.0), (3, 80.0)], true),
        ("ducker-unkeyed", "bodo.ducker", vec![], false),
        ("latency-test-480", "bodo.latency-test", vec![(0, 480.0)], false),
    ]
}

fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join("effects.json")
}

#[test]
fn native_effects_match_their_golden_renders() {
    let rendered: BTreeMap<String, Vec<[f32; 2]>> =
        cases().into_iter().map(|(name, uid, set, sidechain)| (name.to_owned(), fingerprint(&run(uid, &set, sidechain)))).collect();
    if std::env::var_os("BODO_BLESS").is_some() {
        std::fs::create_dir_all(golden().parent().unwrap()).unwrap();
        // One line per case keeps the file readable and its diffs small.
        let lines: Vec<String> =
            rendered.iter().map(|(name, windows)| format!("  {}: {}", serde_json::json!(name), serde_json::json!(windows))).collect();
        std::fs::write(golden(), format!("{{\n{}\n}}\n", lines.join(",\n"))).unwrap();
    }
    let stored: BTreeMap<String, Vec<[f32; 2]>> = serde_json::from_str(&std::fs::read_to_string(golden()).unwrap()).unwrap();
    assert_eq!(stored.keys().collect::<Vec<_>>(), rendered.keys().collect::<Vec<_>>(), "the cases changed; bless the file");
    for (name, windows) in &rendered {
        for (w, (got, want)) in windows.iter().zip(&stored[name]).enumerate() {
            for side in 0..2 {
                let (got, want) = (got[side], want[side]);
                assert!((got - want).abs() <= 1e-5 + 1e-3 * want.abs(), "{name}, window {w}, side {side}: {got} vs golden {want}");
            }
        }
    }
}

#[test]
fn the_cases_hear_what_their_settings_do() {
    let dry = fingerprint(&signal());
    let level = |name: &str| {
        let (_, uid, set, sidechain) = cases().into_iter().find(|c| c.0 == name).unwrap();
        fingerprint(&run(uid, &set, sidechain))
    };
    let total = |f: &[[f32; 2]]| f.iter().map(|w| w[0] + w[1]).sum::<f32>();
    // The utility at -6 dB is about half as loud; a high pass at 3 kHz takes the saws' fundamentals away.
    assert!((total(&level("utility-gain-pan-width")) / total(&dry) - 0.5).abs() < 0.1);
    assert!(total(&level("filter-high-pass")) < total(&dry) * 0.7);
    // The keyed ducker is quieter than the unkeyed one, which leaves the signal alone.
    assert!(total(&level("ducker-keyed")) < total(&level("ducker-unkeyed")) * 0.9);
    let unkeyed = level("ducker-unkeyed");
    assert!(unkeyed.iter().zip(&dry).all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6));
    // The latency test delays by 480 frames: the first window loses its first 480 frames of signal.
    assert!(level("latency-test-480")[0][0] < dry[0][0]);
}
