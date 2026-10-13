//! One mixer strip (project.md section 9.7), in three sizes. From the top: the number and icon, the name written
//! upward beside the mute LED, the pan and stereo separation knobs, the solo, polarity and swap switches and the arm
//! button, the fader with a peak meter and its clip light, the route control and the input it records.

use std::sync::Arc;

use bodo_kit_ui::{Fader, Knob, Led, Meter};
use bodo_model::mixer::{RecordFormat, RecordTap, RouteKey, RouteKind, RouteSource};
use bodo_model::{InsertId, MixerInsert, Project};
use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use super::panel::{EffectDrag, take_drop};
use super::{StripSize, icon_named, insert_label, insert_ref, meter, number_of, port_choices, port_label};
use crate::i18n::{en, t, tr};
use crate::theme::{self, Tokens, rem, size};
use crate::{BodoApp, control};

/// The fader handle's height, as `bodo_kit_ui::Fader` draws it, for lining the dB scale up with it.
const HANDLE: f32 = 14.0;

/// The record taps, with their command names and menu labels.
const TAPS: [(RecordTap, &str, &str); 4] = [
    (RecordTap::Input, "input", en("Input")),
    (RecordTap::PreFx, "preFx", en("Before the FX")),
    (RecordTap::PostFx, "postFx", en("After the FX")),
    (RecordTap::PostFader, "postFader", en("After the fader")),
];

/// The recording formats, with their command names and menu labels; FLAC and Wave64 read the same in every language.
const FORMATS: [(RecordFormat, &str, &str); 4] = [
    (RecordFormat::Float32, "float32", en("32-bit float WAV")),
    (RecordFormat::Int24, "int24", en("24-bit WAV")),
    (RecordFormat::Flac, "flac", "FLAC"),
    (RecordFormat::W64, "w64", "Wave64"),
];

/// Where a strip's parts go for a strip size: the top of the fader, and the space left at the bottom.
fn geometry(size: StripSize) -> (f32, f32) {
    match size {
        StripSize::Wide => (rem(12.8), rem(4.0)),
        StripSize::Compact => (rem(10.4), rem(4.0)),
        StripSize::ExtraCompact => (rem(7.04), rem(2.08)),
    }
}

/// The inserts selected together, the main selected insert among them.
fn selection(app: &BodoApp) -> Vec<InsertId> {
    let editor = app.session.editor();
    let mut selected: Vec<InsertId> = editor.inserts.iter().copied().collect();
    if let Some(main) = editor.insert
        && !selected.contains(&main)
    {
        selected.push(main);
    }
    selected
}

/// Draws strip `number` (0 for the master) of `insert`. Returns the top and bottom of its fader, for the dB scale.
pub(super) fn show(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>, insert: &Arc<MixerInsert>, number: usize, size: StripSize) -> (f32, f32) {
    let tk = Tokens::get();
    let height = ui.available_height().max(rem(19.2));
    let (rect, response) = ui.allocate_exact_size(vec2(size.width(), height - rem(0.64)), Sense::click());
    let id = insert.id;
    // The control channel's English name for the strip, and the name people see on it.
    let name = insert_label(number);
    let shown = insert.name.as_str();
    let selected = selection(app);
    let main = app.session.editor().insert == Some(id);
    let grouped = selected.contains(&id);
    let master = number == 0;
    let painter = ui.painter().with_clip_rect(rect.expand(1.0));
    let fill = if main {
        tk.accent_soft
    } else if master {
        tk.chrome_dark
    } else {
        tk.chrome
    };
    let outline = if main || grouped { tk.accent } else { tk.border };
    painter.rect(rect, rem(0.4), fill, Stroke::new(if grouped && !main { 1.5 } else { 1.0 }, outline), StrokeKind::Inside);
    painter.rect_filled(
        Rect::from_min_size(rect.min + vec2(rem(0.32), rem(0.32)), vec2(rect.width() - rem(0.64), rem(0.24))),
        1.0,
        theme::color(insert.color),
    );
    let label = if master { String::from("M") } else { number.to_string() };
    painter.text(pos2(rect.left() + rem(0.72), rect.top() + rem(1.28)), Align2::LEFT_CENTER, label, FontId::monospace(size::SMALL), tk.text_dim);
    if let Some(icon) = insert.icon.as_deref().and_then(icon_named) {
        let at = pos2(rect.right() - rem(0.88), rect.top() + rem(1.28));
        icon.paint(&painter, Rect::from_center_size(at, vec2(rem(0.96), rem(0.96))), tk.text_dim);
    }
    // The name, written upward.
    let column = if size == StripSize::ExtraCompact { rem(1.12) } else { rem(1.6) };
    let name_height = if size == StripSize::ExtraCompact { rem(4.16) } else { rem(8.0) };
    let name_top = rem(2.08);
    let galley = painter.layout_no_wrap(insert.name.clone(), FontId::proportional(size::SMALL), if insert.muted { tk.text_faint } else { tk.text });
    let name_pos =
        pos2(rect.left() + column / 2.0 - galley.size().y / 2.0 + 1.0, rect.top() + name_top + rem(0.16) + galley.size().x.min(name_height));
    painter
        .with_clip_rect(Rect::from_min_size(rect.min + vec2(0.0, name_top), vec2(column + rem(0.16), name_height)))
        .add(egui::epaint::TextShape::new(name_pos, galley, tk.text).with_angle(-std::f32::consts::FRAC_PI_2));
    control::record(ui, &response, &format!("{name} strip"), "strip", Some(main));
    app.hint(&response, tf!("{name}: click to select, Shift or Cmd/Ctrl click to group, right-click for more", name = shown));
    if response.clicked() {
        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let params = json!({ "inserts": [insert_ref(number)], "toggle": toggle });
        let _ = app.run("edit.select", params);
    }
    if let Some(effect) = take_drop::<EffectDrag>(ui, rect) {
        let _ = app.run("fx.load", json!({ "insert": insert_ref(number), "effect": effect.uid }));
    }
    strip_menu(app, &response, project, insert, number);

    let controls_rect = Rect::from_min_max(
        pos2(rect.left() + column + rem(0.16), rect.top() + name_top),
        pos2(rect.right() - rem(0.16), rect.top() + geometry(size).0 - rem(0.32)),
    );
    let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(controls_rect).layout(egui::Layout::top_down(egui::Align::Center)));
    controls.spacing_mut().item_spacing.y = rem(0.24);
    let mut on = !insert.muted;
    let led = controls.add(Led::new(&mut on).label(tf!("{name} on", name = shown)));
    control::record(&controls, &led, &format!("{name} on"), "led", Some(!insert.muted));
    app.hint(&led, tf!("{name}: click to mute or unmute", name = shown));
    if led.changed() {
        let _ = app.run("mixer.mute", json!({ "insert": insert_ref(number), "on": !on }));
    }
    if size != StripSize::ExtraCompact {
        let diameter = if size == StripSize::Wide { rem(1.6) } else { rem(1.44) };
        let mut pan = insert.pan;
        let knob = controls.add(bodo_kit_ui::knob::pan(&mut pan).diameter(diameter).label(tf!("{name} pan", name = shown)));
        control::record(&controls, &knob, &format!("{name} pan"), "knob", None);
        app.hint(&knob, tf!("{name} pan: {value}", name = shown, value = bodo_kit_ui::knob::format_pan(pan)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_pan", json!({ "insert": insert_ref(number), "pan": pan }));
        }
        automation_menu(app, &knob, number, "pan");
    }
    if size == StripSize::Wide {
        let mut separation = insert.stereo_sep;
        let knob = controls
            .add(Knob::new(&mut separation, -1.0..=1.0).origin(0.0).default(0.0).diameter(rem(1.6)).label(tf!("{name} separation", name = shown)));
        control::record(&controls, &knob, &format!("{name} separation"), "knob", None);
        app.hint(&knob, tf!("{name} stereo separation: {value}", name = shown, value = separation_text(separation)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_stereo_sep", json!({ "insert": insert_ref(number), "separation": separation }));
        }
        automation_menu(app, &knob, number, "stereoSep");
    }
    controls.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let soloed = switch(app, ui, &format!("{name} solo"), "S", insert.solo, tk.warn);
        app.hint(&soloed, tf!("{name} solo: click to solo, Cmd/Ctrl click to solo it alone", name = shown));
        if soloed.clicked() {
            let exclusive = ui.input(|i| i.modifiers.command);
            let params = if exclusive {
                json!({ "insert": insert_ref(number), "on": true, "exclusive": true })
            } else {
                json!({ "insert": insert_ref(number), "on": !insert.solo })
            };
            let _ = app.run("mixer.solo", params);
        }
        if size == StripSize::Wide {
            let flipped = switch(app, ui, &format!("{name} polarity"), "\u{d8}", insert.polarity_invert, tk.accent);
            app.hint(&flipped, tf!("{name}: reverse the polarity, before the effects", name = shown));
            if flipped.clicked() {
                let _ = app.run("mixer.set_polarity", json!({ "insert": insert_ref(number), "on": !insert.polarity_invert }));
            }
            let swapped = switch(app, ui, &format!("{name} swap"), "LR", insert.swap_lr, tk.accent);
            app.hint(&swapped, tf!("{name}: swap left and right, before the effects", name = shown));
            if swapped.clicked() {
                let _ = app.run("mixer.set_swap_lr", json!({ "insert": insert_ref(number), "on": !insert.swap_lr }));
            }
        }
    });
    if size != StripSize::ExtraCompact {
        arm_button(app, &mut controls, insert, number, &name);
    }

    // The fader and its meter.
    let (top, bottom_space) = geometry(size);
    let fader_rect =
        Rect::from_min_max(pos2(rect.left() + rem(0.32), rect.top() + top), pos2(rect.right() - rem(0.32), rect.bottom() - bottom_space));
    let (fader_width, meter_width) = match size {
        StripSize::Wide => (rem(1.76), rem(0.96)),
        StripSize::Compact => (rem(1.44), rem(0.64)),
        StripSize::ExtraCompact => (rem(1.12), rem(0.48)),
    };
    let gap = rem(0.24);
    let pair = fader_width + gap + meter_width;
    let start = fader_rect.center().x - pair / 2.0;
    let fader_area = Rect::from_min_size(pos2(start, fader_rect.top()), vec2(fader_width, fader_rect.height()));
    let mut area = ui.new_child(egui::UiBuilder::new().max_rect(fader_area));
    let old = if insert.volume_db <= -143.0 { f32::NEG_INFINITY } else { insert.volume_db };
    let mut db = old;
    let fader = area.add(Fader::new(&mut db).label(tf!("{name} volume", name = shown)).size(vec2(fader_width, fader_rect.height())).ticks(false));
    control::record(&area, &fader, &format!("{name} volume"), "fader", None);
    app.hint(
        &fader,
        tf!(
            "{name}: {value}; double-click for 0 dB, Alt drags the selected faders together",
            name = shown,
            value = bodo_kit_ui::fader::format_db(db)
        ),
    );
    if fader.changed() {
        fader_moved(app, ui, &fader, insert, number, old, db);
    }
    automation_menu(app, &fader, number, "volume");

    let meter_rect = Rect::from_min_size(pos2(start + fader_width + gap, fader_rect.top()), vec2(meter_width, fader_rect.height()));
    let peaks = meter(app, ui, id);
    let mut gauge = ui.new_child(egui::UiBuilder::new().max_rect(meter_rect));
    gauge.add(Meter::stereo(peaks.peak[0], peaks.peak[1]).size(meter_rect.size()).clip(peaks.clipped).label(tf!("{name} meter", name = shown)));
    let clip_rect = Rect::from_min_size(meter_rect.min - vec2(rem(0.16), rem(0.24)), vec2(meter_rect.width() + rem(0.32), rem(0.8)));
    let clip = ui.interact(clip_rect, ui.id().with(("strip-clip", id)), Sense::click());
    control::record(ui, &clip, &format!("{name} clip"), "led", Some(peaks.clipped));
    let clip_hint = if peaks.clipped {
        tf!("{name} went over 0 dB: click to clear the clip light", name = shown)
    } else {
        tf!("{name}: no clipping", name = shown)
    };
    app.hint(&clip, clip_hint);
    if clip.clicked() {
        app.audio.clear_clip(id);
        if let Some(meter) = app.view.mixer.meters.get_mut(&id) {
            meter.clipped = false;
        }
    }

    let route_rect = if size == StripSize::ExtraCompact {
        Rect::from_min_max(pos2(rect.left() + rem(0.24), rect.bottom() - rem(1.76)), pos2(rect.right() - rem(0.24), rect.bottom() - rem(0.32)))
    } else {
        Rect::from_min_max(pos2(rect.left() + rem(0.32), rect.bottom() - rem(3.68)), pos2(rect.right() - rem(0.32), rect.bottom() - rem(2.16)))
    };
    route_control(app, ui, project, route_rect, insert, number);
    if size != StripSize::ExtraCompact {
        let input_rect =
            Rect::from_min_max(pos2(rect.left() + rem(0.32), rect.bottom() - rem(1.92)), pos2(rect.right() - rem(0.32), rect.bottom() - rem(0.48)));
        input_selector(app, ui, input_rect, insert, number, &name);
    }
    (fader_rect.top(), fader_rect.bottom())
}

/// A fader moved: on its own, or with the selected faders when it is one of them or Alt is held.
fn fader_moved(app: &mut BodoApp, ui: &Ui, fader: &egui::Response, insert: &MixerInsert, number: usize, old: f32, new: f32) {
    let project = Arc::clone(app.session.project());
    let selected = selection(app);
    let alt = ui.input(|i| i.modifiers.alt);
    let together = (selected.len() > 1 && selected.contains(&insert.id)) || (alt && !selected.is_empty());
    let new = if new.is_finite() { new } else { -144.0 };
    if !together {
        app.run_gesture(fader, "mixer.set_volume", json!({ "insert": insert_ref(number), "db": new }));
        return;
    }
    let old = if old.is_finite() { old } else { -144.0 };
    let mut group: Vec<serde_json::Value> = selected.iter().filter_map(|id| number_of(&project, *id)).map(insert_ref).collect();
    if !selected.contains(&insert.id) {
        group.push(insert_ref(number));
    }
    app.run_gesture(fader, "mixer.adjust", json!({ "inserts": group, "volumeDb": new - old }));
}

/// A small square switch with `text`, lit in `color` when on.
fn switch(app: &mut BodoApp, ui: &mut Ui, label: &str, text: &str, on: bool, color: egui::Color32) -> egui::Response {
    let tk = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(rem(0.96), rem(0.96)), Sense::click());
    let fill = if on {
        color
    } else if response.hovered() {
        tk.control_hover
    } else {
        tk.control
    };
    ui.painter().rect(rect, 2.0, fill, Stroke::new(1.0, tk.border), StrokeKind::Inside);
    let ink = if on { theme::text_on(color) } else { tk.text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(rem(0.64)), ink);
    control::record(ui, &response, label, "switch", Some(on));
    let _ = app;
    response
}

/// How a stereo separation reads, in the interface's language: `unchanged`, `40% narrower` (toward mono) or
/// `40% wider`.
pub(crate) fn separation_text(separation: f32) -> String {
    if separation.abs() < 0.005 {
        String::from(t("unchanged"))
    } else if separation >= 0.995 {
        String::from(t("mono"))
    } else if separation > 0.0 {
        tf!("{percent}% narrower", percent = format!("{:.0}", separation * 100.0))
    } else {
        tf!("{percent}% wider", percent = format!("{:.0}", -separation * 100.0))
    }
}

/// The right-click menu of a strip knob or fader: making an automation clip of it.
fn automation_menu(app: &mut BodoApp, response: &egui::Response, number: usize, param: &str) {
    response.context_menu(|ui| {
        if ui.button(t("Create automation clip")).clicked() {
            let _ = app.run("automation.create_for", json!({ "target": { "insert": insert_ref(number), "param": param } }));
            ui.close();
        }
    });
}

/// The strip's menu: renaming, color and icon, routing, presets, automation, recording, ordering and removal.
fn strip_menu(app: &mut BodoApp, response: &egui::Response, project: &Arc<Project>, insert: &MixerInsert, number: usize) {
    let master = number == 0;
    let target = insert_ref(number);
    response.context_menu(|ui| {
        ui.set_min_width(rem(15.2));
        if ui.button(t("Rename...")).clicked() {
            app.view.dialogs.rename = Some(crate::dialogs::Rename::Insert(number, insert.name.clone()));
            ui.close();
        }
        ui.menu_button(t("Color"), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(rem(12.0));
                for hue in bodo_engine::palette::HUES {
                    let (rect, swatch) = ui.allocate_exact_size(vec2(rem(1.28), rem(1.28)), Sense::click());
                    ui.painter().rect_filled(rect, 3.0, theme::color(hue));
                    if swatch.clicked() {
                        let _ = app.run("mixer.set_color", json!({ "insert": target, "color": hue.to_string() }));
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button(t("Icon"), |ui| {
            if ui.radio(insert.icon.is_none(), t("None")).clicked() {
                let _ = app.run("mixer.set_icon", json!({ "insert": target, "icon": null }));
            }
            for (name, _) in super::INSERT_ICONS {
                if ui.radio(insert.icon.as_deref() == Some(name), tr(&icon_title(name))).clicked() {
                    let _ = app.run("mixer.set_icon", json!({ "insert": target, "icon": name }));
                }
            }
        });
        if !master {
            ui.separator();
            if ui.checkbox(&mut insert.to_master.clone(), t("Feeds the master")).clicked() {
                let _ = app.run("mixer.set_to_master", json!({ "insert": target, "on": !insert.to_master }));
            }
            if ui.button(t("Route only to the master")).clicked() {
                let _ = app.run("mixer.route_exclusive", json!({ "from": format!("insert:{number}"), "to": "master" }));
                ui.close();
            }
            if let Some(source) = app.session.editor().insert.and_then(|s| number_of(project, s)).filter(|s| *s != number && *s != 0)
                && ui.button(tf!("Route Insert {number} only here", number = source)).clicked()
            {
                let _ = app.run("mixer.route_exclusive", json!({ "from": format!("insert:{source}"), "to": target }));
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button(t("Create automation clip"), |ui| {
            for (param, label) in [("volume", t("Volume")), ("pan", t("Pan")), ("stereoSep", t("Stereo separation"))] {
                if ui.button(label).clicked() {
                    let _ = app.run("automation.create_for", json!({ "target": { "insert": target, "param": param } }));
                    ui.close();
                }
            }
        });
        ui.menu_button(t("Presets"), |ui| presets_menu(app, ui, number));
        ui.menu_button(t("Record from"), |ui| {
            for (tap, value, label) in TAPS {
                if ui.radio(insert.record_tap == tap, t(label)).clicked() {
                    let _ = app.run("mixer.set_record_tap", json!({ "insert": target, "tap": value }));
                }
            }
        });
        ui.menu_button(t("Record format"), |ui| {
            for (format, value, label) in FORMATS {
                if ui.radio(insert.record_format == format, tr(label)).clicked() {
                    let _ = app.run("mixer.set_record_format", json!({ "insert": target, "format": value }));
                }
            }
        });
        if !master {
            ui.separator();
            let count = project.mixer.order.len();
            if number > 1 && ui.button(t("Move left")).clicked() {
                let _ = app.run("mixer.reorder_insert", json!({ "insert": number, "to": number - 1 }));
                ui.close();
            }
            if number < count && ui.button(t("Move right")).clicked() {
                let _ = app.run("mixer.reorder_insert", json!({ "insert": number, "to": number + 1 }));
                ui.close();
            }
            if ui.button(t("Remove")).clicked() {
                let _ = app.run("mixer.remove_insert", json!({ "insert": number }));
                ui.close();
            }
        }
    });
}

/// The English name of the insert icon `name` as its menu shows it, such as `Drum` for `drum`.
fn icon_title(name: &str) -> String {
    match name {
        "drum" => String::from(en("Drum")),
        "guitar" => String::from(en("Guitar")),
        "piano" => String::from(en("Piano")),
        "keyboard" => String::from(en("Keyboard")),
        "microphone" => String::from(en("Microphone")),
        "music" => String::from(en("Music")),
        "wave" => String::from(en("Wave")),
        "speaker" => String::from(en("Speaker")),
        "headphones" => String::from(en("Headphones")),
        other => other.to_owned(),
    }
}

/// Saving the insert as a mixer preset, and loading the presets in the library onto it.
fn presets_menu(app: &mut BodoApp, ui: &mut Ui, number: usize) {
    if ui.button(t("Save as preset")).clicked() {
        let _ = app.run("preset.save_mixer", json!({ "insert": insert_ref(number) }));
        ui.close();
    }
    let Some(library) = app.user_library() else {
        ui.label(t("Choose a library folder in the Browser to keep presets"));
        return;
    };
    let folder = library.presets(bodo_engine::library::user::PresetFolder::MixerPresets);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&folder)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "bodomixer")).collect())
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        ui.label(t("No mixer presets saved yet"));
    }
    ui.separator();
    for path in files {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if ui.button(&name).clicked() {
            let _ = app.run("preset.load_mixer", json!({ "path": path.display().to_string(), "insert": insert_ref(number) }));
            ui.close();
        }
    }
}

/// The record-arm button: a ring that fills red when the strip is armed.
fn arm_button(app: &mut BodoApp, ui: &mut Ui, insert: &MixerInsert, number: usize, name: &str) {
    let tk = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(rem(1.28), rem(1.28)), Sense::click());
    let painter = ui.painter();
    let ring = if response.hovered() { tk.text } else { tk.text_dim };
    painter.circle_stroke(rect.center(), rem(0.48), Stroke::new(1.5, if insert.armed { tk.record } else { ring }));
    if insert.armed {
        painter.circle_filled(rect.center(), rem(0.32), tk.record);
    }
    let label = format!("{name} record arm");
    control::record(ui, &response, &label, "button", Some(insert.armed));
    let hint = if insert.armed {
        tf!("{name} armed: recording takes its input", name = insert.name)
    } else {
        tf!("{name}: click to arm it for recording", name = insert.name)
    };
    app.hint(&response, hint);
    if response.clicked() {
        let _ = app.run("mixer.arm", json!({ "insert": insert_ref(number), "on": !insert.armed }));
    }
}

/// The route control at the foot of a strip: whether the selected insert routes into this strip. Click to route or
/// unroute it, drag to set the send level, Cmd/Ctrl click for a sidechain-only route. On the master, it is whether the
/// selected insert feeds the master.
fn route_control(app: &mut BodoApp, ui: &mut Ui, project: &Project, rect: Rect, insert: &MixerInsert, number: usize) {
    let tk = Tokens::get();
    let name = insert_label(number);
    let source = app.session.editor().insert.and_then(|s| project.mixer.insert(s).map(|i| (s, i.to_master)));
    let Some((source, to_master)) = source.filter(|(s, _)| *s != insert.id) else {
        // The selected insert itself, or nothing selected: nothing to route.
        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, tk.border), StrokeKind::Inside);
        return;
    };
    let source_number = number_of(project, source).unwrap_or(0);
    let response = ui.interact(rect, ui.id().with(("strip-route", insert.id)), Sense::click_and_drag());
    let master = number == 0;
    let key = RouteKey { from: RouteSource::Insert(source), to: insert.id };
    let route = (!master).then(|| project.mixer.routes.get(&key).copied()).flatten();
    let routed = if master { to_master } else { route.is_some() };
    let sidechain = route.is_some_and(|r| r.kind == RouteKind::Sidechain);
    let painter = ui.painter();
    let fill = if response.hovered() { tk.control_hover } else { tk.field };
    painter.rect_filled(rect, 3.0, fill);
    let color = if sidechain {
        tk.warn
    } else if routed {
        tk.accent
    } else {
        tk.text_faint
    };
    // An arrow up into the strip when routed; with a level arc for sends.
    let (arm, head) = (rem(0.4), rem(0.28));
    let center = pos2(rect.left() + rem(0.72), rect.center().y);
    painter.line_segment([center + vec2(0.0, arm), center - vec2(0.0, arm)], Stroke::new(1.5, color));
    painter.line_segment([center - vec2(0.0, arm), center + vec2(-head, -head / 2.3)], Stroke::new(1.5, color));
    painter.line_segment([center - vec2(0.0, arm), center + vec2(head, -head / 2.3)], Stroke::new(1.5, color));
    if let Some(route) = route {
        let level = (route.send_level / 2.0).clamp(0.0, 1.0);
        let track = Rect::from_min_max(
            pos2(rect.left() + rem(1.36), rect.center().y - rem(0.16)),
            pos2(rect.right() - rem(0.24), rect.center().y + rem(0.16)),
        );
        painter.rect_filled(track, 2.0, tk.control);
        painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * level, track.height())), 2.0, color);
    }
    control::record(ui, &response, &format!("{name} route"), "route", Some(routed));
    let percent = route.map(|r| format!("{:.0}", r.send_level * 100.0)).unwrap_or_default();
    let hint = match (master, route) {
        (true, _) if to_master => tf!("Insert {number} feeds the master: click to change", number = source_number),
        (true, _) => tf!("Insert {number} doesn't feed the master: click to change", number = source_number),
        (false, Some(_)) if sidechain => {
            tf!("Insert {number} sidechains here at {percent}%: click to remove, drag to set the level", number = source_number, percent = percent)
        }
        (false, Some(_)) => {
            tf!("Insert {number} routes here at {percent}%: click to remove, drag to set the level", number = source_number, percent = percent)
        }
        (false, None) => tf!("Insert {number} doesn't route here: click to route it, Cmd/Ctrl click for a sidechain", number = source_number),
    };
    app.hint(&response, hint);
    let from = format!("insert:{source_number}");
    if response.clicked() {
        if master {
            let _ = app.run("mixer.set_to_master", json!({ "insert": source_number, "on": !to_master }));
        } else if route.is_some() {
            let _ = app.run("mixer.remove_route", json!({ "from": from, "to": insert_ref(number) }));
        } else {
            let side = ui.input(|i| i.modifiers.command);
            let _ = app.run("mixer.add_route", json!({ "from": from, "to": insert_ref(number), "sidechain": side }));
        }
    } else if response.dragged()
        && let Some(route) = route
    {
        let dy = response.drag_delta().y;
        if dy != 0.0 {
            let level = (route.send_level - dy * 0.01).clamp(0.0, 2.0);
            app.run_gesture(&response, "mixer.set_route", json!({ "from": from, "to": insert_ref(number), "level": level }));
        }
    }
}

/// The input selector at the foot of a strip: which audio device channels it records, in a menu of none, each mono
/// input and each stereo pair.
fn input_selector(app: &mut BodoApp, ui: &mut Ui, rect: Rect, insert: &MixerInsert, number: usize, name: &str) {
    let tk = Tokens::get();
    let response = ui.interact(rect, ui.id().with(("strip-input", insert.id)), Sense::click());
    let fill = if response.hovered() { tk.control_hover } else { tk.field };
    ui.painter().rect_filled(rect, 3.0, fill);
    let channels = insert.input.as_ref().map(|port| port.channels.as_slice());
    let input = format!("{} ", t("In"));
    let input = input.as_str();
    let text = port_label(input, channels);
    let color = if insert.input.is_some() { tk.text } else { tk.text_faint };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(size::SMALL), color);
    control::record(ui, &response, &format!("{name} input"), "combo", None);
    app.hint(&response, tf!("{name}: the audio input it records; click to choose", name = insert.name));
    let count = super::device_channels(app, true);
    egui::Popup::menu(&response).show(|ui| {
        let current = insert.input.as_ref().map(|port| port.channels.clone());
        for choice in port_choices(count) {
            let label = match &choice {
                None => String::from(t("None")),
                Some(channels) => port_label(input, Some(channels)),
            };
            if ui.selectable_label(current == choice, label).clicked() {
                let _ = app.run("mixer.set_input", json!({ "insert": insert_ref(number), "channels": choice }));
            }
        }
    });
}

/// The dB scale down the mixer's left edge, lined up with faders running from `top` to `bottom`.
pub(super) fn db_scale(ui: &Ui, rect: Rect, top: f32, bottom: f32) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(rect);
    let travel = (bottom - top - HANDLE).max(1.0);
    for db in [6.0, 0.0, -6.0, -12.0, -24.0, -48.0, f32::NEG_INFINITY] {
        let y = bottom - HANDLE / 2.0 - bodo_kit_ui::fader::db_to_position(db) * travel;
        let text = if db.is_finite() { format!("{db:.0}") } else { String::from("-\u{221e}") };
        painter.text(pos2(rect.right() - rem(0.32), y), Align2::RIGHT_CENTER, text, FontId::monospace(rem(0.72)), tk.text_faint);
        painter.hline(rect.right() - rem(0.24)..=rect.right(), y, Stroke::new(1.0, tk.border));
    }
}
