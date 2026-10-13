//! Stable graph and event keys derived from model ids.
//!
//! The graph and the sequencer address nodes and channels by `u64` keys. Each key mixes the id's ULID with a tag for
//! its kind, so a channel and an insert never share a key, and the same id always gives the same key.

use bodo_model::{ChannelId, InsertId, PatternId};

const CHANNEL: u64 = 0x1;
const INSERT: u64 = 0x2;
const PATTERN: u64 = 0x3;

fn mix(value: u128, tag: u64) -> u64 {
    let (low, high) = (value as u64, (value >> 64) as u64);
    low ^ high.rotate_left(23) ^ tag.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The key of a channel: its graph node and its note events.
pub fn channel(id: ChannelId) -> u64 {
    mix(id.ulid().0, CHANNEL)
}

/// The key of a mixer insert's graph node, the master included.
pub fn insert(id: InsertId) -> u64 {
    mix(id.ulid().0, INSERT)
}

/// The key of a pattern.
pub fn pattern(id: PatternId) -> u64 {
    mix(id.ulid().0, PATTERN)
}

/// The key of a graph edge from `from` to `to`.
pub fn edge(from: u64, to: u64) -> u64 {
    from.rotate_left(29) ^ to ^ 0xA076_1D64_78BD_642F
}
