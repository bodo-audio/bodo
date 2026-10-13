//! Dialogs: unsaved changes, renaming, audio settings, crash recovery, shortcuts and about.

use std::path::PathBuf;

use egui::{Align2, RichText, vec2};
use serde_json::json;

use crate::BodoApp;
use crate::theme::{Tokens, size};

/// What to do once unsaved changes are dealt with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    /// Start a new project.
    New,
    /// Open a project: this file, or one the user picks.
    Open(Option<PathBuf>),
    /// Start a new project from a template or a demo, by path or library key.
    NewFrom(String),
    /// Open an autosave as an unsaved project.
    Recover(PathBuf),
    /// Close Bodo.
    Quit,
}

/// What renaming is under way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rename {
    /// The current pattern.
    Pattern(String),
    /// A channel, by id.
    Channel(String, String),
    /// A mixer insert, by number.
    Insert(usize, String),
}

/// Open dialogs and small pieces of interface state.
#[derive(Debug, Default)]
pub struct Dialogs {
    pub(crate) unsaved: Option<Pending>,
    pub(crate) rename: Option<Rename>,
    /// The audio settings dialog.
    pub audio: bool,
    pub(crate) audio_draft: Option<crate::audio::AudioPrefs>,
    /// The MIDI inputs the system listed when the audio settings opened.
    pub(crate) midi_ports: Option<Vec<String>>,
    /// The keyboard shortcuts list.
    pub shortcuts: bool,
    /// The about box.
    pub about: bool,
    /// Autosaves found after a crash, offered for recovery.
    pub recovery: Vec<PathBuf>,
    /// The song position shows minutes and seconds instead of bars, steps and ticks.
    pub clock_seconds: bool,
}

/// Does `action` now if there are no unsaved changes; otherwise asks first.
pub(crate) fn guard_unsaved(app: &mut BodoApp, action: Pending) {
    if app.session.is_dirty() {
        app.view.dialogs.unsaved = Some(action);
    } else {
        perform(app, action);
    }
}

fn perform(app: &mut BodoApp, action: Pending) {
    match action {
        Pending::New => {
            let _ = app.run("project.new", json!({ "discardChanges": true }));
        }
        Pending::Open(path) => {
            let path = path.or_else(|| app.services.pick_open.as_ref().and_then(|pick| pick()));
            if let Some(path) = path {
                let _ = app.run("project.open", json!({ "path": path.display().to_string(), "discardChanges": true }));
            }
        }
        Pending::NewFrom(from) => {
            let _ = app.run("project.new", json!({ "from": from, "discardChanges": true }));
        }
        Pending::Recover(path) => crate::autosave::recover(app, &path),
        Pending::Quit => app.quit_requested = true,
    }
}

/// Saves the project: to its file, or to one the user picks when it has none or `pick` is set. Returns whether it
/// saved.
pub(crate) fn save(app: &mut BodoApp, pick: bool) -> bool {
    if !pick && app.session.file().is_some() {
        return app.run("project.save", json!({})).is_ok();
    }
    let name = format!("{}.bodo", app.title().trim_end_matches(" *"));
    // A project's first save starts in the library's My projects folder.
    let suggestion = match (app.session.file(), app.user_library()) {
        (None, Some(library)) => library.my_projects().join(&name).display().to_string(),
        _ => name,
    };
    let Some(path) = app.services.pick_save.as_ref().and_then(|pick| pick(&suggestion)) else { return false };
    app.run("project.save_as", json!({ "path": path.display().to_string() })).is_ok()
}

/// Starts renaming the current pattern.
pub(crate) fn rename_pattern(app: &mut BodoApp) {
    let name = app.session.editor().pattern.and_then(|p| app.session.project().patterns.get(&p)).map(|p| p.name.clone());
    if let Some(name) = name {
        app.view.dialogs.rename = Some(Rename::Pattern(name));
    }
}

/// Starts renaming channel `id`.
pub(crate) fn rename_channel(app: &mut BodoApp, id: bodo_model::ChannelId) {
    if let Some(channel) = app.session.project().channels.get(&id) {
        app.view.dialogs.rename = Some(Rename::Channel(id.to_string(), channel.name.clone()));
    }
}

/// Asks for a folder of sounds and adds it to the browser.
pub(crate) fn add_sample_folder(app: &mut BodoApp) {
    if let Some(folder) = app.services.pick_folder.as_ref().and_then(|pick| pick())
        && !app.prefs.library.contains(&folder)
    {
        app.prefs.library.push(folder);
    }
}

/// Asks for a MIDI file and imports it as a new pattern placed on the playlist.
pub(crate) fn import_midi(app: &mut BodoApp) {
    if let Some(path) = app.services.pick_midi.as_ref().and_then(|pick| pick()) {
        let _ = app.run("project.import_midi", json!({ "path": path.display().to_string() }));
    }
}

/// Asks where to write a MIDI file and writes the current pattern (`pattern`) or the whole song to it.
pub(crate) fn export_midi(app: &mut BodoApp, pattern: bool) {
    let name = if pattern {
        app.session.editor().pattern.and_then(|p| app.session.project().patterns.get(&p)).map(|p| p.name.clone())
    } else {
        Some(app.title().trim_end_matches(" *").to_owned())
    };
    let Some(name) = name else { return };
    let Some(path) = app.services.pick_midi_save.as_ref().and_then(|pick| pick(&format!("{name}.mid"))) else { return };
    let command = if pattern { "pattern.export_midi" } else { "project.export_midi" };
    let _ = app.run(command, json!({ "path": path.display().to_string() }));
}

/// Asks for audio files and adds a sampler channel for each.
pub(crate) fn add_channels_from_files(app: &mut BodoApp) {
    let files = app.services.pick_samples.as_ref().map(|pick| pick()).unwrap_or_default();
    for file in files {
        let _ = app.run("channel.add", json!({ "sample": file.display().to_string() }));
    }
}

fn modal(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let tk = Tokens::get();
    egui::Window::new(RichText::new(title).size(size::HEADING).color(tk.text))
        .id(egui::Id::new(("bodo-dialog", title)))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
        .order(egui::Order::Foreground)
        .show(ctx, add);
}

/// Draws whichever dialogs are open.
pub(crate) fn show(app: &mut BodoApp, ctx: &egui::Context) {
    if let Some(action) = app.view.dialogs.unsaved.clone() {
        let mut choice = None;
        modal(ctx, "Unsaved changes", |ui| {
            ui.label(format!("Save the changes to {} first?", app.title().trim_end_matches(" *")));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(1);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(2);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
            });
        });
        match choice {
            Some(1) => {
                app.view.dialogs.unsaved = None;
                if save(app, false) {
                    perform(app, action);
                }
            }
            Some(2) => {
                app.view.dialogs.unsaved = None;
                perform(app, action);
            }
            Some(_) => app.view.dialogs.unsaved = None,
            None => {}
        }
    }

    if let Some(rename) = app.view.dialogs.rename.clone() {
        let (title, mut text) = match &rename {
            Rename::Pattern(name) => ("Rename pattern", name.clone()),
            Rename::Channel(_, name) => ("Rename channel", name.clone()),
            Rename::Insert(_, name) => ("Rename insert", name.clone()),
        };
        let mut done = None;
        modal(ctx, title, |ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(220.0));
            field.request_focus();
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if ui.button("Rename").clicked() || enter {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        });
        app.view.dialogs.rename = Some(match &rename {
            Rename::Pattern(_) => Rename::Pattern(text.clone()),
            Rename::Channel(id, _) => Rename::Channel(id.clone(), text.clone()),
            Rename::Insert(number, _) => Rename::Insert(*number, text.clone()),
        });
        match done {
            Some(true) => {
                app.view.dialogs.rename = None;
                let _ = match rename {
                    Rename::Pattern(_) => app.run("pattern.rename", json!({ "name": text })),
                    Rename::Channel(id, _) => app.run("channel.rename", json!({ "channel": id, "name": text })),
                    Rename::Insert(number, _) => app.run("mixer.rename", json!({ "insert": number, "name": text })),
                };
            }
            Some(false) => app.view.dialogs.rename = None,
            None => {}
        }
    }

    if app.view.dialogs.audio {
        audio_settings(app, ctx);
    }

    if !app.view.dialogs.recovery.is_empty() {
        let found = app.view.dialogs.recovery.clone();
        let mut choice = None;
        modal(ctx, "Recover your work", |ui| {
            ui.label("Bodo didn't close properly last time. These autosaves can bring your work back:");
            ui.add_space(6.0);
            for path in &found {
                ui.horizontal(|ui| {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    ui.label(name);
                    if ui.button("Recover").clicked() {
                        choice = Some(Some(path.clone()));
                    }
                });
            }
            ui.add_space(6.0);
            if ui.button("Don't recover").clicked() {
                choice = Some(None);
            }
        });
        if let Some(choice) = choice {
            app.view.dialogs.recovery.clear();
            if let Some(path) = choice {
                crate::autosave::recover(app, &path);
            }
        }
    }

    if app.view.dialogs.shortcuts {
        let mut open = true;
        egui::Window::new("Keyboard shortcuts").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("bodo-shortcuts").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                let mut rows: Vec<(String, String)> = app
                    .session
                    .registry()
                    .iter()
                    .filter_map(|c| c.shortcut.map(|s| (crate::shortcuts::display(ctx, s), c.title.to_owned())))
                    .collect();
                for kind in crate::windows::WindowKind::ALL {
                    rows.push((format!("{:?}", kind.key()), format!("Show or hide the {}", kind.title())));
                }
                rows.push((crate::shortcuts::display(ctx, "Mod+K"), String::from("Command palette")));
                rows.push((String::from("P / D / T"), String::from("Playlist tools: draw, delete, mute")));
                for (keys, what) in rows {
                    ui.label(RichText::new(keys).monospace());
                    ui.label(what);
                    ui.end_row();
                }
            });
        });
        app.view.dialogs.shortcuts = open;
    }

    if app.view.dialogs.about {
        let mut open = true;
        egui::Window::new("About Bodo").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            let tk = Tokens::get();
            ui.label(RichText::new("bodo").size(28.0).color(tk.accent).strong());
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("A free, open-source digital audio workstation written in Rust.");
            ui.label("Licensed under the GNU GPL, version 3 or later.");
            ui.add_space(6.0);
            ui.label(RichText::new("The Bodo Drums factory kit is CC0: use it in your music freely.").color(tk.text_dim));
        });
        app.view.dialogs.about = open;
    }
}

fn audio_settings(app: &mut BodoApp, ctx: &egui::Context) {
    let mut draft = app.view.dialogs.audio_draft.take().unwrap_or_else(|| app.prefs.audio.clone());
    let mut close = false;
    let mut apply = false;
    modal(ctx, "Audio settings", |ui| {
        let hosts = bodo_kit_device::hosts();
        egui::Grid::new("bodo-audio").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            ui.label("Driver");
            let host_label = draft.host.clone().unwrap_or_else(|| String::from("Default"));
            egui::ComboBox::from_id_salt("bodo-audio-host").selected_text(host_label).width(220.0).show_ui(ui, |ui| {
                if ui.selectable_label(draft.host.is_none(), "Default").clicked() {
                    draft.host = None;
                    draft.output = None;
                    draft.input = None;
                }
                for host in &hosts {
                    if ui.selectable_label(draft.host.as_deref() == Some(host.name.as_str()), &host.name).clicked() {
                        draft.host = Some(host.name.clone());
                        draft.output = None;
                        draft.input = None;
                    }
                }
            });
            ui.end_row();
            ui.label("Output");
            let devices = bodo_kit_device::devices(draft.host.as_deref()).unwrap_or_default();
            let output_label = draft.output.clone().unwrap_or_else(|| String::from("Default"));
            egui::ComboBox::from_id_salt("bodo-audio-output").selected_text(output_label).width(220.0).show_ui(ui, |ui| {
                if ui.selectable_label(draft.output.is_none(), "Default").clicked() {
                    draft.output = None;
                }
                for device in devices.iter().filter(|d| d.output_channels >= 1) {
                    if ui.selectable_label(draft.output.as_deref() == Some(device.name.as_str()), &device.name).clicked() {
                        draft.output = Some(device.name.clone());
                    }
                }
            });
            ui.end_row();
            ui.label("Input");
            let input_label = draft.input.clone().unwrap_or_else(|| String::from("Default"));
            egui::ComboBox::from_id_salt("bodo-audio-input").selected_text(input_label).width(220.0).show_ui(ui, |ui| {
                if ui.selectable_label(draft.input.is_none(), "Default").clicked() {
                    draft.input = None;
                }
                for device in devices.iter().filter(|d| d.input_channels >= 1) {
                    let label = format!("{} ({} in)", device.name, device.input_channels);
                    if ui.selectable_label(draft.input.as_deref() == Some(device.name.as_str()), label).clicked() {
                        draft.input = Some(device.name.clone());
                    }
                }
            });
            ui.end_row();
            ui.label("Buffer");
            let frames = draft.buffer_frames.unwrap_or(256);
            egui::ComboBox::from_id_salt("bodo-audio-buffer").selected_text(format!("{frames} frames")).width(220.0).show_ui(ui, |ui| {
                for size in [64u32, 128, 256, 512, 1024, 2048] {
                    let ms = f64::from(size) / f64::from(app.session.project().settings.sample_rate.hz()) * 1000.0;
                    if ui.selectable_label(frames == size, format!("{size} frames ({ms:.1} ms)")).clicked() {
                        draft.buffer_frames = Some(size);
                    }
                }
            });
            ui.end_row();
        });
        ui.label(RichText::new(app.audio.status()).color(Tokens::get().text_dim).size(size::SMALL));
        ui.add_space(6.0);
        midi_settings(app, ui, &mut draft);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                apply = true;
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    if apply {
        app.prefs.audio = draft.clone();
        app.audio.open(&app.session, &draft);
    }
    if close {
        app.view.dialogs.audio = false;
        app.view.dialogs.midi_ports = None;
    } else {
        app.view.dialogs.audio_draft = Some(draft);
    }
}

/// The MIDI part of the audio settings: which inputs open (applied with the device), and which channel each one plays
/// (a project edit, applied right away).
///
/// Complexity: O(p * c) in ports and channels, for the few ports a studio has.
fn midi_settings(app: &mut BodoApp, ui: &mut egui::Ui, draft: &mut crate::audio::AudioPrefs) {
    let tk = Tokens::get();
    ui.horizontal(|ui| {
        ui.label(RichText::new("MIDI inputs").strong());
        if ui.small_button("Refresh").clicked() {
            app.view.dialogs.midi_ports = None;
        }
    });
    let ports = app.view.dialogs.midi_ports.get_or_insert_with(bodo_kit_device::midi::input_ports).clone();
    if ports.is_empty() {
        ui.label(RichText::new("No MIDI inputs found. Connect a keyboard and press Refresh.").color(tk.text_dim).size(size::SMALL));
        return;
    }
    let project = std::sync::Arc::clone(app.session.project());
    let open: Vec<String> = app.audio.midi_inputs().into_iter().map(str::to_owned).collect();
    egui::Grid::new("bodo-midi-inputs").num_columns(3).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        for port in &ports {
            let mut on = draft.midi_inputs.as_ref().is_none_or(|names| names.contains(port));
            let suffix = if open.contains(port) { "" } else { " (closed)" };
            if ui.checkbox(&mut on, format!("{port}{suffix}")).changed() {
                let mut names = draft.midi_inputs.clone().unwrap_or_else(|| ports.clone());
                names.retain(|name| name != port);
                if on {
                    names.push(port.clone());
                }
                draft.midi_inputs = if ports.iter().all(|p| names.contains(p)) { None } else { Some(names) };
            }
            // The channels bound to this port; an unbound port plays the selected channel.
            let bound: Vec<&bodo_model::Channel> = project
                .rack_order
                .iter()
                .filter_map(|id| project.channels.get(id))
                .filter(|c| c.midi_in.as_ref().is_some_and(|b| &b.port == port))
                .map(|c| c.as_ref())
                .collect();
            let shown = if bound.is_empty() {
                String::from("Selected channel")
            } else {
                bound.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
            };
            egui::ComboBox::from_id_salt(("bodo-midi-plays", port)).selected_text(shown).width(160.0).show_ui(ui, |ui| {
                if ui.selectable_label(bound.is_empty(), "Selected channel").clicked() {
                    for channel in &bound {
                        let _ = app.run("channel.set_midi_in", json!({ "channel": channel.id, "port": null }));
                    }
                }
                for id in project.rack_order.iter() {
                    let Some(channel) = project.channels.get(id) else { continue };
                    let is_bound = bound.iter().any(|c| c.id == *id);
                    if ui.selectable_label(is_bound, &channel.name).clicked() {
                        let port = if is_bound { serde_json::Value::Null } else { json!(port) };
                        let _ = app.run("channel.set_midi_in", json!({ "channel": id, "port": port }));
                    }
                }
            });
            let listening = bound.first().and_then(|c| c.midi_in.as_ref()).and_then(|b| b.channel);
            ui.add_enabled_ui(!bound.is_empty(), |ui| {
                let text = listening.map_or_else(|| String::from("Any MIDI channel"), |n| format!("MIDI channel {}", n + 1));
                egui::ComboBox::from_id_salt(("bodo-midi-channel", port)).selected_text(text).width(130.0).show_ui(ui, |ui| {
                    let choices = std::iter::once(None).chain((1..=16u8).map(Some));
                    for choice in choices {
                        let label = choice.map_or_else(|| String::from("Any MIDI channel"), |n| format!("MIDI channel {n}"));
                        if ui.selectable_label(listening.map(|n| n + 1) == choice, label).clicked() {
                            for channel in &bound {
                                let _ = app.run("channel.set_midi_in", json!({ "channel": channel.id, "port": port, "midiChannel": choice }));
                            }
                        }
                    }
                });
            });
            ui.end_row();
        }
    });
    ui.label(
        RichText::new("Unticked inputs stay closed after Apply. A port bound to channels plays them; any other port plays the selected channel.")
            .color(tk.text_dim)
            .size(size::SMALL),
    );
}
