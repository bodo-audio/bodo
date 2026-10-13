//! Keyboard shortcuts (project.md section 14.5, ADR 0032): the commands', the window keys (F5 to F9), the playlist
//! and piano roll tools, and the command palette.
//!
//! Every one is a [`Binding`] the user can change in the Keyboard shortcuts dialog. The changes live in
//! [`crate::Prefs::shortcuts`] as overrides of the defaults, and apply wherever a shortcut shows or works: the key
//! handler here, the menus (in the window and the native one), the hints and the palette.
//!
//! Shortcuts are written like `Mod+Shift+Z`, where Mod is Cmd on macOS and Ctrl elsewhere. While a text field has the
//! keyboard, only the file shortcuts with Mod work, so typing never triggers a command.

use std::collections::BTreeMap;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;

use crate::BodoApp;
use crate::playlist::Tool;
use crate::windows::WindowKind;

/// Parses `Mod+Shift+Z`, `Space` or `Delete` into a shortcut.
pub(crate) fn parse(spec: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in spec.split('+') {
        match part.trim() {
            "Mod" | "Cmd" | "Ctrl" => modifiers |= Modifiers::COMMAND,
            "Shift" => modifiers |= Modifiers::SHIFT,
            "Alt" | "Opt" => modifiers |= Modifiers::ALT,
            name => key = Key::from_name(name),
        }
    }
    key.map(|key| KeyboardShortcut::new(modifiers, key))
}

/// `shortcut` written the way Bodo stores it: `Mod+Shift+Alt+Key`.
pub fn format(shortcut: &KeyboardShortcut) -> String {
    let mut parts = Vec::with_capacity(4);
    if shortcut.modifiers.command || shortcut.modifiers.ctrl || shortcut.modifiers.mac_cmd {
        parts.push("Mod");
    }
    if shortcut.modifiers.shift {
        parts.push("Shift");
    }
    if shortcut.modifiers.alt {
        parts.push("Alt");
    }
    parts.push(shortcut.logical_key.name());
    parts.join("+")
}

/// `spec` in the stored form, so two ways of writing the same keys compare equal; `None` when it names no key.
pub fn normalize(spec: &str) -> Option<String> {
    parse(spec).map(|s| format(&s))
}

/// Whether `ctx` runs on macOS, which names its modifier keys Cmd and Opt.
pub(crate) fn is_mac(ctx: &egui::Context) -> bool {
    ctx.os() == egui::os::OperatingSystem::Mac
}

/// `spec` as this platform writes it: `Cmd+S` on macOS, `Ctrl+S` elsewhere.
pub(crate) fn display(ctx: &egui::Context, spec: &str) -> String {
    let mac = is_mac(ctx);
    spec.split('+')
        .map(|part| match part {
            "Mod" => {
                if mac {
                    "Cmd"
                } else {
                    "Ctrl"
                }
            }
            "Alt" if mac => "Opt",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Where a binding shows in the Keyboard shortcuts dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Bodo's own: the command palette.
    App,
    /// Showing and hiding windows.
    Windows,
    /// The playlist's and piano roll's tools.
    Tools,
    /// Every command of the session and the interface.
    Commands,
}

impl Group {
    /// Its heading in the dialog.
    pub fn label(self) -> &'static str {
        match self {
            Self::App => "Bodo",
            Self::Windows => "Windows",
            Self::Tools => "Tools",
            Self::Commands => "Commands",
        }
    }
}

/// What a binding does when its keys are pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Opens or closes the command palette.
    Palette,
    /// Closes Bodo, asking about unsaved changes first.
    Quit,
    /// Shows or hides a window.
    Window(WindowKind),
    /// Picks a tool of the playlist or the piano roll, whichever is in front.
    Tool(Tool),
    /// Runs a command with no params (it acts on the selection).
    Command(String),
}

/// A key binding the user can change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Its id in [`crate::Prefs::shortcuts`]: `app:palette`, `window:mixer`, `tool:draw` or `command:project.save`.
    pub id: String,
    /// What it does, for the dialog.
    pub title: String,
    /// Where it shows in the dialog.
    pub group: Group,
    /// What it does.
    pub action: Action,
    /// Its keys as Bodo ships them, if any.
    pub default: Option<String>,
    /// Its keys now: the user's choice, or the default. `None` when it has none.
    pub keys: Option<String>,
}

impl Binding {
    /// Whether the user changed it.
    pub fn changed(&self) -> bool {
        self.keys != self.default
    }
}

/// The keys of binding `id`, whose default is `default`, after the user's `overrides`. An empty override removes
/// the shortcut.
fn effective(overrides: &BTreeMap<String, String>, id: &str, default: Option<&str>) -> Option<String> {
    match overrides.get(id) {
        Some(keys) if keys.is_empty() => None,
        Some(keys) => normalize(keys),
        None => default.and_then(normalize),
    }
}

/// Every binding: the palette, the window keys, the tool keys, then every command of the session and the
/// interface, each with its default and its keys now.
///
/// Complexity: O(c log c) in the commands, plus a lookup per binding in the overrides.
pub fn bindings(app: &BodoApp) -> Vec<Binding> {
    let overrides = &app.prefs.shortcuts;
    let mut out = Vec::with_capacity(app.session.registry().iter().count() + 24);
    let mut push = |id: String, title: String, group: Group, action: Action, default: Option<String>| {
        let keys = effective(overrides, &id, default.as_deref());
        out.push(Binding { id, title, group, action, default: default.as_deref().and_then(normalize), keys });
    };
    push(String::from("app:palette"), String::from("Command palette"), Group::App, Action::Palette, Some(String::from("Mod+K")));
    push(String::from("app:quit"), String::from("Quit Bodo"), Group::App, Action::Quit, Some(String::from("Mod+Q")));
    for kind in WindowKind::ALL {
        let default = Some(kind.key().name().to_owned());
        push(format!("window:{}", kind.id()), format!("Show or hide the {}", kind.title()), Group::Windows, Action::Window(kind), default);
    }
    for tool in Tool::ALL {
        let title = format!("{} tool", capitalized(tool.label()));
        push(format!("tool:{}", tool.label()), title, Group::Tools, Action::Tool(tool), Some(tool.key().name().to_owned()));
    }
    let mut commands: Vec<(String, String, Option<String>)> =
        app.session.registry().iter().map(|c| (c.id.to_owned(), c.title.to_owned(), c.shortcut.map(str::to_owned))).collect();
    commands.extend(BodoApp::ui_commands().iter().map(|(id, title, _)| ((*id).to_owned(), (*title).to_owned(), None)));
    commands.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, title, default) in commands {
        push(format!("command:{id}"), title, Group::Commands, Action::Command(id), default);
    }
    out
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// The keys of command `id` now, whose default is `default`.
pub(crate) fn command_keys(app: &BodoApp, id: &str, default: Option<&str>) -> Option<String> {
    effective(&app.prefs.shortcuts, &format!("command:{id}"), default)
}

/// The keys of window `kind`'s show-or-hide binding now.
pub(crate) fn window_keys(app: &BodoApp, kind: WindowKind) -> Option<String> {
    effective(&app.prefs.shortcuts, &format!("window:{}", kind.id()), Some(kind.key().name()))
}

/// The keys of `tool` now.
pub(crate) fn tool_keys(app: &BodoApp, tool: Tool) -> Option<String> {
    effective(&app.prefs.shortcuts, &format!("tool:{}", tool.label()), Some(tool.key().name()))
}

/// A tool's hint, such as "Draw (P): ...", with the key it has now in place of its default, or without a key when
/// it has none.
pub(crate) fn tool_hint(app: &BodoApp, ctx: &egui::Context, tool: Tool, hint: &str) -> String {
    let default = format!("({})", tool.key().name());
    let now = tool_keys(app, tool).map(|keys| format!("({})", display(ctx, &keys)));
    match now {
        Some(now) => hint.replacen(&default, &now, 1),
        None => hint.replacen(&format!(" {default}"), "", 1),
    }
}

/// The other bindings that already use `keys`, besides `id`.
pub fn conflicts<'a>(bindings: &'a [Binding], id: &str, keys: &str) -> Vec<&'a Binding> {
    let Some(keys) = normalize(keys) else { return Vec::new() };
    bindings.iter().filter(|b| b.id != id && b.keys.as_deref() == Some(keys.as_str())).collect()
}

/// Gives binding `id` the keys `keys` (an empty string removes its shortcut), unbinding the others that use the same
/// keys when `steal` is set. Returns the bindings that conflict and were left alone otherwise.
pub fn assign(app: &mut BodoApp, id: &str, keys: &str, steal: bool) -> Result<Vec<String>, String> {
    let all = bindings(app);
    let binding = all.iter().find(|b| b.id == id).ok_or_else(|| format!("there is no shortcut `{id}`"))?;
    let keys = if keys.trim().is_empty() {
        String::new()
    } else {
        normalize(keys).ok_or_else(|| format!("`{keys}` names no key; write keys like Mod+Shift+S, F5 or P"))?
    };
    let clashing: Vec<String> = if keys.is_empty() { Vec::new() } else { conflicts(&all, id, &keys).iter().map(|b| b.id.clone()).collect() };
    if !clashing.is_empty() && !steal {
        return Err(format!("{keys} is already used by {}", clashing.join(", ")));
    }
    for other in &clashing {
        app.prefs.shortcuts.insert(other.clone(), String::new());
    }
    if binding.default.as_deref() == Some(keys.as_str()) || (keys.is_empty() && binding.default.is_none()) {
        app.prefs.shortcuts.remove(id);
    } else {
        app.prefs.shortcuts.insert(id.to_owned(), keys);
    }
    Ok(clashing)
}

/// Puts binding `id` back to its default, or every binding when `id` is `None`.
pub fn reset(app: &mut BodoApp, id: Option<&str>) {
    match id {
        Some(id) => {
            app.prefs.shortcuts.remove(id);
        }
        None => app.prefs.shortcuts.clear(),
    }
}

/// Runs the shortcuts pressed this frame.
pub(crate) fn handle(app: &mut BodoApp, ctx: &egui::Context) {
    // While the shortcuts dialog listens for new keys, they belong to it.
    if app.view.dialogs.recording_shortcut() {
        return;
    }
    let typing = ctx.egui_wants_keyboard_input();
    let mut active: Vec<(KeyboardShortcut, Action)> =
        bindings(app).into_iter().filter_map(|b| b.keys.as_deref().and_then(parse).map(|s| (s, b.action))).collect();
    // egui matches extra Shift and Alt loosely, so the combinations with more modifiers go first: Mod+Shift+Z before
    // Mod+Z.
    active.sort_by_key(|(s, _)| std::cmp::Reverse(usize::from(s.modifiers.command) + usize::from(s.modifiers.shift) + usize::from(s.modifiers.alt)));
    if let Some((palette, _)) = active.iter().find(|(_, a)| *a == Action::Palette)
        && ctx.input_mut(|i| i.consume_shortcut(palette))
    {
        app.view.palette = if app.view.palette.is_some() { None } else { Some(crate::palette::PaletteState::default()) };
        return;
    }
    if app.view.palette.is_some() {
        return;
    }
    let piano = app.view.editor == WindowKind::PianoRoll && app.prefs.layout.is_open(WindowKind::PianoRoll);
    for (shortcut, action) in active {
        let with_mod = shortcut.modifiers.command;
        // While a text field has the keyboard, only the file shortcuts with Mod work; the rest belong to the field.
        let allowed = match &action {
            Action::Command(id) => !typing || (with_mod && id.starts_with("project.")),
            Action::Quit => !typing || with_mod,
            Action::Palette => false,
            Action::Window(_) => !typing,
            Action::Tool(tool) => !typing && !(piano && !crate::piano_roll::TOOLS.contains(tool)),
        };
        if !allowed || !ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
            continue;
        }
        match action {
            Action::Window(kind) => {
                let _ = app.run("ui.toggle_window", json!({ "window": kind.id() }));
            }
            Action::Tool(tool) => {
                let _ = app.run("ui.set_tool", json!({ "tool": tool.label() }));
            }
            Action::Command(id) => run_command(app, &id),
            Action::Quit => crate::dialogs::guard_unsaved(app, crate::dialogs::Pending::Quit),
            Action::Palette => {}
        }
    }
    if typing {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace)) {
        if !app.session.editor().clips.is_empty() {
            let _ = app.run("clip.remove", json!({}));
        } else if !app.session.editor().notes.is_empty() && app.prefs.layout.is_open(WindowKind::PianoRoll) {
            let _ = app.run("note.remove", json!({}));
        }
    }
}

/// Runs command `id` as its shortcut does: the file commands through their dialogs, copy and cut on the playlist's
/// time selection when there is one, and the rest on the selection.
pub(crate) fn run_command(app: &mut BodoApp, id: &str) {
    match id {
        "project.save" => {
            crate::dialogs::save(app, false);
        }
        "project.save_as" => {
            crate::dialogs::save(app, true);
        }
        "project.new" => crate::dialogs::guard_unsaved(app, crate::dialogs::Pending::New),
        "project.open" => crate::dialogs::guard_unsaved(app, crate::dialogs::Pending::Open(None)),
        // With a time selection on the playlist in front, copy and cut take every track's clips in it.
        "edit.copy" | "edit.cut"
            if app.view.editor == WindowKind::Playlist
                && let Some((start, end)) = app.view.playlist.time_selection =>
        {
            let _ = app.run(id, json!({ "start": start, "end": end }));
        }
        _ => {
            let _ = app.run(id, json!({}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_with_modifiers() {
        let undo = parse("Mod+Shift+Z").unwrap();
        assert_eq!(undo.logical_key, Key::Z);
        assert!(undo.modifiers.command && undo.modifiers.shift);
        assert_eq!(parse("Space").map(|s| s.logical_key), Some(Key::Space));
        assert!(parse("Mod+Nothing").is_none());
    }

    #[test]
    fn keys_normalize_to_one_spelling() {
        assert_eq!(normalize("Shift+Cmd+z").as_deref(), Some("Mod+Shift+Z"), "any order and case");
        assert_eq!(normalize("Shift+Ctrl+Z").as_deref(), Some("Mod+Shift+Z"));
        assert_eq!(normalize("Opt+F5").as_deref(), Some("Alt+F5"));
        assert_eq!(normalize("P").as_deref(), Some("P"));
    }

    #[test]
    fn overrides_replace_or_remove_defaults() {
        let mut overrides = BTreeMap::new();
        assert_eq!(effective(&overrides, "command:project.save", Some("Mod+S")).as_deref(), Some("Mod+S"));
        overrides.insert(String::from("command:project.save"), String::from("Mod+Shift+W"));
        assert_eq!(effective(&overrides, "command:project.save", Some("Mod+S")).as_deref(), Some("Mod+Shift+W"));
        overrides.insert(String::from("command:project.save"), String::new());
        assert_eq!(effective(&overrides, "command:project.save", Some("Mod+S")), None);
    }
}
