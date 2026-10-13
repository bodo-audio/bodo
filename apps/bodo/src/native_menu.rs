//! The native macOS menu bar (project.md section 14.1, ADR 0032).
//!
//! It mirrors the window's menus from the same model, [`bodo_ui::menu`]: File, Edit, Add, Patterns, View, Options,
//! Tools and Help, after the app menu macOS expects (About, Services, Hide, Quit). Choosing an item runs the same
//! action as the window's menu. Items carry the keys their shortcuts have now, so macOS shows them and answers them.
//!
//! The menu bar is rebuilt only when its structure changes, such as a new recent file or layout; otherwise labels,
//! checks, enabled states and keys update in place. A few keys stay out of it: the clipboard and undo keys belong to
//! text fields while one has the keyboard, and egui sees them only when macOS doesn't take them first. While the
//! Keyboard shortcuts dialog listens for new keys, the menu bar gives up every key, so any can be recorded.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use bodo_ui::BodoApp;
use bodo_ui::i18n::t;
use bodo_ui::menu::{self, Action, Item, Menu};
use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

/// Keys the menu bar leaves to egui: text fields use them while they have the keyboard.
const FOR_TEXT: [&str; 6] = ["Mod+C", "Mod+X", "Mod+V", "Mod+Z", "Mod+Shift+Z", "Mod+A"];

/// The app menu's own items, which the model doesn't have.
const ABOUT: &str = "app.about";
const QUIT: &str = "app.quit";

/// A native item, kept so it can change in place.
enum Handle {
    Item(MenuItem),
    Check(CheckMenuItem),
    Sub(Submenu),
}

/// What an item shows, to tell when it changed.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    label: String,
    keys: Option<String>,
    enabled: bool,
    checked: Option<bool>,
}

/// The menu bar and what it shows.
pub struct NativeMenu {
    bar: muda::Menu,
    structure: Vec<String>,
    handles: HashMap<String, (Handle, Shown)>,
    model: Vec<Menu>,
    quit_keys: Option<String>,
    listening: bool,
    events: Receiver<String>,
}

impl NativeMenu {
    /// A menu bar whose clicks wake `ctx`; it shows from the first [`NativeMenu::update`].
    pub fn new(ctx: &egui::Context) -> Self {
        let (sender, events) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = sender.send(event.id().0.clone());
            ctx.request_repaint();
        }));
        Self { bar: muda::Menu::new(), structure: Vec::new(), handles: HashMap::new(), model: Vec::new(), quit_keys: None, listening: false, events }
    }

    /// Runs the items chosen since the last frame, then brings the menu bar up to date with `app`.
    ///
    /// Complexity: O(n) in the menu items.
    pub fn update(&mut self, app: &mut BodoApp) {
        while let Ok(id) = self.events.try_recv() {
            let action = match id.as_str() {
                ABOUT => Some(Action::About),
                QUIT => Some(Action::Quit),
                _ => menu::action_of(&self.model, &id),
            };
            if let Some(action) = action {
                menu::perform(app, action);
            }
        }
        let model = menu::menus(app);
        let listening = app.view.dialogs.recording_shortcut();
        let quit_keys = bodo_ui::shortcuts::bindings(app).into_iter().find(|b| b.id == "app:quit").and_then(|b| b.keys);
        let structure = structure_of(&model);
        if structure != self.structure {
            self.rebuild(&model, &quit_keys, listening);
            self.structure = structure;
        } else if model != self.model || listening != self.listening || quit_keys != self.quit_keys {
            self.refresh(&model, &quit_keys, listening);
        }
        self.model = model;
        self.quit_keys = quit_keys;
        self.listening = listening;
    }

    fn rebuild(&mut self, model: &[Menu], quit_keys: &Option<String>, listening: bool) {
        let bar = muda::Menu::new();
        let mut handles = HashMap::new();
        let app_menu = Submenu::with_id("app", "Bodo", true);
        let about = MenuItem::with_id(ABOUT, t("About Bodo"), true, None);
        let quit = MenuItem::with_id(QUIT, t("Quit Bodo"), true, accelerator(quit_keys.as_deref(), listening));
        let appended = app_menu.append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        if let Err(error) = appended.and_then(|()| bar.append(&app_menu)) {
            tracing::warn!(%error, "the app menu could not be built");
        }
        let shown_quit = Shown { label: String::from(t("Quit Bodo")), keys: quit_keys.clone(), enabled: true, checked: None };
        handles.insert(String::from(QUIT), (Handle::Item(quit), shown_quit));
        for top in model {
            let submenu = Submenu::with_id(format!("menu.{}", top.title), t(top.title), true);
            for item in &top.items {
                add(&submenu, item, listening, &mut handles);
            }
            if let Err(error) = bar.append(&submenu) {
                tracing::warn!(%error, menu = top.title, "a menu could not be built");
            }
            if top.title == "Help" {
                submenu.set_as_help_menu_for_nsapp();
            }
        }
        bar.init_for_nsapp();
        self.bar = bar;
        self.handles = handles;
    }

    fn refresh(&mut self, model: &[Menu], quit_keys: &Option<String>, listening: bool) {
        fn walk(items: &[Item], listening: bool, force: bool, handles: &mut HashMap<String, (Handle, Shown)>) {
            for item in items {
                let Some((id, shown, children)) = shown_of(item) else { continue };
                if let Some((handle, before)) = handles.get_mut(&id) {
                    update(handle, before, &shown, listening, force);
                }
                if let Some(children) = children {
                    walk(children, listening, force, handles);
                }
            }
        }
        // Starting or stopping to listen changes every item's keys, even where nothing else changed.
        let force = listening != self.listening;
        for top in model {
            walk(&top.items, listening, force, &mut self.handles);
        }
        if let Some((handle, before)) = self.handles.get_mut(QUIT) {
            let shown = Shown { keys: quit_keys.clone(), ..before.clone() };
            update(handle, before, &shown, listening, force);
        }
    }
}

/// The ids and kinds of every item in order: when this changes, the menu bar is rebuilt.
fn structure_of(model: &[Menu]) -> Vec<String> {
    fn walk(items: &[Item], out: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Separator => out.push(String::from("-")),
                Item::Action { id, checked, .. } => out.push(format!("{}{id}", if checked.is_some() { "c:" } else { "a:" })),
                Item::Submenu { id, items, .. } => {
                    out.push(format!("s:{id}"));
                    walk(items, out);
                    out.push(String::from("end"));
                }
            }
        }
    }
    // A new language rebuilds the menu bar, so the menus' own titles follow it.
    let mut out = vec![format!("language:{}", bodo_ui::i18n::language().code())];
    for top in model {
        out.push(format!("m:{}", top.title));
        walk(&top.items, &mut out);
    }
    out
}

/// A menu label, English in the model, as the interface's language shows it.
fn shown(label: &str) -> String {
    bodo_ui::i18n::lookup(label).map_or_else(|| label.to_owned(), str::to_owned)
}

/// An item's id, what it shows, and its children if it is a submenu.
fn shown_of(item: &Item) -> Option<(String, Shown, Option<&[Item]>)> {
    match item {
        Item::Separator => None,
        Item::Action { id, label, keys, enabled, checked, .. } => {
            // File > Quit shows no keys: the app menu's Quit has them.
            let keys = if id == "file.quit" { None } else { keys.clone() };
            Some((id.clone(), Shown { label: shown(label), keys, enabled: *enabled, checked: *checked }, None))
        }
        Item::Submenu { id, label, items, enabled, .. } => {
            Some((id.clone(), Shown { label: shown(label), keys: None, enabled: *enabled, checked: None }, Some(items.as_slice())))
        }
    }
}

/// Adds `item` to `parent`, recording its handle.
fn add(parent: &Submenu, item: &Item, listening: bool, handles: &mut HashMap<String, (Handle, Shown)>) {
    let result = match item {
        Item::Separator => parent.append(&PredefinedMenuItem::separator()),
        Item::Action { .. } | Item::Submenu { .. } => {
            let Some((id, shown, children)) = shown_of(item) else { return };
            let keys = accelerator(shown.keys.as_deref(), listening);
            let (handle, appended) = match (children, shown.checked) {
                (Some(children), _) => {
                    let submenu = Submenu::with_id(id.as_str(), &shown.label, shown.enabled);
                    for child in children {
                        add(&submenu, child, listening, handles);
                    }
                    let appended = parent.append(&submenu);
                    (Handle::Sub(submenu), appended)
                }
                (None, Some(checked)) => {
                    let check = CheckMenuItem::with_id(id.as_str(), &shown.label, shown.enabled, checked, keys);
                    let appended = parent.append(&check);
                    (Handle::Check(check), appended)
                }
                (None, None) => {
                    let plain = MenuItem::with_id(id.as_str(), &shown.label, shown.enabled, keys);
                    let appended = parent.append(&plain);
                    (Handle::Item(plain), appended)
                }
            };
            handles.insert(id, (handle, shown));
            appended
        }
    };
    if let Err(error) = result {
        tracing::warn!(%error, "a menu item could not be added");
    }
}

/// Brings `handle` from showing `before` to showing `now`; `force` updates it even when nothing it shows changed.
fn update(handle: &mut Handle, before: &mut Shown, now: &Shown, listening: bool, force: bool) {
    if before == now && !force {
        return;
    }
    let keys = accelerator(now.keys.as_deref(), listening);
    let result = match handle {
        Handle::Item(item) => {
            if before.label != now.label {
                item.set_text(&now.label);
            }
            item.set_enabled(now.enabled);
            item.set_accelerator(keys)
        }
        Handle::Check(item) => {
            if before.label != now.label {
                item.set_text(&now.label);
            }
            item.set_enabled(now.enabled);
            item.set_checked(now.checked.unwrap_or(false));
            item.set_accelerator(keys)
        }
        Handle::Sub(submenu) => {
            if before.label != now.label {
                submenu.set_text(&now.label);
            }
            submenu.set_enabled(now.enabled);
            Ok(())
        }
    };
    if let Err(error) = result {
        tracing::warn!(%error, "a menu item's keys could not change");
    }
    *before = now.clone();
}

/// The native accelerator for `keys`, as Bodo writes them, or `None` when the menu bar should leave them to egui:
/// while listening for new keys, for the text keys, and for plain keys without a modifier other than function keys.
fn accelerator(keys: Option<&str>, listening: bool) -> Option<Accelerator> {
    let keys = keys?;
    if listening || FOR_TEXT.contains(&keys) {
        return None;
    }
    let key = keys.rsplit('+').next().unwrap_or_default();
    let function_key = key.len() > 1 && key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit());
    if !keys.contains('+') && !function_key {
        return None;
    }
    keys.replace("Mod", "CmdOrCtrl").parse::<Accelerator>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_skip_text_keys_and_plain_letters() {
        assert!(accelerator(Some("Mod+S"), false).is_some());
        assert!(accelerator(Some("Mod+Shift+S"), false).is_some());
        assert!(accelerator(Some("F5"), false).is_some(), "function keys toggle windows from the menu");
        assert!(accelerator(Some("Mod+C"), false).is_none(), "copy stays with text fields");
        assert!(accelerator(Some("P"), false).is_none(), "plain letters are tools and notes");
        assert!(accelerator(Some("Mod+S"), true).is_none(), "the shortcuts dialog is listening");
        assert!(accelerator(None, false).is_none());
    }

    #[test]
    fn the_structure_names_every_item_in_order() {
        let model = vec![Menu { title: "File", items: vec![Item::Separator] }];
        assert_eq!(structure_of(&model), [String::from("language:en"), String::from("m:File"), String::from("-")]);
        // Another language changes the structure, so the menu is built again with the new labels.
        bodo_ui::i18n::set_language(bodo_ui::i18n::Language::Turkish);
        assert_eq!(structure_of(&model)[0], "language:tr");
        bodo_ui::i18n::set_language(bodo_ui::i18n::Language::English);
    }
}
