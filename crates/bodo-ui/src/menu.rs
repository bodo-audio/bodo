//! The menus (project.md section 14.1, ADR 0032): File, Edit, Add, Patterns, View, Options, Tools and Help, as one
//! model that both the top bar and the native macOS menu bar draw, so they never drift apart. Each item carries the
//! keys its shortcut has now, after the user's changes, and an [`Action`] that [`perform`] runs the same way from
//! either menu.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::i18n::en;
use crate::theme::ThemePref;
use crate::windows::WindowKind;
use crate::{BodoApp, dialogs, layouts, shortcuts};

/// What a menu item does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Runs a command as its shortcut does: file commands through their dialogs, the rest on the selection.
    Command(String),
    /// Runs a command with these params.
    Run(String, Value),
    /// Opens a recent project, asking about unsaved changes first.
    OpenRecent(PathBuf),
    /// Imports a MIDI file the user picks.
    ImportMidi,
    /// Exports the song, or the current pattern when set, as a MIDI file.
    ExportMidi(bool),
    /// Opens the export dialog for the mix, the stems or both.
    Export(crate::dialogs::ExportWhat),
    /// Closes Bodo, asking about unsaved changes first.
    Quit,
    /// Adds a channel playing this sound.
    AddSound(String),
    /// Adds sampler channels for files the user picks.
    AddFromFiles,
    /// Renames the current pattern.
    RenamePattern,
    /// Shows or hides a window.
    ToggleWindow(WindowKind),
    /// Shows or hides the Browser.
    ToggleBrowser,
    /// Detaches a window into its own OS window, or puts it back.
    Detach(WindowKind),
    /// Switches to a layout, by name.
    Layout(String),
    /// Saves the windows as a new layout, asking for its name.
    SaveLayout,
    /// Renames a saved layout, asking for the new name.
    RenameLayout(String),
    /// Deletes a saved layout.
    DeleteLayout(String),
    /// Sets the theme.
    Theme(ThemePref),
    /// Opens the audio settings.
    AudioSettings,
    /// Opens the interface settings: theme, size, language and color-blind-safe colors.
    InterfaceSettings,
    /// Adds a folder of the user's sounds to the Browser.
    AddSampleFolder,
    /// Sets the minutes between autosaves; 0 turns autosave off.
    Autosave(u32),
    /// Opens the Keyboard shortcuts dialog.
    Shortcuts,
    /// Opens the command palette.
    Palette,
    /// Shows the About box.
    About,
}

/// An entry of a menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Something to do.
    Action {
        /// A stable id, for the native menu.
        id: String,
        /// Its text.
        label: String,
        /// What it does.
        action: Action,
        /// The keys of its shortcut now, such as `Mod+S`.
        keys: Option<String>,
        /// Whether it can be chosen now.
        enabled: bool,
        /// Whether it shows a check mark, and whether it is checked; `None` for plain items.
        checked: Option<bool>,
    },
    /// A submenu.
    Submenu {
        /// A stable id, for the native menu.
        id: String,
        /// Its text.
        label: String,
        /// Its entries.
        items: Vec<Item>,
        /// Whether it can be opened now.
        enabled: bool,
    },
    /// A line between groups.
    Separator,
}

/// A top-level menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    /// Its title in the menu bar.
    pub title: &'static str,
    /// Its entries.
    pub items: Vec<Item>,
}

fn item(id: &str, label: impl Into<String>, action: Action) -> Item {
    Item::Action { id: id.to_owned(), label: label.into(), action, keys: None, enabled: true, checked: None }
}

/// An item for command `command`, with its shortcut's keys now.
fn command(app: &BodoApp, id: &str, label: impl Into<String>, command: &str, action: Action) -> Item {
    let default = app.session.registry().get(command).and_then(|c| c.shortcut);
    let keys = shortcuts::command_keys(app, command, default);
    Item::Action { id: id.to_owned(), label: label.into(), action, keys, enabled: true, checked: None }
}

fn with_keys(item: Item, new_keys: Option<String>) -> Item {
    match item {
        Item::Action { id, label, action, enabled, checked, .. } => Item::Action { id, label, action, keys: new_keys, enabled, checked },
        other => other,
    }
}

fn enabled(item: Item, on: bool) -> Item {
    match item {
        Item::Action { id, label, action, keys, checked, .. } => Item::Action { id, label, action, keys, enabled: on, checked },
        Item::Submenu { id, label, items, .. } => Item::Submenu { id, label, items, enabled: on },
        Item::Separator => Item::Separator,
    }
}

fn checked(item: Item, on: bool) -> Item {
    match item {
        Item::Action { id, label, action, keys, enabled, .. } => Item::Action { id, label, action, keys, enabled, checked: Some(on) },
        other => other,
    }
}

fn submenu(id: &str, label: &str, items: Vec<Item>) -> Item {
    let enabled = !items.is_empty();
    Item::Submenu { id: id.to_owned(), label: label.to_owned(), items, enabled }
}

/// The binding of `binding_id`'s keys now.
fn binding_keys(app: &BodoApp, binding_id: &str) -> Option<String> {
    shortcuts::bindings(app).into_iter().find(|b| b.id == binding_id).and_then(|b| b.keys)
}

/// Every menu, as the app stands now.
///
/// Complexity: O(c log c) in the commands, for the shortcuts, plus the recent files, layouts and factory sounds.
pub fn menus(app: &BodoApp) -> Vec<Menu> {
    vec![file(app), edit(app), add(app), patterns(app), view(app), options(app), tools(app), help(app)]
}

fn file(app: &BodoApp) -> Menu {
    let recent: Vec<Item> = app
        .prefs
        .recent
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
            item(&format!("file.recent.{i}"), name, Action::OpenRecent(path.clone()))
        })
        .collect();
    Menu {
        title: "File",
        items: vec![
            command(app, "file.new", "New", "project.new", Action::Command(String::from("project.new"))),
            command(app, "file.open", "Open...", "project.open", Action::Command(String::from("project.open"))),
            submenu("file.recent", "Open recent", recent),
            Item::Separator,
            command(app, "file.save", "Save", "project.save", Action::Command(String::from("project.save"))),
            command(app, "file.save_as", "Save as...", "project.save_as", Action::Command(String::from("project.save_as"))),
            Item::Separator,
            item("file.import_midi", "Import MIDI file...", Action::ImportMidi),
            item("file.export_song_midi", "Export song as MIDI...", Action::ExportMidi(false)),
            item("file.export_pattern_midi", "Export pattern as MIDI...", Action::ExportMidi(true)),
            Item::Separator,
            item("file.export_mix", "Export mix...", Action::Export(crate::dialogs::ExportWhat::Mix)),
            item("file.export_stems", "Export stems...", Action::Export(crate::dialogs::ExportWhat::Stems)),
            Item::Separator,
            with_keys(item("file.quit", "Quit", Action::Quit), binding_keys(app, "app:quit")),
        ],
    }
}

fn edit(app: &BodoApp) -> Menu {
    let undo = app.session.undo_labels().last().map(|l| tf!("Undo {step}", step = crate::i18n::tr(l)));
    let redo = app.session.redo_labels().next().map(|l| tf!("Redo {step}", step = crate::i18n::tr(l)));
    Menu {
        title: "Edit",
        items: vec![
            enabled(
                command(
                    app,
                    "edit.undo",
                    undo.clone().unwrap_or_else(|| String::from(en("Undo"))),
                    "edit.undo",
                    Action::Command(String::from("edit.undo")),
                ),
                undo.is_some(),
            ),
            enabled(
                command(
                    app,
                    "edit.redo",
                    redo.clone().unwrap_or_else(|| String::from(en("Redo"))),
                    "edit.redo",
                    Action::Command(String::from("edit.redo")),
                ),
                redo.is_some(),
            ),
            Item::Separator,
            command(app, "edit.cut", "Cut", "edit.cut", Action::Command(String::from("edit.cut"))),
            command(app, "edit.copy", "Copy", "edit.copy", Action::Command(String::from("edit.copy"))),
            command(app, "edit.paste", "Paste", "edit.paste", Action::Command(String::from("edit.paste"))),
            Item::Separator,
            command(
                app,
                "edit.select_all",
                "Select all clips",
                "edit.select_all",
                Action::Run(String::from("edit.select_all"), json!({ "what": "clips" })),
            ),
            command(app, "edit.deselect", "Deselect", "edit.deselect", Action::Command(String::from("edit.deselect"))),
            Item::Separator,
            with_keys(item("edit.palette", "Command palette...", Action::Palette), binding_keys(app, "app:palette")),
        ],
    }
}

fn add(app: &BodoApp) -> Menu {
    let mut channels: Vec<Item> = bodo_engine::library::search(app.session.env(), "", 64)
        .into_iter()
        .filter(|s| s.path.starts_with(bodo_model::FACTORY_PREFIX))
        .map(|s| item(&format!("add.channel.{}", s.name), s.name.clone(), Action::AddSound(s.path.clone())))
        .collect();
    if !channels.is_empty() {
        channels.push(Item::Separator);
    }
    channels.push(item("add.channel.empty", "Empty sampler", Action::Run(String::from("channel.add"), json!({}))));
    channels.push(item("add.channel.files", "From file...", Action::AddFromFiles));
    Menu {
        title: "Add",
        items: vec![
            submenu("add.channel", "Channel", channels),
            command(app, "add.pattern", "Pattern", "pattern.new", Action::Command(String::from("pattern.new"))),
            command(app, "add.track", "Playlist track", "playlist.add_track", Action::Command(String::from("playlist.add_track"))),
            command(app, "add.insert", "Mixer insert", "mixer.add_insert", Action::Command(String::from("mixer.add_insert"))),
        ],
    }
}

fn patterns(app: &BodoApp) -> Menu {
    Menu {
        title: "Patterns",
        items: vec![
            command(app, "patterns.new", "New pattern", "pattern.new", Action::Command(String::from("pattern.new"))),
            command(app, "patterns.clone", "Clone pattern", "pattern.clone", Action::Command(String::from("pattern.clone"))),
            item("patterns.rename", "Rename pattern...", Action::RenamePattern),
            command(app, "patterns.delete", "Delete pattern", "pattern.delete", Action::Command(String::from("pattern.delete"))),
            Item::Separator,
            item("patterns.next", "Next pattern", Action::Run(String::from("ui.select_pattern"), json!({ "offset": 1 }))),
            item("patterns.previous", "Previous pattern", Action::Run(String::from("ui.select_pattern"), json!({ "offset": -1 }))),
        ],
    }
}

/// The layouts: the presets and the user's, saving the windows as a new one, and renaming or deleting the user's.
pub fn layout_items(app: &BodoApp) -> Vec<Item> {
    let current = app.prefs.layout_name.as_deref();
    let mut items: Vec<Item> = crate::windows::Preset::ALL
        .iter()
        .map(|p| checked(item(&format!("view.layout.{}", p.label()), p.label(), Action::Layout(p.label().to_owned())), current == Some(p.label())))
        .collect();
    if !app.prefs.layouts.is_empty() {
        items.push(Item::Separator);
        for saved in &app.prefs.layouts {
            let name = saved.name.as_str();
            items.push(checked(item(&format!("view.layout.saved.{name}"), name, Action::Layout(name.to_owned())), current == Some(name)));
        }
    }
    items.push(Item::Separator);
    items.push(item("view.layout.save", "Save layout as...", Action::SaveLayout));
    let names: Vec<&str> = app.prefs.layouts.iter().map(|l| l.name.as_str()).collect();
    items.push(submenu(
        "view.layout.rename",
        "Rename layout",
        names.iter().map(|n| item(&format!("view.layout.rename.{n}"), *n, Action::RenameLayout((*n).to_owned()))).collect(),
    ));
    items.push(submenu(
        "view.layout.delete",
        "Delete layout",
        names.iter().map(|n| item(&format!("view.layout.delete.{n}"), *n, Action::DeleteLayout((*n).to_owned()))).collect(),
    ));
    items
}

fn view(app: &BodoApp) -> Menu {
    let mut items: Vec<Item> = WindowKind::ALL
        .iter()
        .map(|kind| {
            let keys = binding_keys(app, &format!("window:{}", kind.id()));
            checked(
                with_keys(item(&format!("view.window.{}", kind.id()), kind.title(), Action::ToggleWindow(*kind)), keys),
                app.prefs.layout.is_open(*kind),
            )
        })
        .collect();
    items.push(checked(item("view.browser", "Browser", Action::ToggleBrowser), app.prefs.layout.browser));
    items.push(Item::Separator);
    let detached = [WindowKind::Playlist, WindowKind::PianoRoll, WindowKind::Mixer]
        .iter()
        .map(|kind| checked(item(&format!("view.detach.{}", kind.id()), kind.title(), Action::Detach(*kind)), app.prefs.layout.state(*kind).detached))
        .collect();
    items.push(submenu("view.detach", "Own window", detached));
    items.push(submenu("view.layout", "Layout", layout_items(app)));
    items.push(submenu(
        "view.theme",
        "Theme",
        ThemePref::ALL
            .iter()
            .map(|t| checked(item(&format!("view.theme.{}", t.label()), t.label(), Action::Theme(*t)), app.prefs.theme == *t))
            .collect(),
    ));
    Menu { title: "View", items }
}

fn options(app: &BodoApp) -> Menu {
    let autosave = [0, 1, 5, 10, 30]
        .iter()
        .map(|m| {
            let label = if *m == 0 { String::from(en("Off")) } else { tf!("Every {minutes} minutes", minutes = m) };
            checked(item(&format!("options.autosave.{m}"), label, Action::Autosave(*m)), app.prefs.autosave_minutes == *m)
        })
        .collect();
    let auto_label = tf!("Auto ({percent}%)", percent = (app.ui_scale() * 100.0).round());
    let auto = Action::Run(String::from("ui.set_ui_scale"), json!({ "scale": "auto" }));
    let mut sizes: Vec<Item> = vec![checked(item("options.size.auto", auto_label, auto), app.prefs.ui_scale.is_none()), Item::Separator];
    sizes.extend(crate::UI_SCALES.iter().map(|scale| {
        let percent = (scale * 100.0).round();
        let on = app.prefs.ui_scale.is_some_and(|s| (s - scale).abs() < 1e-3);
        checked(
            item(&format!("options.size.{percent}"), format!("{percent}%"), Action::Run(String::from("ui.set_ui_scale"), json!({ "scale": scale }))),
            on,
        )
    }));
    sizes.push(Item::Separator);
    for (id, label) in [("ui.zoom_in", en("Larger")), ("ui.zoom_out", en("Smaller")), ("ui.zoom_reset", en("Automatic size"))] {
        let keys = shortcuts::command_keys(app, id, shortcuts::ui_command_keys(id));
        sizes.push(with_keys(item(&format!("options.size.{id}"), label, Action::Command(id.to_owned())), keys));
    }
    let languages = crate::i18n::LanguagePref::ALL
        .iter()
        .map(|l| {
            let action = Action::Run(String::from("ui.set_language"), json!({ "language": l.id() }));
            checked(item(&format!("options.language.{}", l.id()), l.label(), action), app.prefs.language == *l)
        })
        .collect();
    Menu {
        title: "Options",
        items: vec![
            item("options.audio", "Audio settings...", Action::AudioSettings),
            item("options.interface", "Interface settings...", Action::InterfaceSettings),
            item("options.sample_folder", "Add sample folder...", Action::AddSampleFolder),
            submenu("options.autosave", "Autosave", autosave),
            Item::Separator,
            submenu("options.size", "Interface size", sizes),
            submenu("options.language", "Language", languages),
            checked(
                item("options.color_safe", "Color-blind-safe colors", Action::Run(String::from("ui.set_color_safe"), json!({}))),
                app.prefs.color_safe,
            ),
            Item::Separator,
            item("options.shortcuts", "Keyboard shortcuts...", Action::Shortcuts),
        ],
    }
}

fn tools(app: &BodoApp) -> Menu {
    Menu { title: "Tools", items: vec![with_keys(item("tools.palette", "Command palette...", Action::Palette), binding_keys(app, "app:palette"))] }
}

fn help(_app: &BodoApp) -> Menu {
    Menu {
        title: "Help",
        items: vec![item("help.shortcuts", "Keyboard shortcuts", Action::Shortcuts), item("help.about", "About Bodo", Action::About)],
    }
}

/// The action of the item with `id` in `menus`, for the native menu's events.
///
/// Complexity: O(n) in the items.
pub fn action_of(menus: &[Menu], id: &str) -> Option<Action> {
    fn find(items: &[Item], id: &str) -> Option<Action> {
        items.iter().find_map(|i| match i {
            Item::Action { id: item_id, action, .. } if item_id == id => Some(action.clone()),
            Item::Submenu { items, .. } => find(items, id),
            _ => None,
        })
    }
    menus.iter().find_map(|m| find(&m.items, id))
}

/// Does what a menu item says.
pub fn perform(app: &mut BodoApp, action: Action) {
    match action {
        Action::Command(id) => shortcuts::run_command(app, &id),
        Action::Run(id, params) => {
            let _ = app.run(&id, params);
        }
        Action::OpenRecent(path) => dialogs::guard_unsaved(app, dialogs::Pending::Open(Some(path))),
        Action::ImportMidi => dialogs::import_midi(app),
        Action::ExportMidi(pattern) => dialogs::export_midi(app, pattern),
        Action::Export(what) => app.view.dialogs.export = Some(dialogs::Export::new(app, what)),
        Action::Quit => dialogs::guard_unsaved(app, dialogs::Pending::Quit),
        Action::AddSound(path) => {
            let _ = app.run("channel.add", json!({ "sample": path }));
        }
        Action::AddFromFiles => dialogs::add_channels_from_files(app),
        Action::RenamePattern => dialogs::rename_pattern(app),
        Action::ToggleWindow(kind) => {
            let _ = app.run("ui.toggle_window", json!({ "window": kind.id() }));
        }
        Action::ToggleBrowser => {
            let _ = app.run("ui.toggle_browser", json!({}));
        }
        Action::Detach(kind) => {
            let _ = app.run("ui.detach_window", json!({ "window": kind.id() }));
        }
        Action::Layout(name) => {
            let _ = layouts::switch(app, &name).map_err(|e| app.set_status(e, true));
        }
        Action::SaveLayout => app.view.dialogs.save_layout(),
        Action::RenameLayout(name) => app.view.dialogs.rename_layout(&name),
        Action::DeleteLayout(name) => {
            let _ = app.run("ui.delete_layout", json!({ "layout": name }));
        }
        Action::Theme(theme) => {
            let _ = app.run("ui.set_theme", json!({ "theme": theme.label() }));
        }
        Action::AudioSettings => app.view.dialogs.audio = true,
        Action::InterfaceSettings => app.view.dialogs.interface = true,
        Action::AddSampleFolder => dialogs::add_sample_folder(app),
        Action::Autosave(minutes) => app.prefs.autosave_minutes = minutes,
        Action::Shortcuts => app.view.dialogs.shortcuts = true,
        Action::Palette => {
            let _ = app.run("ui.palette", json!({}));
        }
        Action::About => app.view.dialogs.about = true,
    }
}
