//! The top bar (project.md section 14.1): the menu and transport on the first row; the hint bar, layouts, window
//! buttons, snap and the pattern selector on the second.

use std::path::PathBuf;

use bodo_engine::PlayMode;
use bodo_engine::params::STEP;
use bodo_kit_core::Tick;
use bodo_kit_ui::icons;
use egui::{Align, FontId, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use serde_json::json;

use crate::theme::{self, Tokens, size};
use crate::windows::{Preset, WindowKind, icon_button, painted_button};
use crate::{BodoApp, Snap, control, dialogs};

/// Draws the top bar.
pub(crate) fn show(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    egui::Panel::top("bodo-top-bar").exact_size(size::TOP_ROW * 2.0).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let rect = ui.max_rect();
        let first = Rect::from_min_size(rect.min, vec2(rect.width(), size::TOP_ROW));
        let second = Rect::from_min_size(first.left_bottom(), vec2(rect.width(), size::TOP_ROW));
        ui.painter().rect_filled(first, 0.0, tk.chrome_dark);
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, tk.border));
        let mut row = ui.new_child(UiBuilder::new().max_rect(first.shrink2(vec2(8.0, 4.0))).layout(Layout::left_to_right(Align::Center)));
        menus(app, &mut row);
        row.add_space(12.0);
        transport(app, &mut row);
        let mut row = ui.new_child(UiBuilder::new().max_rect(second.shrink2(vec2(8.0, 4.0))).layout(Layout::left_to_right(Align::Center)));
        tools(app, &mut row);
    });
}

fn menus(app: &mut BodoApp, ui: &mut Ui) {
    ui.menu_button("File", |ui| {
        if ui.button("New").clicked() {
            dialogs::guard_unsaved(app, dialogs::Pending::New);
        }
        if ui.button("Open...").clicked() {
            dialogs::guard_unsaved(app, dialogs::Pending::Open(None));
        }
        let recent: Vec<PathBuf> = app.prefs.recent.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Open recent", |ui| {
                for path in recent {
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                        dialogs::guard_unsaved(app, dialogs::Pending::Open(Some(path)));
                    }
                }
            });
        });
        ui.separator();
        if ui.button("Save").clicked() {
            dialogs::save(app, false);
        }
        if ui.button("Save as...").clicked() {
            dialogs::save(app, true);
        }
        ui.separator();
        if ui.button("Import MIDI file...").clicked() {
            dialogs::import_midi(app);
        }
        if ui.button("Export song as MIDI...").clicked() {
            dialogs::export_midi(app, false);
        }
        if ui.button("Export pattern as MIDI...").clicked() {
            dialogs::export_midi(app, true);
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            dialogs::guard_unsaved(app, dialogs::Pending::Quit);
        }
    });
    ui.menu_button("Edit", |ui| {
        let undo = app.session.undo_labels().last().map(|l| format!("Undo {l}"));
        if ui.add_enabled(undo.is_some(), egui::Button::new(undo.unwrap_or_else(|| "Undo".into()))).clicked() {
            let _ = app.run("edit.undo", json!({}));
        }
        let redo = app.session.redo_labels().next().map(|l| format!("Redo {l}"));
        if ui.add_enabled(redo.is_some(), egui::Button::new(redo.unwrap_or_else(|| "Redo".into()))).clicked() {
            let _ = app.run("edit.redo", json!({}));
        }
        ui.separator();
        if ui.button("Select all clips").clicked() {
            let _ = app.run("edit.select_all", json!({ "what": "clips" }));
        }
        if ui.button("Deselect").clicked() {
            let _ = app.run("edit.deselect", json!({}));
        }
        ui.separator();
        if ui.button("Command palette...").clicked() {
            let _ = app.run("ui.palette", json!({}));
        }
    });
    ui.menu_button("Add", |ui| {
        add_channel_menu(app, ui);
        if ui.button("Pattern").clicked() {
            let _ = app.run("pattern.new", json!({}));
        }
        if ui.button("Playlist track").clicked() {
            let _ = app.run("playlist.add_track", json!({}));
        }
        if ui.button("Mixer insert").clicked() {
            let _ = app.run("mixer.add_insert", json!({}));
        }
    });
    ui.menu_button("Patterns", |ui| {
        if ui.button("New pattern").clicked() {
            let _ = app.run("pattern.new", json!({}));
        }
        if ui.button("Clone pattern").clicked() {
            let _ = app.run("pattern.clone", json!({}));
        }
        if ui.button("Rename pattern...").clicked() {
            dialogs::rename_pattern(app);
        }
        if ui.button("Delete pattern").clicked() {
            let _ = app.run("pattern.delete", json!({}));
        }
        ui.separator();
        if ui.button("Next pattern").clicked() {
            let _ = app.run("ui.select_pattern", json!({ "offset": 1 }));
        }
        if ui.button("Previous pattern").clicked() {
            let _ = app.run("ui.select_pattern", json!({ "offset": -1 }));
        }
    });
    ui.menu_button("View", |ui| {
        for kind in WindowKind::ALL {
            let mut open = app.prefs.layout.is_open(kind);
            if ui.checkbox(&mut open, kind.title()).clicked() {
                let _ = app.run("ui.toggle_window", json!({ "window": kind.id() }));
            }
        }
        let mut browser = app.prefs.layout.browser;
        if ui.checkbox(&mut browser, "Browser").clicked() {
            let _ = app.run("ui.toggle_browser", json!({}));
        }
        ui.separator();
        ui.menu_button("Layout", |ui| layout_menu(app, ui));
        ui.menu_button("Theme", |ui| {
            for theme in crate::theme::ThemePref::ALL {
                if ui.radio(app.prefs.theme == theme, theme.label()).clicked() {
                    let _ = app.run("ui.set_theme", json!({ "theme": theme.label() }));
                }
            }
        });
    });
    ui.menu_button("Options", |ui| {
        if ui.button("Audio settings...").clicked() {
            app.view.dialogs.audio = true;
        }
        if ui.button("Add sample folder...").clicked() {
            dialogs::add_sample_folder(app);
        }
        ui.menu_button("Autosave", |ui| {
            for minutes in [0, 1, 5, 10, 30] {
                let label = if minutes == 0 { String::from("Off") } else { format!("Every {minutes} minutes") };
                if ui.radio(app.prefs.autosave_minutes == minutes, label).clicked() {
                    app.prefs.autosave_minutes = minutes;
                }
            }
        });
    });
    ui.menu_button("Tools", |ui| {
        if ui.button("Command palette...").clicked() {
            let _ = app.run("ui.palette", json!({}));
        }
    });
    ui.menu_button("Help", |ui| {
        if ui.button("Keyboard shortcuts").clicked() {
            app.view.dialogs.shortcuts = true;
        }
        if ui.button("About Bodo").clicked() {
            app.view.dialogs.about = true;
        }
    });
}

/// The Add > Channel menu: the factory drum kit, the user's folders, or a file.
pub(crate) fn add_channel_menu(app: &mut BodoApp, ui: &mut Ui) {
    ui.menu_button("Channel", |ui| {
        let sounds = bodo_engine::library::search(app.session.env(), "", 64);
        for sound in sounds.iter().filter(|s| s.path.starts_with(bodo_model::FACTORY_PREFIX)) {
            if ui.button(&sound.name).clicked() {
                let _ = app.run("channel.add", json!({ "sample": sound.path }));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Empty sampler").clicked() {
            let _ = app.run("channel.add", json!({}));
        }
        if ui.button("From file...").clicked() {
            dialogs::add_channels_from_files(app);
        }
    });
}

fn layout_menu(app: &mut BodoApp, ui: &mut Ui) {
    for preset in Preset::ALL {
        if ui.button(preset.label()).clicked() {
            let _ = app.run("ui.set_layout", json!({ "layout": preset.label() }));
        }
    }
}

/// A toggle button with text, accent-filled when on.
fn text_toggle(ui: &mut Ui, text: &str, on: bool, label: &str) -> egui::Response {
    let tk = Tokens::get();
    let fill = if on { tk.accent_soft } else { tk.control };
    let color = if on { tk.accent } else { tk.text_dim };
    let response = ui.add(egui::Button::new(RichText::new(text).color(color).size(size::SMALL).strong()).fill(fill).min_size(vec2(44.0, 22.0)));
    control::record(ui, &response, label, "button", Some(on));
    response
}

fn transport(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    let transport = *app.session.transport();
    let song = transport.mode == PlayMode::Song;
    let response = text_toggle(ui, if song { "SONG" } else { "PAT" }, song, "Play mode");
    app.hint(&response, "Pattern mode loops the current pattern; Song mode plays the playlist");
    if response.clicked() {
        let _ = app.run("transport.set_mode", json!({ "mode": if song { "pattern" } else { "song" } }));
    }
    let play = icon_button(ui, if transport.playing { icons::PLAYER_PAUSE } else { icons::PLAYER_PLAY }, "Play", transport.playing);
    app.hint(&play, "Play or stop (Space)");
    if play.clicked() {
        let _ = app.run("transport.toggle_play", json!({}));
    }
    let stop = icon_button(ui, icons::PLAYER_STOP, "Stop", false);
    app.hint(&stop, "Stop and return to the start");
    if stop.clicked() {
        let _ = app.run("transport.stop", json!({}));
    }
    record_button(app, ui, transport.recording);
    ui.add_space(4.0);
    let metronome = icon_button(ui, icons::METRONOME, "Metronome", transport.metronome);
    app.hint(&metronome, "Metronome: clicks every beat while playing and recording");
    if metronome.clicked() {
        let _ = app.run("transport.metronome", json!({ "on": !transport.metronome }));
    }
    let count_in = painted_button(ui, "Count-in", transport.count_in, |painter, rect, color| {
        // Four beats, the first one stressed: a bar counted in.
        for beat in 0..4 {
            let center = egui::pos2(rect.center().x + (beat as f32 - 1.5) * 4.5, rect.center().y);
            painter.circle_filled(center, if beat == 0 { 2.0 } else { 1.4 }, color);
        }
    });
    app.hint(&count_in, "Count-in: a bar of clicks before recording starts");
    if count_in.clicked() {
        let _ = app.run("transport.count_in", json!({ "on": !transport.count_in }));
    }
    ui.add_space(10.0);

    // Tempo: drag or type.
    let project = std::sync::Arc::clone(app.session.project());
    let mut bpm = project.tempo.bpm_at(Tick::ZERO);
    let tempo = ui.add(egui::DragValue::new(&mut bpm).range(10.0..=999.0).speed(0.1).fixed_decimals(3).suffix(" BPM"));
    control::record(ui, &tempo, "Tempo", "value", None);
    app.hint(&tempo, "The tempo: drag, or double-click to type");
    if tempo.changed() {
        app.run_gesture(&tempo, "project.set_tempo", json!({ "bpm": bpm }));
    }
    ui.add_space(10.0);

    // Song position, as bars:steps:ticks or minutes:seconds.
    let text = if app.view.dialogs.clock_seconds {
        let seconds = project.tempo.seconds_at(transport.position).0;
        format!("{}:{:02}:{:02}", (seconds / 60.0) as i64, (seconds % 60.0) as i64, ((seconds * 100.0) % 100.0) as i64)
    } else {
        let at = project.meter.position(transport.position);
        let in_bar = transport.position.0 - project.meter.bar_start(at.bar).0;
        format!("{}:{:02}:{:02}", at.bar + 1, in_bar / STEP.0 + 1, (in_bar % STEP.0) * 100 / STEP.0)
    };
    let clock = ui.add(egui::Label::new(RichText::new(text).font(FontId::monospace(size::TIME)).color(tk.accent)).sense(Sense::click()));
    control::record(ui, &clock, "Song position", "value", None);
    app.hint(&clock, "The song position; click to switch between bars:steps:ticks and minutes:seconds");
    if clock.clicked() {
        app.view.dialogs.clock_seconds = !app.view.dialogs.clock_seconds;
    }
    ui.add_space(10.0);

    // The Song mode loop, when there is one.
    if let Some((start, end)) = transport.loop_region {
        let text = format!("Loop {} to {}", short_position(&project, start), short_position(&project, end));
        let looped = ui.add(egui::Button::new(RichText::new(text).size(size::SMALL).color(tk.accent)).fill(tk.accent_soft));
        control::record(ui, &looped, "Loop region", "button", Some(true));
        app.hint(&looped, "Song mode loops this region; click to stop looping");
        if looped.clicked() {
            let _ = app.run("transport.set_loop", json!({ "off": true }));
        }
        ui.add_space(10.0);
    }

    // Master volume and level.
    let mut db = project.mixer.master.volume_db;
    let master = ui.add_sized(vec2(90.0, 18.0), egui::Slider::new(&mut db, -60.0..=6.0).show_value(false));
    control::record(ui, &master, "Master volume", "slider", None);
    app.hint(&master, format!("Master volume: {db:.1} dB"));
    if master.changed() {
        let db = if db <= -59.9 { -144.0 } else { db };
        app.run_gesture(&master, "mixer.set_volume", json!({ "insert": "master", "db": db }));
    }
    let (left, right) = app.audio.levels();
    let (rect, meter) = ui.allocate_exact_size(vec2(64.0, 18.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, tk.field);
    for (i, level) in [left, right].into_iter().enumerate() {
        let lane = Rect::from_min_size(rect.min + vec2(2.0, 2.0 + i as f32 * 7.0), vec2(rect.width() - 4.0, 6.0));
        let fill = bodo_kit_ui::meter::db_to_level(20.0 * level.max(1e-6).log10());
        let color = if level >= 1.0 {
            tk.record
        } else if level > 0.5 {
            tk.warn
        } else {
            tk.ok
        };
        painter.rect_filled(Rect::from_min_size(lane.min, vec2(lane.width() * fill, lane.height())), 1.0, color);
    }
    app.hint(&meter, "The master output level");
    ui.add_space(10.0);

    // Performance.
    let status = app.audio.stream_status();
    let text = if app.audio.is_on() { format!("CPU {:>3.0}%  {} dropouts", status.load * 100.0, status.xruns) } else { String::from("No sound") };
    let color = if !app.audio.is_on() || status.xruns > 0 { tk.warn } else { tk.text_dim };
    let perf = ui.add(egui::Label::new(RichText::new(text).monospace().size(size::SMALL).color(color)).sense(Sense::hover()));
    let audio_status = app.audio.status().to_owned();
    app.hint(&perf, format!("Audio: {audio_status}"));
}

/// The record button: red while recording, blinking while a count-in clicks.
fn record_button(app: &mut BodoApp, ui: &mut Ui, recording: bool) {
    let tk = Tokens::get();
    let counting = recording && app.audio.is_counting_in();
    if counting {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let lit = !counting || ui.input(|i| i.time).fract() < 0.5;
    let record = painted_button(ui, "Record", recording, |painter, rect, color| {
        let color = match (recording, lit) {
            (true, true) => tk.record,
            (true, false) => theme::faded(tk.record, 0.3),
            (false, _) => color,
        };
        icons::PLAYER_RECORD.paint(painter, Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), color);
    });
    let hint = if counting {
        "Counting in: recording starts after this bar"
    } else if recording {
        "Recording: click to stop; the takes land on the playlist"
    } else {
        "Record: plays and records the armed mixer inserts and the notes you play"
    };
    app.hint(&record, hint);
    if record.clicked() {
        let _ = app.run("transport.record", json!({}));
    }
}

/// `tick` as bar:beat, with the ticks only when it is between beats.
fn short_position(project: &bodo_model::Project, tick: Tick) -> String {
    let at = project.meter.position(tick);
    if at.tick == 0 { format!("{}:{}", at.bar + 1, at.beat + 1) } else { format!("{}:{}:{}", at.bar + 1, at.beat + 1, at.tick) }
}

fn tools(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    // The hint bar: what the control under the pointer does, or the last command's result.
    let (rect, _) = ui.allocate_exact_size(vec2(340.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, tk.field);
    let (text, color) = match (&app.view.hint, app.status()) {
        (Some(hint), _) => (hint.clone(), tk.text_dim),
        (None, Some((status, true))) => (status.to_owned(), tk.error),
        (None, Some((status, false))) => (status.to_owned(), tk.text_dim),
        (None, None) => (String::new(), tk.text_dim),
    };
    let galley = ui.painter().layout(text, FontId::proportional(size::SMALL), color, rect.width() - 12.0);
    ui.painter().with_clip_rect(rect).galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, color);
    ui.add_space(8.0);

    let layouts = icon_button(ui, icons::LAYOUT, "Layouts", false);
    app.hint(&layouts, "Layouts: arrange the windows");
    egui::Popup::menu(&layouts).show(|ui| layout_menu(app, ui));
    ui.add_space(4.0);
    let browser = icon_button(ui, icons::FOLDER, "Browser", app.prefs.layout.browser);
    app.hint(&browser, "Show or hide the Browser");
    if browser.clicked() {
        let _ = app.run("ui.toggle_browser", json!({}));
    }
    for kind in WindowKind::ALL {
        let open = app.prefs.layout.is_open(kind);
        let button = icon_button(ui, kind.icon(), kind.title(), open);
        app.hint(&button, format!("Show or hide the {} ({:?})", kind.title(), kind.key()));
        if button.clicked() {
            let _ = app.run("ui.toggle_window", json!({ "window": kind.id() }));
        }
    }
    ui.add_space(8.0);

    // Snap.
    let snap = app.view.snap;
    let combo = egui::ComboBox::from_id_salt("bodo-snap").width(70.0).selected_text(snap.label()).show_ui(ui, |ui| {
        for choice in Snap::ALL {
            if ui.selectable_label(snap == choice, choice.label()).clicked() {
                let _ = app.run("ui.set_snap", json!({ "snap": choice.label() }));
            }
        }
    });
    icons::MAGNET.paint(ui.painter(), Rect::from_center_size(combo.response.rect.left_center() - vec2(10.0, 0.0), vec2(12.0, 12.0)), tk.text_faint);
    app.hint(&combo.response, "Snap: the grid the playlist and piano roll place things on");
    ui.add_space(8.0);

    // The pattern selector.
    let project = std::sync::Arc::clone(app.session.project());
    let current = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).map(|p| (p.name.clone(), crate::theme::color(p.color)));
    let previous = icon_button(ui, icons::CHEVRON_LEFT, "Previous pattern", false);
    if previous.clicked() {
        let _ = app.run("ui.select_pattern", json!({ "offset": -1 }));
    }
    let (name, color) = current.unwrap_or_else(|| (String::from("No pattern"), tk.text_faint));
    let (swatch, _) = ui.allocate_exact_size(vec2(6.0, 18.0), Sense::hover());
    ui.painter().rect_filled(swatch, 2.0, color);
    let picker = egui::ComboBox::from_id_salt("bodo-pattern").width(110.0).selected_text(RichText::new(&name).color(tk.text)).show_ui(ui, |ui| {
        for pattern in project.patterns.values() {
            let selected = app.session.editor().pattern == Some(pattern.id);
            if ui.selectable_label(selected, &pattern.name).clicked() {
                let _ = app.run("ui.select_pattern", json!({ "pattern": pattern.id }));
            }
        }
    });
    control::record(ui, &picker.response, "Pattern selector", "combo", None);
    app.hint(&picker.response, "The current pattern: the channel rack edits it and Pattern mode plays it");
    let next = icon_button(ui, icons::CHEVRON_RIGHT, "Next pattern", false);
    if next.clicked() {
        let _ = app.run("ui.select_pattern", json!({ "offset": 1 }));
    }
    let new = icon_button(ui, icons::PLUS, "New pattern", false);
    app.hint(&new, "Make a new pattern");
    if new.clicked() {
        let _ = app.run("pattern.new", json!({}));
    }

    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new("bodo").color(tk.accent).size(size::HEADING).strong());
        ui.add_space(8.0);
        ui.add_enabled_ui(false, |ui| {
            let ai = icon_button(ui, icons::SPARKLES, "AI", false);
            app.hint(&ai, "The Bodo Assistant arrives in a later version");
            let market = icon_button(ui, icons::SHOPPING_BAG, "Marketplace", false);
            app.hint(&market, "The Marketplace arrives in a later version");
        });
        let help = icon_button(ui, icons::HELP, "Help", false);
        if help.clicked() {
            app.view.dialogs.shortcuts = true;
        }
        let mods = ui.input(|i| i.modifiers);
        let mac = crate::shortcuts::is_mac(ui.ctx());
        let (cmd, alt) = if mac { ("Cmd", "Opt") } else { ("Ctrl", "Alt") };
        for (on, name) in [(mods.alt, alt), (mods.command, cmd), (mods.shift, "Shift")] {
            let color = if on { tk.accent } else { tk.text_faint };
            ui.label(RichText::new(name).size(size::SMALL).color(color));
        }
    });
}
