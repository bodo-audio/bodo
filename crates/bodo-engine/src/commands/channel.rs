//! `channel.*`: the channels of the channel rack (project.md section 10.2).

use std::collections::BTreeSet;
use std::sync::Arc;

use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::mixer::RouteSource;
use bodo_model::playlist::ClipSource;
use bodo_model::{Channel, ChannelId, SampleId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{by_channel, in_range, insert_number, remove_where, unique_name, update_where};
use crate::error::CommandError;
use crate::palette;
use crate::params::{ChannelRef, ColorText, InsertRef, Key};
use crate::registry::{Effects, Registry};
use crate::samples;
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Add {
    /// The audio file its sampler plays: a `factory:` path from browser.search, an absolute path, or a path inside
    /// the project's folder. Without one the sampler starts empty.
    sample: Option<String>,
    /// Its name; the sample's file name by default.
    name: Option<String>,
    /// The mixer insert it plays into; none by default, so it plays into the master until channel.assign_insert
    /// gives it a mixer track.
    target: Option<InsertRef>,
    /// Its color; the next palette color by default.
    color: Option<ColorText>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Channels {
    /// The channels; the selected channels by default.
    #[serde(default)]
    channels: Vec<ChannelRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct One {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Rename {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The new name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetColor {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The new color.
    color: ColorText,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reorder {
    /// The channel to move.
    channel: ChannelRef,
    /// Its new position in the channel rack, from 1.
    to: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssignInsert {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The mixer track. By default the one selected in the mixer, or else a free one: the first that no channel plays
    /// into and that still has its default name, made after the last when every track is taken.
    insert: Option<InsertRef>,
    /// Take a free mixer track even when one is selected in the mixer.
    #[serde(default)]
    free: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OneChannel {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetTarget {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The insert it plays into; 0 or "master" for the master.
    insert: InsertRef,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetVolume {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The volume as a linear gain from 0 to 2; 1 is unity (0 dB).
    volume: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPan {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// Pan from -1 (left) to 1 (right); 0 is center.
    pan: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPitch {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// Pitch shift in cents, from -2400 to 2400.
    cents: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetSwing {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// Or: several channels at once, such as every channel of a group.
    #[serde(default)]
    channels: Vec<ChannelRef>,
    /// How late every second step plays, from 0 to 1.
    swing: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetGroup {
    /// The channels; the selected channels by default.
    #[serde(default)]
    channels: Vec<ChannelRef>,
    /// The group they join, by name; null or an empty name leaves them unsorted.
    #[serde(default)]
    group: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameGroup {
    /// The group's current name.
    group: String,
    /// Its new name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mute {
    /// The channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// Muted or not; toggles when left out.
    muted: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetSample {
    /// The sampler channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The audio file: a `factory:` path, an absolute path, or a path inside the project's folder.
    sample: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetRootKey {
    /// The sampler channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The key that plays the sample at its own pitch. Steps play this key.
    key: Key,
}

/// Merge key: the `channel` param, or the `channels` list, so a swing knob dragged for a whole group is one step.
fn by_channels(params: &Value) -> String {
    format!("{}{}", by_channel(params), params.get("channels").map(Value::to_string).unwrap_or_default())
}

/// The name of channel `id`, for undo labels.
fn name(ctx: &Ctx<'_>, id: ChannelId) -> String {
    ctx.project.channels.get(&id).map(|c| c.name.clone()).unwrap_or_default()
}

/// The sampler settings of channel `id`, or an error naming the channel.
fn sampler(ctx: &Ctx<'_>, id: ChannelId) -> Result<Arc<SamplerParams>, CommandError> {
    match ctx.project.channels.get(&id).map(|c| &c.kind) {
        Some(ChannelKind::Sampler(params)) => Ok(Arc::clone(params)),
        _ => Err(CommandError::rejected(format!("{} is not a sampler channel", name(ctx, id)))),
    }
}

fn add(ctx: &mut Ctx<'_>, p: Add) -> Result<Value, CommandError> {
    let sample: Option<SampleId> = p.sample.as_deref().map(|s| samples::import(ctx, s)).transpose()?;
    let base = p.name.filter(|n| !n.trim().is_empty()).or_else(|| p.sample.as_deref().map(samples::display_name)).unwrap_or_else(|| "Sampler".into());
    let name = unique_name(base.trim(), ctx.project.channels.values().map(|c| c.name.as_str()));
    let target = match &p.target {
        Some(r) => Some(ctx.insert(Some(r))?).filter(|i| *i != ctx.project.mixer.master.id),
        None => None,
    };
    let color = match p.color {
        Some(c) => c.resolve()?,
        None => palette::channel_color(ctx.project.channels.len()),
    };
    let id = ctx.ids().channel();
    let channel = Channel {
        id,
        name: name.clone(),
        color,
        kind: ChannelKind::Sampler(Arc::new(SamplerParams { sample, ..SamplerParams::default() })),
        target,
        volume: 1.0,
        pan: 0.0,
        pitch_cents: 0.0,
        swing: 0.0,
        muted: false,
        midi_in: None,
        group: None,
    };
    ctx.project.channels.insert(id, Arc::new(channel));
    ctx.project.rack_order.push_back(id);
    ctx.editor.channel = Some(id);
    ctx.editor.channels = BTreeSet::from([id]);
    ctx.set_label(format!("Add channel {name}"));
    let target = target.and_then(|t| insert_number(&ctx.project, t)).unwrap_or(0);
    Ok(json!({ "channel": id, "name": name, "insert": target, "sample": sample }))
}

fn remove(ctx: &mut Ctx<'_>, p: Channels) -> Result<Value, CommandError> {
    let ids = ctx.channels(&p.channels)?;
    let gone: BTreeSet<ChannelId> = ids.iter().copied().collect();
    for id in &ids {
        ctx.project.channels.remove(id);
    }
    ctx.project.rack_order.retain(|id| !gone.contains(id));
    // Notes of the removed channels, and clip-mode bindings to them.
    let patterns: Vec<_> = ctx.project.patterns.keys().copied().collect();
    for pattern in patterns {
        ctx.edit_pattern(pattern, |p| {
            let before = p.notes.len();
            remove_where(&mut p.notes, |c, _| gone.contains(c));
            let unbind = p.binding.is_some_and(|b| gone.contains(&b));
            if unbind {
                p.binding = None;
            }
            Ok((p.notes.len() != before || unbind, ()))
        })?;
    }
    // Audio and automation clips that play them.
    let tracks: Vec<_> = ctx.project.tracks.keys().copied().collect();
    for track in tracks {
        ctx.edit_track(track, |t| {
            t.clips.retain(|c| !matches!(c.source, ClipSource::Audio(ch) | ClipSource::Automation(ch) if gone.contains(&ch)));
            Ok(())
        })?;
    }
    // Layers that contain them.
    let layers: Vec<ChannelId> = ctx
        .project
        .channels
        .values()
        .filter(|c| matches!(&c.kind, ChannelKind::Layer { children } if children.iter().any(|ch| gone.contains(ch))))
        .map(|c| c.id)
        .collect();
    for layer in layers {
        ctx.edit_channel(layer, |c| {
            if let ChannelKind::Layer { children } = &mut c.kind {
                children.retain(|ch| !gone.contains(ch));
            }
        })?;
    }
    // Routes from them, and slot sidechains fed by them.
    let is_gone = |source: &RouteSource| matches!(source, RouteSource::Channel(c) if gone.contains(c));
    if ctx.project.mixer.routes.keys().any(|k| is_gone(&k.from))
        || ctx
            .project
            .mixer
            .inserts
            .values()
            .chain(std::iter::once(&ctx.project.mixer.master))
            .any(|i| i.slots.iter().any(|s| s.sidechain.as_ref().is_some_and(is_gone)))
    {
        let mixer = Arc::make_mut(&mut ctx.project.mixer);
        remove_where(&mut mixer.routes, |k, _| is_gone(&k.from));
        let fed = |insert: &Arc<bodo_model::MixerInsert>| insert.slots.iter().any(|s| s.sidechain.as_ref().is_some_and(is_gone));
        let unhook = |insert: &mut Arc<bodo_model::MixerInsert>| {
            for slot in &mut Arc::make_mut(insert).slots {
                if slot.sidechain.as_ref().is_some_and(is_gone) {
                    slot.sidechain = None;
                }
            }
        };
        if fed(&mixer.master) {
            unhook(&mut mixer.master);
        }
        update_where(&mut mixer.inserts, fed, unhook);
    }
    // Samples nothing plays any more.
    let used: BTreeSet<SampleId> = ctx
        .project
        .channels
        .values()
        .filter_map(|c| match &c.kind {
            ChannelKind::Sampler(params) => params.sample,
            ChannelKind::AudioClip { sample, .. } => Some(*sample),
            _ => None,
        })
        .collect();
    remove_where(&mut ctx.project.samples, |id, _| !used.contains(id));
    ctx.set_label(if ids.len() == 1 { String::from("Remove channel") } else { format!("Remove {} channels", ids.len()) });
    Ok(json!({ "removed": ids }))
}

fn duplicate(ctx: &mut Ctx<'_>, p: One) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let original = ctx.project.channels.get(&id).cloned().ok_or_else(|| CommandError::not_found("channel", &id.to_string()))?;
    let copy = ctx.ids().channel();
    let name = unique_name(&original.name, ctx.project.channels.values().map(|c| c.name.as_str()));
    ctx.project.channels.insert(copy, Arc::new(Channel { id: copy, name: name.clone(), ..(*original).clone() }));
    let at = ctx.project.rack_order.index_of(&id).map_or(ctx.project.rack_order.len(), |i| i + 1);
    ctx.project.rack_order.insert(at, copy);
    ctx.editor.channel = Some(copy);
    ctx.editor.channels = BTreeSet::from([copy]);
    ctx.set_label(format!("Duplicate {}", original.name));
    Ok(json!({ "channel": copy, "name": name }))
}

fn rename(ctx: &mut Ctx<'_>, p: Rename) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let new = p.name.trim().to_owned();
    if new.is_empty() {
        return Err(CommandError::invalid("a channel name can't be empty"));
    }
    ctx.edit_channel(id, |c| c.name.clone_from(&new))?;
    ctx.set_label(format!("Rename channel to {new}"));
    Ok(json!({ "channel": id, "name": new }))
}

fn set_color(ctx: &mut Ctx<'_>, p: SetColor) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let color = p.color.resolve()?;
    ctx.edit_channel(id, |c| c.color = color)?;
    Ok(json!({ "channel": id, "color": color }))
}

fn reorder(ctx: &mut Ctx<'_>, p: Reorder) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let count = ctx.project.rack_order.len();
    if p.to == 0 || p.to > count {
        return Err(CommandError::invalid(format!("to must be from 1 to {count}")));
    }
    if ctx.project.rack_order.index_of(&id) != Some(p.to - 1) {
        ctx.project.rack_order.retain(|c| *c != id);
        ctx.project.rack_order.insert(p.to - 1, id);
    }
    ctx.set_label(format!("Move {}", name(ctx, id)));
    Ok(json!({ "channel": id, "position": p.to }))
}

fn set_target(ctx: &mut Ctx<'_>, p: SetTarget) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let insert = ctx.insert(Some(&p.insert))?;
    let target = (insert != ctx.project.mixer.master.id).then_some(insert);
    ctx.edit_channel(id, |c| c.target = target)?;
    let number = insert_number(&ctx.project, insert).unwrap_or(0);
    ctx.set_label(format!("Route {} to insert {number}", name(ctx, id)));
    Ok(json!({ "channel": id, "insert": number }))
}

/// Whether `name` is the name a mixer insert starts with, such as "Insert 5".
fn default_insert_name(name: &str) -> bool {
    name.strip_prefix("Insert ").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// A mixer insert no channel plays into that still has its default name, or a new one after the last.
///
/// Complexity: O(c + i) in channels and inserts.
fn free_insert(ctx: &mut Ctx<'_>) -> bodo_model::InsertId {
    let used: BTreeSet<_> = ctx.project.channels.values().filter_map(|c| c.target).collect();
    let mixer = &ctx.project.mixer;
    if let Some(found) =
        mixer.order.iter().copied().find(|i| !used.contains(i) && mixer.inserts.get(i).is_some_and(|insert| default_insert_name(&insert.name)))
    {
        return found;
    }
    let count = mixer.order.len();
    let name = unique_name(&format!("Insert {}", count + 1), mixer.inserts.values().map(|i| i.name.as_str()));
    let id = ctx.ids().insert();
    let mixer = Arc::make_mut(&mut ctx.project.mixer);
    mixer.inserts.insert(id, Arc::new(bodo_model::MixerInsert::new(id, name)));
    mixer.order.push_back(id);
    id
}

fn assign_insert(ctx: &mut Ctx<'_>, p: AssignInsert) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let master = ctx.project.mixer.master.id;
    let chosen = match (&p.insert, p.free) {
        (Some(r), _) => Some(ctx.insert(Some(r))?),
        (None, false) => ctx.editor.insert.filter(|i| *i != master && ctx.project.mixer.inserts.contains_key(i)),
        (None, true) => None,
    };
    if chosen == Some(master) {
        return Err(CommandError::rejected("the master is not a mixer track a channel is assigned to")
            .with_hint("channel.unassign_insert makes a channel play into the master"));
    }
    let insert = match chosen {
        Some(insert) => insert,
        None => free_insert(ctx),
    };
    ctx.edit_channel(id, |c| c.target = Some(insert))?;
    // A track that still has its default name takes the channel's; one the user named keeps its name.
    let channel_name = name(ctx, id);
    let renamed = ctx.project.mixer.inserts.get(&insert).is_some_and(|i| default_insert_name(&i.name));
    if renamed {
        let new_name = unique_name(&channel_name, ctx.project.mixer.inserts.values().filter(|i| i.id != insert).map(|i| i.name.as_str()));
        let mixer = Arc::make_mut(&mut ctx.project.mixer);
        if let Some(track) = mixer.inserts.get_mut(&insert) {
            Arc::make_mut(track).name = new_name;
        }
    }
    let number = insert_number(&ctx.project, insert).unwrap_or(0);
    let track_name = ctx.project.mixer.inserts.get(&insert).map(|i| i.name.clone()).unwrap_or_default();
    ctx.set_label(format!("Assign {channel_name} to mixer track {number}"));
    Ok(json!({ "channel": id, "insert": number, "name": track_name, "renamed": renamed }))
}

fn unassign_insert(ctx: &mut Ctx<'_>, p: OneChannel) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    ctx.edit_channel(id, |c| c.target = None)?;
    ctx.set_label(format!("Unassign {} from its mixer track", name(ctx, id)));
    Ok(json!({ "channel": id, "insert": 0 }))
}

fn set_volume(ctx: &mut Ctx<'_>, p: SetVolume) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    in_range("volume", f64::from(p.volume), 0.0..=2.0)?;
    ctx.edit_channel(id, |c| c.volume = p.volume)?;
    ctx.set_label(format!("Set {} volume", name(ctx, id)));
    Ok(json!({ "channel": id, "volume": p.volume }))
}

fn set_pan(ctx: &mut Ctx<'_>, p: SetPan) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    in_range("pan", f64::from(p.pan), -1.0..=1.0)?;
    ctx.edit_channel(id, |c| c.pan = p.pan)?;
    ctx.set_label(format!("Set {} pan", name(ctx, id)));
    Ok(json!({ "channel": id, "pan": p.pan }))
}

fn set_pitch(ctx: &mut Ctx<'_>, p: SetPitch) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    in_range("cents", f64::from(p.cents), -2400.0..=2400.0)?;
    ctx.edit_channel(id, |c| c.pitch_cents = p.cents)?;
    ctx.set_label(format!("Set {} pitch", name(ctx, id)));
    Ok(json!({ "channel": id, "cents": p.cents }))
}

fn set_swing(ctx: &mut Ctx<'_>, p: SetSwing) -> Result<Value, CommandError> {
    in_range("swing", f64::from(p.swing), 0.0..=1.0)?;
    if !p.channels.is_empty() {
        let ids = ctx.channels(&p.channels)?;
        for id in &ids {
            ctx.edit_channel(*id, |c| c.swing = p.swing)?;
        }
        ctx.set_label(format!("Set swing of {} channels", ids.len()));
        return Ok(json!({ "channels": ids, "swing": p.swing }));
    }
    let id = ctx.channel(p.channel.as_ref())?;
    ctx.edit_channel(id, |c| c.swing = p.swing)?;
    ctx.set_label(format!("Set {} swing", name(ctx, id)));
    Ok(json!({ "channel": id, "swing": p.swing }))
}

fn set_group(ctx: &mut Ctx<'_>, p: SetGroup) -> Result<Value, CommandError> {
    let ids = ctx.channels(&p.channels)?;
    let group = p.group.as_deref().map(str::trim).filter(|g| !g.is_empty()).map(str::to_owned);
    for id in &ids {
        ctx.edit_channel(*id, |c| c.group.clone_from(&group))?;
    }
    ctx.set_label(match &group {
        Some(g) => format!("Move {} to group {g}", if ids.len() == 1 { name(ctx, ids[0]) } else { format!("{} channels", ids.len()) }),
        None => String::from("Ungroup channels"),
    });
    Ok(json!({ "channels": ids, "group": group }))
}

fn rename_group(ctx: &mut Ctx<'_>, p: RenameGroup) -> Result<Value, CommandError> {
    let new = p.name.trim().to_owned();
    if new.is_empty() {
        return Err(CommandError::invalid("a group name can't be empty"));
    }
    let members: Vec<ChannelId> = ctx.project.channels.values().filter(|c| c.group.as_deref() == Some(p.group.as_str())).map(|c| c.id).collect();
    if members.is_empty() {
        return Err(CommandError::not_found("channel group", &p.group));
    }
    if new != p.group && ctx.project.channels.values().any(|c| c.group.as_deref() == Some(new.as_str())) {
        return Err(CommandError::rejected(format!("there is already a channel group named {new}")));
    }
    for id in &members {
        ctx.edit_channel(*id, |c| c.group = Some(new.clone()))?;
    }
    ctx.set_label(format!("Rename channel group to {new}"));
    Ok(json!({ "group": new, "channels": members.len() }))
}

fn mute(ctx: &mut Ctx<'_>, p: Mute) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let muted = p.muted.unwrap_or_else(|| !ctx.project.channels.get(&id).is_some_and(|c| c.muted));
    ctx.edit_channel(id, |c| c.muted = muted)?;
    ctx.set_label(format!("{} {}", if muted { "Mute" } else { "Unmute" }, name(ctx, id)));
    Ok(json!({ "channel": id, "muted": muted }))
}

fn set_sample(ctx: &mut Ctx<'_>, p: SetSample) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let params = sampler(ctx, id)?;
    let sample = samples::import(ctx, &p.sample)?;
    ctx.edit_channel(id, |c| c.kind = ChannelKind::Sampler(Arc::new(SamplerParams { sample: Some(sample), ..(*params).clone() })))?;
    ctx.set_label(format!("Load {} into {}", samples::display_name(&p.sample), name(ctx, id)));
    Ok(json!({ "channel": id, "sample": sample }))
}

fn set_root_key(ctx: &mut Ctx<'_>, p: SetRootKey) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let params = sampler(ctx, id)?;
    let key = p.key.resolve()?;
    ctx.edit_channel(id, |c| c.kind = ChannelKind::Sampler(Arc::new(SamplerParams { root_key: key, ..(*params).clone() })))?;
    Ok(json!({ "channel": id, "rootKey": key }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddLayer {
    /// Its name; "Layer" by default.
    name: Option<String>,
    /// The channels it plays; the selected channels by default.
    #[serde(default)]
    children: Vec<ChannelRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetLayer {
    /// The layer channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// Every channel it plays, replacing the ones it had.
    children: Vec<ChannelRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddMidiOut {
    /// Its name; the port's name by default.
    name: Option<String>,
    /// The MIDI output port it sends to, by name.
    port: String,
    /// The MIDI channel, 1 to 16; 1 by default.
    midi_channel: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetMidiOut {
    /// The MIDI Out channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The MIDI output port it sends to.
    port: Option<String>,
    /// The MIDI channel, 1 to 16.
    midi_channel: Option<u8>,
}

/// A new channel of `kind` named `base` (or a free variant of it), at the end of the channel rack and selected.
fn add_kind(ctx: &mut Ctx<'_>, base: &str, kind: ChannelKind) -> (ChannelId, String) {
    let name = unique_name(base.trim(), ctx.project.channels.values().map(|c| c.name.as_str()));
    let id = ctx.ids().channel();
    let channel = Channel {
        id,
        name: name.clone(),
        color: palette::channel_color(ctx.project.channels.len()),
        kind,
        target: None,
        volume: 1.0,
        pan: 0.0,
        pitch_cents: 0.0,
        swing: 0.0,
        muted: false,
        midi_in: None,
        group: None,
    };
    ctx.project.channels.insert(id, Arc::new(channel));
    ctx.project.rack_order.push_back(id);
    ctx.editor.channel = Some(id);
    ctx.editor.channels = BTreeSet::from([id]);
    (id, name)
}

/// The children for layer `layer`: they must exist, and none may be the layer or a layer that plays it.
fn layer_children(ctx: &Ctx<'_>, layer: Option<ChannelId>, refs: &[ChannelRef]) -> Result<imbl::Vector<ChannelId>, CommandError> {
    let mut children = imbl::Vector::new();
    for r in refs {
        let id = ctx.channel(Some(r))?;
        if let Some(layer) = layer
            && (id == layer || plays(&ctx.project, id, layer))
        {
            return Err(CommandError::rejected(format!("{} would play itself through {}", name(ctx, layer), name(ctx, id))));
        }
        if !children.contains(&id) {
            children.push_back(id);
        }
    }
    Ok(children)
}

/// Whether channel `from`, a layer, plays `target` through its children.
fn plays(project: &bodo_model::Project, from: ChannelId, target: ChannelId) -> bool {
    let mut stack = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(ChannelKind::Layer { children }) = project.channels.get(&id).map(|c| &c.kind) {
            if children.contains(&target) {
                return true;
            }
            stack.extend(children.iter().copied());
        }
    }
    false
}

fn add_layer(ctx: &mut Ctx<'_>, p: AddLayer) -> Result<Value, CommandError> {
    let refs = if p.children.is_empty() { ctx.editor.channels.iter().map(|id| ChannelRef::new(id.to_string())).collect() } else { p.children };
    let children = layer_children(ctx, None, &refs)?;
    let base = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| String::from("Layer"));
    let (id, name) = add_kind(ctx, &base, ChannelKind::Layer { children: children.clone() });
    ctx.set_label(format!("Add layer {name}"));
    Ok(json!({ "channel": id, "name": name, "children": children.iter().collect::<Vec<_>>() }))
}

fn set_layer(ctx: &mut Ctx<'_>, p: SetLayer) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    if !matches!(ctx.project.channels.get(&id).map(|c| &c.kind), Some(ChannelKind::Layer { .. })) {
        return Err(CommandError::rejected(format!("{} is not a layer", name(ctx, id))));
    }
    let children = layer_children(ctx, Some(id), &p.children)?;
    let result: Vec<ChannelId> = children.iter().copied().collect();
    ctx.edit_channel(id, |c| c.kind = ChannelKind::Layer { children })?;
    ctx.set_label(format!("Set the channels {} plays", name(ctx, id)));
    Ok(json!({ "channel": id, "children": result }))
}

/// A MIDI channel from 1 to 16, stored from 0.
fn midi_channel(value: u8) -> Result<u8, CommandError> {
    in_range("midiChannel", f64::from(value), 1.0..=16.0)?;
    Ok(value - 1)
}

fn add_midi_out(ctx: &mut Ctx<'_>, p: AddMidiOut) -> Result<Value, CommandError> {
    if p.port.trim().is_empty() {
        return Err(CommandError::invalid("give the MIDI port's name"));
    }
    let channel = midi_channel(p.midi_channel.unwrap_or(1))?;
    let base = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| p.port.clone());
    let (id, name) = add_kind(ctx, &base, ChannelKind::MidiOut { port: p.port.clone(), channel });
    ctx.set_label(format!("Add MIDI Out {name}"));
    Ok(json!({ "channel": id, "name": name, "port": p.port, "midiChannel": channel + 1 }))
}

fn set_midi_out(ctx: &mut Ctx<'_>, p: SetMidiOut) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let Some(ChannelKind::MidiOut { port, channel }) = ctx.project.channels.get(&id).map(|c| c.kind.clone()) else {
        return Err(CommandError::rejected(format!("{} is not a MIDI Out channel", name(ctx, id))));
    };
    let port = p.port.unwrap_or(port);
    let channel = match p.midi_channel {
        Some(value) => midi_channel(value)?,
        None => channel,
    };
    ctx.edit_channel(id, |c| c.kind = ChannelKind::MidiOut { port: port.clone(), channel })?;
    ctx.set_label(format!("Set the MIDI port of {}", name(ctx, id)));
    Ok(json!({ "channel": id, "port": port, "midiChannel": channel + 1 }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "channel.add_layer",
        "Add layer",
        "Adds a layer channel, which plays every one of its children on each note.",
        Effects::UNDOABLE,
        add_layer,
    )
    .example(json!({ "name": "Big drums", "children": ["Kick", "Clap"] }))
    .menu("Add/Layer");
    r.edit("channel.set_layer", "Set layer children", "Sets the channels a layer plays.", Effects::UNDOABLE, set_layer)
        .example(json!({ "channel": "Big drums", "children": ["Kick", "Snare"] }));
    r.edit("channel.add_midi_out", "Add MIDI Out", "Adds a channel that sends its notes to an external MIDI port.", Effects::UNDOABLE, add_midi_out)
        .example(json!({ "port": "IAC Bus 1", "midiChannel": 2 }))
        .menu("Add/MIDI Out");
    r.edit(
        "channel.set_midi_out",
        "Set MIDI Out port",
        "Sets the port and MIDI channel a MIDI Out channel sends to.",
        Effects::UNDOABLE,
        set_midi_out,
    )
    .example(json!({ "channel": "IAC Bus 1", "midiChannel": 10 }));
    r.edit("channel.add", "Add channel", "Adds a sampler channel to the channel rack, optionally loading a sample.", Effects::UNDOABLE, add)
        .example(json!({ "sample": "factory:Packs/Bodo Drums/Kick.wav", "name": "Kick" }))
        .menu("Add/Channel");
    r.edit("channel.remove", "Remove channels", "Removes channels with their notes, clips and routes.", Effects::UNDOABLE, remove)
        .example(json!({ "channels": ["Clap"] }));
    r.edit(
        "channel.duplicate",
        "Duplicate channel",
        "Copies a channel's settings into a new channel after it (notes are not copied).",
        Effects::UNDOABLE,
        duplicate,
    )
    .example(json!({ "channel": "Kick" }));
    r.edit("channel.rename", "Rename channel", "Renames a channel.", Effects::UNDOABLE, rename)
        .example(json!({ "channel": "Kick", "name": "Big Kick" }));
    r.edit("channel.set_color", "Set channel color", "Sets a channel's color.", Effects::UNDOABLE, set_color)
        .example(json!({ "channel": "Kick", "color": "#f87171" }));
    r.edit("channel.reorder", "Move channel", "Moves a channel to another position in the channel rack.", Effects::UNDOABLE, reorder)
        .example(json!({ "channel": "Snare", "to": 1 }));
    r.edit(
        "channel.assign_insert",
        "Assign to mixer track",
        "Gives a channel its own mixer track: the one selected in the mixer, or a free one; a track with its default name takes the channel's name.",
        Effects::UNDOABLE,
        assign_insert,
    )
    .example(json!({ "channel": "Kick" }));
    r.edit("channel.unassign_insert", "Unassign from mixer track", "Makes a channel play into the master again.", Effects::UNDOABLE, unassign_insert)
        .example(json!({ "channel": "Kick" }));
    r.edit("channel.set_target_insert", "Set channel insert", "Sets the mixer insert a channel plays into.", Effects::UNDOABLE, set_target)
        .example(json!({ "channel": "Kick", "insert": 5 }));
    r.edit("channel.set_volume", "Set channel volume", "Sets a channel's volume (linear, 1 is unity).", Effects::UNDOABLE, set_volume)
        .example(json!({ "channel": "Kick", "volume": 0.8 }))
        .merge(by_channel);
    r.edit("channel.set_pan", "Set channel pan", "Sets a channel's pan from -1 (left) to 1 (right).", Effects::UNDOABLE, set_pan)
        .example(json!({ "channel": "HiHat", "pan": 0.25 }))
        .merge(by_channel);
    r.edit("channel.set_pitch", "Set channel pitch", "Shifts a channel's pitch in cents.", Effects::UNDOABLE, set_pitch)
        .example(json!({ "channel": "Snare", "cents": -100 }))
        .merge(by_channel);
    r.edit("channel.set_swing", "Set channel swing", "Sets how late every second step plays, from 0 to 1.", Effects::UNDOABLE, set_swing)
        .example(json!({ "channel": "HiHat", "swing": 0.55 }))
        .merge(by_channels);
    r.edit("channel.set_group", "Set channel group", "Puts channels in a channel rack group, or leaves them unsorted.", Effects::UNDOABLE, set_group)
        .example(json!({ "channels": ["Kick", "Snare"], "group": "Drums" }));
    r.edit("channel.rename_group", "Rename channel group", "Renames a channel rack group.", Effects::UNDOABLE, rename_group)
        .example(json!({ "group": "Drums", "name": "Beat" }));
    r.edit("channel.mute", "Mute channel", "Mutes or unmutes a channel; toggles without `muted`.", Effects::UNDOABLE, mute)
        .example(json!({ "channel": "Clap" }));
    r.edit("channel.set_sample", "Load sample", "Loads an audio file into a sampler channel.", Effects::UNDOABLE, set_sample)
        .example(json!({ "channel": "Snare", "sample": "factory:Packs/Bodo Drums/Rim.wav" }));
    r.edit(
        "channel.set_root_key",
        "Set root key",
        "Sets the key that plays a sampler's sample at its own pitch; steps play this key.",
        Effects::UNDOABLE,
        set_root_key,
    )
    .example(json!({ "channel": "Kick", "key": "C4" }));
}
