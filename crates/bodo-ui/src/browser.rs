//! The Browser, the left side panel (project.md section 14.1): the sounds of the current project, the factory packs,
//! the user's sample folders and recent projects, with search at the bottom. Drag a sound onto the channel rack to
//! make a channel of it, or double-click it; drag it onto a playlist track to place it there as an audio clip.
//!
//! Folder listings are read once when a folder opens and cached, so drawing never touches the disk.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use bodo_engine::library::{self, Entry};
use bodo_kit_ui::icons;
use egui::{RichText, Sense, Ui, vec2};
use serde_json::json;

use crate::theme::{Tokens, size};
use crate::windows::icon_button;
use crate::{BodoApp, control, dialogs};

/// A sound being dragged: the path to give `channel.add` or `clip.place_sound`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundPayload(pub String);

/// What the browser shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Everything.
    #[default]
    All,
    /// The current project's sounds.
    Project,
    /// Packs and sample folders.
    Sounds,
}

/// The browser's tab, open folders, cached listings and search.
#[derive(Debug, Default)]
pub struct BrowserView {
    /// The tab.
    pub tab: Tab,
    open: BTreeSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Entry>>,
    /// The search text.
    pub search: String,
    results: Option<(String, Vec<library::Sound>)>,
}

impl BrowserView {
    fn listing(&mut self, dir: &Path) -> &[Entry] {
        self.listings.entry(dir.to_path_buf()).or_insert_with(|| library::list(dir))
    }

    /// Forgets cached listings and results, so folders are read again.
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.results = None;
    }
}

/// The path to store for a file in the browser: `factory:...` inside the factory content, absolute otherwise.
fn sound_path(app: &BodoApp, file: &Path) -> String {
    bodo_engine::samples::stored_path(file, None, app.session.env().factory.as_deref())
}

/// One sound row: drag it to the channel rack or the playlist, double-click to add it as a channel.
fn sound_row(app: &mut BodoApp, ui: &mut Ui, name: &str, path: String, depth: usize) {
    let tk = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW - 4.0), Sense::click_and_drag());
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, tk.control_hover);
    }
    let icon = egui::Rect::from_min_size(rect.left_center() + vec2(depth as f32 * 12.0 + 14.0, -6.0), vec2(12.0, 12.0));
    icons::WAVE_SINE.paint(ui.painter(), icon, tk.text_faint);
    ui.painter().with_clip_rect(rect).text(
        icon.right_center() + vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(size::BODY),
        tk.text,
    );
    response.dnd_set_drag_payload(SoundPayload(path.clone()));
    if response.dragged()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        egui::Area::new(ui.id().with("dragged-sound")).order(egui::Order::Tooltip).fixed_pos(pointer + vec2(12.0, 4.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(RichText::new(name).color(tk.text)));
        });
    }
    let response = response.on_hover_text(&path);
    control::record(ui, &response, &format!("Sound {name}"), "sound", None);
    app.hint(&response, format!("{name}: drag it onto the channel rack or a playlist track, or double-click to add a channel"));
    if response.double_clicked() {
        let _ = app.run("channel.add", json!({ "sample": path }));
    }
}

/// A folder row that opens and closes, and its contents when open.
fn folder(app: &mut BodoApp, ui: &mut Ui, name: &str, dir: &Path, depth: usize) {
    let tk = Tokens::get();
    let open = app.view.browser.open.contains(dir);
    let response = ui
        .horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0);
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            (if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }).paint(ui.painter(), rect, tk.text_faint);
            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            (if open { icons::FOLDER_OPEN } else { icons::FOLDER }).paint(ui.painter(), rect, tk.text_dim);
            ui.label(RichText::new(name).color(tk.text).size(size::BODY))
        })
        .response;
    let response = ui.interact(response.rect, ui.id().with(("folder", dir)), Sense::click());
    control::record(ui, &response, &format!("Folder {name}"), "folder", Some(open));
    if response.clicked() {
        if open {
            app.view.browser.open.remove(dir);
        } else {
            app.view.browser.open.insert(dir.to_path_buf());
        }
    }
    if !open {
        return;
    }
    let entries: Vec<Entry> = app.view.browser.listing(dir).to_vec();
    if entries.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0 + 26.0);
            ui.label(RichText::new("No sounds").color(tk.text_faint).size(size::SMALL));
        });
    }
    for entry in entries {
        if entry.is_dir() {
            folder(app, ui, &entry.name, &entry.path, depth + 1);
        } else {
            let name = entry.path.file_stem().map_or_else(|| entry.name.clone(), |s| s.to_string_lossy().into_owned());
            let path = sound_path(app, &entry.path);
            sound_row(app, ui, &name, path, depth + 1);
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    let tk = Tokens::get();
    ui.add_space(6.0);
    ui.label(RichText::new(text).color(tk.text_faint).size(size::SMALL).strong());
}

/// Draws the browser.
pub(crate) fn show(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new("Browser").color(tk.text).size(size::HEADING));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            if icon_button(ui, icons::HISTORY, "Refresh the browser", false).clicked() {
                app.view.browser.refresh();
            }
            if icon_button(ui, icons::PLUS, "Add a sample folder", false).clicked() {
                dialogs::add_sample_folder(app);
                app.view.browser.refresh();
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        for (tab, label) in [(Tab::All, "All"), (Tab::Project, "Project"), (Tab::Sounds, "Sounds")] {
            let selected = app.view.browser.tab == tab;
            let response = ui.selectable_label(selected, RichText::new(label).size(size::SMALL));
            control::record(ui, &response, &format!("Browser tab {label}"), "tab", Some(selected));
            if response.clicked() {
                app.view.browser.tab = tab;
            }
        }
    });
    ui.separator();
    let search_height = 30.0;
    let list_height = (ui.available_height() - search_height).max(40.0);
    egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let tab = app.view.browser.tab;
        let query = app.view.browser.search.trim().to_owned();
        if !query.is_empty() {
            heading(ui, "Search results");
            let stale = app.view.browser.results.as_ref().is_none_or(|(q, _)| *q != query);
            if stale {
                let found = library::search(app.session.env(), &query, 200);
                app.view.browser.results = Some((query.clone(), found));
            }
            let found = app.view.browser.results.as_ref().map(|(_, f)| f.clone()).unwrap_or_default();
            if found.is_empty() {
                ui.label(RichText::new("Nothing found").color(tk.text_faint));
            }
            for sound in found {
                sound_row(app, ui, &sound.name, sound.path, 0);
            }
            return;
        }
        if tab != Tab::Sounds {
            heading(ui, "Current project");
            let project = std::sync::Arc::clone(app.session.project());
            if project.samples.is_empty() {
                ui.label(RichText::new("No sounds yet").color(tk.text_faint).size(size::SMALL));
            }
            for sample in project.samples.values() {
                sound_row(app, ui, &bodo_engine::samples::display_name(&sample.path), sample.path.clone(), 0);
            }
        }
        if tab != Tab::Project {
            heading(ui, "Packs");
            match app.session.env().factory.clone() {
                Some(factory) => {
                    let packs = factory.join("Packs");
                    let entries: Vec<Entry> = app.view.browser.listing(&packs).to_vec();
                    for entry in entries.into_iter().filter(|e| e.is_dir()) {
                        folder(app, ui, &entry.name, &entry.path, 0);
                    }
                }
                None => {
                    ui.label(RichText::new("The factory content wasn't found").color(tk.warn).size(size::SMALL));
                }
            }
            heading(ui, "Your folders");
            let library = app.prefs.library.clone();
            if library.is_empty() {
                ui.label(RichText::new("Add a folder of samples with +").color(tk.text_faint).size(size::SMALL));
            }
            for dir in library {
                let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
                folder(app, ui, &name, &dir, 0);
            }
        }
        if tab == Tab::All && !app.prefs.recent.is_empty() {
            heading(ui, "Recent projects");
            for path in app.prefs.recent.clone() {
                let name = path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let response = ui
                    .add(egui::Label::new(RichText::new(format!("  {name}")).color(tk.text)).sense(Sense::click()))
                    .on_hover_text(path.display().to_string());
                control::record(ui, &response, &format!("Recent project {name}"), "file", None);
                if response.double_clicked() {
                    dialogs::guard_unsaved(app, dialogs::Pending::Open(Some(path)));
                }
            }
        }
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::SEARCH.paint(ui.painter(), rect, tk.text_faint);
        let search = ui.add(egui::TextEdit::singleline(&mut app.view.browser.search).hint_text("Search sounds").desired_width(f32::INFINITY));
        control::record(ui, &search, "Search sounds", "text", None);
    });
}
