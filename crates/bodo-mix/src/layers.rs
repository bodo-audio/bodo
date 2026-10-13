//! Layer channels (project.md section 10.2): one trigger plays every child channel. Notes written on a layer, and
//! notes played live on it, go to its children instead; a child that is itself a layer passes them on to its own.

use bodo_model::channel::ChannelKind;
use bodo_model::{ChannelId, Project};

use crate::keys;

/// Every layer channel's event key and the event keys of the channels it plays, nested layers flattened, sorted by
/// the layer's key.
///
/// Allocates. Complexity: O(c + l) in channels and layer children.
pub fn layer_table(project: &Project) -> Vec<(u64, Vec<u64>)> {
    let mut table: Vec<(u64, Vec<u64>)> = project
        .channels
        .values()
        .filter(|c| matches!(c.kind, ChannelKind::Layer { .. }))
        .map(|c| {
            let mut out = Vec::new();
            let mut seen = vec![c.id];
            collect(project, c.id, &mut seen, &mut out);
            out.sort_unstable();
            out.dedup();
            (keys::channel(c.id), out)
        })
        .collect();
    table.sort_unstable_by_key(|(key, _)| *key);
    table
}

/// Adds the playing channels under layer `id` to `out`, never visiting a layer twice.
fn collect(project: &Project, id: ChannelId, seen: &mut Vec<ChannelId>, out: &mut Vec<u64>) {
    let Some(ChannelKind::Layer { children }) = project.channels.get(&id).map(|c| &c.kind) else { return };
    for child in children {
        if seen.contains(child) {
            continue;
        }
        seen.push(*child);
        match project.channels.get(child).map(|c| &c.kind) {
            Some(ChannelKind::Layer { .. }) => collect(project, *child, seen, out),
            Some(_) => out.push(keys::channel(*child)),
            None => {}
        }
    }
}

/// The channels a note on `key` plays, given `table`: the layer's children, or `key` itself for any other channel.
///
/// RT-safe. Complexity: O(log l) in layers.
pub fn targets(table: &[(u64, Vec<u64>)], key: u64) -> Targets<'_> {
    match table.binary_search_by_key(&key, |(k, _)| *k) {
        Ok(index) => table.get(index).map_or(Targets::One(key), |(_, children)| Targets::Many(children)),
        Err(_) => Targets::One(key),
    }
}

/// The channels a note plays.
#[derive(Clone, Copy, Debug)]
pub enum Targets<'a> {
    /// A plain channel.
    One(u64),
    /// A layer's children.
    Many(&'a [u64]),
}

impl Targets<'_> {
    /// The channels' event keys.
    pub fn keys(&self) -> &[u64] {
        match self {
            Targets::One(key) => std::slice::from_ref(key),
            Targets::Many(keys) => keys,
        }
    }
}
