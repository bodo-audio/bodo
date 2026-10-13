//! Effect editors (project.md sections 7.4 and 9.7): a free window per open effect, detachable like the others.
//! Native effects get a generic editor drawn from their parameters: a knob for each number with its unit and value, a
//! switch for each on/off, a menu for each choice, plus the slot's on switch, its mix and its presets. Right-click a
//! parameter to make an automation clip of it.
//!
//! An editor follows its effect by instance, so moving the slot keeps it open; it closes when the effect is removed.

use std::sync::Arc;

use bodo_kit_ui::{Knob, Led, icons};
use bodo_mix::fx::{EffectInfo, ParamKind, ParamSpec};
use bodo_model::channel::PluginFormat;
use bodo_model::{InsertId, PluginInstanceId, Project};
use egui::{Rect, RichText, Ui, vec2};
use serde_json::json;

use crate::i18n::{t, tr};
use crate::mixer::{insert_label, insert_ref, number_of};
use crate::theme::{Tokens, rem, size};
use crate::windows::{WindowState, show_frame_as};
use crate::{BodoApp, control};

/// An open effect editor.
#[derive(Clone, Debug)]
pub struct EffectEditor {
    /// The insert the effect is on.
    pub insert: InsertId,
    /// The effect's instance.
    pub instance: PluginInstanceId,
    /// Where the window is.
    pub state: WindowState,
}

/// The open effect editors.
#[derive(Debug, Default)]
pub struct EffectEditors {
    /// The windows.
    pub open: Vec<EffectEditor>,
}

impl EffectEditors {
    /// Whether the editor of `instance` is open.
    pub fn is_open(&self, instance: PluginInstanceId) -> bool {
        self.open.iter().any(|e| e.instance == instance)
    }
}

/// The egui id of the editor of `instance`.
pub fn area_id(instance: PluginInstanceId) -> egui::Id {
    egui::Id::new(("bodo-effect-editor", instance))
}

/// Opens the editor of the effect in slot `slot` (0 to 9) of `insert`, or brings it to the front.
pub fn open(app: &mut BodoApp, ctx: Option<&egui::Context>, insert: InsertId, slot: usize) {
    let project = Arc::clone(app.session.project());
    let Some(instance) = project.mixer.insert(insert).and_then(|i| i.slots.get(slot)).and_then(|s| s.plugin.as_ref()).map(|p| p.instance) else {
        return;
    };
    let editors = &mut app.view.effect_editors;
    if let Some(editor) = editors.open.iter_mut().find(|e| e.instance == instance) {
        editor.state.open = true;
        editor.state.minimized = false;
    } else {
        let n = editors.open.len() as f32;
        let frame = [0.28 + 0.02 * (n % 5.0), 0.12 + 0.03 * (n % 5.0), 0.36, 0.44];
        editors.open.push(EffectEditor { insert, instance, state: WindowState::at(true, frame) });
    }
    if let Some(ctx) = ctx {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, area_id(instance)));
    }
}

/// Where the effect of `instance` sits now: its insert's number and its slot, 0 to 9.
fn find(project: &Project, insert: InsertId, instance: PluginInstanceId) -> Option<(usize, usize)> {
    let number = number_of(project, insert)?;
    let slot = project.mixer.insert(insert)?.slots.iter().position(|s| s.plugin.as_ref().is_some_and(|p| p.instance == instance))?;
    Some((number, slot))
}

/// Draws every open effect editor inside `workspace`, and forgets those closed or whose effect is gone.
pub(crate) fn show_all(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let editors = std::mem::take(&mut app.view.effect_editors.open);
    let mut kept = Vec::with_capacity(editors.len());
    for mut editor in editors {
        let Some((number, slot)) = find(project, editor.insert, editor.instance) else { continue };
        let Some(state) = project.mixer.insert(editor.insert).and_then(|i| i.slots.get(slot)).cloned() else { continue };
        let Some(plugin) = state.plugin.clone() else { continue };
        let info = (plugin.format == PluginFormat::Native).then(|| bodo_mix::fx::find(&plugin.uid)).flatten();
        // The control channel knows the window by its English title; the title bar shows it translated.
        let name = info.map_or_else(|| plugin.uid.clone(), |i| i.name.to_owned());
        let title = format!("{name} - {} slot {}", insert_label(number), slot + 1);
        let effect = info.map_or_else(|| plugin.uid.clone(), |i| tr(i.name));
        let shown = if number == 0 {
            tf!("{effect} - Master slot {slot}", effect = effect, slot = slot + 1)
        } else {
            tf!("{effect} - Insert {number} slot {slot}", effect = effect, number = number, slot = slot + 1)
        };
        let (window, events) = show_frame_as(
            ctx,
            area_id(editor.instance),
            icons::ADJUSTMENTS_HORIZONTAL,
            editor.state,
            workspace,
            &title,
            &shown,
            true,
            app,
            |app, ui| header(app, ui, number, slot, &state),
            |app, ui| {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(rem(0.64) as i8, 0)).show(ui, |ui| match info {
                    Some(info) => body(app, ui, number, slot, &state, info),
                    None => {
                        ui.add_space(rem(0.64));
                        ui.label(t("This plugin's own editor opens once plugin hosting arrives; until then it passes audio through."));
                    }
                });
            },
        );
        editor.state = window;
        if !events.closed && editor.state.open {
            kept.push(editor);
        }
    }
    kept.append(&mut app.view.effect_editors.open);
    app.view.effect_editors.open = kept;
}

/// The title bar's controls: the slot's on switch and mix, and its presets.
fn header(app: &mut BodoApp, ui: &mut Ui, number: usize, slot: usize, state: &bodo_model::mixer::FxSlot) {
    let label = format!("{} slot {}", insert_label(number), slot + 1);
    let mut on = state.enabled;
    let led = ui.add(Led::new(&mut on).size(rem(0.96)).label(t("Effect on")));
    control::record(ui, &led, &format!("{label} editor on"), "led", Some(state.enabled));
    app.hint(&led, "Bypass: turn the effect off or on; off, it keeps its latency");
    if led.changed() {
        let _ = app.run("fx.set_enabled", json!({ "insert": insert_ref(number), "slot": slot + 1, "on": on }));
    }
    let mut mix = state.mix;
    let knob = ui.add(Knob::new(&mut mix, 0.0..=1.0).default(1.0).diameter(rem(1.44)).label(t("Mix")));
    control::record(ui, &knob, &format!("{label} editor mix"), "knob", None);
    app.hint(&knob, tf!("Mix: {percent}% wet", percent = format!("{:.0}", mix * 100.0)));
    if knob.changed() {
        app.run_gesture(&knob, "fx.set_mix", json!({ "insert": insert_ref(number), "slot": slot + 1, "mix": mix }));
    }
    let presets = ui.menu_button(RichText::new(t("Presets")).size(size::SMALL), |ui| {
        if ui.button(t("Save as preset")).clicked() {
            let _ = app.run("preset.save_plugin", json!({ "insert": insert_ref(number), "slot": slot + 1 }));
            ui.close();
        }
        let Some(library) = app.user_library() else {
            ui.label(t("Choose a library folder in the Browser to keep presets"));
            return;
        };
        let effect = state.plugin.as_ref().and_then(|p| bodo_mix::fx::find(&p.uid)).map_or("", |i| i.name);
        let folder = library.presets(bodo_engine::library::user::PresetFolder::PluginPresets).join(effect);
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&folder)
            .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "bodopreset")).collect())
            .unwrap_or_default();
        files.sort();
        ui.separator();
        if files.is_empty() {
            ui.label(t("No presets saved yet"));
        }
        for path in files {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if ui.button(&name).clicked() {
                let _ = app.run("preset.load_plugin", json!({ "path": path.display().to_string(), "insert": insert_ref(number), "slot": slot + 1 }));
                ui.close();
            }
        }
    });
    control::record(ui, &presets.response, &format!("{label} editor presets"), "menu", None);
}

/// Every parameter of a native effect.
fn body(app: &mut BodoApp, ui: &mut Ui, number: usize, slot: usize, state: &bodo_model::mixer::FxSlot, info: &EffectInfo) {
    let tk = Tokens::get();
    ui.add_space(rem(0.32));
    ui.label(RichText::new(tr(info.description)).size(size::SMALL).color(tk.text_dim));
    ui.add_space(rem(0.48));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(rem(0.8), rem(0.64));
        for spec in info.params {
            let value = state.param(spec.id).map_or(spec.default, |v| spec.clamp(v));
            ui.vertical(|ui| {
                ui.set_width(rem(6.72));
                param(app, ui, number, slot, info, spec, value);
            });
        }
    });
    if info.sidechain {
        ui.add_space(rem(0.64));
        let text = if state.sidechain.is_some() { t("Sidechain: set") } else { t("Sidechain: none, choose one from the slot's menu") };
        ui.label(RichText::new(text).size(size::SMALL).color(tk.text_dim));
    }
}

/// A parameter's value as its editor shows it: a choice's name in the interface's language, else the number with its
/// unit.
fn value_text(spec: &ParamSpec, value: f32) -> String {
    match spec.kind {
        ParamKind::Choice { choices } => choices.get(value as usize).map_or_else(|| spec.display(value), |c| tr(c)),
        _ => spec.display(value),
    }
}

/// One parameter's control, its name and its value.
fn param(app: &mut BodoApp, ui: &mut Ui, number: usize, slot: usize, info: &EffectInfo, spec: &ParamSpec, value: f32) {
    let tk = Tokens::get();
    // The control channel's English name for the parameter, and the name people see.
    let label = format!("{} {}", info.name, spec.name);
    let name = tr(spec.name);
    let set = |app: &mut BodoApp, response: &egui::Response, value: f32| {
        app.run_gesture(response, "plugin.set_param", json!({ "insert": insert_ref(number), "slot": slot + 1, "param": spec.id, "value": value }));
    };
    let response = match spec.kind {
        ParamKind::Toggle => {
            let mut on = value >= 0.5;
            let response = ui.add(Led::new(&mut on).size(rem(1.12)).label(&name));
            if response.changed() {
                set(app, &response, if on { 1.0 } else { 0.0 });
            }
            response
        }
        ParamKind::Choice { choices } => {
            let combo = egui::ComboBox::from_id_salt(("bodo-effect-choice", number, slot, spec.id))
                .width(rem(5.6))
                .selected_text(value_text(spec, value))
                .show_ui(ui, |ui| {
                    for (index, choice) in choices.iter().enumerate() {
                        if ui.selectable_label(index as f32 == value, tr(choice)).clicked() {
                            let _ = app
                                .run("plugin.set_param", json!({ "insert": insert_ref(number), "slot": slot + 1, "param": spec.id, "value": index }));
                        }
                    }
                });
            combo.response
        }
        ParamKind::Linear | ParamKind::Logarithmic | ParamKind::Integer => {
            let mut n = spec.to_normalized(value);
            let default = spec.to_normalized(spec.default);
            let response = ui.add(Knob::new(&mut n, 0.0..=1.0).default(default).diameter(rem(2.4)).label(&name));
            if response.changed() {
                set(app, &response, spec.from_normalized(n));
            }
            response
        }
    };
    control::record(ui, &response, &label, "param", None);
    app.hint(&response, tf!("{name}: {value}; right-click to automate it", name = name, value = value_text(spec, value)));
    response.context_menu(|ui| {
        if ui.button(t("Create automation clip")).clicked() {
            let _ = app.run("automation.create_for", json!({ "target": { "insert": insert_ref(number), "slot": slot + 1, "param": spec.name } }));
            ui.close();
        }
    });
    ui.label(RichText::new(name).size(size::SMALL).color(tk.text));
    ui.label(RichText::new(value_text(spec, value)).size(size::SMALL).monospace().color(tk.text_dim));
}
