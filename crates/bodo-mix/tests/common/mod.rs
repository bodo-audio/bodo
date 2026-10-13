//! Helpers shared by the engine's integration tests.

#![allow(dead_code, clippy::unwrap_used, reason = "shared test helpers; not every test uses every helper")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::{SampleRate, Tick};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::mixer::{Route, RouteKind, SendTap};
use bodo_model::pattern::PatternLen;
use bodo_model::*;

/// A fresh directory for one test's files.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bodo-mix-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a mono 48 kHz sample: a unit impulse followed by silence.
pub fn write_impulse(dir: &Path) {
    let mut samples = vec![0.0f32; 4800];
    samples[0] = 1.0;
    let mut writer = WavWriter::create(&dir.join("impulse.wav"), SampleRate::HZ_48000, 1, WavFormat::Float32).unwrap();
    writer.write(&[&samples], samples.len()).unwrap();
    writer.finish().unwrap();
}

/// Writes a stereo 48 kHz sample: two seconds of a looping saw, different on each side.
pub fn write_tone(dir: &Path) {
    let left: Vec<f32> = (0..96_000).map(|i| ((i % 109) as f32 / 109.0) * 0.5 - 0.25).collect();
    let right: Vec<f32> = (0..96_000).map(|i| ((i % 73) as f32 / 73.0) * 0.4 - 0.2).collect();
    let mut writer = WavWriter::create(&dir.join("tone.wav"), SampleRate::HZ_48000, 2, WavFormat::Float32).unwrap();
    writer.write(&[&left, &right], left.len()).unwrap();
    writer.finish().unwrap();
}

pub struct Builder {
    pub project: Project,
    next: u128,
}

impl Builder {
    pub fn new() -> Self {
        let mut project = Project::new(120.0).unwrap();
        project.id = ProjectId::from_u128(1);
        Self { project, next: 100 }
    }

    pub fn id(&mut self) -> u128 {
        self.next += 1;
        self.next
    }

    pub fn insert(&self, index: usize) -> InsertId {
        self.project.mixer.order[index]
    }

    pub fn sample(&mut self, file: &str) -> SampleId {
        let id = SampleId::from_u128(self.id());
        self.project.samples.insert(id, Arc::new(SampleRef { id, path: file.into(), hash: None, market: None }));
        id
    }

    pub fn sampler(&mut self, sample: SampleId, target: Option<InsertId>, params: SamplerParams) -> ChannelId {
        let id = ChannelId::from_u128(self.id());
        let kind = ChannelKind::Sampler(Arc::new(SamplerParams { sample: Some(sample), ..params }));
        let channel = Channel {
            id,
            name: "Sampler".into(),
            color: Color::GRAY,
            kind,
            target,
            volume: 1.0,
            pan: 0.0,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
        };
        self.project.channels.insert(id, Arc::new(channel));
        self.project.rack_order.push_back(id);
        id
    }

    pub fn pattern(&mut self, length: Option<i64>, notes: &[(ChannelId, u8, i64, i64)]) -> PatternId {
        let id = PatternId::from_u128(self.id());
        let mut pattern = Pattern::new(id, "Pattern", Color::GRAY);
        if let Some(len) = length {
            pattern.length = PatternLen::Fixed(Tick(len));
        }
        for (i, &(channel, key, start, len)) in notes.iter().enumerate() {
            let note = Note::new(NoteId(i as u32), key, Tick(start), Tick(len));
            pattern.notes.entry(channel).or_default().insert(note);
        }
        pattern.next_note_id = notes.len() as u32;
        self.project.patterns.insert(id, Arc::new(pattern));
        id
    }

    pub fn clip(&mut self, pattern: PatternId, start: i64, len: i64, offset: i64) {
        let track_id = match self.project.track_order.front() {
            Some(&id) => id,
            None => {
                let id = TrackId::from_u128(self.id());
                self.project.tracks.insert(id, Arc::new(PlaylistTrack::new(id, "Track", Color::GRAY)));
                self.project.track_order.push_back(id);
                id
            }
        };
        let clip = Clip {
            id: ClipId::from_u128(self.id()),
            start: Tick(start),
            len: Tick(len),
            offset: bodo_model::playlist::ClipOffset::Ticks(Tick(offset)),
            muted: false,
            color: None,
            source: ClipSource::Pattern(pattern),
            fades: None,
        };
        let mut track = (*self.project.tracks[&track_id]).clone();
        track.insert_clip(clip);
        self.project.tracks.insert(track_id, Arc::new(track));
    }

    pub fn route(&mut self, from: InsertId, to: InsertId, level: f32) {
        let mut mixer = (*self.project.mixer).clone();
        mixer
            .routes
            .insert(RouteKey { from: RouteSource::Insert(from), to }, Route { send_level: level, tap: SendTap::PostFader, kind: RouteKind::Audio });
        self.project.mixer = Arc::new(mixer);
    }

    pub fn build(self) -> Project {
        self.project.validate().unwrap();
        self.project
    }
}
