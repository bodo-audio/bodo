//! `fx.*` and `plugin.*` for FX slots: loading native effects, ordering slots, their on switch, dry/wet mix,
//! sidechain, copying whole chains, and effect parameters (project.md sections 9.1 to 9.3 and 12.2).
//!
//! Slots are numbered 1 to 10 in processing order. An effect is named by its uid (`bodo.delay`) or its name
//! (`Delay`); `plugin.list` lists them and `plugin.describe` gives their parameters. A parameter is named by its id
//! or its name and set in its own unit; values are clamped to its range.

use std::sync::Arc;

use bodo_mix::fx::{EffectInfo, ParamSpec, catalog, find};
use bodo_model::InsertId;
use bodo_model::channel::{PluginFormat, PluginRef, SandboxMode};
use bodo_model::mixer::{FX_SLOTS, FxSlot, Route, RouteKey, RouteKind, RouteSource, SendTap};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{in_range, insert_number};
use crate::error::CommandError;
use crate::params::{InsertRef, RouteFromRef};
use crate::registry::{Effects, NoParams, Registry};
use crate::session::Ctx;

/// The slot `number` (1 to 10) as an index.
pub(super) fn slot_index(number: usize) -> Result<usize, CommandError> {
    if (1..=FX_SLOTS).contains(&number) {
        Ok(number - 1)
    } else {
        Err(CommandError::invalid(format!("slot must be from 1 to {FX_SLOTS}, not {number}")))
    }
}

/// The native effect `text` names: its uid or its name, ignoring case.
fn effect(text: &str) -> Result<&'static EffectInfo, CommandError> {
    let text = text.trim();
    find(text).or_else(|| catalog().iter().find(|i| i.name.eq_ignore_ascii_case(text))).ok_or_else(|| {
        CommandError::not_found("effect", text).with_candidates(catalog().iter().filter(|i| !i.hidden).map(|i| i.uid.to_owned()).collect())
    })
}

/// The native effect in `slot`, if any.
pub(super) fn effect_in(slot: &FxSlot) -> Option<&'static EffectInfo> {
    slot.plugin.as_ref().filter(|p| p.format == PluginFormat::Native).and_then(|p| find(&p.uid))
}

/// The parameter of `info` that `name` names: its id, or its name ignoring case.
pub(super) fn param<'a>(info: &'a EffectInfo, name: &Value) -> Result<&'a ParamSpec, CommandError> {
    let found = match name {
        Value::Number(n) => n.as_u64().and_then(|id| info.params.iter().find(|p| u64::from(p.id) == id)),
        Value::String(s) => {
            s.trim().parse::<u32>().ok().and_then(|id| info.params.iter().find(|p| p.id == id)).or_else(|| info.param_named(s.trim()))
        }
        _ => None,
    };
    let text = name.as_str().map_or_else(|| name.to_string(), str::to_owned);
    found.ok_or_else(|| {
        CommandError::not_found(&format!("parameter of {}", info.name), &text)
            .with_candidates(info.params.iter().map(|p| format!("{} ({})", p.name, p.id)).collect())
    })
}

/// The slot's effect, its parameters and their values, for results and inspection.
pub(crate) fn describe_slot(slot: &FxSlot, number: usize) -> Value {
    let Some(plugin) = slot.plugin.as_ref() else { return json!({ "slot": number, "effect": null }) };
    let Some(info) = effect_in(slot) else {
        return json!({ "slot": number, "effect": plugin.uid, "format": plugin.format, "enabled": slot.enabled, "mix": slot.mix });
    };
    let params: Vec<Value> = info
        .params
        .iter()
        .map(|p| {
            let value = slot.param(p.id).map_or(p.default, |v| p.clamp(v));
            json!({ "id": p.id, "name": p.name, "value": value, "text": p.display(value) })
        })
        .collect();
    json!({
        "slot": number,
        "effect": info.uid,
        "name": info.name,
        "instance": plugin.instance,
        "enabled": slot.enabled,
        "mix": slot.mix,
        "sidechain": slot.sidechain,
        "params": params,
    })
}

/// A native effect with its parameters, for `plugin.list` and `plugin.describe`.
pub(crate) fn describe_effect(info: &EffectInfo) -> Value {
    json!({
        "uid": info.uid,
        "name": info.name,
        "format": PluginFormat::Native,
        "description": info.description,
        "category": info.category,
        "sidechain": info.sidechain,
        "params": info.params,
    })
}

/// The slot `index` of insert `id`, or an empty slot.
fn slot_of(ctx: &Ctx<'_>, id: InsertId, index: usize) -> FxSlot {
    ctx.project.mixer.insert(id).and_then(|i| i.slots.get(index).cloned()).unwrap_or_default()
}

/// A fresh native plugin reference to `info`, with a new instance id.
fn instance_of(ctx: &mut Ctx<'_>, info: &EffectInfo) -> PluginRef {
    PluginRef {
        instance: ctx.ids().plugin_instance(),
        format: PluginFormat::Native,
        uid: info.uid.to_owned(),
        state: Arc::from(Vec::new()),
        sandbox: SandboxMode::InProcess,
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Load {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot, 1 to 10; the first empty slot by default.
    slot: Option<usize>,
    /// The effect by uid or name, such as `bodo.delay` or `Delay`.
    effect: String,
    /// Parameter values, by id or name, in each parameter's unit; the others start at their defaults.
    #[serde(default)]
    params: serde_json::Map<String, Value>,
}

fn load(ctx: &mut Ctx<'_>, p: Load) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let number = match p.slot {
        Some(number) => number,
        None => ctx
            .project
            .mixer
            .insert(id)
            .and_then(|i| i.slots.iter().position(|s| s.plugin.is_none()))
            .map(|i| i + 1)
            .ok_or_else(|| CommandError::rejected("every slot of that insert is full").with_hint("name a slot to replace its effect"))?,
    };
    let index = slot_index(number)?;
    let info = effect(&p.effect)?;
    let mut values = Vec::new();
    for (key, value) in &p.params {
        let spec = param(info, &Value::String(key.clone()))?;
        let value = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| CommandError::invalid(format!("{} must be a number", spec.name)))? as f32;
        values.push((spec.id, spec.clamp(value)));
    }
    let mut fresh = FxSlot { plugin: Some(instance_of(ctx, info)), ..FxSlot::default() };
    for (param, value) in values {
        fresh.set_param(param, value);
    }
    // A sidechain stays with the slot when the new effect listens to one too.
    fresh.sidechain = slot_of(ctx, id, index).sidechain.filter(|_| info.sidechain);
    let result = describe_slot(&fresh, number);
    ctx.edit_insert(id, |i| i.slots[index] = fresh)?;
    ctx.set_label(format!("Load {} in slot {number}", info.name));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": result }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OneSlot {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot, 1 to 10.
    slot: usize,
}

fn remove(ctx: &mut Ctx<'_>, p: OneSlot) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let index = slot_index(p.slot)?;
    ctx.edit_insert(id, |i| i.slots[index] = FxSlot::default())?;
    ctx.set_label(format!("Remove the effect in slot {}", p.slot));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": p.slot, "effect": null }))
}

fn get_params(ctx: &mut Ctx<'_>, p: OneSlot) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let index = slot_index(p.slot)?;
    Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": describe_slot(&slot_of(ctx, id, index), p.slot) }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveSlot {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot to move, 1 to 10.
    from: usize,
    /// Where it goes, 1 to 10; the slots between shift over.
    to: usize,
}

fn move_slot(ctx: &mut Ctx<'_>, p: MoveSlot) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let (from, to) = (slot_index(p.from)?, slot_index(p.to)?);
    ctx.edit_insert(id, |i| {
        if from < to {
            i.slots[from..=to].rotate_left(1);
        } else {
            i.slots[to..=from].rotate_right(1);
        }
    })?;
    ctx.set_label(format!("Move slot {} to {}", p.from, p.to));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "from": p.from, "to": p.to }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SwapSlots {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// One slot, 1 to 10.
    a: usize,
    /// The other slot, 1 to 10.
    b: usize,
}

fn swap(ctx: &mut Ctx<'_>, p: SwapSlots) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let (a, b) = (slot_index(p.a)?, slot_index(p.b)?);
    ctx.edit_insert(id, |i| i.slots.swap(a, b))?;
    ctx.set_label(format!("Swap slots {} and {}", p.a, p.b));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "a": p.a, "b": p.b }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SlotSwitch {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot, 1 to 10.
    slot: usize,
    /// On or off; toggles when left out.
    on: Option<bool>,
}

fn set_enabled(ctx: &mut Ctx<'_>, p: SlotSwitch) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let index = slot_index(p.slot)?;
    let on = p.on.unwrap_or_else(|| !slot_of(ctx, id, index).enabled);
    ctx.edit_insert(id, |i| i.slots[index].enabled = on)?;
    ctx.set_label(format!("{} slot {}", if on { "Enable" } else { "Disable" }, p.slot));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": p.slot, "enabled": on }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SlotMix {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot, 1 to 10.
    slot: usize,
    /// Dry/wet mix, 0 (dry) to 1 (wet).
    mix: f32,
}

fn set_mix(ctx: &mut Ctx<'_>, p: SlotMix) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let index = slot_index(p.slot)?;
    in_range("mix", f64::from(p.mix), 0.0..=1.0)?;
    ctx.edit_insert(id, |i| i.slots[index].mix = p.mix)?;
    ctx.set_label(format!("Set slot {} mix", p.slot));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": p.slot, "mix": p.mix }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SlotSidechain {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot, 1 to 10.
    slot: usize,
    /// The channel or insert to listen to; null to stop. A sidechain route is added when there is none.
    from: Option<RouteFromRef>,
}

fn set_sidechain(ctx: &mut Ctx<'_>, p: SlotSidechain) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let index = slot_index(p.slot)?;
    let Some(from) = p.from else {
        ctx.edit_insert(id, |i| i.slots[index].sidechain = None)?;
        ctx.set_label(format!("Clear slot {} sidechain", p.slot));
        return Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": p.slot, "sidechain": null }));
    };
    let source = super::mixer::route_from(ctx, &from)?;
    if source == RouteSource::Insert(id) {
        return Err(CommandError::rejected("an insert can't sidechain itself"));
    }
    match effect_in(&slot_of(ctx, id, index)) {
        Some(info) if info.sidechain => {}
        Some(info) => return Err(CommandError::rejected(format!("{} has no sidechain input", info.name))),
        None => return Err(CommandError::rejected(format!("slot {} has no effect with a sidechain input", p.slot))),
    }
    let key = RouteKey { from: source, to: id };
    match ctx.project.mixer.routes.get(&key) {
        Some(route) if route.kind == RouteKind::Audio => {
            return Err(CommandError::rejected("that source already routes audio into this insert")
                .with_hint("turn that route into a sidechain with mixer.set_route sidechain: true"));
        }
        Some(_) => {}
        None => {
            if let Some(path) = super::mixer::cycle(&ctx.project, key) {
                return Err(super::mixer::loop_error(&ctx.project, &path));
            }
            Arc::make_mut(&mut ctx.project.mixer).routes.insert(key, Route { send_level: 1.0, tap: SendTap::PreFader, kind: RouteKind::Sidechain });
        }
    }
    ctx.edit_insert(id, |i| i.slots[index].sidechain = Some(source))?;
    ctx.set_label(format!("Sidechain slot {}", p.slot));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "slot": p.slot, "sidechain": source }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CopyChain {
    /// The insert whose effects to copy; the selected insert by default.
    insert: Option<InsertRef>,
}

fn copy_chain(ctx: &mut Ctx<'_>, p: CopyChain) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let slots: Vec<FxSlot> = ctx.project.mixer.insert(id).map(|i| i.slots.to_vec()).unwrap_or_default();
    let effects = slots.iter().filter(|s| s.plugin.is_some()).count();
    ctx.editor.chain = Some(Arc::from(slots));
    Ok(json!({ "insert": insert_number(&ctx.project, id), "effects": effects }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PasteChain {
    /// The inserts to paste into; the selected inserts by default.
    #[serde(default)]
    inserts: Vec<InsertRef>,
}

fn paste_chain(ctx: &mut Ctx<'_>, p: PasteChain) -> Result<Value, CommandError> {
    let chain =
        ctx.editor.chain.clone().ok_or_else(|| CommandError::rejected("no effects were copied").with_hint("copy them with fx.copy_chain first"))?;
    let targets = ctx.inserts(&p.inserts)?;
    for id in &targets {
        let mut slots = Vec::with_capacity(chain.len());
        for slot in chain.iter() {
            let mut slot = slot.clone();
            if let Some(plugin) = slot.plugin.as_mut() {
                // Each pasted effect is an instance of its own.
                plugin.instance = ctx.ids().plugin_instance();
            }
            // A sidechain only comes along when this insert has the route for it.
            let routed =
                |from: RouteSource| ctx.project.mixer.routes.get(&RouteKey { from, to: *id }).is_some_and(|r| r.kind == RouteKind::Sidechain);
            slot.sidechain = slot.sidechain.filter(|from| routed(*from));
            slots.push(slot);
        }
        ctx.edit_insert(*id, |i| {
            for (target, slot) in i.slots.iter_mut().zip(slots) {
                *target = slot;
            }
        })?;
    }
    ctx.set_label(if targets.len() == 1 { "Paste effects".to_owned() } else { format!("Paste effects into {} inserts", targets.len()) });
    let numbers: Vec<Option<usize>> = targets.iter().map(|id| insert_number(&ctx.project, *id)).collect();
    Ok(json!({ "inserts": numbers, "effects": chain.iter().filter(|s| s.plugin.is_some()).count() }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetParam {
    /// The insert; the selected insert by default.
    insert: Option<InsertRef>,
    /// The slot, 1 to 10.
    slot: usize,
    /// The parameter, by id or name.
    param: Value,
    /// Its value, in its own unit; clamped to its range.
    value: f32,
}

fn set_param(ctx: &mut Ctx<'_>, p: SetParam) -> Result<Value, CommandError> {
    let id = ctx.insert(p.insert.as_ref())?;
    let index = slot_index(p.slot)?;
    let info = effect_in(&slot_of(ctx, id, index)).ok_or_else(|| CommandError::rejected(format!("slot {} holds no native effect", p.slot)))?;
    let spec = param(info, &p.param)?;
    if !p.value.is_finite() {
        return Err(CommandError::invalid("value must be a number"));
    }
    let value = spec.clamp(p.value);
    ctx.edit_insert(id, |i| i.slots[index].set_param(spec.id, value))?;
    ctx.set_label(format!("Set {} {}", info.name, spec.name));
    Ok(json!({
        "insert": insert_number(&ctx.project, id),
        "slot": p.slot,
        "param": spec.id,
        "name": spec.name,
        "value": value,
        "text": spec.display(value),
    }))
}

/// Merge key: the insert, slot and parameter.
fn by_param(params: &Value) -> String {
    format!("{}/{}/{}", super::field(params, "insert"), super::field(params, "slot"), super::field(params, "param"))
}

/// Merge key: the insert and slot.
fn by_slot(params: &Value) -> String {
    format!("{}/{}", super::field(params, "insert"), super::field(params, "slot"))
}

fn list(_ctx: &mut Ctx<'_>, _p: NoParams) -> Result<Value, CommandError> {
    let plugins: Vec<Value> = catalog().iter().filter(|i| !i.hidden).map(describe_effect).collect();
    Ok(json!({ "plugins": plugins }))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Describe {
    /// The effect by uid or name, such as `bodo.delay` or `Delay`.
    effect: String,
}

fn describe(_ctx: &mut Ctx<'_>, p: Describe) -> Result<Value, CommandError> {
    Ok(describe_effect(effect(&p.effect)?))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("fx.load", "Load effect", "Loads a native effect into an FX slot, the first empty one by default.", Effects::UNDOABLE, load)
        .example(json!({ "insert": 2, "slot": 1, "effect": "bodo.ducker", "params": { "Depth": 18 } }));
    r.edit("fx.remove", "Remove effect", "Empties an FX slot.", Effects::UNDOABLE, remove).example(json!({ "insert": 2, "slot": 1 }));
    r.edit("fx.move", "Move effect", "Moves an FX slot to another place in the chain; the slots between shift.", Effects::UNDOABLE, move_slot)
        .example(json!({ "insert": 2, "from": 1, "to": 3 }));
    r.edit("fx.swap", "Swap effects", "Swaps two FX slots.", Effects::UNDOABLE, swap).example(json!({ "insert": 2, "a": 1, "b": 2 }));
    r.edit(
        "fx.set_enabled",
        "Turn effect on or off",
        "Turns an FX slot on or off; a slot that is off keeps its latency. Toggles without `on`.",
        Effects::UNDOABLE,
        set_enabled,
    )
    .example(json!({ "insert": 2, "slot": 1, "on": false }));
    r.edit("fx.set_mix", "Set effect mix", "Sets an FX slot's dry/wet mix.", Effects::UNDOABLE, set_mix)
        .example(json!({ "insert": 2, "slot": 1, "mix": 0.5 }))
        .merge(by_slot);
    r.edit(
        "fx.set_sidechain",
        "Set effect sidechain",
        "Feeds an FX slot's sidechain input from a channel or insert, adding the sidechain route.",
        Effects::UNDOABLE,
        set_sidechain,
    )
    .example(json!({ "insert": 2, "slot": 1, "from": "insert:1" }));
    r.edit("fx.copy_chain", "Copy effects", "Copies an insert's FX slots, for fx.paste_chain.", Effects::NONE, copy_chain)
        .example(json!({ "insert": 2 }));
    r.edit(
        "fx.paste_chain",
        "Paste effects",
        "Replaces the FX slots of one or more inserts with the copied ones, each effect a new instance.",
        Effects::UNDOABLE,
        paste_chain,
    )
    .example(json!({ "inserts": [3, 4] }));
    r.edit("plugin.list", "List plugins", "Lists the effects FX slots can load, with their parameters.", Effects::NONE, list).example(json!({}));
    r.edit("plugin.describe", "Describe plugin", "Describes an effect and its parameters: ids, units, ranges and defaults.", Effects::NONE, describe)
        .example(json!({ "effect": "bodo.ducker" }));
    r.edit(
        "plugin.get_params",
        "Get effect parameters",
        "Lists the parameters of the effect in an FX slot with their values.",
        Effects::NONE,
        get_params,
    )
    .example(json!({ "insert": 2, "slot": 1 }));
    r.edit(
        "plugin.set_param",
        "Set effect parameter",
        "Sets a parameter of the effect in an FX slot, in its own unit.",
        Effects::UNDOABLE,
        set_param,
    )
    .example(json!({ "insert": 2, "slot": 1, "param": "Depth", "value": 24 }))
    .merge(by_param);
}
