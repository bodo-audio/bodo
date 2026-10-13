//! Notes played live (project.md sections 6.2 and 14.1): from MIDI inputs and from the computer keyboard.
//!
//! Any thread plays notes into a [`LiveInput`], a bounded lock-free queue the audio thread drains at the start of each
//! block, so a channel sounds within one block whether the transport runs or not. A full queue drops the new note and
//! counts it. [`MidiRouter`] turns MIDI messages into live notes on the MIDI system's thread: a channel bound to the
//! port (and MIDI channel) gets them, and otherwise the channel the editor has selected does.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bodo_kit_core::rt_safe;
use bodo_kit_midi::{MidiMessage, U4};
use bodo_model::{ChannelId, Project};
use crossbeam_queue::ArrayQueue;

use crate::keys;

/// How many live events wait for the audio thread at most.
pub const LIVE_CAPACITY: usize = 1024;

/// A live event for the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiveEvent {
    /// A key goes down or up on a channel.
    Note {
        /// The channel's event key.
        channel: u64,
        /// The MIDI key.
        key: u8,
        /// Velocity, 0 to 1.
        velocity: f32,
        /// Down or up.
        on: bool,
    },
    /// Every live note of a channel stops.
    AllOff {
        /// The channel's event key.
        channel: u64,
    },
}

/// Where live notes go. Any thread may play into it; cloning it is cheap and every clone feeds the same engine.
#[derive(Clone)]
pub struct LiveInput {
    queue: Arc<ArrayQueue<LiveEvent>>,
    dropped: Arc<AtomicU64>,
}

impl std::fmt::Debug for LiveInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveInput").field("waiting", &self.queue.len()).field("dropped", &self.dropped()).finish()
    }
}

impl LiveInput {
    /// A queue for up to `capacity` waiting events.
    ///
    /// Allocates.
    pub(crate) fn new(capacity: usize) -> Self {
        Self { queue: Arc::new(ArrayQueue::new(capacity.max(1))), dropped: Arc::new(AtomicU64::new(0)) }
    }

    fn push(&self, event: LiveEvent) -> bool {
        let sent = self.queue.push(event).is_ok();
        if !sent {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        sent
    }

    /// Presses (`on`) or releases key `key` on `channel` with `velocity` from 0 to 1. Returns false, and counts it,
    /// when the queue is full.
    pub fn note(&self, channel: ChannelId, key: u8, velocity: f32, on: bool) -> bool {
        self.push(LiveEvent::Note { channel: keys::channel(channel), key: key.min(127), velocity: velocity.clamp(0.0, 1.0), on })
    }

    /// Releases every live note of `channel`.
    pub fn all_off(&self, channel: ChannelId) -> bool {
        self.push(LiveEvent::AllOff { channel: keys::channel(channel) })
    }

    /// Events dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// The next waiting event.
    ///
    /// RT-safe: lock-free and allocation-free.
    #[rt_safe]
    pub(crate) fn pop(&self) -> Option<LiveEvent> {
        self.queue.pop()
    }
}

/// One channel's MIDI binding, flattened.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    port: String,
    /// The MIDI channel, 0 to 15, or `None` for all.
    midi_channel: Option<u8>,
    channel: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Routes {
    bindings: Vec<Binding>,
    fallback: Option<u64>,
}

/// Routes MIDI input to channels. The control thread updates it when the project or the selection changes; MIDI
/// threads read it for every message. Neither ever runs on the audio thread, so a short lock is fine.
#[derive(Debug)]
pub struct MidiRouter {
    routes: Mutex<Routes>,
    live: LiveInput,
}

impl MidiRouter {
    /// A router that plays into `live`, with no routes yet.
    pub fn new(live: LiveInput) -> Arc<Self> {
        Arc::new(Self { routes: Mutex::new(Routes::default()), live })
    }

    /// Takes the bindings from `project`'s channels, and `selected` as the channel for everything else.
    ///
    /// Complexity: O(c) in channels.
    pub fn update(&self, project: &Project, selected: Option<ChannelId>) {
        let bindings = project
            .channels
            .values()
            .filter_map(|c| c.midi_in.as_ref().map(|b| Binding { port: b.port.clone(), midi_channel: b.channel, channel: keys::channel(c.id) }))
            .collect();
        let fallback = selected.filter(|c| project.channels.contains_key(c)).map(keys::channel);
        let routes = Routes { bindings, fallback };
        let mut current = self.routes.lock().unwrap_or_else(PoisonError::into_inner);
        if *current != routes {
            *current = routes;
        }
    }

    /// Plays `message`, which arrived on MIDI channel `channel` of port `port`. Only notes play; other messages are
    /// ignored until instruments take them (M5).
    ///
    /// Complexity: O(b) in bindings.
    pub fn route(&self, port: &str, channel: U4, message: MidiMessage) {
        let (key, velocity, on) = match message {
            MidiMessage::NoteOn { key, velocity } => (key.get(), f32::from(velocity.get()) / 127.0, true),
            MidiMessage::NoteOff { key, .. } => (key.get(), 0.0, false),
            _ => return,
        };
        let routes = self.routes.lock().unwrap_or_else(PoisonError::into_inner);
        let mut matched = false;
        for binding in routes.bindings.iter().filter(|b| b.port == port && b.midi_channel.is_none_or(|c| c == channel.get())) {
            matched = true;
            self.live.push(LiveEvent::Note { channel: binding.channel, key, velocity, on });
        }
        if !matched && let Some(fallback) = routes.fallback {
            self.live.push(LiveEvent::Note { channel: fallback, key, velocity, on });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bodo_kit_midi::U7;
    use bodo_model::channel::{ChannelKind, MidiBinding, SamplerParams};
    use bodo_model::{Channel, Color};

    use super::*;

    fn channel(id: ChannelId, midi_in: Option<MidiBinding>) -> Arc<Channel> {
        Arc::new(Channel {
            id,
            name: String::from("C"),
            color: Color::GRAY,
            kind: ChannelKind::Sampler(Arc::new(SamplerParams::default())),
            target: None,
            volume: 1.0,
            pan: 0.0,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in,
            group: None,
        })
    }

    fn drain(live: &LiveInput) -> Vec<LiveEvent> {
        std::iter::from_fn(|| live.pop()).collect()
    }

    #[test]
    fn a_full_queue_drops_and_counts() {
        let live = LiveInput::new(4);
        let id = ChannelId::from_u128(1);
        for key in 0..9 {
            live.note(id, key, 1.0, true);
        }
        assert_eq!(live.dropped(), 5);
        assert_eq!(drain(&live).len(), 4);
    }

    #[test]
    fn bound_channels_take_their_port_and_the_selection_takes_the_rest() {
        let (piano, bass, drums) = (ChannelId::from_u128(1), ChannelId::from_u128(2), ChannelId::from_u128(3));
        let mut project = Project::new(120.0).unwrap();
        project.channels.insert(piano, channel(piano, Some(MidiBinding { port: String::from("Keys"), channel: None })));
        project.channels.insert(bass, channel(bass, Some(MidiBinding { port: String::from("Pads"), channel: Some(1) })));
        project.channels.insert(drums, channel(drums, None));
        let live = LiveInput::new(64);
        let router = MidiRouter::new(live.clone());
        router.update(&project, Some(drums));
        let on = MidiMessage::NoteOn { key: U7::from_masked(60), velocity: U7::from_masked(127) };
        router.route("Keys", U4::from_masked(5), on);
        router.route("Pads", U4::from_masked(1), on);
        router.route("Pads", U4::from_masked(2), on);
        router.route("Pads", U4::from_masked(1), MidiMessage::ControlChange { controller: U7::from_masked(1), value: U7::from_masked(3) });
        let channels: Vec<u64> = drain(&live)
            .into_iter()
            .map(|e| match e {
                LiveEvent::Note { channel, velocity, on, .. } => {
                    assert!(on && (velocity - 1.0).abs() < 1e-6);
                    channel
                }
                LiveEvent::AllOff { channel } => channel,
            })
            .collect();
        assert_eq!(channels, [keys::channel(piano), keys::channel(bass), keys::channel(drums)]);
    }
}
