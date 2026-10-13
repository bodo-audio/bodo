//! The Channel Rack (project.md section 10.2): one row per channel with its mute LED, pan and volume knobs, mixer
//! insert, name button and the step sequencer for the current pattern. Steps paint by dragging, and one stroke is one
//! undo step. Drop a sound from the browser to add a channel. Audio clip and automation channels play from the
//! playlist, so their rows have no steps; an automation channel has no knobs either. Right-click a knob to automate it.

use std::sync::Arc;

use bodo_engine::params::STEP;
use bodo_kit_ui::{Knob, Led, StepRow, icons};
use bodo_model::{Channel, ChannelId, ChannelKind, PatternId, Project};
use egui::{Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;

use crate::browser::SoundPayload;
use crate::theme::{Tokens, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control, dialogs, topbar};

/// The height of a channel row.
const ROW: f32 = 26.0;
/// The size of one step.
const STEP_SIZE: egui::Vec2 = vec2(17.0, 22.0);

/// Draws the Channel Rack window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let pattern = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).map(|p| (p.id, p.name.clone()));
    let mut layout = std::mem::take(&mut app.prefs.layout);
    let title = match &pattern {
        Some((_, name)) => format!("Channel rack - {name}"),
        None => String::from("Channel rack"),
    };
    let pattern = pattern.map(|(id, _)| id);
    windows::show(ctx, &mut layout, WindowKind::ChannelRack, workspace, &title, app, |_, _| {}, |app, ui| body(app, ui, project, pattern));
    app.prefs.layout = layout;
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>, pattern: Option<PatternId>) {
    let tk = Tokens::get();
    let steps = pattern.map_or(16, |p| (bodo_engine::pattern_length(project, p).0 / STEP.0).max(16) as usize);
    let paint_id = ui.id().with("step-paint");
    let rect = ui.max_rect();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        for id in project.rack_order.iter() {
            if let Some(channel) = project.channels.get(id) {
                row(app, ui, project, channel, pattern, steps, paint_id);
            }
        }
        if project.rack_order.is_empty() {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("No channels yet. Drag a sound here from the browser, or press +.").color(tk.text_faint));
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let add = icon_button(ui, icons::PLUS, "Add channel", false);
            app.hint(&add, "Add a channel: a factory sound, an empty sampler, or a file");
            egui::Popup::menu(&add).show(|ui| topbar::add_channel_menu(app, ui));
        });
        ui.add_space(8.0);
    });
    // Painting ends when the pointer comes up.
    if !ui.input(|i| i.pointer.any_down()) {
        app.view.step_gesture = None;
    }
    // A sound dropped anywhere on the rack becomes a channel.
    let drop = ui.interact(rect, ui.id().with("rack-drop"), Sense::hover());
    if let Some(payload) = drop.dnd_release_payload::<SoundPayload>() {
        let _ = app.run("channel.add", json!({ "sample": payload.0 }));
    }
    if drop.dnd_hover_payload::<SoundPayload>().is_some() {
        ui.painter().rect_stroke(rect.shrink(2.0), 6.0, Stroke::new(2.0, tk.accent), StrokeKind::Inside);
    }
}

#[allow(clippy::too_many_arguments, reason = "a row draws from the project, the channel and the pattern at once")]
fn row(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, pattern: Option<PatternId>, steps: usize, paint_id: egui::Id) {
    let id = channel.id;
    let name = channel.name.clone();
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            ui.add_space(8.0);
            // Mute: the LED is lit while the channel plays.
            let mut on = !channel.muted;
            let led = ui.add(Led::new(&mut on).label(format!("{name} on")));
            control::record(ui, &led, &format!("{name} on"), "led", Some(!channel.muted));
            app.hint(&led, format!("{name}: click to mute or unmute"));
            if led.changed() {
                let _ = app.run("channel.mute", json!({ "channel": id, "muted": !on }));
            }
            if matches!(channel.kind, ChannelKind::Automation(_)) {
                // An automation channel plays no sound: room where the knobs and insert would be keeps the rows lined up.
                for width in [18.0, 18.0, 26.0] {
                    ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
                }
            } else {
                knobs(app, ui, channel);
                insert_picker(app, ui, project, channel);
            }
            name_button(app, ui, channel);
            let clip_kind = match channel.kind {
                ChannelKind::AudioClip { .. } => Some("Audio clip: place it on the playlist"),
                ChannelKind::Automation(_) => Some("Automation clip: edit it on the playlist"),
                _ => None,
            };
            if let Some(text) = clip_kind {
                let label = ui.label(RichText::new(text).size(size::SMALL).color(Tokens::get().text_faint));
                control::record(ui, &label, &format!("{name} kind"), "label", None);
                return;
            }
            let on: Vec<bool> = match pattern.and_then(|p| project.patterns.get(&p)) {
                Some(pattern) => bodo_engine::pattern_step_text(project, pattern, id)
                    .chars()
                    .map(|c| c == 'x')
                    .chain(std::iter::repeat(false))
                    .take(steps)
                    .collect(),
                None => vec![false; steps],
            };
            let step_row = StepRow::new(name.clone(), &on).paint_id(paint_id).step_size(STEP_SIZE);
            let offsets: Vec<f32> = (0..on.len()).map(|i| step_row.offset(i)).collect();
            let shown = step_row.show(ui);
            for (index, offset) in offsets.iter().enumerate() {
                let rect = Rect::from_min_size(shown.response.rect.min + vec2(*offset, 0.0), STEP_SIZE);
                control::record_rect(
                    ui,
                    shown.response.id.with(index),
                    rect,
                    &format!("{name} step {}", index + 1),
                    "step",
                    on.get(index).copied(),
                    true,
                );
            }
            if shown.response.hovered() || ui.rect_contains_pointer(shown.response.rect) {
                app.view.hint = Some(format!("{name} steps: click to toggle, drag to paint, right-drag to clear"));
            }
            if !shown.edits.is_empty() {
                paint(app, id, pattern, &shown.edits);
            }
        });
    });
}

/// The channel's pan and volume knobs. Right-clicking one offers an automation clip for it.
fn knobs(app: &mut BodoApp, ui: &mut Ui, channel: &Channel) {
    let (id, name) = (channel.id, channel.name.as_str());
    let mut pan = channel.pan;
    let knob = ui.add(bodo_kit_ui::knob::pan(&mut pan).diameter(18.0).label(format!("{name} pan")));
    control::record(ui, &knob, &format!("{name} pan"), "knob", None);
    app.hint(&knob, format!("{name} pan: {}; right-click to automate it", bodo_kit_ui::knob::format_pan(pan)));
    if knob.changed() {
        app.run_gesture(&knob, "channel.set_pan", json!({ "channel": id, "pan": pan }));
    }
    knob.context_menu(|ui| automate_menu(app, ui, id, "pan"));
    let mut volume = channel.volume;
    let knob = ui
        .add(Knob::new(&mut volume, 0.0..=2.0).default(1.0).diameter(18.0).label(format!("{name} volume")).format(|v| format!("{:.0}%", v * 100.0)));
    control::record(ui, &knob, &format!("{name} volume"), "knob", None);
    app.hint(&knob, format!("{name} volume: {:.0}%; right-click to automate it", volume * 100.0));
    if knob.changed() {
        app.run_gesture(&knob, "channel.set_volume", json!({ "channel": id, "volume": volume }));
    }
    knob.context_menu(|ui| automate_menu(app, ui, id, "volume"));
}

/// A knob's menu: an automation clip for `param` of `channel`, which becomes the playlist's clip source.
fn automate_menu(app: &mut BodoApp, ui: &mut Ui, channel: ChannelId, param: &str) {
    let button = ui.button("Create automation clip");
    control::record(ui, &button, "Create automation clip", "menu item", None);
    if button.clicked()
        && let Ok(result) = app.run("automation.create_for", json!({ "target": { "channel": channel, "param": param } }))
        && let Ok(created) = serde_json::from_value::<ChannelId>(result["channel"].clone())
    {
        app.view.playlist.source = Some(created);
    }
}

/// Turns the painted steps on or off, as one undo step per stroke. Without a current pattern, it makes one first.
fn paint(app: &mut BodoApp, channel: ChannelId, pattern: Option<PatternId>, edits: &[(usize, bool)]) {
    if pattern.is_none() && app.run("pattern.new", json!({})).is_err() {
        return;
    }
    let gesture = match app.view.step_gesture {
        Some(gesture) => gesture,
        None => {
            let gesture = app.new_gesture();
            app.view.step_gesture = Some(gesture);
            gesture
        }
    };
    for (index, on) in edits {
        let call = bodo_engine::Call::new("pattern.toggle_step", json!({ "channel": channel, "step": index + 1, "on": on })).in_gesture(gesture);
        let _ = app.invoke(call);
    }
}

/// The mixer insert the channel plays into: click for the list.
fn insert_picker(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    let tk = Tokens::get();
    let number = channel.target.and_then(|t| bodo_engine::insert_number(project, t)).unwrap_or(0);
    let text = if number == 0 { String::from("M") } else { number.to_string() };
    let button = ui.add(egui::Button::new(RichText::new(text).monospace().size(size::SMALL).color(tk.text_dim)).min_size(vec2(26.0, 18.0)));
    control::record(ui, &button, &format!("{} insert", channel.name), "button", None);
    app.hint(
        &button,
        format!("{} plays into {}; click to choose", channel.name, if number == 0 { String::from("the master") } else { format!("insert {number}") }),
    );
    egui::Popup::menu(&button).show(|ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if ui.selectable_label(number == 0, "Master").clicked() {
                let _ = app.run("channel.set_target_insert", json!({ "channel": channel.id, "insert": 0 }));
            }
            for (n, insert) in project.mixer.order.iter().filter_map(|i| project.mixer.inserts.get(i)).enumerate() {
                if ui.selectable_label(number == n + 1, format!("{} {}", n + 1, insert.name)).clicked() {
                    let _ = app.run("channel.set_target_insert", json!({ "channel": channel.id, "insert": n + 1 }));
                }
            }
        });
    });
}

/// The channel's name button: click selects, double-click opens the piano roll, right-click shows its menu.
fn name_button(app: &mut BodoApp, ui: &mut Ui, channel: &Channel) {
    let tk = Tokens::get();
    let editor = app.session.editor();
    let selected = editor.channels.contains(&channel.id) || editor.channel == Some(channel.id);
    let (rect, response) = ui.allocate_exact_size(vec2(130.0, 22.0), Sense::click());
    let fill = if selected {
        tk.accent_soft
    } else if response.hovered() {
        tk.control_hover
    } else {
        tk.control
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, fill);
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(5.0, rect.height())),
        egui::CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 },
        crate::theme::color(channel.color),
    );
    let text_color = if channel.muted {
        tk.text_faint
    } else if selected {
        tk.accent
    } else {
        tk.text
    };
    let galley = painter.layout(channel.name.clone(), egui::FontId::proportional(size::BODY), text_color, rect.width() - 16.0);
    painter.with_clip_rect(rect.shrink(2.0)).galley(egui::pos2(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
    control::record(ui, &response, &channel.name, "channel", Some(selected));
    // Audio clip and automation channels have no notes: double-clicking one makes it the playlist's clip source.
    let plays_notes = !matches!(channel.kind, ChannelKind::AudioClip { .. } | ChannelKind::Automation(_));
    let double = if plays_notes { "double-click for the piano roll" } else { "double-click to draw it on the playlist" };
    app.hint(&response, format!("{}: click to select, {double}, right-click for more", channel.name));
    if response.double_clicked() {
        if plays_notes {
            let _ = app.run("ui.open_piano_roll", json!({ "channel": channel.id }));
        } else {
            app.view.playlist.source = Some(channel.id);
        }
    } else if response.clicked() {
        let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
        let _ = app.run("edit.select", json!({ "channels": [channel.id], "channel": channel.id, "add": add }));
    }
    response.context_menu(|ui| {
        if ui.button("Rename...").clicked() {
            dialogs::rename_channel(app, channel.id);
        }
        if plays_notes && ui.button("Open in piano roll").clicked() {
            let _ = app.run("ui.open_piano_roll", json!({ "channel": channel.id }));
        }
        if ui.button("Duplicate").clicked() {
            let _ = app.run("channel.duplicate", json!({ "channel": channel.id }));
        }
        if plays_notes
            && ui.button("Load sample...").clicked()
            && let Some(file) = app.services.pick_samples.as_ref().map(|pick| pick()).and_then(|files| files.into_iter().next())
        {
            let _ = app.run("channel.set_sample", json!({ "channel": channel.id, "sample": file.display().to_string() }));
        }
        ui.separator();
        if ui.button("Delete").clicked() {
            let _ = app.run("channel.remove", json!({ "channels": [channel.id] }));
        }
    });
}
