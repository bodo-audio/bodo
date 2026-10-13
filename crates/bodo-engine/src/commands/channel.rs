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
    /// The mixer insert it plays into; by default the first insert no channel uses yet.
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
    /// How late every second step plays, from 0 to 1.
    swing: f32,
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
        None => {
            let used: BTreeSet<_> = ctx.project.channels.values().filter_map(|c| c.target).collect();
            ctx.project.mixer.order.iter().copied().find(|i| !used.contains(i))
        }
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
    let id = ctx.channel(p.channel.as_ref())?;
    in_range("swing", f64::from(p.swing), 0.0..=1.0)?;
    ctx.edit_channel(id, |c| c.swing = p.swing)?;
    ctx.set_label(format!("Set {} swing", name(ctx, id)));
    Ok(json!({ "channel": id, "swing": p.swing }))
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

pub(super) fn register(r: &mut Registry) {
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
        .merge(by_channel);
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
