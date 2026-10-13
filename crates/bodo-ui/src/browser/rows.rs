//! The Browser's rows and what using them does.
//!
//! - A **sound** previews on a click, becomes a channel on a double-click, and can be dragged onto the channel rack or
//!   a playlist track.
//! - A **project** opens on a double-click; one in Templates or Demo projects starts a new project from it, and an
//!   autosave in Backup opens as an unsaved project.
//! - A **channel preset**, **mixer preset** or **score** loads on a double-click (a new channel, the selected insert,
//!   the piano roll's channel), or where it is dropped: on a rack channel, a mixer strip, or a window.
//! - A **MIDI file** imports as a new pattern.
//! - A **folder** opens and closes on a click, and a double-click enters it, so back and up can move around.
//!
//! Every file's context menu adds and removes the user's tags.

use std::path::{Path, PathBuf};

use bodo_engine::library::{EntryKind, UserLibrary};
use bodo_kit_ui::Icon;
use bodo_kit_ui::icons;
use egui::{RichText, Sense, Ui, vec2};
use serde_json::json;

use super::{FilePayload, SoundPayload};
use crate::theme::{Tokens, size};
use crate::{BodoApp, control, dialogs};

/// How far each level of a tree is indented.
pub(super) const INDENT: f32 = 12.0;

/// The icon of an entry of kind `kind`.
pub(super) fn icon_of(kind: EntryKind) -> Icon {
    match kind {
        EntryKind::Folder => icons::FOLDER,
        EntryKind::Sound => icons::WAVE_SINE,
        EntryKind::Project => icons::FILE_MUSIC,
        EntryKind::ChannelPreset => icons::CHANNEL_RACK,
        EntryKind::MixerPreset => icons::ADJUSTMENTS_HORIZONTAL,
        EntryKind::Score | EntryKind::Midi => icons::PIANO,
        EntryKind::PluginPreset | EntryKind::Plugin => icons::SPARKLES,
    }
}

/// How an element is labelled for agents and tests: "Sound Kick", "Channel preset Deep Kick".
pub(super) fn label_of(kind: EntryKind, name: &str) -> String {
    let noun = kind.noun();
    let mut first = noun.chars();
    let noun = first.next().map(|c| c.to_uppercase().chain(first).collect::<String>()).unwrap_or_default();
    format!("{noun} {name}")
}

/// A row: an icon and a name, indented by `depth`, highlighted when hovered or `lit`. Returns its response.
pub(super) fn row(ui: &mut Ui, depth: usize, icon: Icon, name: &str, lit: bool) -> egui::Response {
    let tk = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW - 4.0), Sense::click_and_drag());
    if response.hovered() || lit {
        ui.painter().rect_filled(rect, 4.0, if lit { tk.accent_soft } else { tk.control_hover });
    }
    let icon_rect = egui::Rect::from_min_size(rect.left_center() + vec2(depth as f32 * INDENT + 14.0, -6.0), vec2(12.0, 12.0));
    icon.paint(ui.painter(), icon_rect, if lit { tk.accent } else { tk.text_faint });
    ui.painter().with_clip_rect(rect).text(
        icon_rect.right_center() + vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(size::BODY),
        if lit { tk.accent } else { tk.text },
    );
    response
}

/// Shows `name` beside the pointer while `response` is dragged.
fn drag_label(ui: &Ui, response: &egui::Response, name: &str) {
    if response.dragged()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        let tk = Tokens::get();
        egui::Area::new(ui.id().with("browser-drag")).order(egui::Order::Tooltip).fixed_pos(pointer + vec2(12.0, 4.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(RichText::new(name).color(tk.text)));
        });
    }
}

/// The path to store for a sound: `factory:...` inside the factory content, absolute otherwise.
pub(super) fn sound_path(app: &BodoApp, file: &Path) -> String {
    bodo_engine::samples::stored_path(file, None, app.session.env().factory.as_deref())
}

/// One sound: a click previews it (and stops the preview on a second click), a double-click adds it as a channel,
/// and it can be dragged onto the channel rack or a playlist track.
pub(super) fn sound(app: &mut BodoApp, ui: &mut Ui, name: &str, file: &Path, stored: String, depth: usize) {
    let previewing = app.audio.previewing() == Some(file);
    let response = row(ui, depth, if previewing { icons::VOLUME } else { icons::WAVE_SINE }, name, previewing);
    response.dnd_set_drag_payload(SoundPayload(stored.clone()));
    drag_label(ui, &response, name);
    let response = response.on_hover_text(&stored);
    control::record(ui, &response, &format!("Sound {name}"), "sound", Some(previewing));
    app.hint(&response, format!("{name}: click to hear it, double-click to add a channel, or drag it onto the channel rack or a playlist track"));
    if response.double_clicked() {
        app.audio.stop_preview();
        let _ = app.run("channel.add", json!({ "sample": stored }));
    } else if response.clicked() {
        if previewing {
            app.audio.stop_preview();
        } else if !app.audio.preview(file) {
            app.set_status("There is no sound to preview through", false);
        }
    }
    tag_menu(app, &response, file);
}

/// Which folder of the library `file` is in, among those that change what opening a project does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectPlace {
    Template,
    Backup,
    Elsewhere,
}

fn project_place(library: Option<&UserLibrary>, file: &Path) -> ProjectPlace {
    let Some(library) = library else { return ProjectPlace::Elsewhere };
    let parent = file.parent();
    if parent == Some(library.templates().as_path()) || parent == Some(library.demos().as_path()) {
        ProjectPlace::Template
    } else if parent == Some(library.backup().as_path()) {
        ProjectPlace::Backup
    } else {
        ProjectPlace::Elsewhere
    }
}

/// One file of any kind but a sound or a folder.
pub(super) fn file(app: &mut BodoApp, ui: &mut Ui, kind: EntryKind, name: &str, path: &Path, depth: usize) {
    let response = row(ui, depth, icon_of(kind), name, false);
    let text = path.display().to_string();
    if matches!(kind, EntryKind::ChannelPreset | EntryKind::MixerPreset | EntryKind::Score) {
        response.dnd_set_drag_payload(FilePayload { path: path.to_path_buf(), kind });
        drag_label(ui, &response, name);
    }
    let response = response.on_hover_text(&text);
    control::record(ui, &response, &label_of(kind, name), "file", None);
    let library = app.user_library();
    let place = project_place(library.as_ref(), path);
    let hint = match (kind, place) {
        (EntryKind::Project, ProjectPlace::Template) => format!("{name}: double-click to start a new project from it"),
        (EntryKind::Project, ProjectPlace::Backup) => format!("{name}: an autosave; double-click to open it as an unsaved project"),
        (EntryKind::Project, ProjectPlace::Elsewhere) => format!("{name}: double-click to open it"),
        (EntryKind::ChannelPreset, _) => format!("{name}: double-click to add it as a channel, or drop it on a channel to replace that channel"),
        (EntryKind::MixerPreset, _) => format!("{name}: double-click to load it on the selected insert, or drop it on a mixer strip"),
        (EntryKind::Score, _) => format!("{name}: double-click to add its notes to the piano roll's channel, or drop it on a channel"),
        (EntryKind::Midi, _) => format!("{name}: double-click to import it as a new pattern"),
        (EntryKind::PluginPreset, _) => format!("{name}: a plugin preset; plugins load them once plugin hosting arrives"),
        (EntryKind::Plugin, _) => format!("{name}: a plugin; Bodo loads it once plugin hosting arrives"),
        (EntryKind::Folder | EntryKind::Sound, _) => name.to_owned(),
    };
    app.hint(&response, hint);
    if response.double_clicked() {
        open(app, kind, path, place);
    }
    tag_menu(app, &response, path);
}

/// What a double-click on a file does.
fn open(app: &mut BodoApp, kind: EntryKind, path: &Path, place: ProjectPlace) {
    let text = path.display().to_string();
    match kind {
        EntryKind::Project => match place {
            ProjectPlace::Template => dialogs::guard_unsaved(app, dialogs::Pending::NewFrom(text)),
            ProjectPlace::Backup => dialogs::guard_unsaved(app, dialogs::Pending::Recover(path.to_path_buf())),
            ProjectPlace::Elsewhere => dialogs::guard_unsaved(app, dialogs::Pending::Open(Some(path.to_path_buf()))),
        },
        EntryKind::ChannelPreset => {
            let _ = app.run("preset.load_channel", json!({ "path": text }));
        }
        EntryKind::MixerPreset => {
            let _ = app.run("preset.load_mixer", json!({ "path": text }));
        }
        EntryKind::Score => {
            let _ = app.run("preset.load_score", json!({ "path": text }));
        }
        EntryKind::Midi => {
            let _ = app.run("project.import_midi", json!({ "path": text }));
        }
        EntryKind::PluginPreset | EntryKind::Plugin => app.set_status("Plugins arrive with plugin hosting", false),
        EntryKind::Folder | EntryKind::Sound => {}
    }
}

/// The context menu of a file: its tags, to add and remove.
fn tag_menu(app: &mut BodoApp, response: &egui::Response, path: &Path) {
    if app.user_library().is_none() {
        return;
    }
    let key = bodo_engine::library::tags::file_key(app.session.env(), path);
    response.context_menu(|ui| {
        let store = app.user_library().map(|l| bodo_engine::library::tags::TagStore::load(&l)).unwrap_or_default();
        let tags: Vec<String> = store.of(&key).map(str::to_owned).collect();
        for tag in tags {
            if ui.button(format!("Remove tag {tag}")).clicked() {
                if app.run("library.tag", json!({ "path": key, "remove": [tag] })).is_ok() {
                    let library = app.user_library();
                    app.view.browser.retag(library.as_ref());
                }
                ui.close();
            }
        }
        if ui.button("Add tag...").clicked() {
            app.view.browser.tagging = Some((key.clone(), String::new()));
            ui.close();
        }
    });
}

/// A folder row: a click opens or closes it, a double-click enters it. `badge` marks it, such as "new".
pub(super) fn folder_row(app: &mut BodoApp, ui: &mut Ui, name: &str, key: &Path, depth: usize, icon: Icon, badge: Option<&str>) -> egui::Response {
    let tk = Tokens::get();
    let open = app.view.browser.is_open(key);
    let response = ui
        .horizontal(|ui| {
            ui.add_space(depth as f32 * INDENT);
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            (if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).paint(ui.painter(), rect, tk.text_faint);
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            let icon = if icon == icons::FOLDER && open { icons::FOLDER_OPEN } else { icon };
            icon.paint(ui.painter(), rect, tk.text_dim);
            // Truncated, so a long folder name never widens the panel; the badge keeps room after it.
            let room = (ui.available_width() - if badge.is_some() { 32.0 } else { 0.0 }).max(20.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(RichText::new(name).color(tk.text).size(size::BODY)).truncate());
            });
            if let Some(badge) = badge {
                ui.label(RichText::new(badge).color(tk.accent).size(size::SMALL).strong());
            }
            ui.allocate_space(vec2(ui.available_width(), 0.0));
        })
        .response;
    let response = ui.interact(response.rect, ui.id().with(("folder", key)), Sense::click());
    let label = match badge {
        Some(badge) => format!("Folder {name} {badge}"),
        None => format!("Folder {name}"),
    };
    control::record(ui, &response, &label, "folder", Some(open));
    if response.double_clicked() {
        app.view.browser.enter(key.to_path_buf());
    } else if response.clicked() {
        app.view.browser.toggle(key);
    }
    response
}

/// The rows of `entries`: folders as folder rows, then their files.
pub(super) fn entries(app: &mut BodoApp, ui: &mut Ui, dir: &Path, depth: usize) {
    let listed = app.view.browser.listing(dir).to_vec();
    if listed.is_empty() {
        empty(ui, depth, "Nothing here yet");
    }
    for entry in listed {
        let name = entry.stem();
        match entry.kind {
            EntryKind::Folder => {
                folder_row(app, ui, &entry.name, &entry.path, depth, icons::FOLDER, None);
                if app.view.browser.is_open(&entry.path) {
                    entries(app, ui, &entry.path, depth + 1);
                }
            }
            EntryKind::Sound => {
                let stored = sound_path(app, &entry.path);
                sound(app, ui, &name, &entry.path, stored, depth);
            }
            kind => file(app, ui, kind, &name, &entry.path, depth),
        }
    }
}

/// A faint line saying a list is empty.
pub(super) fn empty(ui: &mut Ui, depth: usize, text: &str) {
    let tk = Tokens::get();
    // Wrapping, so a long message never widens the panel.
    ui.horizontal_top(|ui| {
        ui.add_space(depth as f32 * INDENT + 26.0);
        ui.add(egui::Label::new(RichText::new(text).color(tk.text_faint).size(size::SMALL)).wrap());
    });
}

/// The path of `entry` in a folder the user picked, for the "Add tag" dialog's title.
pub(super) fn tagging_title(key: &str) -> String {
    let name = key.rsplit(['/', '\\', ':']).next().unwrap_or(key);
    let name = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    format!("Tag {name}")
}

/// The library folder that holds `path`, as a path, for the Browser's breadcrumb.
pub(super) fn relative_name(path: &Path, roots: &[(PathBuf, String)]) -> String {
    for (root, name) in roots {
        if let Ok(rest) = path.strip_prefix(root) {
            let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            return std::iter::once(name.clone()).chain(parts).collect::<Vec<_>>().join(" > ");
        }
    }
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}
