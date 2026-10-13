//! The top bar (project.md section 14.1): the menu and transport on the first row; the hint bar, layouts, window
//! buttons, snap and the pattern selector on the second.

use bodo_engine::PlayMode;
use bodo_engine::params::STEP;
use bodo_kit_core::Tick;
use bodo_kit_ui::icons;
use egui::{Align, FontId, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use serde_json::json;

use crate::i18n::{en, t};
use crate::theme::{self, Tokens, rem, size};
use crate::windows::{WindowKind, icon_button, painted_button};
use crate::{BodoApp, Snap, control, dialogs, menu};

/// Draws the top bar.
pub(crate) fn show(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    egui::Panel::top("bodo-top-bar").exact_size(size::TOP_ROW * 2.0).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let rect = ui.max_rect();
        let first = Rect::from_min_size(rect.min, vec2(rect.width(), size::TOP_ROW));
        let second = Rect::from_min_size(first.left_bottom(), vec2(rect.width(), size::TOP_ROW));
        ui.painter().rect_filled(first, 0.0, tk.chrome_dark);
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, tk.border));
        let mut row = ui.new_child(UiBuilder::new().max_rect(first.shrink2(vec2(rem(0.64), rem(0.32)))).layout(Layout::left_to_right(Align::Center)));
        menus(app, &mut row);
        row.add_space(rem(0.96));
        transport(app, &mut row);
        let mut row =
            ui.new_child(UiBuilder::new().max_rect(second.shrink2(vec2(rem(0.64), rem(0.32)))).layout(Layout::left_to_right(Align::Center)));
        tools(app, &mut row);
    });
}

/// The menu bar, drawn from the same model as the native macOS menu.
fn menus(app: &mut BodoApp, ui: &mut Ui) {
    let model = menu::menus(app);
    let mut chosen = None;
    for (index, top) in model.iter().enumerate() {
        let opened = ui.menu_button(crate::i18n::t(top.title), |ui| {
            ui.set_min_width(rem(16.0));
            menu_items(ui, &top.items, &mut chosen);
        });
        control::record(ui, &opened.response, &format!("Menu {}", top.title), "menu", None);
        // F10 (or the key the user gave "Go to the menus") puts keyboard focus on the first menu.
        if index == 0 && app.view.focus_menu {
            opened.response.request_focus();
            app.view.focus_menu = false;
        }
    }
    if let Some(action) = chosen {
        menu::perform(app, action);
    }
}

/// Draws menu `items`, putting the one clicked in `chosen`.
pub(crate) fn menu_items(ui: &mut Ui, items: &[menu::Item], chosen: &mut Option<menu::Action>) {
    let tk = Tokens::get();
    for item in items {
        match item {
            menu::Item::Separator => {
                ui.separator();
            }
            // Menu labels are English in the model; they show translated, and the control channel records them as
            // they are.
            menu::Item::Submenu { label, items, enabled, .. } => {
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(crate::i18n::lookup(label).unwrap_or(label), |ui| menu_items(ui, items, chosen));
                });
            }
            menu::Item::Action { label, action, keys, enabled, checked, .. } => {
                // A checked item shows selected, in the accent color.
                let shown = crate::i18n::lookup(label).unwrap_or(label);
                let text = if *checked == Some(true) { RichText::new(shown).color(tk.accent) } else { RichText::new(shown) };
                let mut button = egui::Button::new(text).selected(*checked == Some(true));
                if let Some(keys) = keys {
                    button = button.shortcut_text(crate::shortcuts::display(ui.ctx(), keys));
                }
                let response = ui.add_enabled(*enabled, button);
                control::record(ui, &response, label, "menu-item", *checked);
                if response.clicked() {
                    *chosen = Some(action.clone());
                    ui.close();
                }
            }
        }
    }
}

/// The Add > Channel menu: the factory drum kit, the user's folders, or a file.
pub(crate) fn add_channel_menu(app: &mut BodoApp, ui: &mut Ui) {
    ui.menu_button(t("Channel"), |ui| {
        let sounds = bodo_engine::library::search(app.session.env(), "", 64);
        for sound in sounds.iter().filter(|s| s.path.starts_with(bodo_model::FACTORY_PREFIX)) {
            if ui.button(&sound.name).clicked() {
                let _ = app.run("channel.add", json!({ "sample": sound.path }));
                ui.close();
            }
        }
        ui.separator();
        if ui.button(t("Empty sampler")).clicked() {
            let _ = app.run("channel.add", json!({}));
        }
        if ui.button(t("From file...")).clicked() {
            dialogs::add_channels_from_files(app);
        }
    });
}

/// The layouts menu of the Layouts button: the same as View > Layout.
fn layout_menu(app: &mut BodoApp, ui: &mut Ui) {
    ui.set_min_width(rem(14.4));
    let items = menu::layout_items(app);
    let mut chosen = None;
    menu_items(ui, &items, &mut chosen);
    if let Some(action) = chosen {
        menu::perform(app, action);
    }
}

/// A toggle button with text, accent-filled when on.
fn text_toggle(ui: &mut Ui, text: &str, on: bool, label: &str) -> egui::Response {
    let tk = Tokens::get();
    let fill = if on { tk.accent_soft } else { tk.control };
    let color = if on { tk.accent } else { tk.text_dim };
    let response =
        ui.add(egui::Button::new(RichText::new(text).color(color).size(size::SMALL).strong()).fill(fill).min_size(vec2(rem(3.52), rem(1.76))));
    control::record(ui, &response, label, "button", Some(on));
    response
}

fn transport(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    let transport = *app.session.transport();
    let song = transport.mode == PlayMode::Song;
    let response = text_toggle(ui, if song { t("SONG") } else { t("PAT") }, song, "Play mode");
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
    ui.add_space(rem(0.32));
    let typing = icon_button(ui, icons::KEYBOARD, "Typing keyboard", app.view.typing.on);
    app.hint(&typing, "Typing keyboard: the computer keyboard plays the selected channel, Z to M from C4 and Q to U from C5");
    if typing.clicked() {
        app.view.typing.on = !app.view.typing.on;
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
    let loop_record = icon_button(ui, icons::LOOP_RECORD, "Loop record", transport.loop_record);
    app.hint(&loop_record, "Loop record: while recording in a loop, every pass becomes its own take and MIDI passes add up");
    if loop_record.clicked() {
        let _ = app.run("transport.loop_record", json!({ "on": !transport.loop_record }));
    }
    let metronome = icon_button(ui, icons::METRONOME, "Metronome", transport.metronome);
    app.hint(&metronome, "Metronome: clicks every beat while playing and recording");
    if metronome.clicked() {
        let _ = app.run("transport.metronome", json!({ "on": !transport.metronome }));
    }
    ui.add_space(rem(0.8));

    // Tempo: drag or type.
    let project = std::sync::Arc::clone(app.session.project());
    let mut bpm = project.tempo.bpm_at(Tick::ZERO);
    let tempo = ui.add(egui::DragValue::new(&mut bpm).range(10.0..=999.0).speed(0.1).fixed_decimals(3).suffix(" BPM"));
    control::record(ui, &tempo, "Tempo", "value", None);
    app.hint(&tempo, "The tempo: drag, or double-click to type");
    if tempo.changed() {
        app.run_gesture(&tempo, "project.set_tempo", json!({ "bpm": bpm }));
    }
    ui.add_space(rem(0.8));

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
    ui.add_space(rem(0.8));

    // The Song mode loop, when there is one.
    if let Some((start, end)) = transport.loop_region {
        let text = tf!("Loop {start} to {end}", start = short_position(&project, start), end = short_position(&project, end));
        let looped = ui.add(egui::Button::new(RichText::new(text).size(size::SMALL).color(tk.accent)).fill(tk.accent_soft));
        control::record(ui, &looped, "Loop region", "button", Some(true));
        app.hint(&looped, "Song mode loops this region; click to stop looping");
        if looped.clicked() {
            let _ = app.run("transport.set_loop", json!({ "off": true }));
        }
        ui.add_space(rem(0.8));
    }

    // Master volume and level.
    let mut db = project.mixer.master.volume_db;
    let master = ui.add_sized(vec2(rem(7.2), rem(1.44)), egui::Slider::new(&mut db, -60.0..=6.0).show_value(false));
    control::record(ui, &master, "Master volume", "slider", None);
    app.hint(&master, tf!("Master volume: {db} dB", db = format!("{db:.1}")));
    if master.changed() {
        let db = if db <= -59.9 { -144.0 } else { db };
        app.run_gesture(&master, "mixer.set_volume", json!({ "insert": "master", "db": db }));
    }
    let (left, right) = app.audio.levels();
    let (rect, meter) = ui.allocate_exact_size(vec2(rem(5.12), rem(1.44)), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, tk.field);
    for (i, level) in [left, right].into_iter().enumerate() {
        let lane = Rect::from_min_size(rect.min + vec2(rem(0.16), rem(0.16) + i as f32 * rem(0.56)), vec2(rect.width() - rem(0.32), rem(0.48)));
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
    ui.add_space(rem(0.3));
    oscilloscope(app, ui);
    ui.add_space(rem(0.8));

    // Performance: CPU load, dropouts, and the memory in use, measured once a second.
    let status = app.audio.stream_status();
    let memory = app.services.memory.and_then(|measure| {
        let id = egui::Id::new("bodo-memory");
        let now = ui.input(|i| i.time);
        let cached: Option<(f64, Option<u64>)> = ui.ctx().data(|d| d.get_temp(id));
        match cached {
            Some((when, bytes)) if now - when < 1.0 => bytes,
            _ => {
                let bytes = measure();
                ui.ctx().data_mut(|d| d.insert_temp(id, (now, bytes)));
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                bytes
            }
        }
    });
    let memory = memory.map(|bytes| format!("  {}", tf!("{megabytes} MB", megabytes = bytes / 1_000_000))).unwrap_or_default();
    // The mix's latency from delay compensation, in milliseconds, when there is any.
    let rate = f64::from(app.session.project().settings.sample_rate.hz());
    let latency = crate::mixer::total_latency(app)
        .filter(|frames| *frames > 0)
        .map(|frames| format!("  {:.1} ms", f64::from(frames) * 1000.0 / rate))
        .unwrap_or_default();
    let text = if app.audio.is_on() {
        format!("{}{latency}{memory}", tf!("CPU {load}%  {dropouts} dropouts", load = format!("{:>3.0}", status.load * 100.0), dropouts = status.xruns))
    } else {
        format!("{}{latency}{memory}", t("No sound"))
    };
    let color = if !app.audio.is_on() || status.xruns > 0 { tk.warn } else { tk.text_dim };
    let perf = ui.add(egui::Label::new(RichText::new(text).monospace().size(size::SMALL).color(color)).sense(Sense::hover()));
    let audio_status = app.audio.status().to_owned();
    app.hint(&perf, tf!("Audio: {status}", status = audio_status));
}

/// How many frames of the master the oscilloscope shows: about 20 ms at 48 kHz.
const SCOPE_FRAMES: usize = 1024;

/// The oscilloscope: the master output's latest frames, mixed to mono, as a line.
fn oscilloscope(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(72.0, 18.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 3.0, tk.field);
    let mut frames = vec![0.0f32; SCOPE_FRAMES];
    let live = app.audio.scope(&mut frames);
    // One point per column: the sample of largest size among the frames it covers, so peaks don't vanish.
    let columns = rect.width().max(1.0) as usize;
    let per = (SCOPE_FRAMES / columns).max(1);
    let half = rect.height() / 2.0 - 1.5;
    let points: Vec<egui::Pos2> = (0..columns)
        .map(|c| {
            let chunk = &frames[(c * per).min(SCOPE_FRAMES)..((c + 1) * per).min(SCOPE_FRAMES)];
            let peak = chunk.iter().copied().fold(0.0f32, |a, b| if b.abs() > a.abs() { b } else { a });
            egui::pos2(rect.left() + c as f32 + 0.5, rect.center().y - peak.clamp(-1.0, 1.0) * half)
        })
        .collect();
    painter.hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, tk.grid_beat));
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, if live { tk.accent } else { tk.text_faint })));
    control::record(ui, &response, "Oscilloscope", "scope", None);
    app.hint(&response, "The oscilloscope: the master output's wave, the last 20 ms or so");
    if live {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
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
        icons::PLAYER_RECORD.paint(painter, Rect::from_center_size(rect.center(), vec2(rem(1.12), rem(1.12))), color);
    });
    let hint = if counting {
        en("Counting in: recording starts after this bar")
    } else if recording {
        en("Recording: click to stop; the takes land on the playlist")
    } else {
        en("Record: plays and records the armed mixer inserts and the notes you play")
    };
    app.hint(&record, hint);
    if record.clicked() {
        let _ = app.run("transport.record", json!({}));
    }
}

/// One-click audio recording: arms the selected insert when nothing plays into it, or else the first such insert that
/// isn't armed, records its input (the device's first two when it has none), and records into the playlist. Clicking
/// again stops, and the take lands on the playlist.
fn one_click_recording(app: &mut BodoApp, ui: &mut Ui) {
    let recording = app.session.transport().recording;
    let mic = icon_button(ui, icons::MICROPHONE, "One-click audio recording", recording);
    app.hint(
        &mic,
        if recording {
            en("Recording: click to stop; the take lands on the playlist")
        } else {
            en("One-click audio recording: arm an insert and record into the playlist")
        },
    );
    if !mic.clicked() {
        return;
    }
    if recording {
        let _ = app.run("transport.stop", json!({}));
        return;
    }
    let project = std::sync::Arc::clone(app.session.project());
    let mixer = &project.mixer;
    let used: std::collections::BTreeSet<_> = project.channels.values().filter_map(|c| c.target).collect();
    // An insert nothing plays into, so the take holds only what comes in.
    let chosen = app
        .session
        .editor()
        .insert
        .filter(|i| *i != mixer.master.id && mixer.inserts.contains_key(i) && !used.contains(i))
        .or_else(|| mixer.order.iter().copied().find(|i| !used.contains(i) && mixer.inserts.get(i).is_some_and(|ins| !ins.armed)));
    let Some(insert) = chosen.and_then(|i| mixer.inserts.get(&i)) else {
        app.set_status(t("No free insert to record into: add one in the mixer"), true);
        return;
    };
    if insert.input.is_none() && app.run("mixer.set_input", json!({ "insert": insert.id, "channels": [0, 1] })).is_err() {
        return;
    }
    if insert.record_tap != bodo_model::mixer::RecordTap::Input
        && app.run("mixer.set_record_tap", json!({ "insert": insert.id, "tap": "input" })).is_err()
    {
        return;
    }
    if !insert.armed && app.run("mixer.arm", json!({ "insert": insert.id, "on": true })).is_err() {
        return;
    }
    if app.session.transport().mode != PlayMode::Song {
        let _ = app.run("transport.set_mode", json!({ "mode": "song" }));
    }
    let _ = app.run("transport.record", json!({}));
}

/// `tick` as bar:beat, with the ticks only when it is between beats.
fn short_position(project: &bodo_model::Project, tick: Tick) -> String {
    let at = project.meter.position(tick);
    if at.tick == 0 { format!("{}:{}", at.bar + 1, at.beat + 1) } else { format!("{}:{}:{}", at.bar + 1, at.beat + 1, at.tick) }
}

fn tools(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    // The hint bar: what the control under the pointer does, or the last command's result.
    let (rect, _) = ui.allocate_exact_size(vec2(rem(27.2), rem(1.76)), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, tk.field);
    let (text, color) = match (&app.view.hint, app.status()) {
        (Some(hint), _) => (hint.clone(), tk.text_dim),
        (None, Some((status, true))) => (status.to_owned(), tk.error),
        (None, Some((status, false))) => (status.to_owned(), tk.text_dim),
        (None, None) => (String::new(), tk.text_dim),
    };
    let galley = ui.painter().layout(text, FontId::proportional(size::SMALL), color, rect.width() - 12.0);
    ui.painter().with_clip_rect(rect).galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, color);
    ui.add_space(rem(0.64));

    let layouts = icon_button(ui, icons::LAYOUT, "Layouts", false);
    app.hint(&layouts, "Layouts: arrange the windows");
    egui::Popup::menu(&layouts).show(|ui| layout_menu(app, ui));
    ui.add_space(rem(0.32));
    let browser = icon_button(ui, icons::FOLDER, "Browser", app.prefs.layout.browser);
    app.hint(&browser, "Show or hide the Browser");
    if browser.clicked() {
        let _ = app.run("ui.toggle_browser", json!({}));
    }
    for kind in WindowKind::ALL {
        let open = app.prefs.layout.is_open(kind);
        let button = icon_button(ui, kind.icon(), kind.title(), open);
        let keys = crate::shortcuts::window_keys(app, kind).map_or_else(String::new, |k| format!(" ({})", crate::shortcuts::display(ui.ctx(), &k)));
        app.hint(&button, format!("{}{keys}", tf!("Show or hide the {window}", window = t(kind.title()))));
        if button.clicked() {
            let _ = app.run("ui.toggle_window", json!({ "window": kind.id() }));
        }
    }
    ui.add_space(rem(0.32));
    let follow = icon_button(ui, icons::ARROW_RIGHT, "Auto scroll", app.view.auto_scroll);
    app.hint(&follow, "Auto scroll: the playlist and the piano roll follow the playhead while it plays");
    if follow.clicked() {
        app.view.auto_scroll = !app.view.auto_scroll;
    }
    one_click_recording(app, ui);
    ui.add_space(rem(1.28));

    // Snap.
    let snap = app.view.snap;
    let combo = egui::ComboBox::from_id_salt("bodo-snap").width(rem(5.6)).selected_text(t(snap.label())).show_ui(ui, |ui| {
        for choice in Snap::ALL {
            if ui.selectable_label(snap == choice, t(choice.label())).clicked() {
                let _ = app.run("ui.set_snap", json!({ "snap": choice.label() }));
            }
        }
    });
    icons::MAGNET.paint(
        ui.painter(),
        Rect::from_center_size(combo.response.rect.left_center() - vec2(rem(0.8), rem(0.0)), vec2(rem(0.96), rem(0.96))),
        tk.text_faint,
    );
    app.hint(&combo.response, "Snap: the grid the playlist and piano roll place things on");
    ui.add_space(rem(0.64));

    // The pattern selector.
    let project = std::sync::Arc::clone(app.session.project());
    let current = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).map(|p| (p.name.clone(), crate::theme::color(p.color)));
    let previous = icon_button(ui, icons::CHEVRON_LEFT, "Previous pattern", false);
    if previous.clicked() {
        let _ = app.run("ui.select_pattern", json!({ "offset": -1 }));
    }
    let (name, color) = current.unwrap_or_else(|| (String::from(t("No pattern")), tk.text_faint));
    let (swatch, _) = ui.allocate_exact_size(vec2(rem(0.48), rem(1.44)), Sense::hover());
    ui.painter().rect_filled(swatch, 2.0, color);
    let picker = egui::ComboBox::from_id_salt("bodo-pattern").width(rem(8.8)).selected_text(RichText::new(&name).color(tk.text)).show_ui(ui, |ui| {
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
        ui.add_space(rem(0.64));
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
