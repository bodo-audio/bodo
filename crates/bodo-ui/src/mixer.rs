//! The Mixer (project.md section 9.7): the master and the inserts as strips, each with its number, its name written
//! upward, a mute LED, a pan knob, a record-arm button, a fader and the audio input it records from; the master also
//! shows its output level. Click a strip to select it; right-click for its menu, which also sets what and how it
//! records. The insert panel with FX slots arrives with the full strip in M4.

use std::sync::Arc;

use bodo_kit_ui::{Fader, Led, Meter, icons};
use bodo_model::mixer::{HwPort, RecordFormat, RecordTap};
use bodo_model::{MixerInsert, Project};
use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{self, Tokens, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control};

/// A strip's width.
const STRIP: f32 = 58.0;

/// How many input channels to offer when the device doesn't say: four stereo pairs.
const UNKNOWN_INPUTS: u16 = 8;

/// What the mixer remembers between frames.
#[derive(Debug, Default)]
pub struct MixerView {
    /// The input channel count of the audio device in use, and the host and input device it was looked up for.
    inputs: Option<(Option<String>, Option<String>, u16)>,
}

/// How many input channels the open audio device offers, looked up once per device; [`UNKNOWN_INPUTS`] without sound.
///
/// Complexity: O(d) in the host's devices when the device changed, O(1) otherwise.
fn input_count(app: &mut BodoApp) -> u16 {
    if !app.audio.is_on() {
        return UNKNOWN_INPUTS;
    }
    let prefs = app.audio.prefs();
    let (host, input) = (prefs.host.clone(), prefs.input.clone());
    if let Some((h, i, count)) = &app.view.mixer.inputs
        && *h == host
        && *i == input
    {
        return *count;
    }
    let devices = bodo_kit_device::devices(host.as_deref()).unwrap_or_default();
    let device = match &input {
        Some(name) => devices.iter().find(|d| &d.name == name),
        None => devices.iter().find(|d| d.is_default_input),
    };
    let count = device.map_or(UNKNOWN_INPUTS, |d| d.input_channels);
    app.view.mixer.inputs = Some((host, input, count));
    count
}

/// How an input reads on a strip: `-` for none, `In 1` for mono, `In 1-2` for a pair.
fn input_label(input: Option<&HwPort>) -> String {
    match input.map(|port| port.channels.as_slice()) {
        None | Some([]) => String::from("-"),
        Some([one]) => format!("In {}", one + 1),
        Some([first, .., last]) => format!("In {}-{}", first + 1, last + 1),
    }
}

/// The record taps, with their command names and menu labels.
const TAPS: [(RecordTap, &str, &str); 4] = [
    (RecordTap::Input, "input", "Input"),
    (RecordTap::PreFx, "preFx", "Before the FX"),
    (RecordTap::PostFx, "postFx", "After the FX"),
    (RecordTap::PostFader, "postFader", "After the fader"),
];

/// The recording formats, with their command names and menu labels.
const FORMATS: [(RecordFormat, &str, &str); 3] =
    [(RecordFormat::Float32, "float32", "32-bit float WAV"), (RecordFormat::Int24, "int24", "24-bit WAV"), (RecordFormat::Flac, "flac", "FLAC")];

/// Draws the Mixer window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let mut layout = std::mem::take(&mut app.prefs.layout);
    windows::show(
        ctx,
        &mut layout,
        WindowKind::Mixer,
        workspace,
        "Mixer",
        app,
        |app, ui| {
            let add = icon_button(ui, icons::PLUS, "Add insert", false);
            app.hint(&add, "Add a mixer insert");
            if add.clicked() {
                let _ = app.run("mixer.add_insert", json!({}));
            }
        },
        |app, ui| body(app, ui, project),
    );
    app.prefs.layout = layout;
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>) {
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(4.0);
            strip(app, ui, &project.mixer.master, 0);
            ui.add_space(6.0);
            for (n, id) in project.mixer.order.iter().enumerate() {
                if let Some(insert) = project.mixer.inserts.get(id) {
                    strip(app, ui, insert, n + 1);
                }
            }
        });
    });
}

fn strip(app: &mut BodoApp, ui: &mut Ui, insert: &MixerInsert, number: usize) {
    let tk = Tokens::get();
    let height = ui.available_height().max(220.0);
    let (rect, response) = ui.allocate_exact_size(vec2(STRIP, height - 8.0), Sense::click());
    let id = insert.id;
    let selected = app.session.editor().insert == Some(id);
    let master = number == 0;
    let painter = ui.painter().with_clip_rect(rect.expand(1.0));
    let fill = if selected {
        tk.accent_soft
    } else if master {
        tk.chrome_dark
    } else {
        tk.chrome
    };
    painter.rect(rect, 5.0, fill, Stroke::new(1.0, if selected { tk.accent } else { tk.border }), StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(rect.width() - 8.0, 3.0)), 1.0, theme::color(insert.color));
    let label = if master { String::from("M") } else { number.to_string() };
    painter.text(pos2(rect.center().x, rect.top() + 16.0), Align2::CENTER_CENTER, label, FontId::monospace(size::SMALL), tk.text_dim);
    // The name, written upward.
    let galley = painter.layout_no_wrap(insert.name.clone(), FontId::proportional(size::SMALL), if insert.muted { tk.text_faint } else { tk.text });
    let name_pos = pos2(rect.left() + 8.0, rect.top() + 30.0 + galley.size().x.min(90.0));
    painter
        .with_clip_rect(Rect::from_min_size(rect.min + vec2(0.0, 26.0), vec2(22.0, 96.0)))
        .add(egui::epaint::TextShape::new(name_pos, galley, tk.text).with_angle(-std::f32::consts::FRAC_PI_2));
    let name = if master { String::from("Master") } else { format!("Insert {number}") };
    control::record(ui, &response, &format!("{name} strip"), "strip", Some(selected));
    app.hint(&response, format!("{}: click to select, right-click for more", insert.name));
    if response.clicked() {
        let _ = app.run("edit.select", json!({ "insert": number }));
    }
    response.context_menu(|ui| {
        if ui.button("Rename...").clicked() {
            app.view.dialogs.rename = Some(crate::dialogs::Rename::Insert(number, insert.name.clone()));
        }
        if !master && ui.button("Remove").clicked() {
            let _ = app.run("mixer.remove_insert", json!({ "insert": number }));
        }
        ui.separator();
        ui.menu_button("Record from", |ui| {
            for (tap, value, label) in TAPS {
                if ui.radio(insert.record_tap == tap, label).clicked() {
                    let _ = app.run("mixer.set_record_tap", json!({ "insert": number, "tap": value }));
                }
            }
        });
        ui.menu_button("Record format", |ui| {
            for (format, value, label) in FORMATS {
                if ui.radio(insert.record_format == format, label).clicked() {
                    let _ = app.run("mixer.set_record_format", json!({ "insert": number, "format": value }));
                }
            }
        });
    });

    let mut controls = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.left() + 24.0, rect.top() + 28.0), pos2(rect.right() - 2.0, rect.top() + 120.0)))
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    let mut on = !insert.muted;
    let led = controls.add(Led::new(&mut on).label(format!("{name} on")));
    control::record(&controls, &led, &format!("{name} on"), "led", Some(!insert.muted));
    app.hint(&led, format!("{name}: click to mute or unmute"));
    if led.changed() {
        let _ = app.run("mixer.mute", json!({ "insert": number, "on": !on }));
    }
    controls.add_space(6.0);
    let mut pan = insert.pan;
    let knob = controls.add(bodo_kit_ui::knob::pan(&mut pan).diameter(22.0).label(format!("{name} pan")));
    control::record(&controls, &knob, &format!("{name} pan"), "knob", None);
    app.hint(&knob, format!("{name} pan: {}", bodo_kit_ui::knob::format_pan(pan)));
    if knob.changed() {
        app.run_gesture(&knob, "mixer.set_pan", json!({ "insert": number, "pan": pan }));
    }
    controls.add_space(6.0);
    arm_button(app, &mut controls, insert, number, &name);

    let fader_rect = Rect::from_min_max(pos2(rect.left() + 6.0, rect.top() + 126.0), pos2(rect.right() - 6.0, rect.bottom() - 34.0));
    let mut area = ui.new_child(egui::UiBuilder::new().max_rect(fader_rect).layout(egui::Layout::left_to_right(egui::Align::Min)));
    let mut db = if insert.volume_db <= -143.0 { f32::NEG_INFINITY } else { insert.volume_db };
    let fader = area.add(Fader::new(&mut db).label(format!("{name} volume")).size(vec2(22.0, fader_rect.height())).ticks(master));
    control::record(&area, &fader, &format!("{name} volume"), "fader", None);
    app.hint(&fader, format!("{name}: {}", bodo_kit_ui::fader::format_db(db)));
    if fader.changed() {
        let db = if db.is_finite() { db } else { -144.0 };
        app.run_gesture(&fader, "mixer.set_volume", json!({ "insert": number, "db": db }));
    }
    if master {
        let (left, right) = app.audio.levels();
        area.add(Meter::stereo(left, right).size(vec2(14.0, fader_rect.height())).clip(left >= 1.0 || right >= 1.0).label("Master level"));
    }
    let input_rect = Rect::from_min_max(pos2(rect.left() + 4.0, rect.bottom() - 28.0), pos2(rect.right() - 4.0, rect.bottom() - 8.0));
    input_selector(app, ui, input_rect, insert, number, &name);
}

/// The record-arm button: a ring that fills red when the strip is armed.
fn arm_button(app: &mut BodoApp, ui: &mut Ui, insert: &MixerInsert, number: usize, name: &str) {
    let tk = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    let painter = ui.painter();
    let ring = if response.hovered() { tk.text } else { tk.text_dim };
    painter.circle_stroke(rect.center(), 7.0, Stroke::new(1.5, if insert.armed { tk.record } else { ring }));
    if insert.armed {
        painter.circle_filled(rect.center(), 4.5, tk.record);
    }
    let label = format!("{name} record arm");
    control::record(ui, &response, &label, "button", Some(insert.armed));
    let what = if insert.armed { "armed: recording takes its input" } else { "click to arm it for recording" };
    app.hint(&response, format!("{name}: {what}"));
    if response.clicked() {
        let _ = app.run("mixer.arm", json!({ "insert": number, "on": !insert.armed }));
    }
}

/// The input selector at the foot of a strip: which audio device channels it records, in a menu of none, each mono
/// input and each stereo pair.
fn input_selector(app: &mut BodoApp, ui: &mut Ui, rect: Rect, insert: &MixerInsert, number: usize, name: &str) {
    let tk = Tokens::get();
    let response = ui.interact(rect, ui.id().with(("strip-input", insert.id)), Sense::click());
    let fill = if response.hovered() { tk.control_hover } else { tk.field };
    ui.painter().rect_filled(rect, 3.0, fill);
    let text = input_label(insert.input.as_ref());
    let color = if insert.input.is_some() { tk.text } else { tk.text_faint };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(size::SMALL), color);
    control::record(ui, &response, &format!("{name} input"), "combo", None);
    app.hint(&response, format!("{name}: the audio input it records; click to choose"));
    let count = input_count(app);
    egui::Popup::menu(&response).show(|ui| {
        let current = insert.input.as_ref().map(|port| port.channels.clone());
        let mut choices: Vec<Option<Vec<u16>>> = vec![None];
        choices.extend((0..count).map(|n| Some(vec![n])));
        choices.extend((0..count / 2).map(|pair| Some(vec![pair * 2, pair * 2 + 1])));
        for choice in choices {
            let label = match &choice {
                None => String::from("None"),
                Some(channels) => input_label(Some(&HwPort { channels: channels.clone() })),
            };
            if ui.selectable_label(current == choice, label).clicked() {
                let _ = app.run("mixer.set_input", json!({ "insert": number, "channels": choice }));
            }
        }
    });
}
