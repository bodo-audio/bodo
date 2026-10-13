//! Tests for the project model: invariants, serialization and structural sharing.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

use std::sync::Arc;

use bodo_kit_core::{TempoMap, Tick};
use bodo_model::automation::{AutoPoint, AutomationLane, CurveShape, InsertParam, ParamTarget};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::mixer::{Route, RouteKind, SendTap};
use bodo_model::pattern::PatternLen;
use bodo_model::project::{Marker, Meta, Settings};
use bodo_model::*;
use imbl::{OrdMap, Vector};
use proptest::prelude::*;

/// Ids from a counter, so fixtures are reproducible.
struct Ids(u128);

impl Ids {
    fn next(&mut self) -> u128 {
        self.0 += 1;
        self.0
    }
}

fn channel(id: ChannelId, name: &str, kind: ChannelKind, target: Option<InsertId>) -> Arc<Channel> {
    Arc::new(Channel {
        id,
        name: name.into(),
        color: Color::rgb(0xf8, 0x71, 0x71),
        kind,
        target,
        volume: 1.0,
        pan: 0.0,
        pitch_cents: 0.0,
        swing: 0.0,
        muted: false,
        midi_in: None,
    })
}

/// A small but complete project: two sampler channels, an automation channel, a pattern, a track with a pattern
/// clip and an automation clip, three inserts with an audio route and a sidechain, and markers.
fn fixture() -> Project {
    let mut ids = Ids(0);
    let master = InsertId::from_u128(ids.next());
    let inserts: Vec<InsertId> = (0..3).map(|_| InsertId::from_u128(ids.next())).collect();
    let mut mixer = Mixer::with_inserts(master, inserts.clone());
    mixer.routes.insert(
        RouteKey { from: RouteSource::Insert(inserts[0]), to: inserts[2] },
        Route { send_level: 0.5, tap: SendTap::PostFader, kind: RouteKind::Audio },
    );
    mixer.routes.insert(
        RouteKey { from: RouteSource::Insert(inserts[1]), to: inserts[2] },
        Route { send_level: 1.0, tap: SendTap::PreFader, kind: RouteKind::Sidechain },
    );
    let mut ducked = (**mixer.inserts.get(&inserts[2]).unwrap()).clone();
    ducked.slots[0].sidechain = Some(RouteSource::Insert(inserts[1]));
    mixer.inserts.insert(inserts[2], Arc::new(ducked));

    let sample = SampleId::from_u128(ids.next());
    let kick = ChannelId::from_u128(ids.next());
    let clap = ChannelId::from_u128(ids.next());
    let auto = ChannelId::from_u128(ids.next());
    let sampler = |s| ChannelKind::Sampler(Arc::new(SamplerParams { sample: Some(s), ..SamplerParams::default() }));
    let lane = AutomationLane {
        target: ParamTarget::Insert(inserts[0], InsertParam::Volume),
        points: Vector::from(vec![
            AutoPoint { at: Tick(0), value: 0.2, curve: CurveShape::Exp(0.3) },
            AutoPoint { at: Tick(3840), value: 0.9, curve: CurveShape::Hold },
        ]),
        default: 0.8,
    };
    let channels: OrdMap<ChannelId, Arc<Channel>> = [
        (kick, channel(kick, "808 Kick", sampler(sample), Some(inserts[0]))),
        (clap, channel(clap, "808 Clap", sampler(sample), Some(inserts[1]))),
        (auto, channel(auto, "Fader ride", ChannelKind::Automation(Arc::new(lane)), None)),
    ]
    .into_iter()
    .collect();

    let pattern_id = PatternId::from_u128(ids.next());
    let mut pattern = Pattern::new(pattern_id, "Beat", Color::rgb(0x4a, 0xde, 0x80));
    pattern.length = PatternLen::Fixed(Tick(3840));
    pattern.notes.insert(kick, NoteSeq::new((0..4).map(|i| Note::new(NoteId(i), 60, Tick(i64::from(i) * 960), Tick(240)))));
    pattern.notes.insert(clap, NoteSeq::new([Note::new(NoteId(4), 60, Tick(960), Tick(240)), Note::new(NoteId(5), 60, Tick(2880), Tick(240))]));
    pattern.next_note_id = 6;

    let track_id = TrackId::from_u128(ids.next());
    let mut track = PlaylistTrack::new(track_id, "Track 1", Color::GRAY);
    for bar in 0..4 {
        track.insert_clip(Clip {
            id: ClipId::from_u128(ids.next()),
            start: Tick(bar * 3840),
            len: Tick(3840),
            offset: Default::default(),
            muted: false,
            color: None,
            source: ClipSource::Pattern(pattern_id),
            fades: None,
        });
    }
    track.insert_clip(Clip {
        id: ClipId::from_u128(ids.next()),
        start: Tick(0),
        len: Tick(15_360),
        offset: Default::default(),
        muted: false,
        color: None,
        source: ClipSource::Automation(auto),
        fades: None,
    });

    Project {
        id: ProjectId::from_u128(ids.next()),
        meta: Arc::new(Meta { title: "Fixture".into(), ..Meta::default() }),
        settings: Arc::new(Settings::default()),
        tempo: Arc::new(TempoMap::constant(130.0).unwrap()),
        meter: Arc::new(MeterMap::common_time()),
        rack_order: Vector::from(vec![kick, clap, auto]),
        channels,
        patterns: [(pattern_id, Arc::new(pattern))].into_iter().collect(),
        track_order: Vector::from(vec![track_id]),
        tracks: [(track_id, Arc::new(track))].into_iter().collect(),
        mixer: Arc::new(mixer),
        samples: [(sample, Arc::new(SampleRef { id: sample, path: "Samples/kick.wav".into(), hash: None, market: None }))].into_iter().collect(),
        markers: Vector::from(vec![
            Marker { at: Tick(0), name: "Intro".into(), color: None },
            Marker { at: Tick(7680), name: "Drop".into(), color: None },
        ]),
    }
}

fn violations(project: &Project) -> Vec<Violation> {
    project.validate().err().map(|e| e.0).unwrap_or_default()
}

#[test]
fn the_fixture_is_valid_and_round_trips_through_json() {
    let project = fixture();
    project.validate().unwrap();
    let json = serde_json::to_string_pretty(&project).unwrap();
    assert!(json.contains("\"rackOrder\""), "fields are camelCase");
    assert!(json.contains("\"ch_"), "ids carry their prefix");
    assert!(json.contains("\"sendLevel\""), "routes serialize as a list");
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(back, project);
}

#[test]
fn a_new_project_is_valid() {
    let project = Project::new(120.0).unwrap();
    project.validate().unwrap();
    assert_eq!(project.mixer.order.len(), 16);
}

#[test]
fn edits_share_everything_they_do_not_touch() {
    let before = fixture();
    let mut after = before.clone();
    let (&pattern_id, pattern) = after.patterns.iter().next().unwrap();
    let mut pattern = (**pattern).clone();
    pattern.name = "Renamed".into();
    after.patterns.insert(pattern_id, Arc::new(pattern));
    assert!(Arc::ptr_eq(&before.mixer, &after.mixer));
    let (a, b) = (before.channels.values().next().unwrap(), after.channels.values().next().unwrap());
    assert!(Arc::ptr_eq(a, b));
    assert_ne!(before.patterns[&pattern_id].name, after.patterns[&pattern_id].name);
}

#[test]
fn every_invariant_is_checked() {
    let base = fixture();
    let kick = base.rack_order[0];
    let clap = base.rack_order[1];
    let auto = base.rack_order[2];
    let (&pattern_id, pattern) = base.patterns.iter().next().unwrap();
    let inserts: Vec<InsertId> = base.mixer.order.iter().copied().collect();

    // Order lists.
    let mut p = base.clone();
    p.rack_order.push_back(kick);
    assert_eq!(violations(&p), [Violation::Order { list: "rackOrder" }]);

    // A dangling channel target.
    let mut p = base.clone();
    let missing = InsertId::from_u128(999);
    let mut c = (*p.channels[&kick]).clone();
    c.target = Some(missing);
    p.channels.insert(kick, Arc::new(c));
    assert!(matches!(&violations(&p)[..], [Violation::Dangling { target, .. }] if *target == missing.to_string()));

    // Unsorted notes, a bad note id, and a clip-mode binding with a foreign channel.
    let mut p = base.clone();
    let mut pat = (**pattern).clone();
    let mut seq: Vec<Note> = pat.notes[&kick].notes().iter().cloned().collect();
    seq.swap(0, 1);
    pat.notes.insert(kick, NoteSeq::from(Vector::from(seq)));
    pat.binding = Some(kick);
    pat.next_note_id = 3;
    p.patterns.insert(pattern_id, Arc::new(pat));
    let found = violations(&p);
    assert!(found.iter().any(|v| matches!(v, Violation::Unsorted { .. })), "{found:?}");
    assert!(found.iter().any(|v| matches!(v, Violation::BindingBroken { other, .. } if *other == clap.to_string())), "{found:?}");
    assert!(found.iter().any(|v| matches!(v, Violation::BadNoteId { note: 3, .. })), "{found:?}");

    // A clip with no length, and an automation clip that plays a sampler.
    let mut p = base.clone();
    let (&track_id, track) = p.tracks.iter().next().unwrap();
    let mut track = (**track).clone();
    track.clips[0].len = Tick(0);
    track.clips[1].source = ClipSource::Automation(kick);
    p.tracks.insert(track_id, Arc::new(track));
    let found = violations(&p);
    assert!(found.iter().any(|v| matches!(v, Violation::NonPositiveLength { .. })), "{found:?}");
    assert!(found.iter().any(|v| matches!(v, Violation::WrongClipSource { .. })), "{found:?}");
    let _ = auto;

    // A routing cycle names its inserts.
    let mut p = base.clone();
    let mut mixer = (*p.mixer).clone();
    mixer.routes.insert(
        RouteKey { from: RouteSource::Insert(inserts[2]), to: inserts[0] },
        Route { send_level: 1.0, tap: SendTap::PostFader, kind: RouteKind::Audio },
    );
    p.mixer = Arc::new(mixer);
    let found = violations(&p);
    assert!(matches!(&found[..], [Violation::Cycle { path }] if path.len() == 2), "{found:?}");

    // Routes to the master or to the same insert, and a sidechain without its route.
    let mut p = base.clone();
    let mut mixer = (*p.mixer).clone();
    let master = mixer.master.id;
    mixer.routes.insert(
        RouteKey { from: RouteSource::Insert(inserts[0]), to: master },
        Route { send_level: 1.0, tap: SendTap::PostFader, kind: RouteKind::Audio },
    );
    mixer.routes.remove(&RouteKey { from: RouteSource::Insert(inserts[1]), to: inserts[2] });
    p.mixer = Arc::new(mixer);
    let found = violations(&p);
    assert!(found.iter().any(|v| matches!(v, Violation::BadRoute { .. })), "{found:?}");
    assert!(found.iter().any(|v| matches!(v, Violation::SidechainWithoutRoute { slot: 1, .. })), "{found:?}");

    // A layer that contains itself.
    let mut p = base.clone();
    let layer = ChannelId::from_u128(500);
    p.channels.insert(layer, channel(layer, "Layer", ChannelKind::Layer { children: Vector::from(vec![kick, layer]) }, None));
    p.rack_order.push_back(layer);
    assert!(matches!(&violations(&p)[..], [Violation::LayerCycle { .. }]));
}

/// A random but valid project: routes only go from lower to higher inserts, so the routing is acyclic.
fn random_project() -> impl Strategy<Value = Project> {
    (
        1usize..6,
        1usize..5,
        0usize..4,
        prop::collection::vec((0usize..8, 0usize..8, 0.0f32..1.0), 0..12),
        prop::collection::vec((0u8..128, 0i64..7680, 1i64..960), 0..40),
    )
        .prop_map(|(channel_count, insert_count, track_count, routes, notes)| {
            let mut project = fixture();
            let mut mixer = Mixer::with_inserts(InsertId::from_u128(10_000), (0..insert_count).map(|i| InsertId::from_u128(10_001 + i as u128)));
            let inserts: Vec<InsertId> = mixer.order.iter().copied().collect();
            for (a, b, level) in routes {
                let (a, b) = (a % insert_count, b % insert_count);
                if a < b {
                    mixer.routes.insert(
                        RouteKey { from: RouteSource::Insert(inserts[a]), to: inserts[b] },
                        Route { send_level: level, tap: SendTap::PostFader, kind: RouteKind::Audio },
                    );
                }
            }
            project.mixer = Arc::new(mixer);
            let sample = *project.samples.keys().next().unwrap();
            project.channels = OrdMap::new();
            project.rack_order = Vector::new();
            for i in 0..channel_count {
                let id = ChannelId::from_u128(20_000 + i as u128);
                let kind = ChannelKind::Sampler(Arc::new(SamplerParams { sample: Some(sample), ..SamplerParams::default() }));
                project.channels.insert(id, channel(id, &format!("Ch {i}"), kind, Some(inserts[i % insert_count])));
                project.rack_order.push_back(id);
            }
            let first = project.rack_order[0];
            let pattern_id = PatternId::from_u128(30_000);
            let mut pattern = Pattern::new(pattern_id, "Random", Color::GRAY);
            pattern.notes.insert(
                first,
                NoteSeq::new(notes.iter().enumerate().map(|(i, &(key, start, len))| Note::new(NoteId(i as u32), key, Tick(start), Tick(len)))),
            );
            pattern.next_note_id = notes.len() as u32;
            project.patterns = [(pattern_id, Arc::new(pattern))].into_iter().collect();
            project.tracks = OrdMap::new();
            project.track_order = Vector::new();
            for t in 0..track_count {
                let id = TrackId::from_u128(40_000 + t as u128);
                let mut track = PlaylistTrack::new(id, "Track", Color::GRAY);
                track.insert_clip(Clip {
                    id: ClipId::from_u128(50_000 + t as u128),
                    start: Tick(t as i64 * 960),
                    len: Tick(3840),
                    offset: Default::default(),
                    muted: false,
                    color: None,
                    source: ClipSource::Pattern(pattern_id),
                    fades: None,
                });
                project.tracks.insert(id, Arc::new(track));
                project.track_order.push_back(id);
            }
            project
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn random_valid_projects_validate_and_round_trip(project in random_project()) {
        prop_assert!(project.validate().is_ok(), "{:?}", project.validate());
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, project);
    }
}

#[test]
fn sample_paths_resolve_from_the_project_or_the_factory() {
    let roots = SampleRoots::new("/songs/demo").with_factory(Some("/opt/bodo/factory".into()));
    assert_eq!(roots.resolve("Samples/kick.wav"), std::path::Path::new("/songs/demo/Samples/kick.wav"));
    assert_eq!(roots.resolve("/elsewhere/snare.wav"), std::path::Path::new("/elsewhere/snare.wav"));
    assert_eq!(roots.resolve("factory:Packs/Bodo Drums/Kick.wav"), std::path::Path::new("/opt/bodo/factory/Packs/Bodo Drums/Kick.wav"));
    let missing = SampleRoots::new("/songs/demo").resolve("factory:Packs/Kick.wav");
    assert!(!missing.starts_with("/songs/demo"), "a factory path never resolves into the project: {}", missing.display());
}
