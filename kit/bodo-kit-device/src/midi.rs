//! MIDI devices (project.md section 6.2): listing ports, inputs that hand timestamped messages on, and outputs.
//!
//! An input runs its handler on the MIDI system's own thread, never on the audio thread. [`MidiInput::open`] takes any
//! handler; [`MidiInput::open_queue`] puts the messages in a bounded queue for a real-time consumer instead, dropping
//! and counting new messages when the queue is full. Listing again finds devices plugged in since. On a machine with
//! no MIDI system, such as some CI runners, listing reports none and opening reports why.
//!
//! Only channel voice messages pass: system messages (clock, sysex, active sensing) are left out.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bodo_kit_core::queue::{Consumer, spsc};
use bodo_kit_midi::{MidiMessage, U4, U7, U14};

/// The client name Bodo shows to the MIDI system.
const CLIENT: &str = "Bodo";

/// A channel message from a MIDI input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MidiEvent {
    /// When it arrived, in microseconds on the MIDI system's own clock. Only differences between events are meaningful.
    pub time_us: u64,
    /// The MIDI channel, 0 to 15.
    pub channel: U4,
    /// The message. A note-on with velocity zero arrives as a note-off.
    pub message: MidiMessage,
}

/// Why a MIDI port could not be listed, opened or used.
#[derive(Debug, thiserror::Error)]
pub enum MidiError {
    /// The machine has no usable MIDI system.
    #[error("MIDI is not available: {0}")]
    Unavailable(String),
    /// No port has that name.
    #[error("no MIDI port named {0}")]
    NoPort(String),
    /// The port is there but could not be opened.
    #[error("could not open the MIDI port {port}: {reason}")]
    Connect {
        /// The port's name.
        port: String,
        /// What the MIDI system said.
        reason: String,
    },
    /// A message could not be sent.
    #[error("could not send to the MIDI port: {0}")]
    Send(String),
}

/// The names of the MIDI inputs on this machine, in the system's order. Empty when there is no MIDI system.
///
/// Talks to the MIDI system. Complexity: O(p) in ports.
pub fn input_ports() -> Vec<String> {
    let Ok(input) = midir::MidiInput::new(CLIENT) else { return Vec::new() };
    input.ports().iter().filter_map(|port| input.port_name(port).ok()).collect()
}

/// The names of the MIDI outputs on this machine, in the system's order. Empty when there is no MIDI system.
///
/// Talks to the MIDI system. Complexity: O(p) in ports.
pub fn output_ports() -> Vec<String> {
    let Ok(output) = midir::MidiOutput::new(CLIENT) else { return Vec::new() };
    output.ports().iter().filter_map(|port| output.port_name(port).ok()).collect()
}

/// An open MIDI input. Dropping it closes the port.
pub struct MidiInput {
    _connection: midir::MidiInputConnection<()>,
    name: String,
}

impl std::fmt::Debug for MidiInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiInput").field("name", &self.name).finish_non_exhaustive()
    }
}

impl MidiInput {
    /// Opens the input named `name` and calls `handler` for every channel message, on the MIDI system's thread.
    ///
    /// Talks to the MIDI system.
    pub fn open(name: &str, mut handler: impl FnMut(MidiEvent) + Send + 'static) -> Result<Self, MidiError> {
        let mut input = midir::MidiInput::new(CLIENT).map_err(|e| MidiError::Unavailable(e.to_string()))?;
        input.ignore(midir::Ignore::All);
        let port = input
            .ports()
            .into_iter()
            .find(|port| input.port_name(port).is_ok_and(|n| n == name))
            .ok_or_else(|| MidiError::NoPort(name.to_owned()))?;
        let connection = input
            .connect(
                &port,
                "Bodo input",
                move |time_us, bytes, _| {
                    if let Some((channel, message)) = decode(bytes) {
                        handler(MidiEvent { time_us, channel, message });
                    }
                },
                (),
            )
            .map_err(|e| MidiError::Connect { port: name.to_owned(), reason: e.to_string() })?;
        Ok(Self { _connection: connection, name: name.to_owned() })
    }

    /// Opens the input named `name` with a queue of `capacity` messages for one consumer, such as the audio thread.
    /// When the queue is full, new messages are dropped and counted in the returned counter.
    ///
    /// Talks to the MIDI system and allocates the queue.
    pub fn open_queue(name: &str, capacity: usize) -> Result<(Self, Consumer<MidiEvent>, Arc<AtomicU64>), MidiError> {
        let (mut producer, consumer) = spsc(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);
        let input = Self::open(name, move |event| {
            if !producer.push_or_drop(event) {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        })?;
        Ok((input, consumer, dropped))
    }

    /// The port's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// An open MIDI output. Dropping it closes the port.
pub struct MidiOutput {
    connection: midir::MidiOutputConnection,
    name: String,
}

impl std::fmt::Debug for MidiOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiOutput").field("name", &self.name).finish_non_exhaustive()
    }
}

impl MidiOutput {
    /// Opens the output named `name`.
    ///
    /// Talks to the MIDI system.
    pub fn open(name: &str) -> Result<Self, MidiError> {
        let output = midir::MidiOutput::new(CLIENT).map_err(|e| MidiError::Unavailable(e.to_string()))?;
        let port = output
            .ports()
            .into_iter()
            .find(|port| output.port_name(port).is_ok_and(|n| n == name))
            .ok_or_else(|| MidiError::NoPort(name.to_owned()))?;
        let connection = output.connect(&port, "Bodo output").map_err(|e| MidiError::Connect { port: name.to_owned(), reason: e.to_string() })?;
        Ok(Self { connection, name: name.to_owned() })
    }

    /// Sends `message` on MIDI channel `channel`.
    ///
    /// Talks to the MIDI system.
    pub fn send(&mut self, channel: U4, message: MidiMessage) -> Result<(), MidiError> {
        let (bytes, len) = encode(channel, message);
        self.connection.send(&bytes[..len]).map_err(|e| MidiError::Send(e.to_string()))
    }

    /// The port's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The channel and message of a channel voice message's bytes, or `None` for anything else. A note-on with velocity
/// zero becomes a note-off, as MIDI defines.
pub fn decode(bytes: &[u8]) -> Option<(U4, MidiMessage)> {
    let (&status, data) = bytes.split_first()?;
    let channel = U4::from_masked(status & 0x0F);
    let byte = |i: usize| data.get(i).copied().and_then(U7::new);
    let message = match status & 0xF0 {
        0x80 => MidiMessage::NoteOff { key: byte(0)?, velocity: byte(1)? },
        0x90 => {
            let (key, velocity) = (byte(0)?, byte(1)?);
            if velocity.get() == 0 { MidiMessage::NoteOff { key, velocity } } else { MidiMessage::NoteOn { key, velocity } }
        }
        0xA0 => MidiMessage::PolyPressure { key: byte(0)?, pressure: byte(1)? },
        0xB0 => MidiMessage::ControlChange { controller: byte(0)?, value: byte(1)? },
        0xC0 => MidiMessage::ProgramChange { program: byte(0)? },
        0xD0 => MidiMessage::ChannelPressure { pressure: byte(0)? },
        0xE0 => {
            let (low, high) = (byte(0)?, byte(1)?);
            MidiMessage::PitchBend { value: U14::from_masked(u16::from(high.get()) << 7 | u16::from(low.get())) }
        }
        _ => return None,
    };
    Some((channel, message))
}

/// The bytes of `message` on channel `channel`, and how many of the three are used.
pub fn encode(channel: U4, message: MidiMessage) -> ([u8; 3], usize) {
    let ch = channel.get();
    match message {
        MidiMessage::NoteOff { key, velocity } => ([0x80 | ch, key.get(), velocity.get()], 3),
        MidiMessage::NoteOn { key, velocity } => ([0x90 | ch, key.get(), velocity.get()], 3),
        MidiMessage::PolyPressure { key, pressure } => ([0xA0 | ch, key.get(), pressure.get()], 3),
        MidiMessage::ControlChange { controller, value } => ([0xB0 | ch, controller.get(), value.get()], 3),
        MidiMessage::ProgramChange { program } => ([0xC0 | ch, program.get(), 0], 2),
        MidiMessage::ChannelPressure { pressure } => ([0xD0 | ch, pressure.get(), 0], 2),
        MidiMessage::PitchBend { value } => ([0xE0 | ch, (value.get() & 0x7F) as u8, (value.get() >> 7) as u8], 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u7(v: u8) -> U7 {
        U7::from_masked(v)
    }

    #[test]
    fn channel_messages_round_trip_through_bytes() {
        let messages = [
            MidiMessage::NoteOn { key: u7(60), velocity: u7(100) },
            MidiMessage::NoteOff { key: u7(60), velocity: u7(64) },
            MidiMessage::PolyPressure { key: u7(61), pressure: u7(3) },
            MidiMessage::ControlChange { controller: u7(64), value: u7(127) },
            MidiMessage::ProgramChange { program: u7(5) },
            MidiMessage::ChannelPressure { pressure: u7(90) },
            MidiMessage::PitchBend { value: U14::from_masked(0x1234) },
        ];
        for channel in [0, 9, 15] {
            for message in messages {
                let (bytes, len) = encode(U4::from_masked(channel), message);
                assert_eq!(decode(&bytes[..len]), Some((U4::from_masked(channel), message)), "{message:?}");
            }
        }
    }

    #[test]
    fn note_on_at_velocity_zero_is_a_note_off_and_other_bytes_are_ignored() {
        assert_eq!(decode(&[0x93, 60, 0]), Some((U4::from_masked(3), MidiMessage::NoteOff { key: u7(60), velocity: u7(0) })));
        assert_eq!(decode(&[0xF8]), None, "clock");
        assert_eq!(decode(&[0x90, 60]), None, "truncated");
        assert_eq!(decode(&[0x90, 200, 1]), None, "data bytes have the top bit clear");
        assert_eq!(decode(&[]), None);
    }

    #[test]
    fn listing_and_opening_degrade_gracefully() {
        // CI runners may have no MIDI system or no ports; listing must not fail either way.
        let _ = input_ports();
        let _ = output_ports();
        assert!(MidiInput::open("Bodo test port that does not exist", |_| {}).is_err());
        assert!(MidiOutput::open("Bodo test port that does not exist").is_err());
    }
}
