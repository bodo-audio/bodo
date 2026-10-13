//! Text-first summaries of a project for agents and people (project.md section 12.3).
//!
//! Each summary comes as compact text, which reads well in a terminal or an agent's context, and as JSON with the ids,
//! for programs. Steps show as strings such as `x...x...x...x...`, positions as `bar:beat:tick` counted from 1.

use std::collections::BTreeMap;

use bodo_kit_core::Tick;
use bodo_model::channel::ChannelKind;
use bodo_model::mixer::{RouteKind, RouteSource};
use bodo_model::playlist::ClipSource;
use bodo_model::{ChannelId, InsertId, Pattern, Project};
use serde_json::{Value, json};

use crate::commands::{insert_number, pattern_length, pattern_step_text};
use crate::error::CommandError;
use crate::params::{PatternRef, STEP, format_length, format_position, key_name};
use crate::resolve;
use crate::session::Session;
use crate::state::PlayMode;

/// A summary as text and as JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    /// Compact text.
    pub text: String,
    /// The same facts as JSON, with ids.
    pub data: Value,
}

fn channel_name(project: &Project, id: ChannelId) -> String {
    project.channels.get(&id).map_or_else(|| id.to_string(), |c| c.name.clone())
}

fn insert_label(project: &Project, id: InsertId) -> String {
    match insert_number(project, id) {
        Some(0) => String::from("master"),
        Some(n) => format!("insert {n}"),
        None => id.to_string(),
    }
}

fn source_label(project: &Project, source: RouteSource) -> String {
    match source {
        RouteSource::Channel(c) => format!("channel {}", channel_name(project, c)),
        RouteSource::Insert(i) => insert_label(project, i),
    }
}

/// Whether every note of channel `channel` in `pattern` sits on the step grid, so a step string shows it all.
fn on_grid(pattern: &Pattern, channel: ChannelId) -> bool {
    pattern.notes.get(&channel).is_some_and(|seq| {
        let first = seq.notes().front().map(|n| n.key);
        seq.notes().iter().all(|n| n.start.0 % STEP.0 == 0 && Some(n.key) == first && n.len <= STEP)
    })
}

/// The end of the last clip on the playlist.
///
/// Complexity: O(n) in clips.
fn song_end(project: &Project) -> Tick {
    project.tracks.values().flat_map(|t| t.clips.iter().map(|c| c.end())).max().unwrap_or(Tick::ZERO)
}

/// The whole project: transport, channels, patterns with their steps, the playlist and the mixer's changes.
///
/// Complexity: O(N log N) in channels, notes and clips.
pub fn project(session: &Session) -> Inspection {
    let project = session.project();
    let editor = session.editor();
    let transport = session.transport();
    let mut text = String::new();
    let mut line = |s: String| {
        text.push_str(&s);
        text.push('\n');
    };
    let title = if project.meta.title.is_empty() { "Untitled" } else { project.meta.title.as_str() };
    let meter = project.meter.signature_at(Tick::ZERO);
    let bpm = project.tempo.bpm_at(Tick::ZERO);
    let file = session.file().map(|p| p.display().to_string());
    line(format!(
        "Project \"{title}\": {bpm} BPM, {}/{}, {} Hz; file: {}{}",
        meter.numerator,
        meter.denominator,
        project.settings.sample_rate.hz(),
        file.as_deref().unwrap_or("none"),
        if session.is_dirty() { " (unsaved changes)" } else { "" }
    ));
    let current = editor.pattern.and_then(|p| project.patterns.get(&p)).map(|p| p.name.clone());
    line(format!(
        "Transport: {} at {}, {} mode; current pattern: {}",
        if transport.playing { "playing" } else { "stopped" },
        format_position(project, transport.position),
        match transport.mode {
            PlayMode::Pattern => "pattern",
            PlayMode::Song => "song",
        },
        current.as_deref().unwrap_or("none")
    ));

    line(format!("Channels ({}):", project.rack_order.len()));
    let mut channels = Vec::new();
    for (n, id) in project.rack_order.iter().enumerate() {
        let Some(channel) = project.channels.get(id) else { continue };
        let (kind, sample) = match &channel.kind {
            ChannelKind::Sampler(params) => ("sampler", params.sample.and_then(|s| project.samples.get(&s)).map(|s| s.path.clone())),
            ChannelKind::Instrument { .. } => ("instrument", None),
            ChannelKind::AudioClip { sample, .. } => ("audio clip", project.samples.get(sample).map(|s| s.path.clone())),
            ChannelKind::Automation(_) => ("automation", None),
            ChannelKind::Layer { .. } => ("layer", None),
            ChannelKind::MidiOut { .. } => ("MIDI out", None),
        };
        let target = channel.target.map_or_else(|| String::from("master"), |t| insert_label(project, t));
        line(format!(
            "  {} {}: {kind}{} -> {target}, volume {:.2}, pan {:+.2}{}",
            n + 1,
            channel.name,
            sample.as_ref().map(|s| format!(" {s}")).unwrap_or_default(),
            channel.volume,
            channel.pan,
            if channel.muted { ", muted" } else { "" }
        ));
        channels.push(json!({
            "number": n + 1, "id": channel.id, "name": channel.name, "kind": kind, "sample": sample,
            "insert": channel.target.and_then(|t| insert_number(project, t)).unwrap_or(0),
            "volume": channel.volume, "pan": channel.pan, "muted": channel.muted, "color": channel.color,
        }));
    }

    line(format!("Patterns ({}):", project.patterns.len()));
    let mut patterns = Vec::new();
    for (n, pattern) in project.patterns.values().enumerate() {
        let length = pattern_length(project, pattern.id);
        let notes: usize = pattern.notes.values().map(|s| s.len()).sum();
        let marker = if editor.pattern == Some(pattern.id) { " (current)" } else { "" };
        line(format!("  {} {}{marker}: {}, {notes} notes", n + 1, pattern.name, format_length(project, length)));
        let mut rows = serde_json::Map::new();
        for id in project.rack_order.iter().filter(|c| pattern.notes.contains_key(*c)) {
            let name = channel_name(project, *id);
            if on_grid(pattern, *id) {
                let steps = pattern_step_text(project, pattern, *id);
                line(format!("      {name:<12} {steps}"));
                rows.insert(name, json!(steps));
            } else if let Some(seq) = pattern.notes.get(id) {
                let low = seq.notes().iter().map(|n| n.key).min().unwrap_or(0);
                let high = seq.notes().iter().map(|n| n.key).max().unwrap_or(0);
                line(format!("      {name:<12} {} notes, {} to {}", seq.len(), key_name(low), key_name(high)));
                rows.insert(name, json!({ "notes": seq.len(), "low": key_name(low), "high": key_name(high) }));
            }
        }
        patterns.push(json!({
            "number": n + 1, "id": pattern.id, "name": pattern.name, "length": format_length(project, length),
            "notes": notes, "channels": rows, "current": editor.pattern == Some(pattern.id),
        }));
    }

    let end = song_end(project);
    line(format!(
        "Playlist ({} tracks, song length {}):",
        project.track_order.len(),
        if end > Tick::ZERO { format_length(project, end) } else { String::from("empty") }
    ));
    let mut tracks = Vec::new();
    let mut empty = 0;
    for (n, id) in project.track_order.iter().enumerate() {
        let Some(track) = project.tracks.get(id) else { continue };
        if track.clips.is_empty() && !track.muted {
            empty += 1;
            continue;
        }
        let clips: Vec<(String, Value)> = track
            .clips
            .iter()
            .map(|clip| {
                let source = match clip.source {
                    ClipSource::Pattern(p) => project.patterns.get(&p).map_or_else(|| p.to_string(), |p| p.name.clone()),
                    ClipSource::Audio(c) | ClipSource::Automation(c) => channel_name(project, c),
                };
                let at = format_position(project, clip.start);
                let text = format!("{source} at {at} ({}){}", format_length(project, clip.len), if clip.muted { " muted" } else { "" });
                (text, json!({ "id": clip.id, "source": source, "start": at, "length": format_length(project, clip.len), "muted": clip.muted }))
            })
            .collect();
        line(format!(
            "  {} {}{}: {}",
            n + 1,
            track.name,
            if track.muted { " (muted)" } else { "" },
            clips.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(", ")
        ));
        tracks.push(json!({ "number": n + 1, "id": track.id, "name": track.name, "muted": track.muted, "clips": clips.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }));
    }
    if empty > 0 {
        line(format!("  ({empty} empty tracks)"));
    }
    let mixer = mixer(session);
    text.push_str(&mixer.text);
    Inspection {
        text,
        data: json!({
            "title": project.meta.title, "file": file, "dirty": session.is_dirty(), "bpm": bpm,
            "meter": format!("{}/{}", meter.numerator, meter.denominator), "sampleRate": project.settings.sample_rate.hz(),
            "transport": { "playing": transport.playing, "mode": transport.mode, "position": format_position(project, transport.position) },
            "currentPattern": editor.pattern, "channels": channels, "patterns": patterns, "tracks": tracks,
            "songLength": format_length(project, end.max(Tick(1))), "mixer": mixer.data,
        }),
    }
}

/// One pattern: its length and every note with id, key, position, length and velocity.
///
/// Complexity: O(n log n) in the pattern's notes.
pub fn pattern(session: &Session, r: Option<&PatternRef>) -> Result<Inspection, CommandError> {
    let project = session.project();
    let id = match r {
        Some(r) => resolve::pattern(project, r)?,
        None => session.editor().pattern.ok_or_else(|| CommandError::nothing_selected("pattern"))?,
    };
    let pattern = project.patterns.get(&id).ok_or_else(|| CommandError::not_found("pattern", &id.to_string()))?;
    let length = pattern_length(project, id);
    let mut text = format!("Pattern \"{}\": {}\n", pattern.name, format_length(project, length));
    let mut by_channel = BTreeMap::new();
    for channel in project.rack_order.iter().filter(|c| pattern.notes.contains_key(*c)) {
        let Some(seq) = pattern.notes.get(channel) else { continue };
        let name = channel_name(project, *channel);
        text.push_str(&format!("  {name}: steps {}\n", pattern_step_text(project, pattern, *channel)));
        let notes: Vec<Value> = seq
            .notes()
            .iter()
            .map(|n| {
                let velocity = f32::from(n.velocity) / f32::from(u16::MAX);
                text.push_str(&format!(
                    "    #{} {} at {} for {}, velocity {velocity:.2}{}\n",
                    n.id.0,
                    key_name(n.key),
                    format_position(project, n.start),
                    format_length(project, n.len),
                    if n.muted { ", muted" } else { "" }
                ));
                json!({ "id": n.id.0, "key": n.key, "note": key_name(n.key), "start": format_position(project, n.start), "ticks": n.start.0, "length": n.len.0, "velocity": velocity, "muted": n.muted })
            })
            .collect();
        by_channel.insert(name, json!({ "channel": channel, "steps": pattern_step_text(project, pattern, *channel), "notes": notes }));
    }
    if by_channel.is_empty() {
        text.push_str("  (no notes)\n");
    }
    Ok(Inspection {
        text,
        data: json!({ "id": id, "name": pattern.name, "length": format_length(project, length), "ticks": length.0, "channels": by_channel }),
    })
}

/// The mixer: the master, every insert that differs from the defaults or has channels, and the routes.
///
/// Complexity: O(i + c + r) in inserts, channels and routes.
pub fn mixer(session: &Session) -> Inspection {
    let project = session.project();
    let mixer = &project.mixer;
    let mut text =
        format!("Mixer: master {:.1} dB{}, {} inserts\n", mixer.master.volume_db, if mixer.master.muted { " muted" } else { "" }, mixer.order.len());
    let mut inserts = Vec::new();
    for (n, id) in mixer.order.iter().enumerate() {
        let Some(insert) = mixer.inserts.get(id) else { continue };
        let fed: Vec<String> =
            project.rack_order.iter().filter_map(|c| project.channels.get(c)).filter(|c| c.target == Some(*id)).map(|c| c.name.clone()).collect();
        let default =
            insert.volume_db == 0.0 && insert.pan == 0.0 && insert.stereo_sep == 0.0 && !insert.muted && !insert.polarity_invert && !insert.swap_lr;
        if default && fed.is_empty() {
            continue;
        }
        let mut parts = vec![format!("{:.1} dB", insert.volume_db)];
        if insert.pan != 0.0 {
            parts.push(format!("pan {:+.2}", insert.pan));
        }
        if insert.stereo_sep != 0.0 {
            parts.push(format!("separation {:+.2}", insert.stereo_sep));
        }
        for (on, word) in [(insert.muted, "muted"), (insert.polarity_invert, "polarity inverted"), (insert.swap_lr, "left/right swapped")] {
            if on {
                parts.push(word.to_owned());
            }
        }
        if !fed.is_empty() {
            parts.push(format!("from {}", fed.join(", ")));
        }
        text.push_str(&format!("  insert {} {}: {}\n", n + 1, insert.name, parts.join(", ")));
        inserts.push(json!({ "number": n + 1, "id": insert.id, "name": insert.name, "db": insert.volume_db, "pan": insert.pan, "muted": insert.muted, "channels": fed }));
    }
    let routes: Vec<Value> = mixer
        .routes
        .iter()
        .map(|(key, route)| {
            let kind = if route.kind == RouteKind::Sidechain { " (sidechain)" } else { "" };
            text.push_str(&format!("  route {} -> {} at {:.2}{kind}\n", source_label(project, key.from), insert_label(project, key.to), route.send_level));
            json!({ "from": source_label(project, key.from), "to": insert_label(project, key.to), "level": route.send_level, "sidechain": !kind.is_empty() })
        })
        .collect();
    Inspection {
        text,
        data: json!({ "master": { "db": mixer.master.volume_db, "muted": mixer.master.muted }, "inserts": inserts, "routes": routes }),
    }
}
