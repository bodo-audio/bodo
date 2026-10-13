//! The insert panel on the mixer's right (project.md section 9.7), titled with the selected insert's name: its input,
//! the ten FX slots, the strip EQ, its outputs, the latency and CPU readouts, the track latency and the delay
//! compensation bypass.
//!
//! A slot shows an enable light, its effect and a mix knob. Click an empty slot to pick an effect, double-click a full
//! one to open its editor, drag one onto another to move it, and drop an effect from the Browser on a slot (or a
//! strip) to load it. The context menu replaces, saves and loads presets, moves, sets the sidechain source and the
//! sandbox mode.

use std::sync::Arc;

use bodo_kit_ui::{Knob, Led};
use bodo_mix::fx::{Category, EffectInfo};
use bodo_model::channel::{PluginFormat, SandboxMode};
use bodo_model::mixer::{FxSlot, PdcBypass, RouteKind, RouteSource};
use bodo_model::{InsertId, MixerInsert, Project};
use egui::{Align2, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use super::{device_channels, eq, insert_at, insert_label, insert_ref, number_of, port_choices, port_label};
use crate::i18n::{en, t, tr};
use crate::theme::{Tokens, rem, size};
use crate::{BodoApp, control};

/// An effect dragged from the Browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EffectDrag {
    /// The effect's uid, such as `bodo.delay`.
    pub uid: &'static str,
}

/// An FX slot dragged to another place in its chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlotDrag {
    /// The insert.
    pub insert: InsertId,
    /// The slot, 0 to 9.
    pub slot: usize,
}

/// Takes the drag payload of type `T` dropped on `rect` this frame, when `rect` is the topmost thing under the pointer.
pub(crate) fn take_drop<T: Send + Sync + 'static>(ui: &Ui, rect: Rect) -> Option<Arc<T>> {
    let ctx = ui.ctx();
    egui::DragAndDrop::payload::<T>(ctx)?;
    let (released, at) = ctx.input(|i| (i.pointer.any_released(), i.pointer.interact_pos().or_else(|| i.pointer.hover_pos())));
    let at = at?;
    if !released || !rect.contains(at) || ctx.layer_id_at(at) != Some(ui.layer_id()) {
        return None;
    }
    egui::DragAndDrop::take_payload::<T>(ctx)
}

/// Whether a payload of type `T` is being dragged over `rect`, for drawing drop targets.
fn drag_over<T: Send + Sync + 'static>(ui: &Ui, rect: Rect) -> bool {
    egui::DragAndDrop::has_payload_of_type::<T>(ui.ctx()) && ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p))
}

/// The effects the slots offer, by category, hidden test effects left out.
pub(crate) fn effects_by_category() -> Vec<(Category, Vec<&'static EffectInfo>)> {
    let mut out: Vec<(Category, Vec<&'static EffectInfo>)> = Vec::new();
    for info in bodo_mix::fx::catalog().iter().filter(|i| !i.hidden) {
        match out.iter_mut().find(|(c, _)| *c == info.category) {
            Some((_, list)) => list.push(info),
            None => out.push((info.category, vec![info])),
        }
    }
    out
}

/// A category's name in menus, in English; [`t`] shows it in the interface's language.
pub(crate) fn category_name(category: Category) -> &'static str {
    match category {
        Category::Utility => en("Utility"),
        Category::Filter => en("Filter"),
        Category::Delay => en("Delay"),
        Category::Dynamics => en("Dynamics"),
    }
}

/// Draws the insert panel for the selected insert, or the master.
pub(super) fn show(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>) {
    let id = app.session.editor().insert.filter(|i| project.mixer.insert(*i).is_some()).unwrap_or(project.mixer.master.id);
    let Some(number) = number_of(project, id) else { return };
    let Some(insert) = insert_at(project, number).cloned() else { return };
    let tk = Tokens::get();
    let title = ui.label(RichText::new(tf!("Mixer - {insert}", insert = insert.name)).strong().color(tk.text));
    control::record(ui, &title, "Insert panel", "heading", None);
    egui::ScrollArea::vertical().id_salt("bodo-insert-panel").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = rem(0.32);
        input(app, ui, &insert, number);
        ui.separator();
        slots(app, ui, project, &insert, number);
        ui.separator();
        eq::show(app, ui, project, &insert, number);
        ui.separator();
        outputs(app, ui, &insert, number);
        ui.separator();
        timing(app, ui, &insert, number);
    });
}

/// A label on the left, in the interface's language, and a control on the right.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let label = tr(label);
        ui.add_sized(vec2(rem(6.24), rem(1.44)), egui::Label::new(RichText::new(label).size(size::SMALL).color(Tokens::get().text_dim)).truncate());
        add(ui);
    });
}

/// A device port's name in the interface's language, such as "In 1-2" or "None".
fn port_text(prefix: &str, channels: Option<&[u16]>) -> String {
    match channels {
        None | Some([]) => String::from(t("None")),
        Some(_) => port_label(&format!("{} ", tr(prefix)), channels),
    }
}

fn input(app: &mut BodoApp, ui: &mut Ui, insert: &MixerInsert, number: usize) {
    let count = device_channels(app, true);
    row(ui, "Audio input", |ui| {
        let current = insert.input.as_ref().map(|port| port.channels.clone());
        let combo = egui::ComboBox::from_id_salt(("bodo-panel-input", insert.id))
            .width(rem(9.6))
            .height(rem(19.2))
            .selected_text(port_text(en("In"), current.as_deref()))
            .show_ui(ui, |ui| {
                for choice in port_choices(count) {
                    if ui.selectable_label(current == choice, port_text(en("In"), choice.as_deref())).clicked() {
                        let _ = app.run("mixer.set_input", json!({ "insert": insert_ref(number), "channels": choice }));
                    }
                }
            });
        control::record(ui, &combo.response, "Panel input", "combo", None);
        app.hint(&combo.response, "The device input this insert records and monitors");
    });
}

fn slots(app: &mut BodoApp, ui: &mut Ui, project: &Project, insert: &MixerInsert, number: usize) {
    let tk = Tokens::get();
    ui.label(RichText::new(t("Effects")).size(size::SMALL).color(tk.text_dim));
    for (index, slot) in insert.slots.iter().enumerate() {
        slot_row(app, ui, project, insert, number, index, slot);
    }
}

/// The name a slot shows: its effect's, in the interface's language, or the plugin's uid until plugins load.
fn slot_name(slot: &FxSlot) -> Option<String> {
    let plugin = slot.plugin.as_ref()?;
    Some(match plugin.format {
        PluginFormat::Native => bodo_mix::fx::find(&plugin.uid).map_or_else(|| plugin.uid.clone(), |i| tr(i.name)),
        _ => plugin.uid.clone(),
    })
}

/// The English name of a slot's effect, which the library's preset folders are named after.
fn effect_folder(slot: &FxSlot) -> String {
    slot.plugin.as_ref().map_or_else(String::new, |p| bodo_mix::fx::find(&p.uid).map_or_else(|| p.uid.clone(), |i| i.name.to_owned()))
}

#[allow(clippy::too_many_arguments, reason = "one row of the panel, kept apart for reading")]
fn slot_row(app: &mut BodoApp, ui: &mut Ui, project: &Project, insert: &MixerInsert, number: usize, index: usize, slot: &FxSlot) {
    let tk = Tokens::get();
    let label = format!("{} slot {}", insert_label(number), index + 1);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, rem(1.6)), Sense::click_and_drag());
    let chosen = app.view.mixer.slot == Some(index);
    let target = drag_over::<EffectDrag>(ui, rect) || drag_over::<SlotDrag>(ui, rect);
    let fill = if chosen {
        tk.accent_soft
    } else if response.hovered() {
        tk.control_hover
    } else {
        tk.field
    };
    ui.painter().rect(rect, 3.0, fill, Stroke::new(1.0, if target { tk.accent } else { tk.border }), StrokeKind::Inside);
    ui.painter().text(
        pos2(rect.left() + rem(0.32), rect.center().y),
        Align2::LEFT_CENTER,
        format!("{}", index + 1),
        FontId::monospace(rem(0.72)),
        tk.text_faint,
    );
    let name = slot_name(slot);
    let shown = name.clone().unwrap_or_else(|| String::from(t("(empty)")));
    let color = match (&name, slot.enabled) {
        (None, _) => tk.text_faint,
        (Some(_), false) => tk.text_dim,
        (Some(_), true) => tk.text,
    };
    let text_rect = Rect::from_min_max(pos2(rect.left() + rem(2.56), rect.top()), pos2(rect.right() - rem(1.92), rect.bottom()));
    ui.painter().with_clip_rect(text_rect).text(
        pos2(text_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &shown,
        FontId::proportional(size::SMALL),
        color,
    );
    if slot.sidechain.is_some() {
        ui.painter().text(pos2(rect.right() - rem(2.08), rect.center().y), Align2::RIGHT_CENTER, "SC", FontId::proportional(rem(0.64)), tk.warn);
    }
    control::record(ui, &response, &label, "slot", Some(chosen));
    let hint = match &name {
        Some(effect) => {
            tf!("Slot {slot}: {effect}. Double-click to edit it, drag to move it, right-click for more", slot = index + 1, effect = effect)
        }
        None => tf!("Slot {slot}: click to load an effect, or drop one from the Browser", slot = index + 1),
    };
    app.hint(&response, hint);
    if response.clicked() {
        app.view.mixer.slot = Some(index);
    }
    if name.is_some() && response.double_clicked() {
        crate::effect_editor::open(app, Some(ui.ctx()), insert.id, index);
    }
    if name.is_some() {
        response.dnd_set_drag_payload(SlotDrag { insert: insert.id, slot: index });
    } else {
        // Its own id: the slot's right-click menu has the response's.
        egui::Popup::menu(&response).id(response.id.with("picker")).show(|ui| picker(app, ui, number, index));
    }
    if let Some(moved) = take_drop::<SlotDrag>(ui, rect)
        && moved.insert == insert.id
        && moved.slot != index
    {
        let _ = app.run("fx.move", json!({ "insert": insert_ref(number), "from": moved.slot + 1, "to": index + 1 }));
        app.view.mixer.slot = Some(index);
    }
    if let Some(effect) = take_drop::<EffectDrag>(ui, rect) {
        let _ = app.run("fx.load", json!({ "insert": insert_ref(number), "slot": index + 1, "effect": effect.uid }));
        app.view.mixer.slot = Some(index);
    }
    slot_menu(app, &response, project, insert, number, index, slot);

    if name.is_some() {
        // The enable light on the left and the mix knob on the right sit above the row.
        let led_rect = Rect::from_center_size(pos2(rect.left() + rem(1.68), rect.center().y), vec2(rem(0.96), rem(0.96)));
        let mut on = slot.enabled;
        let led = ui.put(led_rect, Led::new(&mut on).size(rem(0.8)).label(tf!("Slot {slot} on", slot = index + 1)));
        control::record(ui, &led, &format!("{label} on"), "led", Some(slot.enabled));
        app.hint(&led, tf!("Slot {slot}: turn it on or off; off, it keeps its latency", slot = index + 1));
        if led.changed() {
            let _ = app.run("fx.set_enabled", json!({ "insert": insert_ref(number), "slot": index + 1, "on": on }));
        }
        let knob_rect = Rect::from_center_size(pos2(rect.right() - rem(0.88), rect.center().y), vec2(rem(1.36), rem(1.36)));
        let mut mix = slot.mix;
        let knob = ui.put(knob_rect, Knob::new(&mut mix, 0.0..=1.0).default(1.0).diameter(rem(1.2)).label(tf!("Slot {slot} mix", slot = index + 1)));
        control::record(ui, &knob, &format!("{label} mix"), "knob", None);
        app.hint(&knob, tf!("Slot {slot} mix: {percent}% wet", slot = index + 1, percent = format!("{:.0}", mix * 100.0)));
        if knob.changed() {
            app.run_gesture(&knob, "fx.set_mix", json!({ "insert": insert_ref(number), "slot": index + 1, "mix": mix }));
        }
        knob.context_menu(|ui| {
            if ui.button(t("Create automation clip")).clicked() {
                let _ = app.run("automation.create_for", json!({ "target": { "insert": insert_ref(number), "slot": index + 1, "param": "mix" } }));
                ui.close();
            }
        });
    }
}

/// The effect picker: every effect by category.
fn picker(app: &mut BodoApp, ui: &mut Ui, number: usize, index: usize) {
    ui.set_min_width(rem(13.6));
    for (category, effects) in effects_by_category() {
        ui.label(RichText::new(t(category_name(category))).size(size::SMALL).color(Tokens::get().text_faint));
        for info in effects {
            let button = ui.button(tr(info.name)).on_hover_text(tr(info.description));
            if button.clicked() {
                let _ = app.run("fx.load", json!({ "insert": insert_ref(number), "slot": index + 1, "effect": info.uid }));
                app.view.mixer.slot = Some(index);
                ui.close();
            }
        }
    }
}

/// The sandbox modes, with their command names and menu labels.
const SANDBOXES: [(SandboxMode, &str, &str); 4] = [
    (SandboxMode::InProcess, "inProcess", en("Inside Bodo")),
    (SandboxMode::SharedBridge, "sharedBridge", en("Shared bridge")),
    (SandboxMode::PerVendor, "perVendor", en("A bridge per vendor")),
    (SandboxMode::PerPlugin, "perPlugin", en("A bridge of its own")),
];

#[allow(clippy::too_many_arguments, reason = "one menu of the panel, kept apart for reading")]
fn slot_menu(app: &mut BodoApp, response: &egui::Response, project: &Project, insert: &MixerInsert, number: usize, index: usize, slot: &FxSlot) {
    let target = insert_ref(number);
    let full = slot.plugin.is_some();
    response.context_menu(|ui| {
        ui.set_min_width(rem(15.2));
        if full && ui.button(t("Open editor")).clicked() {
            crate::effect_editor::open(app, Some(ui.ctx()), insert.id, index);
            ui.close();
        }
        ui.menu_button(if full { t("Replace") } else { t("Load") }, |ui| picker(app, ui, number, index));
        if full {
            ui.menu_button(t("Presets"), |ui| plugin_presets(app, ui, slot, number, index));
            ui.separator();
            if index > 0 && ui.button(t("Move up")).clicked() {
                let _ = app.run("fx.move", json!({ "insert": target, "from": index + 1, "to": index }));
                ui.close();
            }
            if index + 1 < insert.slots.len() && ui.button(t("Move down")).clicked() {
                let _ = app.run("fx.move", json!({ "insert": target, "from": index + 1, "to": index + 2 }));
                ui.close();
            }
            ui.menu_button(t("Sidechain from"), |ui| sidechain_menu(app, ui, project, insert, number, index, slot));
            ui.menu_button(t("Run plugin"), |ui| {
                let current = slot.plugin.as_ref().map(|p| p.sandbox);
                let native = slot.plugin.as_ref().is_some_and(|p| p.format == PluginFormat::Native);
                if native {
                    ui.label(RichText::new(t("Native effects always run inside Bodo; this applies to plugins")).size(size::SMALL));
                    ui.separator();
                }
                for (mode, id, label) in SANDBOXES {
                    if ui.radio(current == Some(mode), t(label)).clicked() {
                        let _ = app.run("fx.set_sandbox", json!({ "insert": target, "slot": index + 1, "mode": id }));
                    }
                }
            });
            ui.separator();
            if ui.button(t("Remove")).clicked() {
                let _ = app.run("fx.remove", json!({ "insert": target, "slot": index + 1 }));
                ui.close();
            }
        }
        ui.separator();
        if ui.button(t("Copy the chain")).clicked() {
            let _ = app.run("fx.copy_chain", json!({ "insert": target }));
            ui.close();
        }
        if app.session.editor().chain.is_some() && ui.button(t("Paste the chain")).clicked() {
            let _ = app.run("fx.paste_chain", json!({ "inserts": [target] }));
            ui.close();
        }
    });
}

/// Saving the slot's settings as a plugin preset, and loading the presets of its effect from the library.
fn plugin_presets(app: &mut BodoApp, ui: &mut Ui, slot: &FxSlot, number: usize, index: usize) {
    if ui.button(t("Save as preset")).clicked() {
        let _ = app.run("preset.save_plugin", json!({ "insert": insert_ref(number), "slot": index + 1 }));
        ui.close();
    }
    let Some(library) = app.user_library() else {
        ui.label(t("Choose a library folder in the Browser to keep presets"));
        return;
    };
    let effect = effect_folder(slot);
    let folder = library.presets(bodo_engine::library::user::PresetFolder::PluginPresets).join(&effect);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&folder)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "bodopreset")).collect())
        .unwrap_or_default();
    files.sort();
    ui.separator();
    if files.is_empty() {
        ui.label(tf!("No {effect} presets saved yet", effect = slot_name(slot).unwrap_or(effect)));
    }
    for path in files {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if ui.button(&name).clicked() {
            let _ = app.run("preset.load_plugin", json!({ "path": path.display().to_string(), "insert": insert_ref(number), "slot": index + 1 }));
            ui.close();
        }
    }
}

/// Choosing the channel or insert a slot's sidechain listens to: sources with a sidechain route here first, then
/// every other insert and channel, which `fx.set_sidechain` routes.
#[allow(clippy::too_many_arguments, reason = "one menu of the panel, kept apart for reading")]
fn sidechain_menu(app: &mut BodoApp, ui: &mut Ui, project: &Project, insert: &MixerInsert, number: usize, index: usize, slot: &FxSlot) {
    let target = insert_ref(number);
    let takes_sidechain = slot.plugin.as_ref().and_then(|p| bodo_mix::fx::find(&p.uid)).is_some_and(|i| i.sidechain);
    if !takes_sidechain {
        ui.label(RichText::new(t("This effect has no sidechain input")).size(size::SMALL));
    }
    if ui.radio(slot.sidechain.is_none(), t("None")).clicked() {
        let _ = app.run("fx.set_sidechain", json!({ "insert": target, "slot": index + 1, "from": null }));
    }
    let routed: Vec<RouteSource> =
        project.mixer.routes.iter().filter(|(k, r)| k.to == insert.id && r.kind == RouteKind::Sidechain).map(|(k, _)| k.from).collect();
    let mut sources: Vec<(RouteSource, String)> = routed.iter().map(|s| (*s, source_name(project, *s))).collect();
    for (n, id) in project.mixer.order.iter().enumerate() {
        let source = RouteSource::Insert(*id);
        if *id != insert.id && !routed.contains(&source) {
            sources.push((source, tf!("Insert {number}", number = n + 1)));
        }
    }
    for id in project.rack_order.iter() {
        let source = RouteSource::Channel(*id);
        if !routed.contains(&source) {
            sources.push((source, source_name(project, source)));
        }
    }
    ui.separator();
    egui::ScrollArea::vertical().max_height(rem(20.8)).show(ui, |ui| {
        for (source, name) in sources {
            let text = match source {
                RouteSource::Insert(id) => {
                    let n = number_of(project, id).unwrap_or(0);
                    let insert_name = insert_at(project, n).map_or_else(String::new, |i| i.name.clone());
                    if insert_name == name || insert_name == format!("Insert {n}") { name.clone() } else { format!("{name}: {insert_name}") }
                }
                RouteSource::Channel(_) => name.clone(),
            };
            if ui.radio(slot.sidechain == Some(source), text).clicked() {
                let from = match source {
                    RouteSource::Insert(id) => format!("insert:{}", number_of(project, id).unwrap_or(0)),
                    RouteSource::Channel(id) => project.channels.get(&id).map_or_else(String::new, |c| format!("channel:{}", c.name)),
                };
                let _ = app.run("fx.set_sidechain", json!({ "insert": target, "slot": index + 1, "from": from }));
            }
        }
    });
}

/// A route source's name, in the interface's language: `Insert N` or the channel's name.
fn source_name(project: &Project, source: RouteSource) -> String {
    match source {
        RouteSource::Insert(id) => tf!("Insert {number}", number = number_of(project, id).unwrap_or(0)),
        RouteSource::Channel(id) => project.channels.get(&id).map_or_else(|| String::from(t("A channel")), |c| c.name.clone()),
    }
}

fn outputs(app: &mut BodoApp, ui: &mut Ui, insert: &MixerInsert, number: usize) {
    let tk = Tokens::get();
    ui.label(RichText::new(t("Outputs")).size(size::SMALL).color(tk.text_dim));
    if number != 0 {
        let mut to_master = insert.to_master;
        let check = ui.checkbox(&mut to_master, t("To the master"));
        control::record(ui, &check, "Panel to master", "checkbox", Some(insert.to_master));
        app.hint(&check, "Whether this insert feeds the master");
        if check.changed() {
            let _ = app.run("mixer.set_to_master", json!({ "insert": insert_ref(number), "on": to_master }));
        }
    }
    let count = device_channels(app, false);
    row(ui, "Direct out", |ui| {
        let current = insert.output.as_ref().map(|port| port.channels.clone());
        let combo = egui::ComboBox::from_id_salt(("bodo-panel-output", insert.id))
            .width(rem(9.6))
            .height(rem(19.2))
            .selected_text(port_text(en("Out"), current.as_deref()))
            .show_ui(ui, |ui| {
                for choice in port_choices(count) {
                    if ui.selectable_label(current == choice, port_text(en("Out"), choice.as_deref())).clicked() {
                        let _ = app.run("mixer.set_output", json!({ "insert": insert_ref(number), "channels": choice }));
                    }
                }
            });
        control::record(ui, &combo.response, "Panel direct out", "combo", None);
        app.hint(&combo.response, "Device outputs this insert also plays straight to, such as a cue mix");
    });
}

/// The delay compensation bypass modes, with their command names and labels.
const BYPASSES: [(PdcBypass, &str, &str); 3] = [
    (PdcBypass::Never, "never", en("Never")),
    (PdcBypass::Always, "always", en("Always")),
    (PdcBypass::WhileRecording, "whileRecording", en("While recording")),
];

fn timing(app: &mut BodoApp, ui: &mut Ui, insert: &MixerInsert, number: usize) {
    let tk = Tokens::get();
    let rate = f64::from(app.session.project().settings.sample_rate.hz());
    let ms = |frames: u32| format!("{:.1}", f64::from(frames) * 1000.0 / rate);
    let latency = super::insert_latency(app, insert.id);
    let cpu = super::cpu(app, ui, insert.id);
    let text = match latency {
        Some(l) => tf!("Latency {total} ms (in {input}, own {own})", total = ms(l.output), input = ms(l.input), own = ms(l.own)),
        None => String::from(t("Latency: -")),
    };
    let readout = ui.label(RichText::new(text).size(size::SMALL).monospace().color(tk.text_dim));
    control::record(ui, &readout, "Panel latency", "readout", None);
    app.hint(&readout, "How late this insert's sound is: what reaches it, what its effects add, and what leaves it");
    let cpu_text = cpu.map_or_else(|| String::from(t("CPU: no sound")), |load| tf!("CPU {load}%", load = format!("{:.1}", load * 100.0)));
    let cpu_label = ui.label(RichText::new(cpu_text).size(size::SMALL).monospace().color(tk.text_dim));
    control::record(ui, &cpu_label, "Panel CPU", "readout", None);
    app.hint(&cpu_label, "The share of real time this insert takes to process");
    row(ui, "Track latency", |ui| {
        let mut offset = (f64::from(insert.latency_offset.0 as f32) * 1000.0 / rate) as f32;
        let drag = ui.add(egui::DragValue::new(&mut offset).speed(0.1).range(-500.0..=500.0).suffix(" ms").fixed_decimals(1));
        control::record(ui, &drag, "Panel track latency", "value", None);
        app.hint(&drag, "Track latency: positive declares latency the insert has, negative delays it");
        if drag.changed() {
            app.run_gesture(&drag, "mixer.set_latency_offset", json!({ "insert": insert_ref(number), "ms": offset }));
        }
    });
    row(ui, "PDC bypass", |ui| {
        let shown = BYPASSES.iter().find(|(mode, _, _)| *mode == insert.pdc_bypass).map_or("", |(_, _, label)| t(label));
        let combo = egui::ComboBox::from_id_salt(("bodo-panel-pdc", insert.id)).width(rem(9.6)).selected_text(shown).show_ui(ui, |ui| {
            for (mode, id, label) in BYPASSES {
                if ui.selectable_label(insert.pdc_bypass == mode, t(label)).clicked() {
                    let _ = app.run("mixer.set_pdc_bypass", json!({ "insert": insert_ref(number), "mode": id }));
                }
            }
        });
        control::record(ui, &combo.response, "Panel PDC bypass", "combo", None);
        app.hint(&combo.response, "When this insert skips delay compensation, for monitoring with the least latency");
    });
}
