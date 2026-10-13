# ADR 0033: Keyboard use, interface sizes, high contrast and translations

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

[project.md section 14.7](../../project.md#147-accessibility-and-localization) asks for screen readers through AccessKit with full keyboard navigation, an interface size from 75% to 200%, a high-contrast theme, color-blind-safe meters and clips, and every string through `t("English text")` with TSV catalogs. The owner asked that M4 end with no gap there, and added that everything looked too small: sizes should come from one CSS-like `rem` unit and follow the screen's width.

Before this, sizes were scattered point values around a 12.5-point body text, Kit's knobs, faders, LEDs and meters had fixed sizes, the custom widgets told AccessKit little or nothing, shortcuts stopped working whenever any widget had keyboard focus, and every string was English in the code.

## Decision

### Sizes

- **One unit.** `theme::REM` is 14 points, the body text, and `theme::rem(n)` is `n` of it rounded to whole points so edges stay on pixels. Fonts (`theme::size`), rows, title bars, spacing, margins, panel and window widths, and the app's custom widgets are all written in `rem`. Kit stays free of the DAW: its widgets take an optional size and otherwise size themselves from `bodo_kit_ui::em(ui)`, the host's body text size.
- **The interface size is egui's zoom factor.** `Prefs.ui_scale` is `None` for Auto (the default) or a fixed size from 75% to 200%. Auto is the screen's width in points over 1440, clamped to that range and rounded to 5% steps, so a 1440-point laptop gets 100% and a 1920-point monitor 135%; the screen is the monitor's size when the platform reports it and the window's otherwise. The Options menu, the Interface settings dialog and `ui.set_ui_scale` choose it, and Mod+= / Mod+- step through the fixed sizes from the size showing, and Mod+0 goes back to Auto. egui's own zoom keys are off so ours own them. The preference is saved with the others.

### Contrast and color

- **A third look.** `ThemePref` is System, Light, Dark or High contrast; `theme::Look` is what it resolves to. The high-contrast tokens put near-white text on black with yellow accents, outline every control, and thicken strokes. A test checks that every text and background pair of every look meets WCAG AA (4.5:1), and that every pair of the high-contrast look meets AAA (7:1).
- **Color-blind-safe colors are a switch, not a theme.** With `Prefs.color_safe`, the meters use blue, yellow and orange-red from Okabe and Ito's published palette instead of green, yellow and red, and the sixteen clip and channel hues map to a set built from that palette, so no two neighbors differ only in red against green. Tests check that the meter's levels differ in lightness, not just hue.
- **The focus ring** is the accent token at 1.5 points, from the same tokens in every look.

### Keyboard and screen readers

- **Tab order is drawing order:** the menus, the transport and the rest of the top bar, the Browser, then each window's title bar and its controls. Pointer-only handles (window edges and bodies, track headers and height edges, the lane divider) don't take focus.
- **F10** (`ui.focus_menu`) puts focus on the File menu; Space or Enter opens a focused menu, the arrows move in it and Escape closes it. **Mod+F6** (`ui.next_window`) brings the next open window forward and focuses its title bar, where Enter maximizes or restores it. Both are editable shortcuts ([ADR 0032](0032-detached-windows-layouts-menus-and-shortcuts.md)).
- **Escape** closes the dialogs, the palette and popups.
- **Kit widgets drive from the keyboard.** A focused knob or fader steps with the arrows, takes big steps with Page Up and Page Down, goes to its ends with Home and End, and resets with Delete; a focused step toggles with Space and Enter and the arrows move along its row. Each reports its role, label and value text, such as "-6.0 dB", to AccessKit.
- **Shortcuts while something has focus.** Only a text field takes every key. Another focused control keeps the keys it uses itself (Space, Enter, the arrows, page keys, Home, End, Delete, Backspace, Tab and Escape) and everything else still runs its shortcut.
- **Custom elements get names.** When an element the control channel records told AccessKit nothing, `control::record` gives it its label, translated where the catalogs have it, and a widget type from its role.

### Translations

- **English text is the key.** `i18n::t("Save")` returns the translation or the English; `tf!("Delete {name}", name = channel.name)` translates, then fills named placeholders. `i18n::en("Major")` marks English that is translated where it shows, such as an enum's name that is also an id. Helpers that receive English and show it translate it themselves: `BodoApp::hint`, `set_status`, `windows::icon_button`, `painted_button`, `placeholder`, the menu model, window titles (piece by piece, so names stay), the palette's command titles and summaries, and the channel windows' captions.
- **Catalogs** are `crates/bodo-ui/i18n/<code>.tsv`, `English<TAB>translation` with `\t`, `\n` and `\\` escapes, built into the binary and parsed once. A missing key shows in English. Turkish (`tr.tsv`) ships complete.
- **The language** is `Prefs.language`: English (the default), Turkish, or System, which follows the OS locale through the `sys-locale` crate (MIT OR Apache-2.0). It is per thread, so the interface (on one thread) and tests running side by side each have their own. The Options menu, the Interface settings dialog and `ui.set_language` choose it, and the native menu rebuilds when it changes.
- **What stays English:** the control channel's element labels and MCP, so agents and tests find the same elements in every language; command ids and params; error messages commands return; and names that are document data, such as "Track 3". Undo steps the engine names, such as "Set Kick volume", show by their command's translated title, since the engine builds them in English.
- **Tests keep the catalogs honest.** `tests/i18n.rs` reads the sources and finds keys the way the code passes them (the literal given to `t`, `tf!`, `en`, or a translating helper), adds every command title and summary from the registry, and checks that every key is in every catalog, that no catalog keeps a key nothing uses, that no key is listed twice, that each translation has its key's placeholders, that every `tf!` fills exactly the placeholders its text has, and that no egui label, button, hint or the like takes an English literal. Files another stream is changing (`mixer.rs`, `browser/`) are listed as pending and join the checks once that work lands.

## Consequences

- Sizes change in one place, and the interface fits the screen without the user choosing. Snapshot tests pin the size at 100% so they stay the same everywhere.
- Contributors write English and wrap it; the catalog test names every key a translation lacks, so a new string can't ship untranslated. Adding a language is a TSV file and a `Language` variant.
- A translation that is much longer than its English can crowd a header; window titles truncate, and the Turkish snapshots of the main window, a beat and the Sampler watch for it.
- The engine's own labels and error messages stay English. Translating them needs labels the engine builds from keys and values instead of finished text, which is follow-up work for the engine, with the same catalogs.
- Context menus open with the pointer only; everything in them is also in the menus or the palette, which the keyboard reaches.

## Alternatives considered

- **A gettext or Fluent setup:** more machinery than English keys and a flat TSV need, and pixel3d's TSV catalogs already worked well for contributors.
- **Keys as identifiers (`menu.file.save`):** a missing translation would show an identifier instead of English, and every string would need two edits.
- **A global language setting:** tests in other languages run side by side with English ones and would disturb each other.
- **Scaling fonts only:** controls and spacing would stay small; egui's zoom factor scales everything together.
- **Auto from the window's width:** the interface would change size as the window is resized; the monitor's width is stable.
