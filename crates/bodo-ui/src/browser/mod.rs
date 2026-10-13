//! The Browser, the left side panel (project.md section 14.1, ADR 0029): project files, plugins, presets and sounds.
//!
//! - The header has back and up buttons, the library button and the tabs All, Project, Plugins and Sounds.
//! - The overview lists the top entries (Current project, Recent files, Plugin database, Plugin presets, Channel
//!   presets, Mixer presets, Scores), the user library's folders, the user's own sample folders, and the Marketplace
//!   section with what the user owns, marked "new" when something arrived since they last looked.
//! - Double-clicking a folder enters it; back returns to where the Browser was, up goes to the folder above.
//! - At the bottom, tag filters and search look through the whole library, through an index built on a background
//!   thread.
//!
//! [`rows`] says what each kind of row does. Folder listings are read once when a folder opens and kept until a
//! refresh, so drawing never touches the disk.

mod drop;
mod index;
mod rows;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bodo_engine::library::user::{FOLDERS, MarketFolder, PresetFolder};
use bodo_engine::library::{self, Entry, EntryKind, UserLibrary};
use bodo_kit_ui::icons;
use egui::{RichText, Sense, Ui, vec2};
use serde_json::json;

pub(crate) use self::drop::handle_drops;
use self::rows::{empty, entries, folder_row, row, sound, sound_path};
use crate::theme::{Tokens, size};
use crate::windows::icon_button;
use crate::{BodoApp, control, dialogs};

/// A sound being dragged: the path to give `channel.add` or `clip.place_sound`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundPayload(pub String);

/// A preset or score being dragged out of the Browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePayload {
    /// The file.
    pub path: PathBuf,
    /// What it is.
    pub kind: EntryKind,
}

/// What the browser shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Everything.
    #[default]
    All,
    /// The current project's sounds.
    Project,
    /// Plugins and plugin presets.
    Plugins,
    /// Packs and sample folders.
    Sounds,
}

/// The top entries of the overview, opened and closed like folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    CurrentProject,
    RecentFiles,
    PluginDatabase,
}

impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::CurrentProject => "Current project",
            Self::RecentFiles => "Recent files",
            Self::PluginDatabase => "Plugin database",
        }
    }

    /// The key it is opened by, in the same set as folders.
    fn key(self) -> PathBuf {
        PathBuf::from(format!("bodo:{}", self.name()))
    }
}

/// What each Marketplace folder holds.
type MarketListing = Vec<(MarketFolder, Vec<library::market::Owned>)>;

/// How often the Marketplace folders are looked at again, so arrivals show while Bodo runs.
const MARKET_CHECK: Duration = Duration::from_secs(2);

/// The browser's tab, open folders, place, cached listings, tags and search.
#[derive(Debug, Default)]
pub struct BrowserView {
    /// The tab.
    pub tab: Tab,
    open: BTreeSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Entry>>,
    /// The search text.
    pub search: String,
    /// The tags the list is filtered by.
    pub tags: BTreeSet<String>,
    /// The folder entered, or `None` for the overview.
    location: Option<PathBuf>,
    /// Where back goes: the places before this one.
    history: Vec<Option<PathBuf>>,
    index: index::IndexState,
    /// The file being tagged, by key, and the tag typed so far.
    tagging: Option<(String, String)>,
    /// What the Marketplace folders hold and whether each is new, and when they were read.
    market: Option<(Instant, MarketListing)>,
    /// The folders that start open have been opened: Packs, so a new user sees the drum kit right away.
    started: bool,
}

impl BrowserView {
    fn listing(&mut self, dir: &Path) -> &[Entry] {
        self.listings.entry(dir.to_path_buf()).or_insert_with(|| library::list(dir))
    }

    /// Forgets cached listings and the index, so folders are read again.
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.index = index::IndexState::Empty;
        self.market = None;
    }

    fn is_open(&self, key: &Path) -> bool {
        self.open.contains(key)
    }

    fn toggle(&mut self, key: &Path) {
        if !self.open.remove(key) {
            self.open.insert(key.to_path_buf());
        }
    }

    /// Enters `folder`, remembering where the Browser was for back.
    fn enter(&mut self, folder: PathBuf) {
        if self.location.as_ref() != Some(&folder) {
            self.history.push(self.location.take());
            self.location = Some(folder);
        }
    }

    /// Returns to where the Browser was before.
    fn back(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.location = previous;
        }
    }

    /// The folder entered, if any.
    pub fn location(&self) -> Option<&Path> {
        self.location.as_deref()
    }

    /// Whether the library's index is being read, for tests waiting on tags and search.
    pub fn indexing(&self) -> bool {
        self.index.is_building()
    }

    /// Takes the user's tags again from `library`, after they changed.
    fn retag(&mut self, library: Option<&UserLibrary>) {
        if let Some(library) = library {
            self.index.retag(library);
        }
    }
}

/// The folders back and up stop at, with the names the breadcrumb gives them.
fn roots(app: &BodoApp) -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();
    if let Some(library) = app.user_library() {
        roots.push((library.root().to_path_buf(), String::from("Library")));
    }
    if let Some(factory) = app.session.env().factory.clone() {
        roots.push((factory.join("Packs"), String::from("Packs")));
    }
    for dir in &app.prefs.library {
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        roots.push((dir.clone(), name));
    }
    roots
}

/// Where up goes from `location`: the folder above, until a root, then the overview.
fn up_from(app: &BodoApp, location: &Path) -> Option<PathBuf> {
    let roots = roots(app);
    if roots.iter().any(|(root, _)| root == location) {
        return None;
    }
    location.parent().filter(|parent| roots.iter().any(|(root, _)| parent.starts_with(root))).map(Path::to_path_buf)
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
    header(app, ui);
    navigation(app, ui);
    ui.separator();
    let bottom = 30.0 + tag_bar_height(app);
    let list_height = (ui.available_height() - bottom).max(40.0);
    egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let query = app.view.browser.search.trim().to_owned();
        if !query.is_empty() || !app.view.browser.tags.is_empty() {
            results(app, ui, &query);
        } else if let Some(location) = app.view.browser.location.clone() {
            entries(app, ui, &location, 0);
        } else {
            overview(app, ui);
        }
    });
    ui.add_space(2.0);
    tag_bar(app, ui);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::SEARCH.paint(ui.painter(), rect, tk.text_faint);
        let search = ui.add(egui::TextEdit::singleline(&mut app.view.browser.search).hint_text("Search the library").desired_width(f32::INFINITY));
        control::record(ui, &search, "Search sounds", "text", None);
    });
    tag_dialog(app, ui.ctx());
}

fn header(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        back_and_up(app, ui);
        ui.label(RichText::new("Browser").color(tk.text).size(size::HEADING));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            let refresh = icon_button(ui, icons::HISTORY, "Refresh the browser", false);
            app.hint(&refresh, "Read the folders and the library again");
            if refresh.clicked() {
                app.view.browser.refresh();
            }
            let add = icon_button(ui, icons::PLUS, "Add a sample folder", false);
            app.hint(&add, "Add a folder of your own sounds to the Browser");
            if add.clicked() {
                dialogs::add_sample_folder(app);
                app.view.browser.refresh();
            }
            let library = icon_button(ui, icons::SETTINGS, "Library folder", false);
            let place = app.prefs.user_library.as_ref().map_or_else(|| String::from("none"), |p| p.display().to_string());
            app.hint(&library, format!("The library folder ({place}): click to choose another"));
            if library.clicked()
                && let Some(folder) = app.services.pick_folder.as_ref().and_then(|pick| pick())
            {
                match crate::library::prepare(&folder, app.session.env().factory.as_deref()) {
                    Ok(()) => {
                        app.prefs.user_library = Some(folder);
                        app.view.browser.refresh();
                    }
                    Err(error) => app.set_status(error, true),
                }
            }
        });
    });
}

/// Back to where the Browser was, and up to the folder above.
fn back_and_up(app: &mut BodoApp, ui: &mut Ui) {
    let can_back = !app.view.browser.history.is_empty();
    let back = ui.add_enabled_ui(can_back, |ui| icon_button(ui, icons::ARROW_LEFT, "Browser back", false)).inner;
    app.hint(&back, "Back to where the Browser was");
    if back.clicked() {
        app.view.browser.back();
    }
    let location = app.view.browser.location.clone();
    let up = ui.add_enabled_ui(location.is_some(), |ui| icon_button(ui, icons::ARROW_UP, "Browser up", false)).inner;
    app.hint(&up, "Up to the folder above");
    if up.clicked()
        && let Some(location) = location
    {
        match up_from(app, &location) {
            Some(folder) => app.view.browser.enter(folder),
            None => {
                app.view.browser.history.push(Some(location));
                app.view.browser.location = None;
            }
        }
    }
}

fn navigation(app: &mut BodoApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        for (tab, label) in [(Tab::All, "All"), (Tab::Project, "Project"), (Tab::Plugins, "Plugins"), (Tab::Sounds, "Sounds")] {
            let selected = app.view.browser.tab == tab && app.view.browser.location.is_none();
            let response = ui.selectable_label(selected, RichText::new(label).size(size::SMALL));
            control::record(ui, &response, &format!("Browser tab {label}"), "tab", Some(selected));
            if response.clicked() {
                app.view.browser.tab = tab;
                if app.view.browser.location.is_some() {
                    let current = app.view.browser.location.take();
                    app.view.browser.history.push(current);
                }
            }
        }
    });
    if let Some(location) = app.view.browser.location.clone() {
        let tk = Tokens::get();
        let crumb = rows::relative_name(&location, &roots(app));
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let label = ui.label(RichText::new(&crumb).color(tk.text_dim).size(size::SMALL));
            control::record(ui, &label, &format!("Browser in {crumb}"), "label", None);
        });
    }
}

/// The overview of the tab.
fn overview(app: &mut BodoApp, ui: &mut Ui) {
    let tab = app.view.browser.tab;
    let library = app.user_library();
    match tab {
        Tab::All => {
            section(app, ui, Section::CurrentProject);
            section(app, ui, Section::RecentFiles);
            section(app, ui, Section::PluginDatabase);
            if let Some(library) = &library {
                for kind in PresetFolder::ALL {
                    folder_with_entries(app, ui, kind.name(), &library.presets(kind), icons::FOLDER);
                }
                heading(ui, "Library");
                for name in FOLDERS {
                    if name == "Packs" {
                        packs(app, ui, Some(library));
                    } else {
                        folder_with_entries(app, ui, name, &library.folder(name), icons::FOLDER);
                    }
                }
            } else {
                heading(ui, "Library");
                packs(app, ui, None);
                empty(ui, 0, "No library folder: choose one with the gear above");
            }
            sample_folders(app, ui);
            if let Some(library) = &library {
                marketplace(app, ui, library);
            }
        }
        Tab::Project => {
            if !app.view.browser.is_open(&Section::CurrentProject.key()) {
                app.view.browser.open.insert(Section::CurrentProject.key());
            }
            section(app, ui, Section::CurrentProject);
        }
        Tab::Plugins => {
            section(app, ui, Section::PluginDatabase);
            if let Some(library) = &library {
                folder_with_entries(app, ui, PresetFolder::PluginPresets.name(), &library.presets(PresetFolder::PluginPresets), icons::FOLDER);
                folder_with_entries(app, ui, "Plugins (owned)", &library.market(MarketFolder::Plugins), icons::SHOPPING_BAG);
            }
        }
        Tab::Sounds => {
            heading(ui, "Sounds");
            packs(app, ui, library.as_ref());
            if let Some(library) = &library {
                for name in ["Audio", "Recorded", "Rendered", "Sliced audio", "Impulses", "Speech", "Soundfonts"] {
                    folder_with_entries(app, ui, name, &library.folder(name), icons::FOLDER);
                }
                folder_with_entries(app, ui, "Stems (owned)", &library.market(MarketFolder::Stems), icons::SHOPPING_BAG);
            }
            sample_folders(app, ui);
        }
    }
}

/// A folder row and, while open, its entries.
fn folder_with_entries(app: &mut BodoApp, ui: &mut Ui, name: &str, dir: &Path, icon: bodo_kit_ui::Icon) {
    let response = folder_row(app, ui, name, dir, 0, icon, None);
    folder_menu(app, &response, dir);
    if app.view.browser.is_open(dir) {
        entries(app, ui, dir, 1);
    }
}

/// Packs: the factory packs and the user's own, in one folder.
fn packs(app: &mut BodoApp, ui: &mut Ui, library: Option<&UserLibrary>) {
    let key = library.map_or_else(|| PathBuf::from("bodo:Packs"), UserLibrary::packs);
    if !app.view.browser.started {
        app.view.browser.started = true;
        app.view.browser.open.insert(key.clone());
    }
    folder_row(app, ui, "Packs", &key, 0, icons::FOLDER, None);
    if !app.view.browser.is_open(&key) {
        return;
    }
    match app.session.env().factory.clone() {
        Some(factory) => {
            let dir = factory.join("Packs");
            let listed = app.view.browser.listing(&dir).to_vec();
            for entry in listed.into_iter().filter(Entry::is_dir) {
                folder_row(app, ui, &entry.name, &entry.path, 1, icons::FOLDER, None);
                if app.view.browser.is_open(&entry.path) {
                    entries(app, ui, &entry.path, 2);
                }
            }
        }
        None => empty(ui, 1, "The factory content wasn't found"),
    }
    if let Some(library) = library {
        let own = library.packs();
        if !app.view.browser.listing(&own).is_empty() {
            entries(app, ui, &own, 1);
        }
    }
}

/// The user's own sample folders.
fn sample_folders(app: &mut BodoApp, ui: &mut Ui) {
    let folders = app.prefs.library.clone();
    if folders.is_empty() {
        return;
    }
    heading(ui, "Sample folders");
    for dir in folders {
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        folder_row(app, ui, &name, &dir, 0, icons::FOLDER, None);
        if app.view.browser.is_open(&dir) {
            entries(app, ui, &dir, 1);
        }
    }
}

/// The Marketplace section: what the user owns, marked new when it arrived since they last looked.
fn marketplace(app: &mut BodoApp, ui: &mut Ui, library: &UserLibrary) {
    let stale = app.view.browser.market.as_ref().is_none_or(|(at, _)| at.elapsed() > MARKET_CHECK);
    if stale {
        let owned = MarketFolder::ALL.into_iter().map(|kind| (kind, library::market::owned(library, kind))).collect();
        app.view.browser.market = Some((Instant::now(), owned));
    }
    let owned = app.view.browser.market.as_ref().map(|(_, o)| o.clone()).unwrap_or_default();
    let any_new = owned.iter().any(|(_, items)| items.iter().any(|o| o.new));
    let tk = Tokens::get();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Marketplace").color(tk.text_faint).size(size::SMALL).strong());
        if any_new {
            let badge = ui.label(RichText::new("new").color(tk.accent).size(size::SMALL).strong());
            control::record(ui, &badge, "Marketplace new", "badge", None);
        }
    });
    for (kind, items) in owned {
        let dir = library.market(kind);
        let name = format!("{} (owned)", kind.name());
        let new = items.iter().any(|o| o.new);
        let response = folder_row(app, ui, &name, &dir, 0, icons::SHOPPING_BAG, new.then_some("new"));
        if response.clicked() || response.double_clicked() {
            if let Err(error) = library::market::mark_seen(library, kind) {
                tracing::warn!(%error, "could not record that the Marketplace folder was seen");
            }
            app.view.browser.market = None;
        }
        if app.view.browser.is_open(&dir) {
            if items.is_empty() {
                empty(ui, 1, "Nothing yet: what you get from the Marketplace lands here");
            }
            for item in items {
                let entry = item.entry;
                match entry.kind {
                    EntryKind::Sound => {
                        let stored = sound_path(app, &entry.path);
                        sound(app, ui, &entry.stem(), &entry.path, stored, 1);
                    }
                    EntryKind::Folder => {
                        folder_row(app, ui, &entry.name, &entry.path, 1, icons::FOLDER, None);
                        if app.view.browser.is_open(&entry.path) {
                            entries(app, ui, &entry.path, 2);
                        }
                    }
                    kind => rows::file(app, ui, kind, &entry.stem(), &entry.path, 1),
                }
            }
        }
    }
}

/// A top entry: Current project, Recent files or Plugin database.
fn section(app: &mut BodoApp, ui: &mut Ui, section: Section) {
    let key = section.key();
    let icon = match section {
        Section::CurrentProject => icons::MUSIC,
        Section::RecentFiles => icons::HISTORY,
        Section::PluginDatabase => icons::SPARKLES,
    };
    folder_row(app, ui, section.name(), &key, 0, icon, None);
    if !app.view.browser.is_open(&key) {
        return;
    }
    match section {
        Section::CurrentProject => {
            let project = std::sync::Arc::clone(app.session.project());
            if project.samples.is_empty() {
                empty(ui, 1, "No sounds yet");
            }
            let roots = app.session.sample_roots();
            for sample in project.samples.values() {
                let file = roots.resolve(&sample.path);
                sound(app, ui, &bodo_engine::samples::display_name(&sample.path), &file, sample.path.clone(), 1);
            }
        }
        Section::RecentFiles => {
            let recent = app.prefs.recent.clone();
            if recent.is_empty() {
                empty(ui, 1, "Projects you open and save show here");
            }
            for path in recent {
                let name = path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                rows::file(app, ui, EntryKind::Project, &name, &path, 1);
            }
        }
        Section::PluginDatabase => {
            let response = row(ui, 1, icons::WAVE_SINE, "Sampler (built in)", false);
            control::record(ui, &response, "Plugin Sampler", "plugin", None);
            app.hint(&response, "Bodo's sampler: double-click to add an empty sampler channel, then give it a sound");
            if response.double_clicked() {
                let _ = app.run("channel.add", json!({ "name": "Sampler" }));
            }
        }
    }
}

/// The context menu of a library folder: saving the current channel, insert, notes or project into it.
fn folder_menu(app: &mut BodoApp, response: &egui::Response, dir: &Path) {
    let Some(library) = app.user_library() else { return };
    let action: Option<(&str, &str, serde_json::Value)> = if dir == library.presets(PresetFolder::ChannelPresets) {
        Some(("Save the selected channel here", "preset.save_channel", json!({})))
    } else if dir == library.presets(PresetFolder::MixerPresets) {
        Some(("Save the selected insert here", "preset.save_mixer", json!({})))
    } else if dir == library.presets(PresetFolder::Scores) {
        Some(("Save the piano roll's notes here", "preset.save_score", json!({})))
    } else if dir == library.templates() {
        Some(("Save this project as a template", "project.save_template", json!({})))
    } else if dir == library.rendered() {
        Some(("Export the song as MIDI here", "project.export_midi", json!({ "path": unused(dir, &app.title(), "mid") })))
    } else {
        None
    };
    response.context_menu(|ui| {
        if let Some((label, command, params)) = &action
            && ui.button(*label).clicked()
        {
            if app.run(command, params.clone()).is_ok() {
                app.view.browser.listings.remove(dir);
                app.view.browser.open.insert(dir.to_path_buf());
            }
            ui.close();
        }
        if ui.button("Refresh").clicked() {
            app.view.browser.listings.remove(dir);
            ui.close();
        }
    });
}

/// A file name in `dir` that isn't taken: `name.extension`, or with a number.
fn unused(dir: &Path, name: &str, extension: &str) -> String {
    let name = name.trim_end_matches(" *");
    bodo_engine::library::user::unused_file(dir, name, extension).display().to_string()
}

/// The height the tag bar takes.
fn tag_bar_height(app: &BodoApp) -> f32 {
    if app.user_library().is_some() || app.session.env().factory.is_some() { 30.0 } else { 0.0 }
}

/// The tags to filter by: the active ones, then the most common, at most a dozen.
fn tag_bar(app: &mut BodoApp, ui: &mut Ui) {
    if tag_bar_height(app) == 0.0 {
        return;
    }
    let env = app.session.env().clone();
    let index = app.view.browser.index.get(&env);
    let tk = Tokens::get();
    let active = app.view.browser.tags.clone();
    // The active tags, then the user's own, then the most common ones.
    let mut shown: Vec<String> = active.iter().cloned().collect();
    if let Some(index) = &index {
        let own: BTreeSet<&str> = index.files.iter().flat_map(|f| f.user_tags.iter().map(String::as_str)).collect();
        let common = index.tag_counts();
        let candidates = own.iter().map(|t| (*t).to_owned()).chain(common.into_iter().map(|(tag, _)| tag));
        for tag in candidates {
            if shown.len() >= 16 {
                break;
            }
            if !shown.contains(&tag) {
                shown.push(tag);
            }
        }
    }
    // One row of as many tags as fit, so they never push the search box down or widen the panel.
    let font = egui::FontId::proportional(size::SMALL);
    let padding = ui.spacing().button_padding.x * 2.0 + 4.0;
    let mut room = ui.available_width() - 6.0 - if active.is_empty() { 0.0 } else { 44.0 };
    let shown: Vec<String> = shown
        .into_iter()
        .take_while(|tag| {
            let width = ui.fonts_mut(|f| f.layout_no_wrap(format!("#{tag}"), font.clone(), tk.text).size().x) + padding;
            room -= width;
            room >= 0.0
        })
        .collect();
    {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
            if index.is_none() {
                ui.label(RichText::new("Reading the library...").color(tk.text_faint).size(size::SMALL));
            }
            for tag in shown {
                let on = active.contains(&tag);
                let chip = ui.selectable_label(on, RichText::new(format!("#{tag}")).size(size::SMALL));
                control::record(ui, &chip, &format!("Tag {tag}"), "tag", Some(on));
                app.hint(&chip, if on { format!("Showing what is tagged {tag}: click to stop") } else { format!("Show only what is tagged {tag}") });
                if chip.clicked() {
                    if on {
                        app.view.browser.tags.remove(&tag);
                    } else {
                        app.view.browser.tags.insert(tag);
                    }
                }
            }
            if !active.is_empty() {
                let clear = ui.small_button("Clear");
                control::record(ui, &clear, "Clear tags", "button", None);
                if clear.clicked() {
                    app.view.browser.tags.clear();
                }
            }
        });
    }
}

/// What the search and the tags find, of the kinds the tab shows.
fn results(app: &mut BodoApp, ui: &mut Ui, query: &str) {
    let tk = Tokens::get();
    heading(ui, "Search results");
    let tab = app.view.browser.tab;
    if tab == Tab::Project {
        let project = std::sync::Arc::clone(app.session.project());
        let roots = app.session.sample_roots();
        let lower = query.to_lowercase();
        let mut any = false;
        for sample in project.samples.values().filter(|s| s.path.to_lowercase().contains(&lower)) {
            any = true;
            sound(app, ui, &bodo_engine::samples::display_name(&sample.path), &roots.resolve(&sample.path), sample.path.clone(), 0);
        }
        if !any {
            ui.label(RichText::new("Nothing found").color(tk.text_faint));
        }
        return;
    }
    let env = app.session.env().clone();
    let Some(index) = app.view.browser.index.get(&env) else {
        ui.label(RichText::new("Reading the library...").color(tk.text_faint));
        ui.ctx().request_repaint_after(Duration::from_millis(50));
        return;
    };
    let tags: Vec<String> = app.view.browser.tags.iter().cloned().collect();
    let wanted = |kind: EntryKind| match tab {
        Tab::Sounds => kind == EntryKind::Sound,
        Tab::Plugins => matches!(kind, EntryKind::Plugin | EntryKind::PluginPreset),
        Tab::All | Tab::Project => true,
    };
    let found: Vec<_> = index.find(query, &tags, 500).into_iter().filter(|f| wanted(f.kind)).take(200).cloned().collect();
    if found.is_empty() {
        ui.label(RichText::new("Nothing found").color(tk.text_faint));
    }
    for file in found {
        match file.kind {
            EntryKind::Sound => {
                let stored = sound_path(app, &file.path);
                sound(app, ui, &file.name, &file.path, stored, 0);
            }
            kind => rows::file(app, ui, kind, &file.name, &file.path, 0),
        }
    }
}

/// The small window that asks for a tag.
fn tag_dialog(app: &mut BodoApp, ctx: &egui::Context) {
    let Some((key, mut text)) = app.view.browser.tagging.take() else { return };
    let mut open = true;
    let mut done = false;
    egui::Window::new(rows::tagging_title(&key)).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut text).hint_text("A tag, such as warm or kick"));
        control::record(ui, &field, "Tag name", "text", None);
        field.request_focus();
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add = ui.button("Add tag");
        control::record(ui, &add, "Add tag", "button", None);
        if (add.clicked() || enter) && !text.trim().is_empty() {
            done = true;
        }
    });
    if done {
        if app.run("library.tag", json!({ "path": key, "add": [text.trim()] })).is_ok() {
            let library = app.user_library();
            app.view.browser.retag(library.as_ref());
        }
    } else if open {
        app.view.browser.tagging = Some((key, text));
    }
}
