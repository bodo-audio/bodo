//! Stable graph and event keys derived from model ids.
//!
//! The graph and the sequencer address nodes and channels by `u64` keys. Each key mixes the id's ULID with a tag for
//! its kind, so a channel and an insert never share a key, and the same id always gives the same key.

use bodo_model::mixer::{RouteKey, RouteSource};
use bodo_model::{ChannelId, InsertId, PatternId, PluginInstanceId};

const CHANNEL: u64 = 0x1;
const INSERT: u64 = 0x2;
const PATTERN: u64 = 0x3;
const INSERT_INPUT: u64 = 0x4;
const INSERT_FX: u64 = 0x5;
const PLUGIN: u64 = 0x6;

fn mix(value: u128, tag: u64) -> u64 {
    let (low, high) = (value as u64, (value >> 64) as u64);
    low ^ high.rotate_left(23) ^ tag.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The key of a channel: its graph node and its note events.
pub fn channel(id: ChannelId) -> u64 {
    mix(id.ulid().0, CHANNEL)
}

/// The key of a mixer insert, the master included: the key of its output stage, and the key its automated fader,
/// pan and stereo separation are keyed by.
pub fn insert(id: InsertId) -> u64 {
    mix(id.ulid().0, INSERT)
}

/// The key of an insert's input stage (input sum, polarity, swap).
pub fn insert_input(id: InsertId) -> u64 {
    mix(id.ulid().0, INSERT_INPUT)
}

/// The key of an insert's FX stage (slots, EQ, stereo separation).
pub fn insert_fx(id: InsertId) -> u64 {
    mix(id.ulid().0, INSERT_FX)
}

/// The automation key of a slot's dry/wet mix (`which` 0) or on switch (`which` 1), for slot `index` of `insert`.
pub fn slot_param(insert: InsertId, index: u8, which: u8) -> u64 {
    insert_fx(insert).rotate_left(11) ^ (u64::from(index) << 8 | u64::from(which)).wrapping_mul(0xA24B_AED4_963E_E407)
}

/// The automation key of parameter `param` of plugin `instance`.
pub fn plugin_param(instance: PluginInstanceId, param: u32) -> u64 {
    mix(instance.ulid().0, PLUGIN).rotate_left(7) ^ u64::from(param).wrapping_mul(0x9FB2_1C65_1E98_DF25)
}

/// The key of the node a route leaves from, for `tap` where it leaves an insert.
pub fn route_source(from: RouteSource, tap: bodo_model::mixer::SendTap) -> u64 {
    match from {
        RouteSource::Channel(channel) => self::channel(channel),
        RouteSource::Insert(insert) => match tap {
            bodo_model::mixer::SendTap::PreFx => insert_input(insert),
            bodo_model::mixer::SendTap::PreFader => insert_fx(insert),
            bodo_model::mixer::SendTap::PostFader => self::insert(insert),
        },
    }
}

/// The edge key of an audio route, the same whatever its tap.
pub fn route(key: RouteKey) -> u64 {
    let from = match key.from {
        RouteSource::Channel(channel) => self::channel(channel),
        RouteSource::Insert(insert) => self::insert(insert),
    };
    edge(from, self::insert(key.to)) ^ 2
}

/// The automation key of a route's send level.
pub fn route_level(key: RouteKey) -> u64 {
    route(key).rotate_left(31) ^ 0x51DE_C4A1_7F3B_9E05
}

/// The key of a pattern.
pub fn pattern(id: PatternId) -> u64 {
    mix(id.ulid().0, PATTERN)
}

/// The key of a graph edge from `from` to `to`.
pub fn edge(from: u64, to: u64) -> u64 {
    from.rotate_left(29) ^ to ^ 0xA076_1D64_78BD_642F
}
