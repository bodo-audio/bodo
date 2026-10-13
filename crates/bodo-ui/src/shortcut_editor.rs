//! The Keyboard shortcuts dialog (project.md section 14.5, ADR 0032): every shortcut, grouped and searchable, each
//! with a button that listens for new keys. Keys another shortcut already uses ask first, and can take them over. A
//! shortcut the user changed has a button that puts it back, and Reset all puts every one back.

use egui::{RichText, Sense, Ui, vec2};

use crate::shortcuts::{self, Binding, Group};
use crate::theme::{Tokens, size};
use crate::{BodoApp, control};

/// The dialog's state between frames.
#[derive(Debug, Default)]
pub(crate) struct EditorState {
    /// What the search field holds.
    pub(crate) filter: String,
    /// The shortcut listening for new keys.
    pub(crate) listening: Option<String>,
    /// New keys that another shortcut uses, waiting for the user to take them over or not.
    pub(crate) conflict: Option<Conflict>,
}

/// Keys the user pressed for a shortcut while another one has them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Conflict {
    /// The shortcut getting the keys.
    pub(crate) binding: String,
    /// The keys.
    pub(crate) keys: String,
    /// What the shortcuts that have them now do.
    pub(crate) others: Vec<String>,
}

/// What the user did in the dialog this frame, done once it has drawn.
enum Change {
    Listen(Option<String>),
    Assign { binding: String, keys: String, steal: bool },
    Reset(Option<String>),
    Dismiss,
}

/// Draws the dialog.
pub(crate) fn show(app: &mut BodoApp, ctx: &egui::Context) {
    capture(app, ctx);
    let tk = Tokens::get();
    let all = shortcuts::bindings(app);
    let filter = app.view.dialogs.shortcut_editor.filter.to_lowercase();
    let listening = app.view.dialogs.shortcut_editor.listening.clone();
    let conflict = app.view.dialogs.shortcut_editor.conflict.clone();
    let mut change = None;
    let mut open = true;
    let mut filter_text = app.view.dialogs.shortcut_editor.filter.clone();
    egui::Window::new("Keyboard shortcuts")
        .id(egui::Id::new("bodo-shortcuts-dialog"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(560.0, 520.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let search = ui.add(egui::TextEdit::singleline(&mut filter_text).hint_text("Search shortcuts").desired_width(260.0));
                control::record(ui, &search, "Search shortcuts", "text", None);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reset = ui.button("Reset all");
                    control::record(ui, &reset, "Reset all shortcuts", "button", None);
                    if reset.on_hover_text("Put every shortcut back to Bodo's keys").clicked() {
                        change = Some(Change::Reset(None));
                    }
                });
            });
            ui.label(
                RichText::new("Click a shortcut's keys, then press the new ones. Escape stops listening; right-click removes a shortcut.")
                    .color(tk.text_dim)
                    .size(size::SMALL),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for group in [Group::App, Group::Windows, Group::Tools, Group::Commands] {
                    let rows: Vec<&Binding> = all
                        .iter()
                        .filter(|b| b.group == group)
                        .filter(|b| filter.is_empty() || b.title.to_lowercase().contains(&filter) || b.id.to_lowercase().contains(&filter))
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new(group.label()).strong().color(tk.text));
                    egui::Grid::new(("bodo-shortcuts", group)).num_columns(3).striped(true).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                        for binding in rows {
                            if let Some(c) = row(ui, ctx, binding, listening.as_deref(), conflict.as_ref()) {
                                change = Some(c);
                            }
                            ui.end_row();
                        }
                    });
                }
            });
        });
    app.view.dialogs.shortcut_editor.filter = filter_text;
    if !open {
        change = Some(Change::Dismiss);
    }
    match change {
        Some(Change::Listen(id)) => {
            app.view.dialogs.shortcut_editor.listening = id;
            app.view.dialogs.shortcut_editor.conflict = None;
        }
        Some(Change::Assign { binding, keys, steal }) => {
            app.view.dialogs.shortcut_editor.conflict = None;
            if let Err(error) = shortcuts::assign(app, &binding, &keys, steal) {
                app.set_status(error, true);
            }
        }
        Some(Change::Reset(id)) => {
            shortcuts::reset(app, id.as_deref());
            app.view.dialogs.shortcut_editor.conflict = None;
        }
        Some(Change::Dismiss) => {
            app.view.dialogs.shortcuts = false;
            app.view.dialogs.shortcut_editor = EditorState::default();
        }
        None => {}
    }
}

/// One shortcut's row: what it does, its keys (a button that listens for new ones), and a reset button when the user
/// changed it. A conflict about it shows under it.
fn row(ui: &mut Ui, ctx: &egui::Context, binding: &Binding, listening: Option<&str>, conflict: Option<&Conflict>) -> Option<Change> {
    let tk = Tokens::get();
    let mut change = None;
    ui.label(&binding.title).on_hover_text(&binding.id);
    let is_listening = listening == Some(binding.id.as_str());
    let text = if is_listening {
        RichText::new("Press keys...").color(tk.accent)
    } else {
        match &binding.keys {
            Some(keys) => RichText::new(shortcuts::display(ctx, keys)).monospace().color(tk.text),
            None => RichText::new("None").color(tk.text_faint),
        }
    };
    let keys = ui.add(egui::Button::new(text).selected(is_listening).min_size(vec2(120.0, 0.0)));
    control::record(ui, &keys, &format!("Shortcut {}", binding.id), "button", Some(is_listening));
    if keys.clicked() {
        change = Some(Change::Listen(if is_listening { None } else { Some(binding.id.clone()) }));
    }
    keys.context_menu(|ui| {
        if ui.button("Remove shortcut").clicked() {
            change = Some(Change::Assign { binding: binding.id.clone(), keys: String::new(), steal: false });
            ui.close();
        }
    });
    if binding.changed() {
        let default = binding.default.as_deref().map_or_else(|| String::from("none"), |d| shortcuts::display(ctx, d));
        let reset = ui.add(egui::Button::new(RichText::new("Reset").size(size::SMALL)).small());
        control::record(ui, &reset, &format!("Reset shortcut {}", binding.id), "button", None);
        if reset.on_hover_text(format!("Bodo's keys: {default}")).clicked() {
            change = Some(Change::Reset(Some(binding.id.clone())));
        }
    } else {
        ui.allocate_exact_size(vec2(1.0, 1.0), Sense::hover());
    }
    if let Some(conflict) = conflict.filter(|c| c.binding == binding.id) {
        ui.end_row();
        ui.label("");
        ui.horizontal(|ui| {
            let keys = shortcuts::display(ctx, &conflict.keys);
            ui.label(RichText::new(format!("{keys} is used by {}.", conflict.others.join(", "))).color(tk.warn).size(size::SMALL));
            let replace = ui.small_button("Use it here");
            control::record(ui, &replace, "Use the keys here", "button", None);
            if replace.clicked() {
                change = Some(Change::Assign { binding: conflict.binding.clone(), keys: conflict.keys.clone(), steal: true });
            }
            let cancel = ui.small_button("Cancel");
            control::record(ui, &cancel, "Keep the keys there", "button", None);
            if cancel.clicked() {
                change = Some(Change::Listen(None));
            }
        });
        ui.label("");
    }
    change
}

/// While a shortcut listens, takes this frame's first key press for it: Escape stops listening, anything else
/// becomes its keys, or a conflict to answer when another shortcut has them. The press is removed from the input,
/// so no button or field reacts to it as well.
fn capture(app: &mut BodoApp, ctx: &egui::Context) {
    let Some(binding) = app.view.dialogs.shortcut_editor.listening.clone() else { return };
    let pressed = ctx.input_mut(|i| {
        let found = i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
            _ => None,
        });
        if found.is_some() {
            i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
        }
        found
    });
    let Some((key, modifiers)) = pressed else { return };
    app.view.dialogs.shortcut_editor.listening = None;
    if key == egui::Key::Escape && modifiers.is_none() {
        return;
    }
    let keys = shortcuts::format(&egui::KeyboardShortcut::new(modifiers, key));
    let all = shortcuts::bindings(app);
    let others: Vec<String> = shortcuts::conflicts(&all, &binding, &keys).iter().map(|b| b.title.clone()).collect();
    if others.is_empty() {
        if let Err(error) = shortcuts::assign(app, &binding, &keys, false) {
            app.set_status(error, true);
        }
    } else {
        app.view.dialogs.shortcut_editor.conflict = Some(Conflict { binding, keys, others });
    }
}
