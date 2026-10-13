//! `mixer.*`: inserts and routes (project.md section 9).
//!
//! Volumes are in dB from -144 (silence) to +6. Routes run from a channel or an insert to another insert; a route
//! that would close a loop between inserts is refused with the loop's path.

use std::sync::Arc;

use bodo_kit_graph::{DagError, IncrementalDag};
use bodo_model::mixer::{Route, RouteKey, RouteKind, RouteSource, SendTap};
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

/// The ends of a route, checked: it never targets the master or runs from an insert to itself.
fn route_key(ctx: &Ctx<'_>, from: &RouteFromRef, to: &InsertRef) -> Result<RouteKey, CommandError> {
    let from = ctx.route_source(from)?;
    let to = ctx.insert(Some(to))?;
    if to == ctx.project.mixer.master.id {
        return Err(CommandError::rejected("routes never target the master; every insert reaches it on its own"));
    }
    if from == RouteSource::Insert(to) {
        return Err(CommandError::rejected("an insert can't route to itself"));
    }
    Ok(RouteKey { from, to })
}

/// The inserts on the loop that a route from `from` to `to` would close, if it would.
///
/// Complexity: O(V + E) in inserts and routes.
fn cycle(project: &Project, key: RouteKey) -> Option<Vec<String>> {
    if !matches!(key.from, RouteSource::Insert(_)) {
        return None;
    }
    let mut dag = IncrementalDag::new();
    let nodes: std::collections::HashMap<InsertId, _> = project.mixer.inserts.keys().map(|id| (*id, dag.add_node())).collect();
    let names: std::collections::HashMap<_, InsertId> = nodes.iter().map(|(id, node)| (*node, *id)).collect();
    for existing in project.mixer.routes.keys().chain(std::iter::once(&key)) {
        if let (RouteSource::Insert(a), Some(&b)) = (existing.from, nodes.get(&existing.to))
            && let Some(&a) = nodes.get(&a)
            && let Err(DagError::Cycle { path }) = dag.add_edge(a, b)
        {
            let number = |id: InsertId| insert_number(project, id).map_or_else(|| id.to_string(), |n| format!("insert {n}"));
            return Some(path.iter().filter_map(|n| names.get(n)).map(|id| number(*id)).collect());
        }
    }
    None
}

fn add_route(ctx: &mut Ctx<'_>, p: AddRoute) -> Result<Value, CommandError> {
    let key = route_key(ctx, &p.from, &p.to)?;
    let level = p.level.unwrap_or(1.0);
    in_range("level", f64::from(level), 0.0..=2.0)?;
    if let Some(path) = cycle(&ctx.project, key) {
        return Err(CommandError::rejected(format!("that route would close a loop: {}", path.join(" -> "))));
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
    if ctx.project.mixer.routes.get(&key) != Some(&route) {
        Arc::make_mut(&mut ctx.project.mixer).routes.insert(key, route);
    }
    ctx.set_label("Set route");
    Ok(json!({ "level": route.send_level, "tap": route.tap }))
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
    r.edit("mixer.set_route", "Set route", "Sets a route's send level or tap point.", Effects::UNDOABLE, set_route)
        .example(json!({ "from": "insert:1", "to": "insert:5", "level": 0.5 }))
        .merge(by_route);
}
