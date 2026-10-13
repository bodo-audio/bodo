//! Dialogs: unsaved changes, renaming, audio settings, crash recovery, shortcuts and about.

use std::path::PathBuf;

use egui::{Align2, RichText, vec2};
use serde_json::json;

use crate::BodoApp;
use crate::i18n::t;
use crate::theme::{Tokens, rem, size};

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
    /// The keyboard shortcuts dialog.
    pub shortcuts: bool,
    /// The shortcuts dialog's filter, the shortcut listening for new keys, and a conflict waiting for an answer.
    pub(crate) shortcut_editor: crate::shortcut_editor::EditorState,
    /// The layout name dialog: saving the windows as a layout, or renaming a saved one.
    pub(crate) layout_name: Option<LayoutName>,
    /// The about box.
    pub about: bool,
    /// The interface settings: theme, size, language and color-blind-safe colors.
    pub interface: bool,
    /// Autosaves found after a crash, offered for recovery.
    pub recovery: Vec<PathBuf>,
    /// The song position shows minutes and seconds instead of bars, steps and ticks.
    pub clock_seconds: bool,
    /// The export dialog, while it is open.
    pub export: Option<Export>,
    /// The export rendering on a background thread, if one is.
    pub exporting: Option<ExportTask>,
}

/// An export rendering on a background thread, so the interface keeps working meanwhile.
pub struct ExportTask {
    /// Where it writes: the mix file or the stems folder.
    pub place: String,
    /// How far it has come; it can also cancel it.
    pub progress: std::sync::Arc<bodo_mix::RenderProgress>,
    handle: Option<std::thread::JoinHandle<Result<serde_json::Value, String>>>,
}

impl std::fmt::Debug for ExportTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportTask").field("place", &self.place).field("fraction", &self.progress.fraction()).finish()
    }
}

impl ExportTask {
    /// Starts rendering `job` on its own thread.
    fn start(job: bodo_engine::ExportJob) -> Result<Self, String> {
        let place = job.place().display().to_string();
        let progress = std::sync::Arc::new(bodo_mix::RenderProgress::default());
        let shared = std::sync::Arc::clone(&progress);
        let handle = std::thread::Builder::new()
            .name(String::from("bodo-export"))
            .spawn(move || job.run(&shared).map_err(|e| e.to_string()))
            .map_err(|e| format!("the export could not start: {e}"))?;
        Ok(Self { place, progress, handle: Some(handle) })
    }

    /// The export's outcome once it has finished: what it wrote, or why it couldn't.
    fn finished(&mut self) -> Option<Result<serde_json::Value, String>> {
        if !self.handle.as_ref().is_some_and(std::thread::JoinHandle::is_finished) {
            return None;
        }
        let handle = self.handle.take()?;
        Some(handle.join().unwrap_or_else(|_| Err(String::from("the export stopped unexpectedly"))))
    }
}

/// Takes the background export's outcome when it finishes, into the export dialog's message and the status line.
pub(crate) fn poll_export(app: &mut BodoApp, ctx: &egui::Context) {
    let Some(task) = app.view.dialogs.exporting.as_mut() else { return };
    let Some(outcome) = task.finished() else {
        // Keep the progress bar moving while it renders.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    };
    app.view.dialogs.exporting = None;
    let (message, error) = match outcome {
        Ok(result) => {
            let place = result.get("path").or_else(|| result.get("folder")).and_then(serde_json::Value::as_str).unwrap_or_default();
            (format!("Wrote {place}"), false)
        }
        Err(error) => (error, true),
    };
    app.set_status(message.clone(), error);
    if let Some(dialog) = app.view.dialogs.export.as_mut() {
        dialog.message = Some((message, error));
    }
}

/// What the export dialog renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportWhat {
    /// The whole mix, through the master, as one file.
    #[default]
    Mix,
    /// One file per insert, in a folder.
    Stems,
    /// One file per insert, and the mix beside them.
    Both,
}

/// Which stretch of the song the export dialog renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportRange {
    /// From the start to the end of the song.
    #[default]
    Song,
    /// The Song mode loop.
    Loop,
    /// The playlist's time selection.
    Selection,
}

/// The export dialog: what to render, which stretch, in which format, and where.
#[derive(Clone, Debug, Default)]
pub struct Export {
    /// What to render.
    pub what: ExportWhat,
    /// Which stretch.
    pub range: ExportRange,
    /// The file format: `wav`, `flac` or `w64`.
    pub format: &'static str,
    /// The folder the files go in.
    pub folder: String,
    /// The mix file's name, or the stems folder's, without an extension.
    pub name: String,
    /// What the last export said: where it wrote, or why it couldn't.
    pub message: Option<(String, bool)>,
}

impl Export {
    /// The dialog for `what`, with the project's Rendered folder and title.
    pub(crate) fn new(app: &BodoApp, what: ExportWhat) -> Self {
        let folder = match (app.session.file(), app.user_library()) {
            (Some(file), _) => bodo_io::project_folder(file).join("Rendered"),
            (None, Some(library)) => library.rendered(),
            (None, None) => std::env::temp_dir().join("Bodo Rendered"),
        };
        let name = app.title().trim_end_matches(" *").to_owned();
        Self { what, range: ExportRange::Song, format: "wav", folder: folder.display().to_string(), name, message: None }
    }
}

/// The layout name dialog's purpose and what has been typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LayoutName {
    /// The saved layout being renamed, or `None` to save the windows as a new layout.
    pub(crate) renaming: Option<String>,
    /// The name typed so far.
    pub(crate) text: String,
    /// Why the last attempt didn't work.
    pub(crate) error: Option<String>,
}

impl Dialogs {
    /// Whether the shortcuts dialog is listening for new keys, which then belong to it alone.
    pub fn recording_shortcut(&self) -> bool {
        self.shortcuts && self.shortcut_editor.listening.is_some()
    }

    /// Opens the dialog that saves the windows as a new layout.
    pub fn save_layout(&mut self) {
        self.layout_name = Some(LayoutName { renaming: None, text: String::new(), error: None });
    }

    /// Opens the dialog that renames saved layout `name`.
    pub fn rename_layout(&mut self, name: &str) {
        self.layout_name = Some(LayoutName { renaming: Some(name.to_owned()), text: name.to_owned(), error: None });
    }
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
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -rem(3.2)))
        .order(egui::Order::Foreground)
        .show(ctx, add);
}

/// Draws whichever dialogs are open. Escape closes the one in front, as its Cancel or Close button would, unless a
/// menu or another popup is open, which Escape closes first.
pub(crate) fn show(app: &mut BodoApp, ctx: &egui::Context) {
    poll_export(app, ctx);
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !egui::Popup::is_any_open(ctx);
    if escape {
        let dialogs = &mut app.view.dialogs;
        if dialogs.unsaved.is_some() {
            dialogs.unsaved = None;
        } else if dialogs.about {
            dialogs.about = false;
        } else if dialogs.interface {
            dialogs.interface = false;
        } else if dialogs.export.is_some() {
            // The export keeps rendering in the background.
            dialogs.export = None;
        } else if dialogs.audio {
            dialogs.audio = false;
            dialogs.audio_draft = None;
            dialogs.midi_ports = None;
        } else if dialogs.shortcuts && !dialogs.recording_shortcut() && dialogs.shortcut_editor.conflict.is_none() {
            dialogs.shortcuts = false;
        }
    }
    if app.view.dialogs.interface {
        interface_settings(app, ctx);
    }
    if let Some(action) = app.view.dialogs.unsaved.clone() {
        let mut choice = None;
        modal(ctx, t("Unsaved changes"), |ui| {
            ui.label(format!("Save the changes to {} first?", app.title().trim_end_matches(" *")));
            ui.add_space(rem(0.64));
            ui.horizontal(|ui| {
                if ui.button(t("Save")).clicked() {
                    choice = Some(1);
                }
                if ui.button(t("Don't save")).clicked() {
                    choice = Some(2);
                }
                if ui.button(t("Cancel")).clicked() {
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
            let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(rem(17.6)));
            field.request_focus();
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if ui.button(t("Rename")).clicked() || enter {
                    done = Some(true);
                }
                if ui.button(t("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
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
        modal(ctx, t("Recover your work"), |ui| {
            ui.label(t("Bodo didn't close properly last time. These autosaves can bring your work back:"));
            ui.add_space(rem(0.48));
            for path in &found {
                ui.horizontal(|ui| {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    ui.label(name);
                    if ui.button(t("Recover")).clicked() {
                        choice = Some(Some(path.clone()));
                    }
                });
            }
            ui.add_space(rem(0.48));
            if ui.button(t("Don't recover")).clicked() {
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
        crate::shortcut_editor::show(app, ctx);
    }

    if let Some(mut dialog) = app.view.dialogs.layout_name.take() {
        let title = if dialog.renaming.is_some() { "Rename layout" } else { "Save layout" };
        let mut done = None;
        modal(ctx, title, |ui| {
            ui.label(if dialog.renaming.is_some() { "The layout's new name:" } else { "Save the windows as they are now, under the name:" });
            let field = ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(rem(19.2)).hint_text(t("Layout name")));
            field.request_focus();
            crate::control::record(ui, &field, "Layout name", "text", None);
            if let Some(error) = &dialog.error {
                ui.label(RichText::new(error).color(Tokens::get().error).size(size::SMALL));
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let label = if dialog.renaming.is_some() { "Rename" } else { "Save" };
                if ui.button(label).clicked() || enter {
                    done = Some(true);
                }
                if ui.button(t("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                let result = match &dialog.renaming {
                    Some(old) => app.run("ui.rename_layout", json!({ "layout": old, "name": dialog.text })),
                    None => app.run("ui.save_layout", json!({ "name": dialog.text })),
                };
                if let Err(error) = result {
                    dialog.error = Some(error);
                    app.view.dialogs.layout_name = Some(dialog);
                }
            }
            Some(false) => {}
            None => app.view.dialogs.layout_name = Some(dialog),
        }
    }

    if app.view.dialogs.about {
        let mut open = true;
        egui::Window::new(t("About Bodo")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            let tk = Tokens::get();
            ui.label(RichText::new("bodo").size(rem(2.24)).color(tk.accent).strong());
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label(t("A free, open-source digital audio workstation written in Rust."));
            ui.label(t("Licensed under the GNU GPL, version 3 or later."));
            ui.add_space(rem(0.48));
            ui.label(RichText::new(t("The Bodo Drums factory kit is CC0: use it in your music freely.")).color(tk.text_dim));
        });
        app.view.dialogs.about = open;
    }
    export(app, ctx);
}

/// The export dialog: the mix, the stems or both, over the song, the loop or the time selection, as WAV, FLAC or
/// Wave64, into a folder.
fn export(app: &mut BodoApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.view.dialogs.export.take() else { return };
    let tk = Tokens::get();
    let transport = *app.session.transport();
    let selection = app.view.playlist.time_selection;
    let mut open = true;
    let mut run = false;
    let center = ctx.content_rect().center();
    let window = egui::Window::new("Export").id(egui::Id::new("bodo-export")).pivot(Align2::CENTER_CENTER).default_pos(center);
    window.open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.set_min_width(340.0);
        egui::Grid::new("bodo-export-grid").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            ui.label("Render");
            ui.horizontal(|ui| {
                for (value, label) in [(ExportWhat::Mix, "The mix"), (ExportWhat::Stems, "Stems"), (ExportWhat::Both, "Stems and the mix")] {
                    let choice = ui.radio(dialog.what == value, label);
                    crate::control::record(ui, &choice, &format!("Export {label}"), "radio", Some(dialog.what == value));
                    if choice.clicked() {
                        dialog.what = value;
                    }
                }
            });
            ui.end_row();
            ui.label("Range");
            ui.horizontal(|ui| {
                for (value, label, available) in [
                    (ExportRange::Song, "The song", true),
                    (ExportRange::Loop, "The loop", transport.loop_region.is_some()),
                    (ExportRange::Selection, "The selection", selection.is_some()),
                ] {
                    let choice = ui.add_enabled(available, egui::RadioButton::new(dialog.range == value, label));
                    crate::control::record(ui, &choice, &format!("Export range {label}"), "radio", Some(dialog.range == value));
                    if choice.clicked() {
                        dialog.range = value;
                    }
                }
            });
            ui.end_row();
            ui.label("Format");
            ui.horizontal(|ui| {
                for (value, label) in [("wav", "WAV"), ("flac", "FLAC"), ("w64", "Wave64")] {
                    let choice = ui.radio(dialog.format == value, label);
                    crate::control::record(ui, &choice, &format!("Export format {label}"), "radio", Some(dialog.format == value));
                    if choice.clicked() {
                        dialog.format = value;
                    }
                }
            });
            ui.end_row();
            ui.label("Folder");
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut dialog.folder).desired_width(220.0));
                crate::control::record(ui, &field, "Export folder", "text", None);
                if app.services.pick_folder.is_some()
                    && ui.button("Choose...").clicked()
                    && let Some(folder) = app.services.pick_folder.as_ref().and_then(|pick| pick())
                {
                    dialog.folder = folder.display().to_string();
                }
            });
            ui.end_row();
            ui.label(if dialog.what == ExportWhat::Mix { "File name" } else { "Stems folder" });
            let field = ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(220.0));
            crate::control::record(ui, &field, "Export name", "text", None);
            ui.end_row();
        });
        let running = app.view.dialogs.exporting.as_ref().map(|task| (task.place.clone(), std::sync::Arc::clone(&task.progress)));
        if let Some((place, progress)) = &running {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Exporting to {place}")).size(size::SMALL).color(tk.text_dim));
            let bar = ui.add(egui::ProgressBar::new(progress.fraction()).show_percentage());
            crate::control::record(ui, &bar, "Export progress", "progress", None);
        } else if let Some((message, error)) = &dialog.message {
            ui.add_space(4.0);
            ui.label(RichText::new(message).size(size::SMALL).color(if *error { tk.error } else { tk.text_dim }));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ready = running.is_none() && !dialog.name.trim().is_empty() && !dialog.folder.trim().is_empty();
            let button = ui.add_enabled(ready, egui::Button::new("Export"));
            crate::control::record(ui, &button, "Export run", "button", None);
            if button.clicked() {
                run = true;
            }
            if let Some((_, progress)) = &running {
                let cancel = ui.button("Cancel export");
                crate::control::record(ui, &cancel, "Export cancel", "button", None);
                app.hint(&cancel, "Stops the export; the files written so far stay");
                if cancel.clicked() {
                    progress.cancel();
                }
            }
        });
        if running.is_some() {
            ui.label(RichText::new("You can close this dialog and keep working while it renders.").size(size::SMALL).color(tk.text_faint));
        }
    });
    if run {
        let range = match dialog.range {
            ExportRange::Song => None,
            ExportRange::Loop => transport.loop_region.map(|(s, e)| (s.0, e.0)),
            ExportRange::Selection => selection,
        };
        let folder = PathBuf::from(dialog.folder.trim());
        let name = dialog.name.trim().to_owned();
        let mut params = match dialog.what {
            ExportWhat::Mix => json!({ "path": folder.join(format!("{name}.{}", dialog.format)).display().to_string(), "format": dialog.format }),
            ExportWhat::Stems | ExportWhat::Both => json!({
                "folder": folder.join(format!("{name} stems")).display().to_string(),
                "format": dialog.format,
                "mix": dialog.what == ExportWhat::Both,
            }),
        };
        if let (Some((start, end)), Some(map)) = (range, params.as_object_mut()) {
            map.insert(String::from("start"), json!(start));
            map.insert(String::from("end"), json!(end));
        }
        let id = if dialog.what == ExportWhat::Mix { "render.export_mix" } else { "render.export_stems" };
        // The render runs on its own thread; the dialog shows its progress and the outcome when it finishes.
        match app.session.prepare_export(id, params).map_err(|e| e.to_string()).and_then(ExportTask::start) {
            Ok(task) => {
                dialog.message = None;
                app.view.dialogs.exporting = Some(task);
            }
            Err(error) => dialog.message = Some((error, true)),
        }
    }
    if open {
        app.view.dialogs.export = Some(dialog);
    }
}

/// The interface settings (project.md section 14.7): the theme, the interface's size, the language, and the
/// color-blind-safe colors. Every change applies at once and is kept between runs.
fn interface_settings(app: &mut BodoApp, ctx: &egui::Context) {
    use crate::i18n::LanguagePref;
    let mut open = true;
    egui::Window::new(RichText::new(t("Interface settings")).size(size::HEADING))
        .id(egui::Id::new("bodo-interface-settings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -rem(3.2)))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Grid::new("bodo-interface-grid").num_columns(2).spacing(vec2(rem(0.96), rem(0.64))).show(ui, |ui| {
                ui.label(t("Theme"));
                ui.horizontal(|ui| {
                    for theme in crate::theme::ThemePref::ALL {
                        let on = app.prefs.theme == theme;
                        let response = ui.radio(on, crate::i18n::t(theme.label()));
                        crate::control::record(ui, &response, &format!("Theme {}", theme.label()), "radio", Some(on));
                        if response.clicked() {
                            app.prefs.theme = theme;
                        }
                    }
                });
                ui.end_row();

                ui.label(t("Interface size"));
                ui.horizontal(|ui| {
                    let mut auto = app.prefs.ui_scale.is_none();
                    let check = ui.checkbox(&mut auto, t("Auto"));
                    crate::control::record(ui, &check, "Interface size auto", "checkbox", Some(auto));
                    app.hint(&check, "Auto sizes the interface from the screen's width, as a web page does");
                    if check.changed() {
                        app.prefs.ui_scale = if auto { None } else { Some(app.ui_scale()) };
                    }
                    let mut percent = (app.ui_scale() * 100.0).round();
                    let slider = ui.add(egui::Slider::new(&mut percent, 75.0..=200.0).step_by(5.0).suffix("%"));
                    crate::control::record(ui, &slider, "Interface size", "slider", None);
                    if slider.changed() {
                        app.prefs.ui_scale = Some(percent / 100.0);
                    }
                });
                ui.end_row();

                ui.label(t("Language"));
                let current = app.prefs.language;
                let combo = egui::ComboBox::from_id_salt("bodo-language").selected_text(current.label()).show_ui(ui, |ui| {
                    for language in LanguagePref::ALL {
                        if ui.selectable_label(current == language, language.label()).clicked() {
                            let _ = app.run("ui.set_language", json!({ "language": language.id() }));
                        }
                    }
                });
                crate::control::record(ui, &combo.response, "Language", "combo", None);
                ui.end_row();

                ui.label(t("Colors"));
                let mut safe = app.prefs.color_safe;
                let check = ui.checkbox(&mut safe, t("Color-blind-safe meters and clip colors"));
                crate::control::record(ui, &check, "Color-blind-safe colors", "checkbox", Some(safe));
                if check.changed() {
                    app.prefs.color_safe = safe;
                }
                ui.end_row();
            });
            ui.add_space(rem(0.48));
            ui.label(
                RichText::new(t("The interface keys: Cmd or Ctrl with + and - change its size, and 0 goes back to Auto."))
                    .color(Tokens::get().text_dim)
                    .size(size::SMALL),
            );
        });
    if !open {
        app.view.dialogs.interface = false;
    }
}

fn audio_settings(app: &mut BodoApp, ctx: &egui::Context) {
    let mut draft = app.view.dialogs.audio_draft.take().unwrap_or_else(|| app.prefs.audio.clone());
    let mut close = false;
    let mut apply = false;
    modal(ctx, t("Audio settings"), |ui| {
        let hosts = bodo_kit_device::hosts();
        egui::Grid::new("bodo-audio").num_columns(2).spacing(vec2(rem(0.96), rem(0.48))).show(ui, |ui| {
            ui.label(t("Driver"));
            let host_label = draft.host.clone().unwrap_or_else(|| String::from("Default"));
            egui::ComboBox::from_id_salt("bodo-audio-host").selected_text(host_label).width(rem(17.6)).show_ui(ui, |ui| {
                if ui.selectable_label(draft.host.is_none(), t("Default")).clicked() {
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
            ui.label(t("Output"));
            let devices = bodo_kit_device::devices(draft.host.as_deref()).unwrap_or_default();
            let output_label = draft.output.clone().unwrap_or_else(|| String::from("Default"));
            egui::ComboBox::from_id_salt("bodo-audio-output").selected_text(output_label).width(rem(17.6)).show_ui(ui, |ui| {
                if ui.selectable_label(draft.output.is_none(), t("Default")).clicked() {
                    draft.output = None;
                }
                for device in devices.iter().filter(|d| d.output_channels >= 1) {
                    if ui.selectable_label(draft.output.as_deref() == Some(device.name.as_str()), &device.name).clicked() {
                        draft.output = Some(device.name.clone());
                    }
                }
            });
            ui.end_row();
            ui.label(t("Input"));
            let input_label = draft.input.clone().unwrap_or_else(|| String::from("Default"));
            egui::ComboBox::from_id_salt("bodo-audio-input").selected_text(input_label).width(rem(17.6)).show_ui(ui, |ui| {
                if ui.selectable_label(draft.input.is_none(), t("Default")).clicked() {
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
            ui.label(t("Buffer"));
            let frames = draft.buffer_frames.unwrap_or(256);
            egui::ComboBox::from_id_salt("bodo-audio-buffer").selected_text(format!("{frames} frames")).width(rem(17.6)).show_ui(ui, |ui| {
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
        ui.add_space(rem(0.48));
        midi_settings(app, ui, &mut draft);
        ui.add_space(rem(0.48));
        ui.horizontal(|ui| {
            if ui.button(t("Apply")).clicked() {
                apply = true;
            }
            if ui.button(t("Close")).clicked() {
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
        ui.label(RichText::new(t("MIDI inputs")).strong());
        if ui.small_button(t("Refresh")).clicked() {
            app.view.dialogs.midi_ports = None;
        }
    });
    let ports = app.view.dialogs.midi_ports.get_or_insert_with(bodo_kit_device::midi::input_ports).clone();
    if ports.is_empty() {
        ui.label(RichText::new(t("No MIDI inputs found. Connect a keyboard and press Refresh.")).color(tk.text_dim).size(size::SMALL));
        return;
    }
    let project = std::sync::Arc::clone(app.session.project());
    let open: Vec<String> = app.audio.midi_inputs().into_iter().map(str::to_owned).collect();
    egui::Grid::new("bodo-midi-inputs").num_columns(3).spacing(vec2(rem(0.96), rem(0.48))).show(ui, |ui| {
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
            egui::ComboBox::from_id_salt(("bodo-midi-plays", port)).selected_text(shown).width(rem(12.8)).show_ui(ui, |ui| {
                if ui.selectable_label(bound.is_empty(), t("Selected channel")).clicked() {
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
                egui::ComboBox::from_id_salt(("bodo-midi-channel", port)).selected_text(text).width(rem(10.4)).show_ui(ui, |ui| {
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
        RichText::new(t("Unticked inputs stay closed after Apply. A port bound to channels plays them; any other port plays the selected channel."))
            .color(tk.text_dim)
            .size(size::SMALL),
    );
}
