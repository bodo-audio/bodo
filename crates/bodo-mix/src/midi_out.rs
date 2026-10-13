//! MIDI Out channels (project.md section 10.2): their notes, from the sequencer and from live playing, go to an
//! external MIDI port.
//!
//! Each MIDI Out channel is a silent node of the graph. On the audio thread it turns the channel's note events into
//! MIDI messages and pushes them into a bounded queue of its own; a full queue drops the message and counts it. The
//! control side keeps the other ends in a [`MidiOutTap`], which a thread outside the engine drains to send the
//! messages to the ports, so the audio thread never touches a MIDI driver.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::rt_safe;
use bodo_kit_graph::NodeIo;
use bodo_kit_midi::{MidiMessage, U4, U7};
use bodo_model::ChannelId;

use crate::context::{BlockContext, EventKind};

/// How many messages each MIDI Out channel's queue holds.
pub const MIDI_OUT_CAPACITY: usize = 1024;

/// A message for a port, on a MIDI channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiOutMessage {
    /// The MIDI channel, 0 to 15.
    pub channel: U4,
    /// The message.
    pub message: MidiMessage,
}

/// A MIDI Out channel's processor.
pub struct MidiOutNode {
    key: u64,
    channel: U4,
    /// The queue's audio-thread end. Graph processors must be `Sync` and ring handles are not, so it sits in a mutex
    /// that is never locked: the node always has it to itself, and [`Mutex::get_mut`] borrows it without a lock.
    queue: Mutex<Producer<MidiOutMessage>>,
    /// Keys sounding on the port, so stopping the transport can release them.
    held: [bool; 128],
}

impl std::fmt::Debug for MidiOutNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiOutNode").field("key", &self.key).field("channel", &self.channel).finish_non_exhaustive()
    }
}

impl MidiOutNode {
    /// A node for the channel with event key `key`, sending on MIDI channel `channel` (0 to 15) into `queue`.
    pub fn new(key: u64, channel: u8, queue: Producer<MidiOutMessage>) -> Self {
        Self { key, channel: U4::from_masked(channel), queue: Mutex::new(queue), held: [false; 128] }
    }

    /// Turns the block's note events into messages. The node itself is silent.
    ///
    /// RT-safe. Complexity: O(e) in events, O(128) for an all-off.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        io.output.clear();
        for event in context.events_for(self.key) {
            let key = U7::from_masked(event.key);
            match event.kind {
                EventKind::On => {
                    let velocity = U7::from_masked((event.velocity.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8);
                    self.send(MidiMessage::NoteOn { key, velocity });
                    self.held[usize::from(key.get())] = true;
                }
                EventKind::Off => {
                    self.send(MidiMessage::NoteOff { key, velocity: U7::MIN });
                    self.held[usize::from(key.get())] = false;
                }
                EventKind::AllOff => {
                    for index in 0..self.held.len() {
                        if self.held[index] {
                            self.held[index] = false;
                            self.send(MidiMessage::NoteOff { key: U7::from_masked(index as u8), velocity: U7::MIN });
                        }
                    }
                }
            }
        }
    }

    fn send(&mut self, message: MidiMessage) {
        let queue = self.queue.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = queue.push_or_drop(MidiOutMessage { channel: self.channel, message });
    }
}

/// Each MIDI Out channel's port and the control side's end of its queue.
type Queues = HashMap<ChannelId, (String, Consumer<MidiOutMessage>)>;

/// The control side's ends of the MIDI Out channels' queues, by channel, with the port each one sends to.
#[derive(Clone, Debug, Default)]
pub struct MidiOutTap {
    queues: Arc<Mutex<Queues>>,
}

impl MidiOutTap {
    /// Makes the queue of MIDI Out channel `channel` sending to `port`, replacing any it had, and returns the audio
    /// thread's end.
    ///
    /// Allocates.
    pub(crate) fn open(&self, channel: ChannelId, port: &str) -> Producer<MidiOutMessage> {
        let (producer, consumer) = spsc(MIDI_OUT_CAPACITY);
        if let Ok(mut queues) = self.queues.lock() {
            queues.insert(channel, (port.to_owned(), consumer));
        }
        producer
    }

    /// Forgets the queues of channels that aren't MIDI Out channels any more.
    pub(crate) fn retain(&self, keep: impl Fn(&ChannelId) -> bool) {
        if let Ok(mut queues) = self.queues.lock() {
            queues.retain(|id, _| keep(id));
        }
    }

    /// Hands every waiting message to `send`, with the port it goes to.
    ///
    /// Complexity: O(m) in the messages waiting.
    pub fn drain(&self, mut send: impl FnMut(&str, MidiOutMessage)) {
        let Ok(mut queues) = self.queues.lock() else { return };
        for (port, queue) in queues.values_mut() {
            while let Some(message) = queue.pop() {
                send(port, message);
            }
        }
    }

    /// How many messages the queues dropped because they were full.
    pub fn dropped(&self) -> u64 {
        self.queues.lock().map_or(0, |queues| queues.values().map(|(_, q)| q.overflows()).sum())
    }
}
