# ADR 0032: Detached windows, saved layouts, the native menu and editable shortcuts

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

[ADR 0012](0012-window-model.md) set the window model: free windows that can be detached into OS windows, and layouts that record the arrangement and belong to the user. Until M4 only part of it existed: windows lived inside the workspace and there were four fixed presets. [project.md section 14.1](../../project.md#141-main-window-layout) also asks for a Master layout and custom layouts, and the owner asked that M4 leave no interface gap: the native macOS menu bar and editable shortcuts too. Shortcuts were scattered: the command registry's defaults, hard-coded window and tool keys, and the palette's key, each read in its own place.

## Decision

- **Detaching is a window state.** `WindowState` gains `detached` and `screen` (where the OS window was, in desktop points), both with serde defaults so older preferences load. A detached window is drawn through `egui::Context::show_viewport_immediate`, so it shares the app state with the main window and needs no synchronization; eframe shows it as a native window on the wgpu backend. Its title bar has an Attach button, and closing its OS window closes the window. The viewport is created where `screen` says, and its builder then stays fixed so egui never moves it back while the user drags it. Keys pressed in a detached window run through the same shortcut and typing-keyboard handlers, through the `WindowHost` trait. The Channel Rack can't detach (ADR 0012). Where a backend can't open more windows (`embed_viewports`), a detached window stays in the workspace with its Attach button; the headless tests run this way.
- **Layouts are the preferences' `layout` plus named copies.** `Prefs.layouts` holds the user's layouts by name, and `Prefs.layout_name` marks the one chosen last. Presets keep the Browser as it is; saved layouts restore everything, detached windows included. A saved layout can't take a preset's name. The Master preset fills the workspace with the mixer and selects the master insert. `ui.set_layout`, `ui.save_layout`, `ui.rename_layout`, `ui.delete_layout` and `ui.detach_window` give agents the same control.
- **One keymap.** `shortcuts::bindings` lists every binding: the palette, Quit, the window keys, the tool keys and every command of the session and the interface, each with its default. The user's changes are overrides in `Prefs.shortcuts` (binding id to keys, an empty string for none). The key handler, both menus, the hints and the palette read the effective keys from it. Bindings with more modifiers are matched first, since egui ignores extra Shift and Alt when matching.
- **The Keyboard shortcuts dialog edits it.** Clicking a binding listens for the next key press, which is taken out of the input so no widget or shortcut acts on it. Keys another binding has prompt a choice: take them over, which unbinds the other, or keep them where they are. Each changed binding has a Reset, and Reset all clears every override. `ui.set_shortcut` and `ui.reset_shortcuts` do the same for agents.
- **One menu model.** `bodo_ui::menu` builds File, Edit, Add, Patterns, View, Options, Tools and Help as data: stable ids, labels, actions, the effective keys, enabled and checked states. The top bar draws it with egui. On macOS, `apps/bodo` also builds a native menu bar from it with `muda` (Apache-2.0 OR MIT), after the app menu macOS expects. The native menu rebuilds only when its structure changes and otherwise updates items in place. It gives no native accelerator to the clipboard and undo keys, which text fields need, nor to plain keys other than function keys. While the shortcuts dialog listens, it gives up every accelerator so any key can be recorded.

## Consequences

- Windows can go to another monitor or full screen, and layouts bring them back there.
- Immediate viewports repaint with the main window. That costs a second render while a window is detached, which is acceptable for a handful of windows; deferred viewports would need the app state behind a lock.
- The headless tests can't open OS windows. They cover the detached state, the buttons and the layouts, and the rest is eframe's.
- A menu item, a hint and its shortcut can't disagree any more. A new menu item goes into `menu.rs`, not into the top bar's drawing code.
- Windows and Linux keep the menu in the window; a native menu there can come later through the same model.

## Alternatives considered

- **Deferred viewports for detached windows:** cheaper to repaint, but their callbacks are `'static`, so the whole app would need shared ownership and locking.
- **A separate menu definition for macOS:** quick, but two lists drift apart as commands are added.
- **Storing every binding in prefs:** a new command's default would never reach users who had saved prefs before it existed; overrides keep defaults flowing.
