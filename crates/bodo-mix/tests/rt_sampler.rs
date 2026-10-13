//! Checks that a sampler with every feature on (envelopes, the filter, loops, declicking, the arpeggiator, slices) and
//! a layer never allocate on the audio side, and that MIDI Out channels hand their notes to the control side.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_midi::{MidiMessage, U7};
use bodo_mix::{EngineConfig, engine};
use bodo_model::channel::{Arpeggiator, ChannelKind, Declick, FilterKind, LoopMode, ModEnvelope, SamplerFilter, SamplerParams};
use bodo_model::*;
use common::{Builder, temp_dir, write_tone};
use imbl::Vector;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn plain_channel(id: ChannelId, name: &str, kind: ChannelKind) -> Channel {
    Channel {
        id,
        name: name.into(),
        color: Color::GRAY,
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

#[cfg(debug_assertions)]
#[test]
fn a_full_featured_sampler_and_a_layer_play_without_allocating() {
    let dir = temp_dir("rt-sampler");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let params = SamplerParams {
        filter: SamplerFilter { kind: FilterKind::BandPass, cutoff: 1500.0, resonance: 0.7, key_tracking: 1.0 },
        filter_envelope: ModEnvelope { enabled: true, amount: 4.0, ..ModEnvelope::default() },
        pitch_envelope: ModEnvelope { enabled: true, amount: 7.0, ..ModEnvelope::default() },
        arp: Arpeggiator { enabled: true, rate: 60, octaves: 3, ..Arpeggiator::default() },
        loop_mode: LoopMode::PingPong,
        loop_start: 0.1,
        loop_end: 0.4,
        one_shot: false,
        declick: Declick::Smooth,
        polyphony: 8,
        ..SamplerParams::default()
    };
    let arp = b.sampler(sample, Some(b.insert(0)), params);
    let plain = b.sampler(sample, Some(b.insert(1)), SamplerParams { declick: Declick::Transient, ..SamplerParams::default() });
    let layer = ChannelId::from_u128(b.id());
    b.project.channels.insert(layer, Arc::new(plain_channel(layer, "Layer", ChannelKind::Layer { children: Vector::from(vec![arp, plain]) })));
    b.project.rack_order.push_back(layer);
    let notes: Vec<_> = (0..24).map(|i| (if i % 3 == 0 { layer } else { arp }, 48 + (i % 12) as u8, i64::from(i) * 120, 600)).collect();
    let pattern = b.pattern(Some(3840), &notes);
    b.clip(pattern, 0, 3840 * 4, 0);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    let live = control.live_input();
    control.play();
    let mut buffer = AudioBuffer::new(2, 256);
    for round in 0..30u8 {
        live.note(layer, 60 + round % 12, 0.7, true);
        live.note(layer, 60 + (round + 5) % 12, 0.0, false);
        reset_violation_count();
        assert_no_alloc(|| {
            for _ in 0..20 {
                audio.render(256, &mut buffer);
            }
        });
        assert_eq!(violation_count(), 0);
    }
    control.stop();
    reset_violation_count();
    assert_no_alloc(|| audio.render(256, &mut buffer));
    assert_eq!(violation_count(), 0);
}

#[test]
fn a_midi_out_channel_sends_its_notes() {
    let dir = temp_dir("rt-midi-out");
    let mut b = Builder::new();
    let out = ChannelId::from_u128(b.id());
    b.project.channels.insert(out, Arc::new(plain_channel(out, "Synth", ChannelKind::MidiOut { port: String::from("Synth port"), channel: 2 })));
    b.project.rack_order.push_back(out);
    let pattern = b.pattern(Some(3840), &[(out, 64, 0, 240), (out, 67, 480, 240)]);
    b.clip(pattern, 0, 3840, 0);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    let tap = control.midi_out();
    control.play();
    let mut buffer = AudioBuffer::new(2, 256);
    // A beat: 24 000 frames at 120 BPM.
    for _ in 0..(24_000 / 256 + 1) {
        audio.render(256, &mut buffer);
    }
    assert!(buffer.channel(0).iter().all(|s| *s == 0.0), "MIDI Out channels are silent");
    let mut sent = Vec::new();
    tap.drain(|port, message| sent.push((port.to_owned(), message.channel.get(), message.message)));
    let key = |k| U7::new(k).unwrap();
    let velocity = U7::new(64).unwrap();
    assert_eq!(
        sent,
        [
            (String::from("Synth port"), 2, MidiMessage::NoteOn { key: key(64), velocity }),
            (String::from("Synth port"), 2, MidiMessage::NoteOff { key: key(64), velocity: U7::MIN }),
            (String::from("Synth port"), 2, MidiMessage::NoteOn { key: key(67), velocity }),
            (String::from("Synth port"), 2, MidiMessage::NoteOff { key: key(67), velocity: U7::MIN }),
        ]
    );
}
