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
use crate::theme::{self, Tokens, size};
use crate::{BodoApp, control};

/// The fader handle's height, as `bodo_kit_ui::Fader` draws it, for lining the dB scale up with it.
const HANDLE: f32 = 14.0;

/// The record taps, with their command names and menu labels.
const TAPS: [(RecordTap, &str, &str); 4] = [
    (RecordTap::Input, "input", "Input"),
    (RecordTap::PreFx, "preFx", "Before the FX"),
    (RecordTap::PostFx, "postFx", "After the FX"),
    (RecordTap::PostFader, "postFader", "After the fader"),
];

/// The recording formats, with their command names and menu labels.
const FORMATS: [(RecordFormat, &str, &str); 4] = [
    (RecordFormat::Float32, "float32", "32-bit float WAV"),
    (RecordFormat::Int24, "int24", "24-bit WAV"),
    (RecordFormat::Flac, "flac", "FLAC"),
    (RecordFormat::W64, "w64", "Wave64"),
];

/// Where a strip's parts go for a strip size: the top of the fader, and the space left at the bottom.
fn geometry(size: StripSize) -> (f32, f32) {
    match size {
        StripSize::Wide => (160.0, 50.0),
        StripSize::Compact => (130.0, 50.0),
        StripSize::ExtraCompact => (88.0, 26.0),
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
    let height = ui.available_height().max(240.0);
    let (rect, response) = ui.allocate_exact_size(vec2(size.width(), height - 8.0), Sense::click());
    let id = insert.id;
    let name = insert_label(number);
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
    painter.rect(rect, 5.0, fill, Stroke::new(if grouped && !main { 1.5 } else { 1.0 }, outline), StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(rect.width() - 8.0, 3.0)), 1.0, theme::color(insert.color));
    let label = if master { String::from("M") } else { number.to_string() };
    painter.text(pos2(rect.left() + 9.0, rect.top() + 16.0), Align2::LEFT_CENTER, label, FontId::monospace(size::SMALL), tk.text_dim);
    if let Some(icon) = insert.icon.as_deref().and_then(icon_named) {
        icon.paint(&painter, Rect::from_center_size(pos2(rect.right() - 11.0, rect.top() + 16.0), vec2(12.0, 12.0)), tk.text_dim);
    }
    // The name, written upward.
    let column = if size == StripSize::ExtraCompact { 14.0 } else { 20.0 };
    let name_height = if size == StripSize::ExtraCompact { 52.0 } else { 100.0 };
    let galley = painter.layout_no_wrap(insert.name.clone(), FontId::proportional(size::SMALL), if insert.muted { tk.text_faint } else { tk.text });
    let name_pos = pos2(rect.left() + column / 2.0 - galley.size().y / 2.0 + 1.0, rect.top() + 28.0 + galley.size().x.min(name_height));
    painter
        .with_clip_rect(Rect::from_min_size(rect.min + vec2(0.0, 26.0), vec2(column + 2.0, name_height)))
        .add(egui::epaint::TextShape::new(name_pos, galley, tk.text).with_angle(-std::f32::consts::FRAC_PI_2));
    control::record(ui, &response, &format!("{name} strip"), "strip", Some(main));
    app.hint(&response, format!("{}: click to select, Shift or Cmd/Ctrl click to group, right-click for more", insert.name));
    if response.clicked() {
        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let params = json!({ "inserts": [insert_ref(number)], "toggle": toggle });
        let _ = app.run("edit.select", params);
    }
    if let Some(effect) = take_drop::<EffectDrag>(ui, rect) {
        let _ = app.run("fx.load", json!({ "insert": insert_ref(number), "effect": effect.uid }));
    }
    strip_menu(app, &response, project, insert, number);

    let controls_rect =
        Rect::from_min_max(pos2(rect.left() + column + 2.0, rect.top() + 26.0), pos2(rect.right() - 2.0, rect.top() + geometry(size).0 - 4.0));
    let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(controls_rect).layout(egui::Layout::top_down(egui::Align::Center)));
    controls.spacing_mut().item_spacing.y = 3.0;
    let mut on = !insert.muted;
    let led = controls.add(Led::new(&mut on).label(format!("{name} on")));
    control::record(&controls, &led, &format!("{name} on"), "led", Some(!insert.muted));
    app.hint(&led, format!("{name}: click to mute or unmute"));
    if led.changed() {
        let _ = app.run("mixer.mute", json!({ "insert": insert_ref(number), "on": !on }));
    }
    if size != StripSize::ExtraCompact {
        let diameter = if size == StripSize::Wide { 20.0 } else { 18.0 };
        let mut pan = insert.pan;
        let knob = controls.add(bodo_kit_ui::knob::pan(&mut pan).diameter(diameter).label(format!("{name} pan")));
        control::record(&controls, &knob, &format!("{name} pan"), "knob", None);
        app.hint(&knob, format!("{name} pan: {}", bodo_kit_ui::knob::format_pan(pan)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_pan", json!({ "insert": insert_ref(number), "pan": pan }));
        }
        automation_menu(app, &knob, number, "pan");
    }
    if size == StripSize::Wide {
        let mut separation = insert.stereo_sep;
        let knob = controls.add(Knob::new(&mut separation, -1.0..=1.0).origin(0.0).default(0.0).diameter(20.0).label(format!("{name} separation")));
        control::record(&controls, &knob, &format!("{name} separation"), "knob", None);
        app.hint(&knob, format!("{name} stereo separation: {}", separation_text(separation)));
        if knob.changed() {
            app.run_gesture(&knob, "mixer.set_stereo_sep", json!({ "insert": insert_ref(number), "separation": separation }));
        }
        automation_menu(app, &knob, number, "stereoSep");
    }
    controls.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let soloed = switch(app, ui, &format!("{name} solo"), "S", insert.solo, tk.warn);
        app.hint(&soloed, format!("{name} solo: click to solo, Cmd/Ctrl click to solo it alone"));
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
            app.hint(&flipped, format!("{name}: reverse the polarity, before the effects"));
            if flipped.clicked() {
                let _ = app.run("mixer.set_polarity", json!({ "insert": insert_ref(number), "on": !insert.polarity_invert }));
            }
            let swapped = switch(app, ui, &format!("{name} swap"), "LR", insert.swap_lr, tk.accent);
            app.hint(&swapped, format!("{name}: swap left and right, before the effects"));
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
    let fader_rect = Rect::from_min_max(pos2(rect.left() + 4.0, rect.top() + top), pos2(rect.right() - 4.0, rect.bottom() - bottom_space));
    let (fader_width, meter_width) = match size {
        StripSize::Wide => (22.0, 12.0),
        StripSize::Compact => (18.0, 8.0),
        StripSize::ExtraCompact => (14.0, 6.0),
    };
    let pair = fader_width + 3.0 + meter_width;
    let start = fader_rect.center().x - pair / 2.0;
    let fader_area = Rect::from_min_size(pos2(start, fader_rect.top()), vec2(fader_width, fader_rect.height()));
    let mut area = ui.new_child(egui::UiBuilder::new().max_rect(fader_area));
    let old = if insert.volume_db <= -143.0 { f32::NEG_INFINITY } else { insert.volume_db };
    let mut db = old;
    let fader = area.add(Fader::new(&mut db).label(format!("{name} volume")).size(vec2(fader_width, fader_rect.height())).ticks(false));
    control::record(&area, &fader, &format!("{name} volume"), "fader", None);
    app.hint(&fader, format!("{name}: {}; double-click for 0 dB, Alt drags the selected faders together", bodo_kit_ui::fader::format_db(db)));
    if fader.changed() {
        fader_moved(app, ui, &fader, insert, number, old, db);
    }
    automation_menu(app, &fader, number, "volume");

    let meter_rect = Rect::from_min_size(pos2(start + fader_width + 3.0, fader_rect.top()), vec2(meter_width, fader_rect.height()));
    let shown = meter(app, ui, id);
    let mut gauge = ui.new_child(egui::UiBuilder::new().max_rect(meter_rect));
    gauge.add(Meter::stereo(shown.peak[0], shown.peak[1]).size(meter_rect.size()).clip(shown.clipped).label(format!("{name} meter")));
    let clip_rect = Rect::from_min_size(meter_rect.min - vec2(2.0, 3.0), vec2(meter_rect.width() + 4.0, 10.0));
    let clip = ui.interact(clip_rect, ui.id().with(("strip-clip", id)), Sense::click());
    control::record(ui, &clip, &format!("{name} clip"), "led", Some(shown.clipped));
    app.hint(&clip, if shown.clipped { format!("{name} went over 0 dB: click to clear the clip light") } else { format!("{name}: no clipping") });
    if clip.clicked() {
        app.audio.clear_clip(id);
        if let Some(meter) = app.view.mixer.meters.get_mut(&id) {
            meter.clipped = false;
        }
    }

    let route_rect = if size == StripSize::ExtraCompact {
        Rect::from_min_max(pos2(rect.left() + 3.0, rect.bottom() - 22.0), pos2(rect.right() - 3.0, rect.bottom() - 4.0))
    } else {
        Rect::from_min_max(pos2(rect.left() + 4.0, rect.bottom() - 46.0), pos2(rect.right() - 4.0, rect.bottom() - 27.0))
    };
    route_control(app, ui, project, route_rect, insert, number);
    if size != StripSize::ExtraCompact {
        let input_rect = Rect::from_min_max(pos2(rect.left() + 4.0, rect.bottom() - 24.0), pos2(rect.right() - 4.0, rect.bottom() - 6.0));
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
    let (rect, response) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::click());
    let fill = if on {
        color
    } else if response.hovered() {
        tk.control_hover
    } else {
        tk.control
    };
    ui.painter().rect(rect, 2.0, fill, Stroke::new(1.0, tk.border), StrokeKind::Inside);
    let ink = if on { theme::text_on(color) } else { tk.text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(8.0), ink);
    control::record(ui, &response, label, "switch", Some(on));
    let _ = app;
    response
}

/// How a stereo separation reads: `unchanged`, `40% narrower` (toward mono) or `40% wider`.
pub(crate) fn separation_text(separation: f32) -> String {
    if separation.abs() < 0.005 {
        String::from("unchanged")
    } else if separation >= 0.995 {
        String::from("mono")
    } else if separation > 0.0 {
        format!("{:.0}% narrower", separation * 100.0)
    } else {
        format!("{:.0}% wider", -separation * 100.0)
    }
}

/// The right-click menu of a strip knob or fader: making an automation clip of it.
fn automation_menu(app: &mut BodoApp, response: &egui::Response, number: usize, param: &str) {
    response.context_menu(|ui| {
        if ui.button("Create automation clip").clicked() {
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
        ui.set_min_width(190.0);
        if ui.button("Rename...").clicked() {
            app.view.dialogs.rename = Some(crate::dialogs::Rename::Insert(number, insert.name.clone()));
            ui.close();
        }
        ui.menu_button("Color", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(150.0);
                for hue in bodo_engine::palette::HUES {
                    let (rect, swatch) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    ui.painter().rect_filled(rect, 3.0, theme::color(hue));
                    if swatch.clicked() {
                        let _ = app.run("mixer.set_color", json!({ "insert": target, "color": hue.to_string() }));
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button("Icon", |ui| {
            if ui.radio(insert.icon.is_none(), "None").clicked() {
                let _ = app.run("mixer.set_icon", json!({ "insert": target, "icon": null }));
            }
            for (name, _) in super::INSERT_ICONS {
                if ui.radio(insert.icon.as_deref() == Some(name), name).clicked() {
                    let _ = app.run("mixer.set_icon", json!({ "insert": target, "icon": name }));
                }
            }
        });
        if !master {
            ui.separator();
            if ui.checkbox(&mut insert.to_master.clone(), "Feeds the master").clicked() {
                let _ = app.run("mixer.set_to_master", json!({ "insert": target, "on": !insert.to_master }));
            }
            if ui.button("Route only to the master").clicked() {
                let _ = app.run("mixer.route_exclusive", json!({ "from": format!("insert:{number}"), "to": "master" }));
                ui.close();
            }
            if let Some(source) = app.session.editor().insert.and_then(|s| number_of(project, s)).filter(|s| *s != number && *s != 0)
                && ui.button(format!("Route Insert {source} only here")).clicked()
            {
                let _ = app.run("mixer.route_exclusive", json!({ "from": format!("insert:{source}"), "to": target }));
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button("Create automation clip", |ui| {
            for (param, label) in [("volume", "Volume"), ("pan", "Pan"), ("stereoSep", "Stereo separation")] {
                if ui.button(label).clicked() {
                    let _ = app.run("automation.create_for", json!({ "target": { "insert": target, "param": param } }));
                    ui.close();
                }
            }
        });
        ui.menu_button("Presets", |ui| presets_menu(app, ui, number));
        ui.menu_button("Record from", |ui| {
            for (tap, value, label) in TAPS {
                if ui.radio(insert.record_tap == tap, label).clicked() {
                    let _ = app.run("mixer.set_record_tap", json!({ "insert": target, "tap": value }));
                }
            }
        });
        ui.menu_button("Record format", |ui| {
            for (format, value, label) in FORMATS {
                if ui.radio(insert.record_format == format, label).clicked() {
                    let _ = app.run("mixer.set_record_format", json!({ "insert": target, "format": value }));
                }
            }
        });
        if !master {
            ui.separator();
            let count = project.mixer.order.len();
            if number > 1 && ui.button("Move left").clicked() {
                let _ = app.run("mixer.reorder_insert", json!({ "insert": number, "to": number - 1 }));
                ui.close();
            }
            if number < count && ui.button("Move right").clicked() {
                let _ = app.run("mixer.reorder_insert", json!({ "insert": number, "to": number + 1 }));
                ui.close();
            }
            if ui.button("Remove").clicked() {
                let _ = app.run("mixer.remove_insert", json!({ "insert": number }));
                ui.close();
            }
        }
    });
}

/// Saving the insert as a mixer preset, and loading the presets in the library onto it.
fn presets_menu(app: &mut BodoApp, ui: &mut Ui, number: usize) {
    if ui.button("Save as preset").clicked() {
        let _ = app.run("preset.save_mixer", json!({ "insert": insert_ref(number) }));
        ui.close();
    }
    let Some(library) = app.user_library() else {
        ui.label("Choose a library folder in the Browser to keep presets");
        return;
    };
    let folder = library.presets(bodo_engine::library::user::PresetFolder::MixerPresets);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&folder)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "bodomixer")).collect())
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        ui.label("No mixer presets saved yet");
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
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    let painter = ui.painter();
    let ring = if response.hovered() { tk.text } else { tk.text_dim };
    painter.circle_stroke(rect.center(), 6.0, Stroke::new(1.5, if insert.armed { tk.record } else { ring }));
    if insert.armed {
        painter.circle_filled(rect.center(), 4.0, tk.record);
    }
    let label = format!("{name} record arm");
    control::record(ui, &response, &label, "button", Some(insert.armed));
    let what = if insert.armed { "armed: recording takes its input" } else { "click to arm it for recording" };
    app.hint(&response, format!("{name}: {what}"));
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
    let center = pos2(rect.left() + 9.0, rect.center().y);
    painter.line_segment([center + vec2(0.0, 5.0), center - vec2(0.0, 5.0)], Stroke::new(1.5, color));
    painter.line_segment([center - vec2(0.0, 5.0), center + vec2(-3.5, -1.5)], Stroke::new(1.5, color));
    painter.line_segment([center - vec2(0.0, 5.0), center + vec2(3.5, -1.5)], Stroke::new(1.5, color));
    if let Some(route) = route {
        let level = (route.send_level / 2.0).clamp(0.0, 1.0);
        let track = Rect::from_min_max(pos2(rect.left() + 17.0, rect.center().y - 2.0), pos2(rect.right() - 3.0, rect.center().y + 2.0));
        painter.rect_filled(track, 2.0, tk.control);
        painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * level, track.height())), 2.0, color);
    }
    let what = if sidechain { "sidechain" } else { "route" };
    control::record(ui, &response, &format!("{name} route"), "route", Some(routed));
    let hint = if master {
        format!("Insert {source_number} {} the master: click to change", if to_master { "feeds" } else { "doesn't feed" })
    } else {
        match route {
            Some(route) => format!("Insert {source_number} {what}s here at {:.0}%: click to remove, drag to set the level", route.send_level * 100.0),
            None => format!("Insert {source_number} doesn't route here: click to route it, Cmd/Ctrl click for a sidechain"),
        }
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
    let text = port_label("In ", channels);
    let color = if insert.input.is_some() { tk.text } else { tk.text_faint };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(size::SMALL), color);
    control::record(ui, &response, &format!("{name} input"), "combo", None);
    app.hint(&response, format!("{name}: the audio input it records; click to choose"));
    let count = super::device_channels(app, true);
    egui::Popup::menu(&response).show(|ui| {
        let current = insert.input.as_ref().map(|port| port.channels.clone());
        for choice in port_choices(count) {
            let label = match &choice {
                None => String::from("None"),
                Some(channels) => port_label("In ", Some(channels)),
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
        painter.text(pos2(rect.right() - 4.0, y), Align2::RIGHT_CENTER, text, FontId::monospace(9.0), tk.text_faint);
        painter.hline(rect.right() - 3.0..=rect.right(), y, Stroke::new(1.0, tk.border));
    }
}
