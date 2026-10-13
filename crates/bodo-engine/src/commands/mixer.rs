//! `mixer.*`: inserts and routes (project.md section 9).
//!
//! Volumes are in dB from -144 (silence) to +6. Routes run from a channel or an insert to another insert; a route
//! that would close a loop between inserts is refused with the loop's path.

use std::sync::Arc;

use bodo_kit_dsp::pan::PanLaw;
use bodo_kit_graph::{DagError, IncrementalDag};
use bodo_model::mixer::{HwPort, PdcBypass, Route, RouteKey, RouteKind, RouteSource, SendTap};
use bodo_model::{InsertId, MixerInsert, Project};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{by_insert, by_route, in_range, insert_number, remove_where, unique_name, update_where};
use crate::error::CommandError;
use crate::params::{ColorText, InsertRef, RouteFromRef};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

/// The quietest fader setting; it stands for minus infinity, which JSON can't hold.
pub const SILENT_DB: f32 = -144.0;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddInsert {
    /// Its name; "Insert N" by default.
    name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OneInsert {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Rename {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The new name.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetColor {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The new color.
    color: ColorText,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetVolume {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The fader in dB, from -144 (silence) to 6.
    db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPan {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// Pan from -1 (left) to 1 (right).
    pan: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetStereoSep {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// Stereo separation from -1 (twice as wide) through 0 (unchanged) to 1 (mono).
    separation: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Switch {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// On or off; toggles when left out.
    on: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddRoute {
    /// Where it starts: a channel or an insert.
    from: RouteFromRef,
    /// The insert it goes to (never the master; inserts reach it on their own).
    to: InsertRef,
    /// The send level as a linear gain from 0 to 2; 1 by default.
    level: Option<f32>,
    /// Feed only the sidechain inputs of the target's plugins; false by default.
    #[serde(default)]
    sidechain: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteEnds {
    /// Where it starts: a channel or an insert.
    from: RouteFromRef,
    /// The insert it goes to.
    to: InsertRef,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetRoute {
    /// Where it starts: a channel or an insert.
    from: RouteFromRef,
    /// The insert it goes to.
    to: InsertRef,
    /// The send level as a linear gain from 0 to 2.
    level: Option<f32>,
    /// Where on the source strip it taps.
    tap: Option<Tap>,
    /// True to feed only the target's plugin sidechains, false to carry audio.
    sidechain: Option<bool>,
}

/// Where a send takes its signal.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Tap {
    /// After polarity and swap, before the FX.
    PreFx,
    /// After the FX, EQ and stereo separation, before pan and the fader.
    PreFader,
    /// After the fader and mute.
    PostFader,
}

impl From<Tap> for SendTap {
    fn from(tap: Tap) -> Self {
        match tap {
            Tap::PreFx => Self::PreFx,
            Tap::PreFader => Self::PreFader,
            Tap::PostFader => Self::PostFader,
        }
    }
}

/// The name of insert `id`, for undo labels.
fn label(project: &Project, id: InsertId) -> String {
    project.mixer.insert(id).map(|i| i.name.clone()).unwrap_or_default()
}

fn switch(ctx: &mut Ctx<'_>, p: &Switch, get: fn(&MixerInsert) -> bool, set: fn(&mut MixerInsert, bool)) -> Result<(InsertId, bool), CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let on = p.on.unwrap_or_else(|| !ctx.project.mixer.insert(id).is_some_and(|i| get(i)));
    ctx.edit_insert(id, |i| set(i, on))?;
    Ok((id, on))
}

fn add_insert(ctx: &mut Ctx<'_>, p: AddInsert) -> Result<Value, CommandError> {
    let count = ctx.project.mixer.order.len();
    let base = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Insert {}", count + 1));
    let name = unique_name(base.trim(), ctx.project.mixer.inserts.values().map(|i| i.name.as_str()));
    let id = ctx.ids().insert();
    let mixer = Arc::make_mut(&mut ctx.project.mixer);
    mixer.inserts.insert(id, Arc::new(MixerInsert::new(id, name.clone())));
    mixer.order.push_back(id);
    ctx.editor.insert = Some(id);
    ctx.set_label(format!("Add insert {name}"));
    Ok(json!({ "insert": count + 1, "id": id, "name": name }))
}

fn remove_insert(ctx: &mut Ctx<'_>, p: OneInsert) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    if id == ctx.project.mixer.master.id {
        return Err(CommandError::rejected("the master can't be removed"));
    }
    let name = label(&ctx.project, id);
    let mixer = Arc::make_mut(&mut ctx.project.mixer);
    mixer.inserts.remove(&id);
    mixer.order.retain(|i| *i != id);
    remove_where(&mut mixer.routes, |k, _| k.to == id || k.from == RouteSource::Insert(id));
    let fed = |insert: &Arc<MixerInsert>| insert.slots.iter().any(|s| s.sidechain == Some(RouteSource::Insert(id)));
    let unhook = |insert: &mut Arc<MixerInsert>| {
        for slot in &mut Arc::make_mut(insert).slots {
            if slot.sidechain == Some(RouteSource::Insert(id)) {
                slot.sidechain = None;
            }
        }
    };
    if fed(&mixer.master) {
        unhook(&mut mixer.master);
    }
    update_where(&mut mixer.inserts, fed, unhook);
    let channels: Vec<_> = ctx.project.channels.values().filter(|c| c.target == Some(id)).map(|c| c.id).collect();
    for channel in &channels {
        ctx.edit_channel(*channel, |c| c.target = None)?;
    }
    ctx.set_label(format!("Remove insert {name}"));
    Ok(json!({ "removed": id, "channelsToMaster": channels.len() }))
}

fn rename(ctx: &mut Ctx<'_>, p: Rename) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let name = p.name.trim().to_owned();
    if name.is_empty() {
        return Err(CommandError::invalid("an insert name can't be empty"));
    }
    ctx.edit_insert(id, |i| i.name.clone_from(&name))?;
    ctx.set_label(format!("Rename insert to {name}"));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "name": name }))
}

fn set_color(ctx: &mut Ctx<'_>, p: SetColor) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let color = p.color.resolve()?;
    ctx.edit_insert(id, |i| i.color = color)?;
    Ok(json!({ "insert": insert_number(&ctx.project, id), "color": color }))
}

fn set_volume(ctx: &mut Ctx<'_>, p: SetVolume) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    if p.db.is_nan() || p.db > 6.0 {
        return Err(CommandError::invalid(format!("db must be at most 6, not {}", p.db)));
    }
    let db = p.db.max(SILENT_DB);
    ctx.edit_insert(id, |i| i.volume_db = db)?;
    ctx.set_label(format!("Set {} volume", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "db": db }))
}

fn set_pan(ctx: &mut Ctx<'_>, p: SetPan) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    in_range("pan", f64::from(p.pan), -1.0..=1.0)?;
    ctx.edit_insert(id, |i| i.pan = p.pan)?;
    ctx.set_label(format!("Set {} pan", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "pan": p.pan }))
}

fn set_stereo_sep(ctx: &mut Ctx<'_>, p: SetStereoSep) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    in_range("separation", f64::from(p.separation), -1.0..=1.0)?;
    ctx.edit_insert(id, |i| i.stereo_sep = p.separation)?;
    ctx.set_label(format!("Set {} stereo separation", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "separation": p.separation }))
}

fn mute(ctx: &mut Ctx<'_>, p: Switch) -> Result<Value, CommandError> {
    let (id, on) = switch(ctx, &p, |i| i.muted, |i, on| i.muted = on)?;
    ctx.set_label(format!("{} {}", if on { "Mute" } else { "Unmute" }, label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "muted": on }))
}

fn set_polarity(ctx: &mut Ctx<'_>, p: Switch) -> Result<Value, CommandError> {
    let (id, on) = switch(ctx, &p, |i| i.polarity_invert, |i, on| i.polarity_invert = on)?;
    ctx.set_label(format!("Set {} polarity", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "inverted": on }))
}

fn set_swap_lr(ctx: &mut Ctx<'_>, p: Switch) -> Result<Value, CommandError> {
    let (id, on) = switch(ctx, &p, |i| i.swap_lr, |i, on| i.swap_lr = on)?;
    ctx.set_label(format!("Swap {} left and right", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "swapped": on }))
}

/// The channel or insert a route starts from, checked: the master is the output, so no route starts there.
pub(super) fn route_from(ctx: &Ctx<'_>, from: &RouteFromRef) -> Result<RouteSource, CommandError> {
    let from = ctx.route_source(from)?;
    if from == RouteSource::Insert(ctx.project.mixer.master.id) {
        return Err(CommandError::rejected("the master is the output; no route starts from it"));
    }
    Ok(from)
}

/// The ends of a route, checked: it never targets the master or runs from an insert to itself.
fn route_key(ctx: &Ctx<'_>, from: &RouteFromRef, to: &InsertRef) -> Result<RouteKey, CommandError> {
    let from = route_from(ctx, from)?;
    let to = ctx.insert(Some(to))?;
    if to == ctx.project.mixer.master.id {
        return Err(CommandError::rejected("routes never target the master; every insert reaches it on its own"));
    }
    if from == RouteSource::Insert(to) {
        return Err(CommandError::rejected("an insert can't route to itself"));
    }
    Ok(RouteKey { from, to })
}

/// The inserts on the loop that a route from `from` to `to` would close, if it would, from the route's source around.
///
/// Complexity: O(V + E) in inserts and routes.
pub(super) fn cycle(project: &Project, key: RouteKey) -> Option<Vec<InsertId>> {
    if !matches!(key.from, RouteSource::Insert(_)) {
        return None;
    }
    let mut dag = IncrementalDag::new();
    let nodes: std::collections::HashMap<InsertId, _> = project.mixer.inserts.keys().map(|id| (*id, dag.add_node())).collect();
    let names: std::collections::HashMap<_, InsertId> = nodes.iter().map(|(id, node)| (*node, *id)).collect();
    let others = project.mixer.routes.keys().filter(|k| **k != key);
    for existing in others.chain(std::iter::once(&key)) {
        if let (RouteSource::Insert(a), Some(&b)) = (existing.from, nodes.get(&existing.to))
            && let Some(&a) = nodes.get(&a)
            && let Err(DagError::Cycle { path }) = dag.add_edge(a, b)
        {
            return Some(path.iter().filter_map(|n| names.get(n)).copied().collect());
        }
    }
    None
}

/// The error for a route that would close the loop `path`: it names the inserts and lists them as references.
pub(super) fn loop_error(project: &Project, path: &[InsertId]) -> CommandError {
    // `insert 2 (Drums)`, or just `insert 2` while it has its default name.
    let name = |id: InsertId| match (insert_number(project, id), project.mixer.insert(id)) {
        (Some(n), Some(i)) if i.name == format!("Insert {n}") => format!("insert {n}"),
        (Some(n), Some(i)) => format!("insert {n} ({})", i.name),
        _ => id.to_string(),
    };
    let reference = |id: InsertId| insert_number(project, id).map_or_else(|| id.to_string(), |n| format!("insert:{n}"));
    let names: Vec<String> = path.iter().chain(path.first()).map(|id| name(*id)).collect();
    CommandError::rejected(format!("that route would close a loop: {}", names.join(" -> ")))
        .with_cycle(path.iter().map(|id| reference(*id)).collect())
        .with_hint("remove one of the routes in the loop first")
}

fn add_route(ctx: &mut Ctx<'_>, p: AddRoute) -> Result<Value, CommandError> {
    let key = route_key(ctx, &p.from, &p.to)?;
    let level = p.level.unwrap_or(1.0);
    in_range("level", f64::from(level), 0.0..=2.0)?;
    if let Some(path) = cycle(&ctx.project, key) {
        return Err(loop_error(&ctx.project, &path));
    }
    let kind = if p.sidechain { RouteKind::Sidechain } else { RouteKind::Audio };
    Arc::make_mut(&mut ctx.project.mixer).routes.insert(key, Route { send_level: level, tap: SendTap::PostFader, kind });
    let to = insert_number(&ctx.project, key.to).unwrap_or(0);
    ctx.set_label(format!("Route to insert {to}"));
    Ok(json!({ "from": key.from, "to": to, "level": level, "sidechain": p.sidechain }))
}

fn remove_route(ctx: &mut Ctx<'_>, p: RouteEnds) -> Result<Value, CommandError> {
    let key = route_key(ctx, &p.from, &p.to)?;
    if !ctx.project.mixer.routes.contains_key(&key) {
        return Err(CommandError::not_found("route", "between those two").with_hint("inspect_mixer lists the routes"));
    }
    let mixer = Arc::make_mut(&mut ctx.project.mixer);
    mixer.routes.remove(&key);
    // A slot can't keep a sidechain whose route is gone.
    if let Some(insert) = mixer.inserts.get_mut(&key.to)
        && insert.slots.iter().any(|s| s.sidechain == Some(key.from))
    {
        for slot in &mut Arc::make_mut(insert).slots {
            if slot.sidechain == Some(key.from) {
                slot.sidechain = None;
            }
        }
    }
    ctx.set_label("Remove route");
    Ok(json!({ "removed": true }))
}

fn set_route(ctx: &mut Ctx<'_>, p: SetRoute) -> Result<Value, CommandError> {
    let key = route_key(ctx, &p.from, &p.to)?;
    if let Some(level) = p.level {
        in_range("level", f64::from(level), 0.0..=2.0)?;
    }
    let mut route = *ctx
        .project
        .mixer
        .routes
        .get(&key)
        .ok_or_else(|| CommandError::not_found("route", "between those two").with_hint("add it with mixer.add_route"))?;
    if let Some(level) = p.level {
        route.send_level = level;
    }
    if let Some(tap) = p.tap {
        route.tap = tap.into();
    }
    if let Some(sidechain) = p.sidechain {
        route.kind = if sidechain { RouteKind::Sidechain } else { RouteKind::Audio };
    }
    if ctx.project.mixer.routes.get(&key) != Some(&route) {
        let mixer = Arc::make_mut(&mut ctx.project.mixer);
        mixer.routes.insert(key, route);
        // A route that carries audio again no longer feeds the target's sidechains.
        if route.kind == RouteKind::Audio {
            unhook_sidechain(mixer, key);
        }
    }
    ctx.set_label("Set route");
    Ok(json!({ "level": route.send_level, "tap": route.tap, "sidechain": route.kind == RouteKind::Sidechain }))
}

/// Clears the sidechain of every slot of `key.to` that listens to `key.from`.
fn unhook_sidechain(mixer: &mut bodo_model::Mixer, key: RouteKey) {
    let strip = if mixer.master.id == key.to { Some(&mut mixer.master) } else { mixer.inserts.get_mut(&key.to) };
    if let Some(insert) = strip
        && insert.slots.iter().any(|s| s.sidechain == Some(key.from))
    {
        for slot in &mut Arc::make_mut(insert).slots {
            if slot.sidechain == Some(key.from) {
                slot.sidechain = None;
            }
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetIcon {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The icon's name, such as `drum`, `guitar` or `microphone`; null for none.
    icon: Option<String>,
}

fn set_icon(ctx: &mut Ctx<'_>, p: SetIcon) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let icon = p.icon.map(|i| i.trim().to_lowercase()).filter(|i| !i.is_empty());
    if icon.as_ref().is_some_and(|i| i.len() > 40 || !i.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        return Err(CommandError::invalid("an icon name is a word of letters, digits and dashes"));
    }
    ctx.edit_insert(id, |i| i.icon.clone_from(&icon))?;
    ctx.set_label(format!("Set {} icon", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "icon": icon }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Solo {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// On or off; toggles when left out.
    on: Option<bool>,
    /// Solo this insert alone, unsoloing every other.
    #[serde(default)]
    exclusive: bool,
}

fn solo(ctx: &mut Ctx<'_>, p: Solo) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let on = p.on.unwrap_or_else(|| !ctx.project.mixer.insert(id).is_some_and(|i| i.solo));
    if p.exclusive {
        let mixer = Arc::make_mut(&mut ctx.project.mixer);
        update_where(&mut mixer.inserts, |i| i.solo && i.id != id, |i| Arc::make_mut(i).solo = false);
        if mixer.master.solo && mixer.master.id != id {
            Arc::make_mut(&mut mixer.master).solo = false;
        }
    }
    ctx.edit_insert(id, |i| i.solo = on)?;
    ctx.set_label(format!("{} {}", if on { "Solo" } else { "Unsolo" }, label(&ctx.project, id)));
    let soloed: Vec<usize> = ctx
        .project
        .mixer
        .order
        .iter()
        .filter(|i| ctx.project.mixer.inserts.get(*i).is_some_and(|s| s.solo))
        .filter_map(|i| insert_number(&ctx.project, *i))
        .collect();
    Ok(json!({ "insert": insert_number(&ctx.project, id), "solo": on, "soloed": soloed }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetLatency {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The track latency in frames: positive declares latency the insert has (everything else waits for it),
    /// negative delays the insert.
    frames: Option<i64>,
    /// The same in milliseconds, at the project's sample rate.
    ms: Option<f64>,
}

fn set_latency(ctx: &mut Ctx<'_>, p: SetLatency) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let rate = f64::from(ctx.project.settings.sample_rate.hz());
    let frames = match (p.frames, p.ms) {
        (Some(frames), None) => frames,
        (None, Some(ms)) => {
            in_range("ms", ms, -1000.0..=1000.0)?;
            (ms * rate / 1000.0).round() as i64
        }
        _ => return Err(CommandError::invalid("give frames or ms")),
    };
    let limit = rate as i64;
    if frames.abs() > limit {
        return Err(CommandError::invalid(format!("track latency must be within a second ({limit} frames) either way")));
    }
    ctx.edit_insert(id, |i| i.latency_offset = bodo_kit_core::Frames(frames))?;
    ctx.set_label(format!("Set {} track latency", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "frames": frames, "ms": frames as f64 * 1000.0 / rate }))
}

/// When delay compensation is bypassed.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Bypass {
    /// Never: the insert is always lined up with the rest.
    Never,
    /// Always: the insert is never delayed to line up.
    Always,
    /// While recording, for monitoring through it without delay.
    WhileRecording,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPdcBypass {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// When to bypass delay compensation.
    mode: Bypass,
}

fn set_pdc_bypass(ctx: &mut Ctx<'_>, p: SetPdcBypass) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let mode = match p.mode {
        Bypass::Never => PdcBypass::Never,
        Bypass::Always => PdcBypass::Always,
        Bypass::WhileRecording => PdcBypass::WhileRecording,
    };
    ctx.edit_insert(id, |i| i.pdc_bypass = mode)?;
    ctx.set_label(format!("Set {} delay compensation bypass", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "mode": mode }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetOutput {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// Device output channels, counted from 0: one for mono, two for stereo; null for no direct output.
    channels: Option<Vec<u16>>,
}

fn set_output(ctx: &mut Ctx<'_>, p: SetOutput) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let port = match p.channels {
        None => None,
        Some(channels) if (1..=2).contains(&channels.len()) && channels.iter().all(|c| *c < 256) => Some(HwPort { channels }),
        Some(_) => return Err(CommandError::invalid("give one output channel for mono or two for stereo, each below 256")),
    };
    ctx.edit_insert(id, |i| i.output.clone_from(&port))?;
    ctx.set_label(format!("Set {} output", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "output": port.map(|p| p.channels) }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToMaster {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// On or off; toggles when left out.
    on: Option<bool>,
}

fn set_to_master(ctx: &mut Ctx<'_>, p: ToMaster) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    if id == ctx.project.mixer.master.id {
        return Err(CommandError::rejected("the master is the output; it doesn't route to itself"));
    }
    let on = p.on.unwrap_or_else(|| !ctx.project.mixer.insert(id).is_some_and(|i| i.to_master));
    ctx.edit_insert(id, |i| i.to_master = on)?;
    ctx.set_label(format!("{} {} to the master", if on { "Route" } else { "Unroute" }, label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "toMaster": on }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteExclusively {
    /// Where it starts: a channel or an insert.
    from: RouteFromRef,
    /// The insert it goes to, or the master.
    to: InsertRef,
}

fn route_exclusively(ctx: &mut Ctx<'_>, p: RouteExclusively) -> Result<Value, CommandError> {
    let from = route_from(ctx, &p.from)?;
    let to = ctx.insert(Some(&p.to))?;
    let master = ctx.project.mixer.master.id;
    match from {
        RouteSource::Channel(channel) => {
            let mixer = Arc::make_mut(&mut ctx.project.mixer);
            remove_where(&mut mixer.routes, |k, r| k.from == from && r.kind == RouteKind::Audio);
            ctx.edit_channel(channel, |c| c.target = (to != master).then_some(to))?;
        }
        RouteSource::Insert(source) => {
            if source == to {
                return Err(CommandError::rejected("an insert can't route to itself"));
            }
            if to != master {
                let key = RouteKey { from, to };
                let mut trial = ctx.project.clone();
                remove_where(&mut Arc::make_mut(&mut trial.mixer).routes, |k, r| k.from == from && r.kind == RouteKind::Audio);
                if let Some(path) = cycle(&trial, key) {
                    return Err(loop_error(&ctx.project, &path));
                }
            }
            let mixer = Arc::make_mut(&mut ctx.project.mixer);
            remove_where(&mut mixer.routes, |k, r| k.from == from && r.kind == RouteKind::Audio);
            if to != master {
                mixer.routes.insert(RouteKey { from, to }, Route { send_level: 1.0, tap: SendTap::PostFader, kind: RouteKind::Audio });
            }
            ctx.edit_insert(source, |i| i.to_master = to == master)?;
        }
    }
    ctx.set_label(format!("Route to {} only", label(&ctx.project, to)));
    Ok(json!({ "from": from, "to": insert_number(&ctx.project, to) }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveInsert {
    /// The insert to move (not the master).
    insert: Option<InsertRef>,
    /// Its new number, from 1.
    to: usize,
}

fn move_insert(ctx: &mut Ctx<'_>, p: MoveInsert) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    if id == ctx.project.mixer.master.id {
        return Err(CommandError::rejected("the master stays first"));
    }
    let count = ctx.project.mixer.order.len();
    if p.to == 0 || p.to > count {
        return Err(CommandError::invalid(format!("to must be from 1 to {count}")));
    }
    let mixer = Arc::make_mut(&mut ctx.project.mixer);
    mixer.order.retain(|i| *i != id);
    mixer.order.insert(p.to - 1, id);
    ctx.set_label(format!("Move {}", label(&ctx.project, id)));
    Ok(json!({ "insert": p.to }))
}

/// A band of the strip EQ.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Band {
    /// The low shelf.
    Low,
    /// The peaking mid band.
    Mid,
    /// The high shelf.
    High,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetEq {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// Which band.
    band: Band,
    /// The band's frequency in Hz, from 20 to 20000.
    frequency: Option<f32>,
    /// Its gain in dB, from -24 to 24.
    gain_db: Option<f32>,
    /// Its Q (for shelves, the slope), from 0.1 to 10.
    q: Option<f32>,
}

fn set_eq(ctx: &mut Ctx<'_>, p: SetEq) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    if let Some(f) = p.frequency {
        in_range("frequency", f64::from(f), 20.0..=20_000.0)?;
    }
    if let Some(g) = p.gain_db {
        in_range("gainDb", f64::from(g), -24.0..=24.0)?;
    }
    if let Some(q) = p.q {
        in_range("q", f64::from(q), 0.1..=10.0)?;
    }
    let mut band = None;
    ctx.edit_insert(id, |i| {
        let target = match p.band {
            Band::Low => &mut i.eq.low,
            Band::Mid => &mut i.eq.mid,
            Band::High => &mut i.eq.high,
        };
        if let Some(f) = p.frequency {
            target.frequency = f;
        }
        if let Some(g) = p.gain_db {
            target.gain_db = g;
        }
        if let Some(q) = p.q {
            target.q = q;
        }
        band = Some(*target);
    })?;
    ctx.set_label(format!("Set {} EQ", label(&ctx.project, id)));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "band": band.map(|b| json!({ "frequency": b.frequency, "gainDb": b.gain_db, "q": b.q })) }))
}

/// Merge key: the insert and the band.
fn by_band(params: &Value) -> String {
    format!("{}{}", super::field(params, "insert"), super::field(params, "band"))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Adjust {
    /// The inserts; the selected inserts by default.
    #[serde(default)]
    inserts: Vec<InsertRef>,
    /// dB to add to each fader, clamped to -144..6.
    volume_db: Option<f32>,
    /// Amount to add to each pan, clamped to -1..1.
    pan: Option<f32>,
    /// Amount to add to each stereo separation, clamped to -1..1.
    separation: Option<f32>,
}

fn adjust(ctx: &mut Ctx<'_>, p: Adjust) -> Result<Value, CommandError> {
    let ids = ctx.inserts(&p.inserts)?;
    for (name, value) in [("volumeDb", p.volume_db), ("pan", p.pan), ("separation", p.separation)] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(CommandError::invalid(format!("{name} must be a number")));
        }
    }
    for id in &ids {
        ctx.edit_insert(*id, |i| {
            if let Some(db) = p.volume_db {
                i.volume_db = (i.volume_db + db).clamp(SILENT_DB, 6.0);
            }
            if let Some(pan) = p.pan {
                i.pan = (i.pan + pan).clamp(-1.0, 1.0);
            }
            if let Some(sep) = p.separation {
                i.stereo_sep = (i.stereo_sep + sep).clamp(-1.0, 1.0);
            }
        })?;
    }
    ctx.set_label(format!("Adjust {} inserts", ids.len()));
    let numbers: Vec<Option<usize>> = ids.iter().map(|id| insert_number(&ctx.project, *id)).collect();
    Ok(json!({ "inserts": numbers }))
}

/// Merge key: the inserts adjusted.
fn by_inserts(params: &Value) -> String {
    super::field(params, "inserts")
}

/// The pan law, a project setting.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Law {
    /// Each side -3 dB at center: constant loudness. The default.
    ConstantPower3dB,
    /// Each side -4.5 dB at center.
    Compromise4_5dB,
    /// Each side -6 dB at center: the gains add up to one.
    Linear6dB,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPanLaw {
    /// How mono sources spread as they pan.
    law: Law,
}

fn set_pan_law(ctx: &mut Ctx<'_>, p: SetPanLaw) -> Result<Value, CommandError> {
    let law = match p.law {
        Law::ConstantPower3dB => PanLaw::ConstantPower3dB,
        Law::Compromise4_5dB => PanLaw::Compromise4_5dB,
        Law::Linear6dB => PanLaw::Linear6dB,
    };
    Arc::make_mut(&mut ctx.project.settings).pan_law = law;
    ctx.set_label("Set pan law");
    Ok(json!({ "law": law }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("mixer.add_insert", "Add insert", "Adds a mixer insert after the last one.", Effects::UNDOABLE, add_insert)
        .example(json!({ "name": "Drum bus" }))
        .menu("Add/Mixer insert");
    r.edit(
        "mixer.remove_insert",
        "Remove insert",
        "Removes an insert; channels that played into it go to the master.",
        Effects::UNDOABLE,
        remove_insert,
    )
    .example(json!({ "insert": 16 }));
    r.edit("mixer.rename", "Rename insert", "Renames a mixer insert.", Effects::UNDOABLE, rename).example(json!({ "insert": 1, "name": "Kick" }));
    r.edit("mixer.set_color", "Set insert color", "Sets an insert's color.", Effects::UNDOABLE, set_color)
        .example(json!({ "insert": 1, "color": "#f87171" }));
    r.edit("mixer.set_volume", "Set insert volume", "Sets an insert's fader in dB.", Effects::UNDOABLE, set_volume)
        .example(json!({ "insert": 1, "db": -3.0 }))
        .merge(by_insert);
    r.edit("mixer.set_pan", "Set insert pan", "Sets an insert's pan from -1 to 1.", Effects::UNDOABLE, set_pan)
        .example(json!({ "insert": 2, "pan": -0.2 }))
        .merge(by_insert);
    r.edit(
        "mixer.set_stereo_sep",
        "Set stereo separation",
        "Narrows or widens an insert: -1 wider, 0 unchanged, 1 mono.",
        Effects::UNDOABLE,
        set_stereo_sep,
    )
    .example(json!({ "insert": 3, "separation": 0.5 }))
    .merge(by_insert);
    r.edit("mixer.mute", "Mute insert", "Mutes or unmutes an insert; toggles without `on`.", Effects::UNDOABLE, mute).example(json!({ "insert": 4 }));
    r.edit("mixer.set_polarity", "Reverse polarity", "Inverts an insert's polarity; toggles without `on`.", Effects::UNDOABLE, set_polarity)
        .example(json!({ "insert": 1, "on": true }));
    r.edit("mixer.set_swap_lr", "Swap left and right", "Swaps an insert's channels; toggles without `on`.", Effects::UNDOABLE, set_swap_lr)
        .example(json!({ "insert": 1, "on": true }));
    r.edit("mixer.add_route", "Add route", "Routes a channel or insert into another insert; refuses loops.", Effects::UNDOABLE, add_route)
        .example(json!({ "from": "insert:1", "to": "insert:5", "level": 0.7 }));
    r.edit("mixer.remove_route", "Remove route", "Removes a route.", Effects::UNDOABLE, remove_route)
        .example(json!({ "from": "insert:1", "to": "insert:5" }));
    r.edit(
        "mixer.set_route",
        "Set route",
        "Sets a route's send level, tap point, or whether it only feeds sidechains.",
        Effects::UNDOABLE,
        set_route,
    )
    .example(json!({ "from": "insert:1", "to": "insert:5", "level": 0.5 }))
    .merge(by_route);
    r.edit(
        "mixer.route_exclusive",
        "Route exclusively",
        "Routes a channel or insert to one insert, or the master, only, removing its other audio routes.",
        Effects::UNDOABLE,
        route_exclusively,
    )
    .example(json!({ "from": "insert:1", "to": "insert:5" }));
    r.edit(
        "mixer.set_to_master",
        "Route to master",
        "Sets whether an insert feeds the master; toggles without `on`.",
        Effects::UNDOABLE,
        set_to_master,
    )
    .example(json!({ "insert": 2, "on": false }));
    r.edit("mixer.solo", "Solo insert", "Solos or unsolos an insert, alone with `exclusive`; toggles without `on`.", Effects::UNDOABLE, solo)
        .example(json!({ "insert": 1, "exclusive": true }));
    r.edit("mixer.set_icon", "Set insert icon", "Sets the icon an insert shows, by name, or none.", Effects::UNDOABLE, set_icon)
        .example(json!({ "insert": 1, "icon": "drum" }));
    r.edit(
        "mixer.set_latency_offset",
        "Set track latency",
        "Sets an insert's track latency: positive declares latency, negative delays the insert.",
        Effects::UNDOABLE,
        set_latency,
    )
    .example(json!({ "insert": 1, "ms": -2.5 }))
    .merge(by_insert);
    r.edit(
        "mixer.set_pdc_bypass",
        "Set delay compensation bypass",
        "Sets when an insert skips delay compensation: never, always or whileRecording.",
        Effects::UNDOABLE,
        set_pdc_bypass,
    )
    .example(json!({ "insert": 1, "mode": "whileRecording" }));
    r.edit("mixer.set_output", "Set direct output", "Sends an insert straight to device outputs as well, or stops.", Effects::UNDOABLE, set_output)
        .example(json!({ "insert": 3, "channels": [2, 3] }));
    r.edit("mixer.reorder_insert", "Move insert", "Moves an insert to another place in the mixer.", Effects::UNDOABLE, move_insert)
        .example(json!({ "insert": 3, "to": 1 }));
    r.edit("mixer.set_eq", "Set strip EQ", "Sets the frequency, gain or Q of a band of an insert's EQ.", Effects::UNDOABLE, set_eq)
        .example(json!({ "insert": 1, "band": "mid", "frequency": 2500, "gainDb": -3.0, "q": 1.4 }))
        .merge(by_band);
    r.edit(
        "mixer.adjust",
        "Adjust inserts",
        "Moves the faders, pans or stereo separation of several inserts together, by the same amount.",
        Effects::UNDOABLE,
        adjust,
    )
    .example(json!({ "inserts": [1, 2], "volumeDb": -1.5 }))
    .merge(by_inserts);
    r.edit("project.set_pan_law", "Set pan law", "Sets how mono sources spread as they pan, for the whole project.", Effects::UNDOABLE, set_pan_law)
        .example(json!({ "law": "linear6dB" }));
}
