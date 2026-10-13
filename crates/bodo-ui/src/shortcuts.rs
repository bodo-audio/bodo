//! Keyboard shortcuts: the registry's, the window keys (F5 to F9), the palette, and the playlist tools.
//!
//! Shortcuts are written like `Mod+Shift+Z`, where Mod is Cmd on macOS and Ctrl elsewhere. While a text field has
//! the keyboard, only shortcuts with Mod work, so typing never triggers a command.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;

use crate::BodoApp;
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

/// Whether `ctx` runs on macOS, which names its modifier keys Cmd and Opt.
pub(crate) fn is_mac(ctx: &egui::Context) -> bool {
    ctx.os() == egui::os::OperatingSystem::Mac
}

/// `spec` as this platform writes it: `Cmd+S` on macOS, `Ctrl+S` elsewhere.
pub(crate) fn display(ctx: &egui::Context, spec: &str) -> String {
    spec.replace("Mod", if is_mac(ctx) { "Cmd" } else { "Ctrl" })
}

/// Runs the shortcuts pressed this frame.
pub(crate) fn handle(app: &mut BodoApp, ctx: &egui::Context) {
    let typing = ctx.egui_wants_keyboard_input();
    let palette = parse("Mod+K");
    if palette.is_some_and(|p| ctx.input_mut(|i| i.consume_shortcut(&p))) {
        app.view.palette = if app.view.palette.is_some() { None } else { Some(crate::palette::PaletteState::default()) };
        return;
    }
    if app.view.palette.is_some() {
        return;
    }
    let shortcuts: Vec<(KeyboardShortcut, &'static str)> =
        app.session.registry().iter().filter_map(|c| c.shortcut.and_then(parse).map(|s| (s, c.id))).collect();
    for (shortcut, id) in shortcuts {
        // While a text field has the keyboard, only the file shortcuts work; copy, paste and undo belong to the field.
        let file_command = id.starts_with("project.");
        if (typing && !(shortcut.modifiers.command && file_command)) || !ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
            continue;
        }
        match id {
            "project.save" => {
                crate::dialogs::save(app, false);
            }
            "project.save_as" => {
                crate::dialogs::save(app, true);
            }
            "project.new" => crate::dialogs::guard_unsaved(app, crate::dialogs::Pending::New),
            "project.open" => crate::dialogs::guard_unsaved(app, crate::dialogs::Pending::Open(None)),
            _ => {
                let _ = app.run(id, json!({}));
            }
        }
    }
    if typing {
        return;
    }
    for kind in WindowKind::ALL {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, kind.key())) {
            let _ = app.run("ui.toggle_window", json!({ "window": kind.id() }));
        }
    }
    let piano = app.view.editor == WindowKind::PianoRoll && app.prefs.layout.is_open(WindowKind::PianoRoll);
    for tool in crate::playlist::Tool::ALL {
        if piano && !crate::piano_roll::TOOLS.contains(&tool) {
            continue;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, tool.key())) {
            let _ = app.run("ui.set_tool", json!({ "tool": tool.label() }));
        }
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace)) {
        if !app.session.editor().clips.is_empty() {
            let _ = app.run("clip.remove", json!({}));
        } else if !app.session.editor().notes.is_empty() && app.prefs.layout.is_open(WindowKind::PianoRoll) {
            let _ = app.run("note.remove", json!({}));
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
}
