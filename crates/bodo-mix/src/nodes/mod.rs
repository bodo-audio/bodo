//! The processors of the mix graph.

use bodo_kit_core::rt_safe;
use bodo_kit_graph::{NodeIo, Processor};

use crate::context::BlockContext;

pub mod arp;
pub mod audio_clip;
pub mod sampler;
pub mod strip;

pub use audio_clip::AudioClipNode;
pub use sampler::SamplerNode;
pub use strip::{SlotPatch, SlotQueues, SlotState, StripFx, StripInput, StripOutput};

/// Every kind of node the mix graph has.
#[derive(Debug)]
pub enum MixNode {
    /// A sampler channel.
    Sampler(Box<SamplerNode>),
    /// An audio clip channel.
    AudioClip(Box<AudioClipNode>),
    /// The input stage of a mixer insert or the master: input, polarity and swap.
    StripInput(StripInput),
    /// The FX stage of a mixer insert or the master: slots, EQ and stereo separation.
    StripFx(Box<StripFx>),
    /// The output stage of a mixer insert or the master: pan, fader, mute, meters and the negative latency delay.
    StripOutput(StripOutput),
    /// A MIDI Out channel.
    MidiOut(Box<crate::midi_out::MidiOutNode>),
}

impl Processor<BlockContext> for MixNode {
    /// Dispatches to the node's processor.
    ///
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        match self {
            MixNode::Sampler(node) => node.process(io, context),
            MixNode::AudioClip(node) => node.process(io, context),
            MixNode::StripInput(node) => node.process(io, context),
            MixNode::StripFx(node) => node.process(io, context),
            MixNode::StripOutput(node) => node.process(io, context),
            MixNode::MidiOut(node) => node.process(io, context),
        }
    }
}
