//! The built-in commands, one module per namespace (project.md section 12.2).

mod audio;
mod automation;
mod browser;
mod channel;
mod clip;
mod edit;
mod import;
mod midi;
mod mixer;
mod note;
mod note_tools;
mod pattern;
mod playlist;
mod project;
mod record;
mod transport;

use bodo_kit_core::Tick;
use bodo_model::pattern::PatternLen;
use bodo_model::{InsertId, PatternId, Project};
use serde_json::Value;

use crate::registry::Registry;

pub(crate) use clip::slice_tracks as clip_slice;
pub use pattern::step_text as pattern_step_text;

/// Registers every built-in command.
pub(crate) fn register(registry: &mut Registry) {
    audio::register(registry);
    automation::register(registry);
    import::register(registry);
    midi::register(registry);
    note_tools::register(registry);
    record::register(registry);
    browser::register(registry);
    channel::register(registry);
    clip::register(registry);
    edit::register(registry);
    mixer::register(registry);
    note::register(registry);
    pattern::register(registry);
    playlist::register(registry);
    project::register(registry);
    transport::register(registry);
}

/// The text of param `name`, for merge keys.
fn field(params: &Value, name: &str) -> String {
    params.get(name).map(Value::to_string).unwrap_or_default()
}

/// Merge key: the `channel` param.
fn by_channel(params: &Value) -> String {
    field(params, "channel")
}

/// Merge key: the `insert` param.
fn by_insert(params: &Value) -> String {
    field(params, "insert")
}

/// Merge key: the pattern and the notes.
fn by_notes(params: &Value) -> String {
    format!("{}{}", field(params, "pattern"), field(params, "notes"))
}

/// Merge key: the clips.
fn by_clips(params: &Value) -> String {
    field(params, "clips")
}

/// Merge key: the route's two ends.
fn by_route(params: &Value) -> String {
    format!("{}>{}", field(params, "from"), field(params, "to"))
}

/// Merge key for commands with one target per project, such as the tempo.
fn whole(_: &Value) -> String {
    String::new()
}

/// `base`, or `base 2`, `base 3` and so on, whichever no name in `taken` uses (ignoring case).
///
/// Complexity: O(n log n) in names.
fn unique_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: std::collections::BTreeSet<String> = taken.into_iter().map(str::to_lowercase).collect();
    if !taken.contains(&base.to_lowercase()) {
        return base.to_owned();
    }
    (2..).map(|n| format!("{base} {n}")).find(|name| !taken.contains(&name.to_lowercase())).unwrap_or_else(|| base.to_owned())
}

/// Calls `f` on every value of `map` that `pick` selects, storing the changed values.
///
/// Complexity: O(n log n) in entries.
fn update_where<K: Ord + Clone, V: Clone>(map: &mut imbl::OrdMap<K, V>, pick: impl Fn(&V) -> bool, mut f: impl FnMut(&mut V)) {
    let keys: Vec<K> = map.iter().filter(|(_, v)| pick(v)).map(|(k, _)| k.clone()).collect();
    for key in keys {
        if let Some(value) = map.get_mut(&key) {
            f(value);
        }
    }
}

/// Removes every entry of `map` whose key and value `drop` selects.
///
/// Complexity: O(n log n) in entries.
fn remove_where<K: Ord + Clone, V: Clone>(map: &mut imbl::OrdMap<K, V>, drop: impl Fn(&K, &V) -> bool) {
    let keys: Vec<K> = map.iter().filter(|(k, v)| drop(k, v)).map(|(k, _)| k.clone()).collect();
    for key in keys {
        map.remove(&key);
    }
}

/// How long pattern `id` plays: its fixed length, or its notes rounded up to whole bars and at least one bar. The same
/// rule the sequencer loops by.
///
/// Complexity: O(c log n) in the pattern's channels and notes.
pub fn pattern_length(project: &Project, id: PatternId) -> Tick {
    let bar = project.meter.signature_at(Tick::ZERO).bar_ticks().max(1);
    match project.patterns.get(&id).map(|p| (p.length, p.content_end())) {
        Some((PatternLen::Fixed(len), _)) => len,
        Some((PatternLen::Auto, end)) => Tick((end.0.max(1) + bar - 1) / bar * bar),
        None => Tick(bar),
    }
}

/// The display number of insert `id`: 0 for the master, then 1, 2, 3 in mixer order.
///
/// Complexity: O(i) in inserts.
pub fn insert_number(project: &Project, id: InsertId) -> Option<usize> {
    if project.mixer.master.id == id {
        return Some(0);
    }
    project.mixer.order.iter().position(|i| *i == id).map(|n| n + 1)
}

/// Checks that `value` lies in `range`, naming the param in the error.
fn in_range(name: &str, value: f64, range: std::ops::RangeInclusive<f64>) -> Result<(), crate::error::CommandError> {
    if value.is_finite() && range.contains(&value) {
        Ok(())
    } else {
        Err(crate::error::CommandError::invalid(format!("{name} must be from {} to {}, not {value}", range.start(), range.end())))
    }
}
