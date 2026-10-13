//! The note properties dialog (project.md section 10.4): every property of one note, opened by double-clicking the
//! note or from Tools > Properties. OK applies them all as one undo step.

use bodo_engine::params::key_name;
use bodo_model::{Note, NoteId, Pattern, Project};
use egui::{RichText, Ui};
use serde_json::json;

use crate::{BodoApp, control};

/// The dialog's values while they are edited.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Editing {
    note: NoteId,
    key: u8,
    start: i64,
    length: i64,
    velocity: f32,
    release: f32,
    fine: f32,
    pan: f32,
    mod_x: f32,
    mod_y: f32,
    color: u8,
    slide: bool,
    porta: bool,
    muted: bool,
}

impl Editing {
    fn of(note: &Note) -> Self {
        let level = |v: u16| f32::from(v) / f32::from(u16::MAX);
        Self {
            note: note.id,
            key: note.key,
            start: note.start.0,
            length: note.len.0,
            velocity: level(note.velocity),
            release: level(note.release_velocity),
            fine: note.fine_pitch_cents,
            pan: note.pan,
            mod_x: note.mod_x,
            mod_y: note.mod_y,
            color: note.color,
            slide: note.slide,
            porta: note.porta,
            muted: note.muted,
        }
    }
}

/// A row of the dialog: its label, and the control `add` puts beside it, recorded for tests and agents as `name`.
fn row(ui: &mut Ui, label: &str, name: &str, add: impl FnOnce(&mut Ui) -> egui::Response) {
    ui.label(label);
    let response = add(ui);
    control::record(ui, &response, name, "field", None);
    ui.end_row();
}

/// Draws the dialog while it is open; OK sets every property with one `note.set_props`.
pub(super) fn dialog(app: &mut BodoApp, ctx: &egui::Context, project: &Project, pattern: &Pattern) {
    let Some(id) = app.view.piano.properties else { return };
    let Some(note) = pattern.notes.values().flat_map(|seq| seq.notes().iter()).find(|n| n.id == id).cloned() else {
        app.view.piano.properties = None;
        app.view.piano.editing = None;
        return;
    };
    let mut done = None;
    let length_max = bodo_engine::pattern_length(project, pattern.id).0.max(note.end().0) * 4;
    let editing = match &mut app.view.piano.editing {
        Some(editing) if editing.note == id => editing,
        slot => slot.insert(Editing::of(&note)),
    };
    egui::Modal::new(egui::Id::new("bodo-note-properties")).show(ctx, |ui| {
        ui.label(RichText::new(format!("Note {}", key_name(editing.key))).strong());
        ui.add_space(4.0);
        egui::Grid::new("bodo-note-properties-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            row(ui, "Key", "Note key", |ui| ui.add(egui::DragValue::new(&mut editing.key).range(0..=127).custom_formatter(|v, _| key_name(v as u8))));
            row(ui, "Start (ticks)", "Note start", |ui| ui.add(egui::DragValue::new(&mut editing.start).range(0..=i64::MAX).speed(10.0)));
            row(ui, "Length (ticks)", "Note length", |ui| ui.add(egui::DragValue::new(&mut editing.length).range(1..=length_max).speed(10.0)));
            row(ui, "Velocity", "Note velocity", |ui| ui.add(egui::Slider::new(&mut editing.velocity, 0.0..=1.0)));
            row(ui, "Release", "Note release", |ui| ui.add(egui::Slider::new(&mut editing.release, 0.0..=1.0)));
            row(ui, "Fine pitch (cents)", "Note fine pitch", |ui| ui.add(egui::Slider::new(&mut editing.fine, -1200.0..=1200.0)));
            row(ui, "Pan", "Note pan", |ui| ui.add(egui::Slider::new(&mut editing.pan, -1.0..=1.0)));
            row(ui, "Mod X", "Note Mod X", |ui| ui.add(egui::Slider::new(&mut editing.mod_x, 0.0..=1.0)));
            row(ui, "Mod Y", "Note Mod Y", |ui| ui.add(egui::Slider::new(&mut editing.mod_y, 0.0..=1.0)));
            row(ui, "Color", "Note color", |ui| ui.add(egui::DragValue::new(&mut editing.color).range(0..=15)));
            row(ui, "Slide", "Note slide", |ui| ui.checkbox(&mut editing.slide, "Slide to the next note"));
            row(ui, "Portamento", "Note portamento", |ui| ui.checkbox(&mut editing.porta, "Glide from the last note"));
            row(ui, "Muted", "Note muted", |ui| ui.checkbox(&mut editing.muted, "Silent"));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ok = ui.button("OK");
            control::record(ui, &ok, "Note properties OK", "button", None);
            if ok.clicked() {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
    });
    let Some(apply) = done else { return };
    app.view.piano.properties = None;
    let Some(e) = app.view.piano.editing.take() else { return };
    if !apply {
        return;
    }
    let round = |v: f32, places: f32| (v * places).round() / places;
    let params = json!({
        "notes": [id.0],
        "key": e.key,
        "start": e.start,
        "length": e.length,
        "velocity": round(e.velocity, 1000.0),
        "releaseVelocity": round(e.release, 1000.0),
        "finePitchCents": e.fine.round(),
        "pan": round(e.pan, 100.0),
        "modX": round(e.mod_x, 1000.0),
        "modY": round(e.mod_y, 1000.0),
        "color": e.color,
        "slide": e.slide,
        "porta": e.porta,
        "muted": e.muted,
    });
    let _ = app.run("note.set_props", params);
}
