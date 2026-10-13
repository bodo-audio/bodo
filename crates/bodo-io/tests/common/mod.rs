//! Fixtures shared by the bodo-io tests.

#![allow(dead_code, clippy::unwrap_used, reason = "shared test helpers; not every test uses every helper")]

use std::path::PathBuf;
use std::sync::Arc;

use bodo_kit_core::{Frames, SampleRate, Seconds, TempoCurve, TempoMap, TempoPoint, Tick};
use bodo_kit_dsp::envelope::AdsrParams;
use bodo_kit_dsp::pan::PanLaw;
use bodo_model::automation::{AutoPoint, AutomationLane, ChannelParam, CurveShape, InsertParam, ParamTarget, SlotParam};
use bodo_model::channel::{
    ArpDirection, Arpeggiator, ChannelKind, Declick, FilterKind, LoopMode, MidiBinding, ModEnvelope, PluginFormat, PluginRef, SampleEffects,
    SamplerFilter, SamplerParams, SamplerStretch, SamplerStretchMode, SandboxMode, StretchMode,
};
use bodo_model::meter::MeterChange;
use bodo_model::mixer::{EqBand, HwPort, PdcBypass, RecordFormat, RecordTap, Route, RouteKind, SendTap};
use bodo_model::pattern::{ExprPoint, NoteExpression, PatternLen};
use bodo_model::playlist::{ClipOffset, Fades};
use bodo_model::project::{Marker, MarketRef, Meta, Settings};
use bodo_model::*;
use imbl::{OrdMap, Vector};

/// A fresh, empty folder for one test.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bodo-io-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every file and folder below `dir`, as sorted relative paths with forward slashes.
pub fn tree(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.path().is_dir() {
                pending.push((entry.path(), format!("{name}/")));
                out.push(format!("{name}/"));
            } else {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

/// Ids from a counter, so fixtures are reproducible.
struct Ids(u128);

impl Ids {
    fn next(&mut self) -> u128 {
        self.0 += 1;
        self.0
    }
}

fn channel(id: ChannelId, name: &str, color: Color, kind: ChannelKind) -> Channel {
    Channel {
        id,
        name: name.into(),
        color,
        kind,
        target: None,
        volume: 1.0,
        pan: 0.0,
        pitch_cents: 0.0,
        swing: 0.0,
        muted: false,
        midi_in: None,
        group: None,
    }
}

fn clip(id: ClipId, start: i64, len: i64, source: ClipSource) -> Clip {
    Clip { id, start: Tick(start), len: Tick(len), offset: ClipOffset::default(), muted: false, color: None, source, fades: None }
}

/// The ids of the plugins in [`full`], for tests that give them states.
pub struct PluginIds {
    pub instrument: PluginInstanceId,
    pub slot: PluginInstanceId,
    pub master_slot: PluginInstanceId,
}

/// A project that uses every kind of entity and sets nearly every field away from its default: every channel kind,
/// patterns with notes, expressions and automation for every target kind, tracks with every clip source and offset,
/// inserts with audio and sidechain routes, plugins in slots, markers, meter changes and a tempo ramp.
pub fn full() -> (Project, PluginIds) {
    let mut ids = Ids(0);
    let master = InsertId::from_u128(ids.next());
    let inserts: Vec<InsertId> = (0..4).map(|_| InsertId::from_u128(ids.next())).collect();
    let plugin_ids = PluginIds {
        instrument: PluginInstanceId::from_u128(ids.next()),
        slot: PluginInstanceId::from_u128(ids.next()),
        master_slot: PluginInstanceId::from_u128(ids.next()),
    };

    let kick_sample = SampleId::from_u128(ids.next());
    let clap_sample = SampleId::from_u128(ids.next());
    let break_sample = SampleId::from_u128(ids.next());
    let samples: OrdMap<SampleId, Arc<SampleRef>> = [
        SampleRef { id: kick_sample, path: "Samples/kick.wav".into(), hash: Some("ab".repeat(32)), market: None },
        SampleRef {
            id: clap_sample,
            path: "factory:drums/clap.wav".into(),
            hash: None,
            market: Some(MarketRef { package: "bodo.drums".into(), version: "1.0.0".into(), item: "clap.wav".into() }),
        },
        SampleRef { id: break_sample, path: "Loops/break.wav".into(), hash: None, market: None },
    ]
    .into_iter()
    .map(|s| (s.id, Arc::new(s)))
    .collect();

    let kick = ChannelId::from_u128(ids.next());
    let clap = ChannelId::from_u128(ids.next());
    let synth = ChannelId::from_u128(ids.next());
    let breaks = ChannelId::from_u128(ids.next());
    let ride = ChannelId::from_u128(ids.next());
    let layer = ChannelId::from_u128(ids.next());
    let out = ChannelId::from_u128(ids.next());

    let kick_params = SamplerParams {
        sample: Some(kick_sample),
        root_key: 48,
        key_range: (24, 96),
        envelope: AdsrParams { attack: Seconds(0.001), decay: Seconds(0.2), sustain: 0.5, release: Seconds(0.3) },
        start: 0.125,
        loop_mode: LoopMode::PingPong,
        loop_start: 0.25,
        loop_end: 0.75,
        reverse: true,
        one_shot: true,
        keep_on_disk: true,
        polyphony: 4,
        end: 0.875,
        start_offset: 0.0625,
        resample: false,
        load_regions: true,
        load_slices: true,
        pitch_range: 12.0,
        declick: Declick::Smooth,
        stretch: SamplerStretch { mode: SamplerStretchMode::Stretch, pitch_cents: -150.0, multiplier: 1.5, time_beats: Some(4.0) },
        effects: SampleEffects {
            trim: true,
            remove_dc: true,
            normalize: true,
            reverse_polarity: true,
            swap_stereo: true,
            fade_stereo: true,
            fade_in: 0.01,
            fade_out: 0.25,
            crossfade: 0.125,
        },
        filter: SamplerFilter { kind: FilterKind::BandPass, cutoff: 900.0, resonance: 0.5, key_tracking: 0.75 },
        filter_envelope: ModEnvelope {
            enabled: true,
            adsr: AdsrParams { attack: Seconds(0.01), decay: Seconds(0.4), sustain: 0.25, release: Seconds(0.5) },
            amount: 3.0,
        },
        pitch_envelope: ModEnvelope {
            enabled: true,
            adsr: AdsrParams { attack: Seconds(0.0), decay: Seconds(0.05), sustain: 0.0, release: Seconds(0.05) },
            amount: -12.0,
        },
        arp: Arpeggiator { enabled: true, direction: ArpDirection::UpDown, rate: 120, gate: 0.75, octaves: 2, repeat: 3 },
    };
    let lane = |target, default| AutomationLane {
        target,
        points: Vector::from(vec![
            AutoPoint { at: Tick(0), value: 0.0, curve: CurveShape::Hold },
            AutoPoint { at: Tick(960), value: 0.25, curve: CurveShape::Linear },
            AutoPoint { at: Tick(1920), value: 0.5, curve: CurveShape::Exp(0.3) },
            AutoPoint { at: Tick(2880), value: 0.75, curve: CurveShape::SCurve(-0.4) },
            AutoPoint { at: Tick(3840), value: 1.0, curve: CurveShape::Stairs(4) },
            AutoPoint { at: Tick(4800), value: 0.6, curve: CurveShape::Sine },
            AutoPoint { at: Tick(5760), value: 0.1, curve: CurveShape::Pulse },
        ]),
        default,
    };

    let mut kick_channel = channel(kick, "808 Kick", Color::rgb(0xf8, 0x71, 0x71), ChannelKind::Sampler(Arc::new(kick_params)));
    kick_channel.target = Some(inserts[0]);
    kick_channel.volume = 0.8;
    kick_channel.pan = -0.25;
    kick_channel.pitch_cents = -12.5;
    kick_channel.swing = 0.55;
    kick_channel.group = Some(String::from("Drums"));
    let mut clap_channel = channel(
        clap,
        "808 Clap",
        Color::rgb(0xfb, 0x92, 0x3c),
        ChannelKind::Sampler(Arc::new(SamplerParams { sample: Some(clap_sample), ..SamplerParams::default() })),
    );
    clap_channel.target = Some(inserts[1]);
    clap_channel.muted = true;
    clap_channel.group = Some(String::from("Drums"));
    let mut synth_channel = channel(
        synth,
        "Lead",
        Color::rgb(0xa7, 0x8b, 0xfa),
        ChannelKind::Instrument {
            plugin: PluginRef {
                instance: plugin_ids.instrument,
                format: PluginFormat::Clap,
                uid: "org.example.synth".into(),
                state: Arc::from(Vec::new()),
                sandbox: SandboxMode::PerPlugin,
            },
        },
    );
    synth_channel.target = Some(inserts[2]);
    synth_channel.midi_in = Some(MidiBinding { port: "Keystation 49".into(), channel: Some(3) });
    let channels: Vec<Channel> = vec![
        kick_channel,
        clap_channel,
        synth_channel,
        channel(
            breaks,
            "Break",
            Color::rgb(0x4a, 0xde, 0x80),
            ChannelKind::AudioClip { sample: break_sample, stretch: StretchMode::Stretch, tempo: None },
        ),
        channel(
            ride,
            "Fader ride",
            Color::rgb(0x22, 0xd3, 0xee),
            ChannelKind::Automation(Arc::new(lane(ParamTarget::Channel(kick, ChannelParam::Volume), 0.8))),
        ),
        channel(layer, "Drum layer", Color::GRAY, ChannelKind::Layer { children: Vector::from(vec![kick, clap]) }),
        channel(out, "Hardware synth", Color::GRAY, ChannelKind::MidiOut { port: "IAC Bus 1".into(), channel: 9 }),
    ];
    let rack_order: Vector<ChannelId> = channels.iter().map(|c| c.id).collect();
    let channels: OrdMap<ChannelId, Arc<Channel>> = channels.into_iter().map(|c| (c.id, Arc::new(c))).collect();

    let mut mixer = Mixer::with_inserts(master, inserts.clone());
    let audio_route = RouteKey { from: RouteSource::Insert(inserts[0]), to: inserts[2] };
    mixer.routes.insert(audio_route, Route { send_level: 0.5, tap: SendTap::PostFader, kind: RouteKind::Audio });
    mixer.routes.insert(
        RouteKey { from: RouteSource::Channel(kick), to: inserts[1] },
        Route { send_level: 0.25, tap: SendTap::PreFx, kind: RouteKind::Audio },
    );
    mixer.routes.insert(
        RouteKey { from: RouteSource::Insert(inserts[1]), to: inserts[2] },
        Route { send_level: 1.0, tap: SendTap::PreFader, kind: RouteKind::Sidechain },
    );
    let mut ducked = (**mixer.inserts.get(&inserts[2]).unwrap()).clone();
    ducked.slots[0].sidechain = Some(RouteSource::Insert(inserts[1]));
    ducked.slots[0].plugin = Some(PluginRef {
        instance: plugin_ids.slot,
        format: PluginFormat::Native,
        uid: "dev.bodo.comp".into(),
        state: Arc::from(Vec::new()),
        sandbox: SandboxMode::InProcess,
    });
    ducked.slots[0].mix = 0.75;
    ducked.slots[1].enabled = false;
    mixer.inserts.insert(inserts[2], Arc::new(ducked));
    let mut odd = (**mixer.inserts.get(&inserts[3]).unwrap()).clone();
    odd.name = "Cue".into();
    odd.color = Color::rgb(0x60, 0xa5, 0xfa);
    odd.eq.low = EqBand { frequency: 80.0, gain_db: 2.5, q: 0.5 };
    odd.eq.high.gain_db = -1.5;
    odd.volume_db = -3.5;
    odd.pan = 0.25;
    odd.stereo_sep = -0.5;
    odd.polarity_invert = true;
    odd.swap_lr = true;
    odd.latency_offset = Frames(-64);
    odd.pdc_bypass = PdcBypass::WhileRecording;
    odd.muted = true;
    odd.solo = true;
    odd.to_master = false;
    odd.input = Some(HwPort { channels: vec![0, 1] });
    odd.output = Some(HwPort { channels: vec![2] });
    odd.armed = true;
    odd.record_tap = RecordTap::PreFx;
    odd.record_format = RecordFormat::Flac;
    mixer.inserts.insert(inserts[3], Arc::new(odd));
    let mut master_strip = (*mixer.master).clone();
    master_strip.slots[9].plugin = Some(PluginRef {
        instance: plugin_ids.master_slot,
        format: PluginFormat::Vst3,
        uid: "ABCDEF0123456789ABCDEF0123456789".into(),
        state: Arc::from(Vec::new()),
        sandbox: SandboxMode::SharedBridge,
    });
    master_strip.volume_db = -0.5;
    mixer.master = Arc::new(master_strip);

    let beat_id = PatternId::from_u128(ids.next());
    let mut beat = Pattern::new(beat_id, "Beat", Color::rgb(0xfa, 0xcc, 0x15));
    beat.length = PatternLen::Fixed(Tick(3840));
    beat.notes.insert(kick, NoteSeq::new((0..4).map(|i| Note::new(NoteId(i), 48, Tick(i64::from(i) * 960), Tick(240)))));
    let mut ghost = Note::new(NoteId(4), 60, Tick(1680), Tick(120));
    ghost.velocity = 0x2000;
    ghost.release_velocity = 0x1000;
    ghost.fine_pitch_cents = 7.5;
    ghost.pan = 0.5;
    ghost.mod_x = 0.25;
    ghost.mod_y = 0.75;
    ghost.color = 2;
    ghost.muted = true;
    ghost.slide = true;
    ghost.porta = true;
    ghost.quantized_start = Some(Tick(1680 - 12));
    ghost.expr = Some(Arc::new(NoteExpression {
        pitch: vec![ExprPoint { at: Tick(0), value: 0.0 }, ExprPoint { at: Tick(60), value: 1.5 }],
        pressure: vec![ExprPoint { at: Tick(10), value: 0.5 }],
        timbre: vec![],
    }));
    beat.notes.insert(clap, NoteSeq::new([Note::new(NoteId(5), 60, Tick(960), Tick(240)), ghost]));
    beat.automation = Vector::from(vec![
        Arc::new(lane(ParamTarget::Insert(inserts[0], InsertParam::Pan), 0.5)),
        Arc::new(lane(ParamTarget::Slot(inserts[2], 0, SlotParam::Mix), 1.0)),
        Arc::new(lane(ParamTarget::Plugin(plugin_ids.instrument, 3), 0.2)),
        Arc::new(lane(ParamTarget::Route(audio_route), 0.5)),
        Arc::new(lane(ParamTarget::Tempo, 0.3)),
    ]);
    beat.next_note_id = 6;
    let line_id = PatternId::from_u128(ids.next());
    let mut line = Pattern::new(line_id, "Lead line", Color::rgb(0xa7, 0x8b, 0xfa));
    line.binding = Some(synth);
    line.notes.insert(synth, NoteSeq::new([Note::new(NoteId(0), 72, Tick(0), Tick(1920)), Note::new(NoteId(1), 74, Tick(1920), Tick(1920))]));
    line.next_note_id = 2;
    let patterns: OrdMap<PatternId, Arc<Pattern>> = [(beat_id, Arc::new(beat)), (line_id, Arc::new(line))].into_iter().collect();

    let drums_id = TrackId::from_u128(ids.next());
    let mut drums = PlaylistTrack::new(drums_id, "Drums", Color::rgb(0xf8, 0x71, 0x71));
    drums.insert_clip(clip(ClipId::from_u128(ids.next()), 0, 3840, ClipSource::Pattern(beat_id)));
    let mut muted = clip(ClipId::from_u128(ids.next()), 3840, 7680, ClipSource::Pattern(beat_id));
    muted.muted = true;
    muted.color = Some(Color::rgb(0x34, 0xd3, 0x99));
    muted.offset = ClipOffset::Ticks(Tick(960));
    drums.insert_clip(muted);
    let audio_id = TrackId::from_u128(ids.next());
    let mut audio = PlaylistTrack::new(audio_id, "Break", Color::rgb(0x4a, 0xde, 0x80));
    let mut break_clip = clip(ClipId::from_u128(ids.next()), 1920, 7680, ClipSource::Audio(breaks));
    break_clip.offset = ClipOffset::Frames(Frames(1200));
    break_clip.fades = Some(Fades { fade_in: Tick(30), fade_out: Tick(240) });
    audio.insert_clip(break_clip);
    audio.height = 96.0;
    audio.solo = true;
    drums.group = Some(String::from("Rhythm"));
    audio.group = Some(String::from("Rhythm"));
    let auto_id = TrackId::from_u128(ids.next());
    let mut auto = PlaylistTrack::new(auto_id, "Automation", Color::rgb(0x22, 0xd3, 0xee));
    auto.insert_clip(clip(ClipId::from_u128(ids.next()), 0, 15_360, ClipSource::Automation(ride)));
    auto.insert_clip(clip(ClipId::from_u128(ids.next()), 0, 3840, ClipSource::Pattern(line_id)));
    auto.muted = true;
    auto.locked = true;
    let tracks = [drums, audio, auto];
    let track_order: Vector<TrackId> = tracks.iter().map(|t| t.id).collect();
    let tracks: OrdMap<TrackId, Arc<PlaylistTrack>> = tracks.into_iter().map(|t| (t.id, Arc::new(t))).collect();

    let project = Project {
        id: ProjectId::from_u128(ids.next()),
        meta: Arc::new(Meta {
            title: "Every entity".into(),
            author: "Bodo tests".into(),
            credits: vec!["Clap by someone, CC-BY 4.0".into()],
            created: Some("2026-10-13T12:00:00Z".into()),
        }),
        settings: Arc::new(Settings { sample_rate: SampleRate::HZ_44100, ppq: bodo_kit_core::PPQ, pan_law: PanLaw::Linear6dB }),
        tempo: Arc::new(
            TempoMap::new(vec![TempoPoint::new(Tick(0), 120.0, TempoCurve::Linear), TempoPoint::new(Tick(30_720), 140.5, TempoCurve::Jump)]).unwrap(),
        ),
        meter: Arc::new(
            MeterMap::new(vec![MeterChange { bar: 0, numerator: 4, denominator: 4 }, MeterChange { bar: 8, numerator: 7, denominator: 8 }]).unwrap(),
        ),
        channels,
        rack_order,
        patterns,
        tracks,
        track_order,
        mixer: Arc::new(mixer),
        samples,
        markers: Vector::from(vec![
            Marker { at: Tick(0), name: "Intro".into(), color: None },
            Marker { at: Tick(15_360), name: "Drop".into(), color: Some(Color::rgb(0xfa, 0xcc, 0x15)) },
        ]),
    };
    (project, plugin_ids)
}

/// [`full`] with a saved state for each of its plugins.
pub fn with_states() -> Project {
    let (mut project, ids) = full();
    let synth = *project.rack_order.iter().nth(2).unwrap();
    let mut channel = (*project.channels[&synth]).clone();
    if let ChannelKind::Instrument { plugin } = &mut channel.kind {
        plugin.state = Arc::from(vec![1u8, 2, 3, 0, 255]);
    }
    project.channels.insert(synth, Arc::new(channel));
    let mut mixer = (*project.mixer).clone();
    let mut master = (*mixer.master).clone();
    if let Some(plugin) = master.slots[9].plugin.as_mut() {
        plugin.state = Arc::from(b"master state".to_vec());
    }
    mixer.master = Arc::new(master);
    project.mixer = Arc::new(mixer);
    let _ = ids;
    project
}
