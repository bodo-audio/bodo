//! The processors of the mix graph.

use bodo_kit_core::rt_safe;
use bodo_kit_graph::{NodeIo, Processor};

use crate::context::BlockContext;

pub mod audio_clip;
pub mod sampler;
pub mod strip;

pub use audio_clip::AudioClipNode;
pub use sampler::SamplerNode;
pub use strip::StripNode;

/// Every kind of node the mix graph has.
#[derive(Debug)]
pub enum MixNode {
    /// A sampler channel.
    Sampler(Box<SamplerNode>),
    /// An audio clip channel.
    AudioClip(Box<AudioClipNode>),
    /// A mixer insert or the master.
    Strip(StripNode),
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
            MixNode::Strip(node) => node.process(io, context),
        }
    }
}
