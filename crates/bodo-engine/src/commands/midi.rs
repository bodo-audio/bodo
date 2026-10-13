//! MIDI inputs (project.md section 6.2): which input plays a channel. The metronome and count-in are transport
//! commands.
//!
//! A channel bound to a MIDI input port, and optionally to one MIDI channel, plays the notes that arrive there; notes
//! from an input no channel is bound to play the selected channel. A MIDI recording lands in the current pattern
//! through `note.add_many`, one call per channel inside one batch, so a take is one undo step.

use bodo_model::channel::MidiBinding;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::params::ChannelRef;
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetMidiIn {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The MIDI input port's name, as the system lists it; null or left out removes the binding.
    #[serde(default)]
    port: Option<String>,
    /// The MIDI channel to listen to, 1 to 16; every channel when null or left out.
    #[serde(default)]
    midi_channel: Option<u8>,
}

fn set_midi_in(ctx: &mut Ctx<'_>, p: SetMidiIn) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    if let Some(midi_channel) = p.midi_channel
        && !(1..=16).contains(&midi_channel)
    {
        return Err(CommandError::invalid(format!("midiChannel must be from 1 to 16, not {midi_channel}")));
    }
    let port = p.port.as_deref().map(str::trim).filter(|port| !port.is_empty()).map(str::to_owned);
    if port.is_none() && p.midi_channel.is_some() {
        return Err(CommandError::invalid("a MIDI channel needs a port").with_hint("pass port with the input's name"));
    }
    let binding = port.map(|port| MidiBinding { port, channel: p.midi_channel.map(|c| c - 1) });
    let name = ctx.project.channels.get(&id).map(|c| c.name.clone()).unwrap_or_default();
    ctx.edit_channel(id, |c| c.midi_in.clone_from(&binding))?;
    ctx.set_label(if binding.is_some() { format!("Set {name} MIDI input") } else { format!("Remove {name} MIDI input") });
    Ok(json!({
        "channel": id,
        "port": binding.as_ref().map(|b| b.port.clone()),
        "midiChannel": binding.as_ref().and_then(|b| b.channel).map(|c| c + 1),
    }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "channel.set_midi_in",
        "Set MIDI input",
        "Binds a channel to a MIDI input port and optionally one MIDI channel, or removes the binding.",
        Effects::UNDOABLE,
        set_midi_in,
    )
    .example(json!({ "channel": "Kick", "port": "Pads", "midiChannel": 10 }));
}
