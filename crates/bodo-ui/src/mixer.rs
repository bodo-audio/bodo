//! The Mixer (project.md section 9.7): the master and the inserts as strips, each with its number, its name written
//! upward, a mute LED, a pan knob and a fader; the master also shows its output level. Click a strip to select it;
//! right-click for its menu. The insert panel with FX slots arrives with the full strip in M4.

use std::sync::Arc;

use bodo_kit_ui::{Fader, Led, Meter, icons};
use bodo_model::{MixerInsert, Project};
use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{self, Tokens, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control};

/// A strip's width.
const STRIP: f32 = 58.0;

/// The mixer's scroll.
#[derive(Debug, Default)]
pub struct MixerView {}

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

    let fader_rect = Rect::from_min_max(pos2(rect.left() + 6.0, rect.top() + 126.0), pos2(rect.right() - 6.0, rect.bottom() - 10.0));
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
}
