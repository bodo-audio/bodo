//! The windows of the channel kinds other than the sampler (project.md section 10.2): audio clip channels (sample,
//! stretch mode, tempo and the fades of their clips), automation channels (target, curve and points), layers
//! (children), MIDI Out channels (port and MIDI channel) and instruments (the plugin slot, which plays once plugin
//! hosting arrives).

use bodo_engine::params::format_position;
use bodo_kit_core::Tick;
use bodo_model::automation::{AutomationLane, ChannelParam, InsertParam, ParamTarget, SlotParam};
use bodo_model::channel::{ChannelKind, PluginRef, StretchMode};
use bodo_model::playlist::ClipSource;
use bodo_model::{Channel, ChannelId, ClipId, Project};
use egui::{RichText, Stroke, Ui, pos2};
use imbl::Vector;
use serde_json::json;

use super::{canvas, heading, labeled};
use crate::i18n::t;
use crate::theme::{self, Tokens, rem, size};
use crate::{BodoApp, control};

/// The clips on the playlist that play `channel`.
fn clips_of(project: &Project, channel: ChannelId) -> Vec<(ClipId, Tick, Option<bodo_model::playlist::Fades>)> {
    project
        .tracks
        .values()
        .flat_map(|t| t.clips.iter())
        .filter(|c| matches!(c.source, ClipSource::Audio(id) | ClipSource::Automation(id) if id == channel))
        .map(|c| (c.id, c.start, c.fades))
        .collect()
}

/// An audio clip channel: its sample, how it follows the tempo, and the fades of its clips.
pub(super) fn audio_clip(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    let ChannelKind::AudioClip { sample, stretch, tempo } = &channel.kind else { return };
    let tk = Tokens::get();
    heading(ui, "Sample");
    let name = project.samples.get(sample).map_or_else(|| String::from("Missing sample"), |s| bodo_engine::samples::display_name(&s.path));
    ui.label(RichText::new(name).size(size::BODY));

    heading(ui, "Tempo");
    ui.horizontal(|ui| {
        let current = match stretch {
            StretchMode::None => "Its own speed",
            StretchMode::Resample => "Resample to the tempo",
            StretchMode::Stretch => "Stretch to the tempo",
        };
        let mut chosen = None;
        let combo = egui::ComboBox::from_id_salt(("audio-stretch", channel.id)).width(rem(13.6)).selected_text(current).show_ui(ui, |ui| {
            for (label, mode) in [("Its own speed", "none"), ("Resample to the tempo", "resample"), ("Stretch to the tempo", "stretch")] {
                if ui.selectable_label(label == current, label).clicked() {
                    chosen = Some(mode);
                }
            }
        });
        control::record(ui, &combo.response, &format!("{} stretch mode", channel.name), "combo", None);
        app.hint(&combo.response, "How the clips follow the project's tempo");
        if let Some(mode) = chosen {
            let _ = app.run("clip.set_stretch", json!({ "channel": channel.id, "mode": mode }));
        }
        let mut bpm = tempo.unwrap_or_else(|| project.tempo.bpm_at(Tick::ZERO));
        labeled(ui, "Its tempo", |ui| {
            let drag = ui.add(egui::DragValue::new(&mut bpm).range(10.0..=999.0).speed(0.1).fixed_decimals(2).suffix(" BPM"));
            control::record(ui, &drag, &format!("{} sample tempo", channel.name), "number", None);
            app.hint(&drag, "The tempo the audio was made at, which Resample and Stretch match to the project's");
            if drag.changed() {
                let mode = match stretch {
                    StretchMode::None => "none",
                    StretchMode::Resample => "resample",
                    StretchMode::Stretch => "stretch",
                };
                app.run_gesture(&drag, "clip.set_stretch", json!({ "channel": channel.id, "mode": mode, "tempo": (bpm * 100.0).round() / 100.0 }));
            }
        });
    });

    heading(ui, "Clips");
    let clips = clips_of(project, channel.id);
    if clips.is_empty() {
        ui.label(RichText::new(t("No clips yet: double-click the channel to draw it on the playlist")).size(size::SMALL).color(tk.text_dim));
        return;
    }
    ui.label(
        RichText::new(format!(
            "{} {} on the playlist, from {}",
            clips.len(),
            if clips.len() == 1 { "clip" } else { "clips" },
            format_position(project, clips.iter().map(|c| c.1).min().unwrap_or(Tick::ZERO))
        ))
        .size(size::SMALL)
        .color(tk.text_dim),
    );
    let fades = clips.first().and_then(|c| c.2).unwrap_or_default();
    let ids: Vec<ClipId> = clips.iter().map(|c| c.0).collect();
    ui.horizontal(|ui| {
        for (label, ticks, key) in [("Fade in", fades.fade_in, "fadeIn"), ("Fade out", fades.fade_out, "fadeOut")] {
            let mut beats = ticks.0 as f32 / 960.0;
            labeled(ui, label, |ui| {
                let drag = ui.add(egui::DragValue::new(&mut beats).range(0.0..=64.0).speed(0.02).fixed_decimals(2).suffix(" beats"));
                control::record(ui, &drag, &format!("{} {}", channel.name, label.to_lowercase()), "number", None);
                app.hint(&drag, format!("The {} of every clip of this channel", label.to_lowercase()));
                if drag.changed() {
                    app.run_gesture(&drag, "clip.set_fades", json!({ "clips": ids, key: (beats * 960.0).round() as i64 }));
                }
            });
        }
    });
}

/// What an automation lane controls, in words.
fn target_name(project: &Project, target: &ParamTarget) -> String {
    let insert = |id| bodo_engine::insert_number(project, id).map_or_else(|| String::from("Master"), |n| format!("Insert {n}"));
    match target {
        ParamTarget::Insert(id, param) => {
            let param = match param {
                InsertParam::Volume => "volume",
                InsertParam::Pan => "pan",
                InsertParam::StereoSep => "stereo separation",
            };
            format!("{} {param}", insert(*id))
        }
        ParamTarget::Slot(id, slot, param) => {
            let param = match param {
                SlotParam::Mix => "mix",
                SlotParam::Enabled => "switch",
            };
            format!("{} slot {} {param}", insert(*id), slot + 1)
        }
        ParamTarget::Plugin(_, param) => format!("Plugin parameter {param}"),
        ParamTarget::Channel(id, param) => {
            let name = project.channels.get(id).map_or_else(|| String::from("A removed channel"), |c| c.name.clone());
            let param = match param {
                ChannelParam::Volume => String::from("volume"),
                ChannelParam::Pan => String::from("pan"),
                ChannelParam::Pitch => String::from("pitch"),
                ChannelParam::Cc(n) => format!("CC {n}"),
                ChannelParam::PitchBend => String::from("pitch bend"),
                ChannelParam::Pressure => String::from("channel pressure"),
            };
            format!("{name} {param}")
        }
        ParamTarget::Route(_) => String::from("Send level"),
        ParamTarget::Tempo => String::from("Tempo"),
    }
}

/// An automation channel: its target, its curve, and ways to edit it.
pub(super) fn automation(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, lane: &AutomationLane) {
    let tk = Tokens::get();
    heading(ui, "Target");
    ui.label(RichText::new(target_name(project, &lane.target)).size(size::BODY));
    heading(ui, "Curve");
    let rect = canvas(ui, 90.0);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tk.field);
    let length = lane.points.last().map_or(3840, |p| p.at.0.max(1)).max(960) as f64;
    let columns = rect.width().max(2.0) as usize;
    let points: Vec<_> = (0..=columns)
        .map(|c| {
            let at = c as f64 / columns as f64 * length;
            pos2(rect.left() + c as f32, rect.bottom() - 4.0 - lane.normalized_at(at) * (rect.height() - 8.0))
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, theme::color(channel.color))));
    for point in &lane.points {
        let x = rect.left() + (point.at.0 as f64 / length) as f32 * rect.width();
        painter.circle_filled(pos2(x, rect.bottom() - 4.0 - point.value.clamp(0.0, 1.0) * (rect.height() - 8.0)), 3.0, tk.accent);
    }
    control::record_rect(ui, ui.id().with(("automation-curve", channel.id)), rect, &format!("{} curve", channel.name), "curve", None, false);
    let clips = clips_of(project, channel.id).len();
    ui.label(
        RichText::new(format!("{} points, {clips} {} on the playlist", lane.points.len(), if clips == 1 { "clip" } else { "clips" }))
            .size(size::SMALL)
            .color(tk.text_dim),
    );
    ui.horizontal(|ui| {
        let draw = ui.button(t("Draw on the playlist"));
        control::record(ui, &draw, &format!("{} draw on the playlist", channel.name), "button", None);
        app.hint(&draw, "Make this the clip the playlist's draw and paint tools place, and edit its points there");
        if draw.clicked() {
            app.view.playlist.source = Some(channel.id);
            let _ = app.run("ui.open_window", json!({ "window": "playlist" }));
        }
        let clear = ui.button(t("Clear points"));
        control::record(ui, &clear, &format!("{} clear points", channel.name), "button", None);
        app.hint(&clear, "Remove every point");
        if clear.clicked() {
            let _ = app.run("automation.clear", json!({ "channel": channel.id }));
        }
    });
}

/// A layer: the channels it plays, with buttons to take one out and a list to add one.
pub(super) fn layer(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, children: &Vector<ChannelId>) {
    let tk = Tokens::get();
    heading(ui, "Plays");
    let set = |app: &mut BodoApp, children: Vec<ChannelId>| {
        let _ = app.run("channel.set_layer", json!({ "channel": channel.id, "children": children }));
    };
    if children.is_empty() {
        ui.label(RichText::new(t("Nothing yet: add channels below")).size(size::SMALL).color(tk.text_dim));
    }
    for child in children {
        let Some(c) = project.channels.get(child) else { continue };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(rem(0.48), rem(1.28)), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, theme::color(c.color));
            ui.label(RichText::new(&c.name).size(size::BODY));
            let remove = ui.small_button(t("Remove"));
            control::record(ui, &remove, &format!("{} remove {}", channel.name, c.name), "button", None);
            app.hint(&remove, format!("Stop playing {} from this layer", c.name));
            if remove.clicked() {
                set(app, children.iter().copied().filter(|id| id != child).collect());
            }
        });
    }
    let candidates: Vec<(ChannelId, String)> = project
        .rack_order
        .iter()
        .filter(|id| **id != channel.id && !children.contains(id))
        .filter_map(|id| project.channels.get(id))
        .filter(|c| !matches!(c.kind, ChannelKind::Automation(_) | ChannelKind::AudioClip { .. }))
        .map(|c| (c.id, c.name.clone()))
        .collect();
    let mut chosen = None;
    let combo = egui::ComboBox::from_id_salt(("layer-add", channel.id)).width(rem(12.8)).selected_text(t("Add a channel")).show_ui(ui, |ui| {
        for (id, name) in &candidates {
            if ui.selectable_label(false, name).clicked() {
                chosen = Some(*id);
            }
        }
    });
    control::record(ui, &combo.response, &format!("{} add channel", channel.name), "combo", None);
    app.hint(&combo.response, "Add a channel the layer plays on every note");
    if let Some(id) = chosen {
        set(app, children.iter().copied().chain(std::iter::once(id)).collect());
    }
}

/// A MIDI Out channel: the port it sends to and the MIDI channel.
pub(super) fn midi_out(app: &mut BodoApp, ui: &mut Ui, channel: &Channel, port: &str, midi: u8) {
    heading(ui, "MIDI output");
    ui.horizontal(|ui| {
        let mut ports = bodo_kit_device::midi::output_ports();
        if !ports.iter().any(|p| p == port) {
            ports.insert(0, port.to_owned());
        }
        let mut chosen = None;
        let combo = egui::ComboBox::from_id_salt(("midi-out-port", channel.id)).width(rem(16.0)).selected_text(port).show_ui(ui, |ui| {
            for name in &ports {
                if ui.selectable_label(name == port, name).clicked() {
                    chosen = Some(name.clone());
                }
            }
        });
        control::record(ui, &combo.response, &format!("{} port", channel.name), "combo", None);
        app.hint(&combo.response, "The MIDI port this channel's notes go to");
        if let Some(name) = chosen {
            let _ = app.run("channel.set_midi_out", json!({ "channel": channel.id, "port": name }));
        }
        let mut number = f32::from(midi) + 1.0;
        labeled(ui, "MIDI channel", |ui| {
            let drag = ui.add(egui::DragValue::new(&mut number).range(1.0..=16.0).speed(0.1).fixed_decimals(0));
            control::record(ui, &drag, &format!("{} MIDI channel", channel.name), "number", None);
            app.hint(&drag, "The MIDI channel, 1 to 16");
            if drag.changed() {
                app.run_gesture(&drag, "channel.set_midi_out", json!({ "channel": channel.id, "midiChannel": number.round() as u8 }));
            }
        });
    });
}

/// An instrument channel: its plugin, which plays once plugin hosting arrives (project.md section 20, M5).
pub(super) fn instrument(ui: &mut Ui, plugin: &PluginRef) {
    let tk = Tokens::get();
    heading(ui, "Plugin");
    ui.label(RichText::new(format!("{} ({:?})", plugin.uid, plugin.format)).size(size::BODY));
    ui.label(
        RichText::new(t("Bodo keeps this plugin and its state with the project; it plays and shows its editor once plugin hosting arrives."))
            .size(size::SMALL)
            .color(tk.text_dim),
    );
}
