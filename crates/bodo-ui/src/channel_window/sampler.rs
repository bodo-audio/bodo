//! The Sampler window (project.md section 10.2).
//!
//! The **Sample** tab has the sample file and its content options, a waveform of the sample as it plays with the
//! start, end, start offset and loop markers to drag, the playback and declicking settings, time stretching and the
//! precomputed effects. The **Envelopes** tab has the volume, filter and pitch envelopes, each a curve with points to
//! drag, the filter, the arpeggiator, and the root key and key range.

use bodo_engine::params::key_name;
use bodo_kit_core::Tick;
use bodo_kit_dsp::envelope::AdsrParams;
use bodo_model::channel::{ArpDirection, Declick, FilterKind, LoopMode, SamplerParams, SamplerStretchMode};
use bodo_model::{Channel, Project};
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::{Field, canvas, heading, labeled, sampler_check, sampler_combo, sampler_knob, with_channel};
use crate::theme::{self, Tokens, size};
use crate::{BodoApp, control};

/// The Sampler window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SamplerTab {
    /// The sample, playback, stretching and precomputed effects.
    #[default]
    Sample,
    /// The envelopes, filter, arpeggiator and keys.
    Envelopes,
}

/// The tab buttons, in the title bar.
pub(super) fn tabs(app: &mut BodoApp, ui: &mut Ui, channel: &Channel, tab: &mut SamplerTab) {
    for (choice, label) in [(SamplerTab::Sample, "Sample"), (SamplerTab::Envelopes, "Envelopes")] {
        let button = ui.selectable_label(*tab == choice, label);
        control::record(ui, &button, &format!("{} tab {label}", channel.name), "tab", Some(*tab == choice));
        app.hint(&button, format!("Show the Sampler's {} settings", label.to_lowercase()));
        if button.clicked() {
            *tab = choice;
        }
    }
}

/// The body of the Sampler window.
pub(super) fn show(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, params: &SamplerParams, tab: SamplerTab) {
    match tab {
        SamplerTab::Sample => sample_tab(app, ui, project, channel, params),
        SamplerTab::Envelopes => envelopes_tab(app, ui, channel, params),
    }
}

fn field<'a>(channel: &'a Channel, label: &'a str, hint: &'a str) -> Field<'a> {
    Field { channel, label, hint }
}

fn sample_tab(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, params: &SamplerParams) {
    heading(ui, "Sample");
    ui.horizontal(|ui| {
        let name = params
            .sample
            .and_then(|s| project.samples.get(&s))
            .map_or_else(|| String::from("No sample"), |s| bodo_engine::samples::display_name(&s.path));
        ui.label(egui::RichText::new(name).size(size::BODY));
        let load = ui.button("Load...");
        control::record(ui, &load, &format!("{} load sample", channel.name), "button", None);
        app.hint(&load, "Load another audio file into this sampler");
        if load.clicked()
            && let Some(file) = app.services.pick_samples.as_ref().map(|pick| pick()).and_then(|files| files.into_iter().next())
        {
            let _ = app.run("channel.set_sample", json!({ "channel": channel.id, "sample": file.display().to_string() }));
        }
    });
    ui.horizontal_wrapped(|ui| {
        sampler_check(
            app,
            ui,
            field(channel, "Keep on disk", "Stream the sample from disk instead of loading it, where the playback allows"),
            params.keep_on_disk,
            |v| json!({ "keepOnDisk": v }),
        );
        sampler_check(
            app,
            ui,
            field(channel, "Resample", "Convert the sample to the project's rate when it loads"),
            params.resample,
            |v| json!({ "resample": v }),
        );
        sampler_check(
            app,
            ui,
            field(channel, "Load regions", "Play the regions marked in the file, one per key from the root key"),
            params.load_regions,
            |v| json!({ "loadRegions": v }),
        );
        sampler_check(
            app,
            ui,
            field(channel, "Load slices", "Play the slices between the file's markers, one per key from the root key"),
            params.load_slices,
            |v| json!({ "loadSlices": v }),
        );
    });
    waveform(app, ui, project, channel, params);

    heading(ui, "Playback");
    ui.horizontal_wrapped(|ui| {
        let loop_label = match params.loop_mode {
            LoopMode::Off => "No loop",
            LoopMode::Forward => "Loop",
            LoopMode::PingPong => "Ping-pong loop",
        };
        sampler_combo(
            app,
            ui,
            field(channel, "Loop", "How the sample loops between the loop markers"),
            loop_label,
            vec![
                ("No loop", json!({ "loopMode": "off" })),
                ("Loop", json!({ "loopMode": "forward" })),
                ("Ping-pong loop", json!({ "loopMode": "pingPong" })),
            ],
        );
        let declick = match params.declick {
            Declick::Off => "No declicking",
            Declick::Transient => "Transient",
            Declick::Generic => "Generic",
            Declick::Smooth => "Smooth",
        };
        sampler_combo(
            app,
            ui,
            field(channel, "Declicking", "Short fades at the start and end of notes against clicks"),
            declick,
            vec![
                ("No declicking", json!({ "declick": "off" })),
                ("Transient", json!({ "declick": "transient" })),
                ("Generic", json!({ "declick": "generic" })),
                ("Smooth", json!({ "declick": "smooth" })),
            ],
        );
        sampler_check(
            app,
            ui,
            field(channel, "One-shot", "Play the whole sample on every note, however short"),
            params.one_shot,
            |v| json!({ "oneShot": v }),
        );
        sampler_check(app, ui, field(channel, "Reverse", "Play the sample backwards"), params.reverse, |v| json!({ "reverse": v }));
        let mut polyphony = f32::from(params.polyphony);
        labeled(ui, "Voices", |ui| {
            let drag = ui.add(egui::DragValue::new(&mut polyphony).range(1.0..=256.0).speed(0.3).fixed_decimals(0));
            control::record(ui, &drag, &format!("{} voices", channel.name), "number", None);
            app.hint(&drag, "The most notes the sampler plays at once");
            if drag.changed() {
                app.run_gesture(&drag, "sampler.set", with_channel(channel, json!({ "polyphony": polyphony.round() as u16 })));
            }
        });
    });

    heading(ui, "Time stretching");
    ui.horizontal_wrapped(|ui| {
        let mode = if params.stretch.mode == SamplerStretchMode::Stretch { "Stretch" } else { "Resample" };
        sampler_combo(
            app,
            ui,
            field(channel, "Stretch mode", "Resample changes length and pitch together, like tape; Stretch changes them apart"),
            mode,
            vec![("Resample", json!({ "stretch": { "mode": "resample" } })), ("Stretch", json!({ "stretch": { "mode": "stretch" } }))],
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Pitch", "Pitch shift"),
            params.stretch.pitch_cents / 100.0,
            -24.0..=24.0,
            0.0,
            |v| format!("{v:+.1} st"),
            |v| json!({ "stretch": { "pitchCents": (v * 100.0).round() } }),
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Multiplier", "How many times longer the sample plays"),
            params.stretch.multiplier.max(0.05).log2(),
            0.05f32.log2()..=20f32.log2(),
            0.0,
            |v| format!("x{:.2}", 2f32.powf(v)),
            |v| json!({ "stretch": { "multiplier": (2f32.powf(v) * 1000.0).round() / 1000.0 } }),
        );
        let mut beats = params.stretch.time_beats.unwrap_or(0.0) as f32;
        labeled(ui, "Time", |ui| {
            let drag = ui.add(
                egui::DragValue::new(&mut beats)
                    .range(0.0..=1024.0)
                    .speed(0.1)
                    .fixed_decimals(2)
                    .custom_formatter(|v, _| if v <= 0.0 { String::from("Off") } else { format!("{v:.2} beats") }),
            );
            control::record(ui, &drag, &format!("{} time", channel.name), "number", None);
            app.hint(&drag, "Fit the played part to this many beats at the project's tempo; off uses the multiplier");
            if drag.changed() {
                app.run_gesture(&drag, "sampler.set", with_channel(channel, json!({ "stretch": { "timeBeats": beats } })));
            }
        });
    });

    heading(ui, "Precomputed effects");
    let fx = params.effects;
    ui.horizontal_wrapped(|ui| {
        for (label, hint, value, key) in [
            ("Trim", "Cut silence from both ends", fx.trim, "trim"),
            ("Remove DC", "Remove any DC offset", fx.remove_dc, "removeDc"),
            ("Normalize", "Bring the peak to full scale", fx.normalize, "normalize"),
            ("Reverse polarity", "Invert the waveform", fx.reverse_polarity, "reversePolarity"),
            ("Swap stereo", "Swap left and right", fx.swap_stereo, "swapStereo"),
            ("Fade stereo", "Move the sound from left to right over its length", fx.fade_stereo, "fadeStereo"),
        ] {
            sampler_check(app, ui, field(channel, label, hint), value, |v| json!({ "effects": { key: v } }));
        }
    });
    ui.horizontal(|ui| {
        sampler_knob(
            app,
            ui,
            field(channel, "Fade in", "A fade-in at the start"),
            seconds_knob(fx.fade_in),
            0.0..=1.0,
            0.0,
            format_seconds_knob,
            |v| json!({ "effects": { "fadeIn": knob_seconds(v) } }),
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Fade out", "A fade-out at the end"),
            seconds_knob(fx.fade_out),
            0.0..=1.0,
            0.0,
            format_seconds_knob,
            |v| json!({ "effects": { "fadeOut": knob_seconds(v) } }),
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Crossfade", "Smooth the loop by crossfading its end into the audio before its start"),
            fx.crossfade,
            0.0..=0.5,
            0.0,
            |v| format!("{:.0}%", v * 100.0),
            |v| json!({ "effects": { "crossfade": (v * 1000.0).round() / 1000.0 } }),
        );
    });
}

/// Seconds from 0 to 10 on a knob from 0 to 1, finer near zero.
fn seconds_knob(seconds: f32) -> f32 {
    (seconds.max(0.0) / 10.0).sqrt().min(1.0)
}

fn knob_seconds(v: f32) -> f32 {
    ((10.0 * v * v) * 1000.0).round() / 1000.0
}

fn format_seconds_knob(v: f32) -> String {
    let seconds = 10.0 * v * v;
    if seconds < 1.0 { format!("{:.0} ms", seconds * 1000.0) } else { format!("{seconds:.2} s") }
}

/// The markers on the waveform, and where each sits as a fraction of the sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Start,
    End,
    Offset,
    LoopStart,
    LoopEnd,
}

impl Marker {
    fn label(self) -> &'static str {
        match self {
            Marker::Start => "start marker",
            Marker::End => "end marker",
            Marker::Offset => "start offset marker",
            Marker::LoopStart => "loop start marker",
            Marker::LoopEnd => "loop end marker",
        }
    }
}

/// The waveform of the sample as it plays, with its markers to drag and its slices or regions.
fn waveform(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, params: &SamplerParams) {
    let tk = Tokens::get();
    let rect = canvas(ui, 116.0);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tk.field);
    // A sound dropped from the browser loads into the sampler.
    let drop = ui.interact(rect, ui.id().with(("sampler-drop", channel.id)), Sense::hover());
    if let Some(payload) = drop.dnd_release_payload::<crate::browser::SoundPayload>() {
        let _ = app.run("channel.set_sample", json!({ "channel": channel.id, "sample": payload.0 }));
    }
    if drop.dnd_hover_payload::<crate::browser::SoundPayload>().is_some() {
        painter.rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0, tk.accent), egui::StrokeKind::Inside);
    }
    let path = params.sample.and_then(|s| project.samples.get(&s)).map(|s| app.session.sample_roots().resolve(&s.path));
    let Some(path) = path else {
        crate::windows::placeholder(ui, rect, "Load a sample, or drop one from the browser");
        return;
    };
    let bpm = project.tempo.bpm_at(Tick::ZERO);
    let preview = app.view.channel_windows.previews.get(ui.ctx(), channel.id, &path, params, bpm);
    let x_of = |fraction: f32| rect.left() + fraction.clamp(0.0, 1.0) * rect.width();
    let color = theme::color(channel.color);
    if let Some(preview) = &preview {
        let middle = rect.center().y;
        let half = rect.height() * 0.45;
        let columns = rect.width().max(1.0) as usize;
        for column in 0..columns {
            let (from, to) = (column as f32 / columns as f32, (column + 1) as f32 / columns as f32);
            if let Some((low, high)) = preview.range(from, to) {
                let x = rect.left() + column as f32 + 0.5;
                painter.line_segment(
                    [pos2(x, middle - high.clamp(-1.0, 1.0) * half), pos2(x, middle - low.clamp(-1.0, 1.0) * half + 0.5)],
                    Stroke::new(1.0, color),
                );
            }
        }
        for (start, _) in &preview.zones {
            painter.vline(x_of(*start), rect.y_range(), Stroke::new(1.0, theme::faded(tk.text, 0.5)));
        }
        let slices = preview.zones.len();
        if slices > 0 {
            let note = format!("{slices} {} from {}", if params.load_regions { "regions" } else { "slices" }, key_name(params.root_key));
            painter.text(rect.right_top() + vec2(-6.0, 4.0), Align2::RIGHT_TOP, note, FontId::proportional(size::SMALL), tk.text_dim);
        }
    } else {
        crate::windows::placeholder(ui, rect, "Reading the sample...");
    }
    // The parts outside the start and end markers are dimmed; the loop is tinted.
    let (start, end) = (x_of(params.start), x_of(params.end));
    painter.rect_filled(Rect::from_min_max(rect.min, pos2(start, rect.bottom())), 0.0, theme::faded(tk.chrome_dark, 0.6));
    painter.rect_filled(Rect::from_min_max(pos2(end, rect.top()), rect.max), 0.0, theme::faded(tk.chrome_dark, 0.6));
    if params.loop_mode != LoopMode::Off {
        let looped = Rect::from_min_max(pos2(x_of(params.loop_start), rect.top()), pos2(x_of(params.loop_end), rect.bottom()));
        painter.rect_filled(looped, 0.0, theme::faded(tk.accent, 0.12));
    }
    let offset = params.start + params.start_offset * (params.end - params.start);
    let mut markers = vec![(Marker::Start, params.start), (Marker::End, params.end), (Marker::Offset, offset)];
    if params.loop_mode != LoopMode::Off {
        markers.extend([(Marker::LoopStart, params.loop_start), (Marker::LoopEnd, params.loop_end)]);
    }
    for (marker, fraction) in markers {
        let x = x_of(fraction);
        let top = matches!(marker, Marker::Start | Marker::End | Marker::Offset);
        let handle_y = if top { rect.top() + 6.0 } else { rect.bottom() - 6.0 };
        let handle = Rect::from_center_size(pos2(x, handle_y), vec2(12.0, 12.0));
        let stroke_color = match marker {
            Marker::Start | Marker::End => tk.accent,
            Marker::Offset => tk.text,
            Marker::LoopStart | Marker::LoopEnd => tk.ok,
        };
        painter.vline(x, rect.y_range(), Stroke::new(if marker == Marker::Offset { 1.0 } else { 1.5 }, stroke_color));
        painter.add(egui::Shape::convex_polygon(triangle(handle, top), stroke_color, Stroke::NONE));
        let label = format!("{} {}", channel.name, marker.label());
        let response = ui.interact(handle.expand(3.0), ui.id().with(("sampler-marker", channel.id, marker as u8)), Sense::drag());
        control::record(ui, &response, &label, "marker", None);
        app.hint(&response, format!("Drag the {}", marker.label()));
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let round = |v: f32| (v * 10_000.0).round() / 10_000.0;
            let params_json = match marker {
                Marker::Start => json!({ "start": round(at.min(params.end - 0.001)) }),
                Marker::End => json!({ "end": round(at.max(params.start + 0.001)) }),
                Marker::Offset => json!({ "startOffset": round(((at - params.start) / (params.end - params.start).max(1e-4)).clamp(0.0, 1.0)) }),
                Marker::LoopStart => json!({ "loopStart": round(at.min(params.loop_end - 0.001)) }),
                Marker::LoopEnd => json!({ "loopEnd": round(at.max(params.loop_start + 0.001)) }),
            };
            app.run_gesture(&response, "sampler.set", with_channel(channel, params_json));
        } else if response.drag_stopped() {
            app.run_gesture(&response, "sampler.set", with_channel(channel, json!({})));
        }
    }
    control::record_rect(ui, ui.id().with(("sampler-waveform", channel.id)), rect, &format!("{} waveform", channel.name), "waveform", None, false);
}

/// A marker's handle: a triangle pointing into the waveform.
fn triangle(handle: Rect, top: bool) -> Vec<Pos2> {
    if top {
        vec![handle.left_top(), handle.right_top(), handle.center_bottom()]
    } else {
        vec![handle.left_bottom(), handle.right_bottom(), handle.center_top()]
    }
}

fn envelopes_tab(app: &mut BodoApp, ui: &mut Ui, channel: &Channel, params: &SamplerParams) {
    heading(ui, "Volume envelope");
    envelope(app, ui, channel, "Volume", "envelope", params.envelope, None);

    heading(ui, "Filter");
    ui.horizontal_wrapped(|ui| {
        let kind = match params.filter.kind {
            FilterKind::Off => "No filter",
            FilterKind::LowPass => "Low pass",
            FilterKind::HighPass => "High pass",
            FilterKind::BandPass => "Band pass",
            FilterKind::Notch => "Notch",
        };
        sampler_combo(
            app,
            ui,
            field(channel, "Filter type", "The filter every voice goes through"),
            kind,
            vec![
                ("No filter", json!({ "filter": { "kind": "off" } })),
                ("Low pass", json!({ "filter": { "kind": "lowPass" } })),
                ("High pass", json!({ "filter": { "kind": "highPass" } })),
                ("Band pass", json!({ "filter": { "kind": "bandPass" } })),
                ("Notch", json!({ "filter": { "kind": "notch" } })),
            ],
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Cutoff", "The filter's cutoff"),
            (params.filter.cutoff.clamp(20.0, 20_000.0) / 20.0).log(1000.0),
            0.0..=1.0,
            1.0,
            |v| format!("{:.0} Hz", 20.0 * 1000f32.powf(v)),
            |v| json!({ "filter": { "cutoff": (20.0 * 1000f32.powf(v)).round().clamp(20.0, 20_000.0) } }),
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Resonance", "How much the filter rings at its cutoff"),
            params.filter.resonance,
            0.0..=1.0,
            0.0,
            percent,
            |v| json!({ "filter": { "resonance": round3(v) } }),
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Key tracking", "How far the cutoff follows the key: at 100% an octave per octave"),
            params.filter.key_tracking,
            0.0..=1.0,
            0.0,
            percent,
            |v| json!({ "filter": { "keyTracking": round3(v) } }),
        );
    });

    heading(ui, "Filter envelope");
    envelope(
        app,
        ui,
        channel,
        "Filter",
        "filterEnvelope",
        params.filter_envelope.adsr,
        Some((params.filter_envelope.enabled, params.filter_envelope.amount, 8.0, "oct")),
    );
    heading(ui, "Pitch envelope");
    envelope(
        app,
        ui,
        channel,
        "Pitch",
        "pitchEnvelope",
        params.pitch_envelope.adsr,
        Some((params.pitch_envelope.enabled, params.pitch_envelope.amount, 48.0, "st")),
    );

    heading(ui, "Arpeggiator");
    arpeggiator(app, ui, channel, params);

    heading(ui, "Keys");
    ui.horizontal(|ui| {
        for (label, value, key, hint) in [
            ("Root key", params.root_key, "rootKey", "The key that plays the sample at its own pitch; slices start here"),
            ("Lowest key", params.key_range.0, "lowKey", "The lowest key the sampler plays"),
            ("Highest key", params.key_range.1, "highKey", "The highest key the sampler plays"),
        ] {
            let mut number = f32::from(value);
            labeled(ui, label, |ui| {
                let drag =
                    ui.add(egui::DragValue::new(&mut number).range(0.0..=127.0).speed(0.25).custom_formatter(|v, _| key_name(v.round() as u8)));
                control::record(ui, &drag, &format!("{} {}", channel.name, label.to_lowercase()), "number", None);
                app.hint(&drag, hint);
                if drag.changed() {
                    app.run_gesture(&drag, "sampler.set", with_channel(channel, json!({ key: number.round() as u8 })));
                }
            });
        }
    });
}

fn percent(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// An envelope: its curve with points to drag, and knobs for its times and sustain. A filter or pitch envelope also
/// has its switch and amount, `(enabled, amount, range, unit)`.
fn envelope(app: &mut BodoApp, ui: &mut Ui, channel: &Channel, name: &str, key: &str, adsr: AdsrParams, modulation: Option<(bool, f32, f32, &str)>) {
    let tk = Tokens::get();
    let enabled = modulation.is_none_or(|(on, ..)| on);
    if let Some((on, amount, range, unit)) = modulation {
        ui.horizontal(|ui| {
            let mut on = on;
            let check = ui.checkbox(&mut on, "On");
            let element = format!("{} {} envelope on", channel.name, name.to_lowercase());
            control::record(ui, &check, &element, "checkbox", Some(on));
            app.hint(&check, format!("Let the envelope move the {}", name.to_lowercase()));
            if check.changed() {
                let _ = app.run("sampler.set", with_channel(channel, json!({ key: { "enabled": on } })));
            }
            let mut value = amount;
            let knob = ui.add(bodo_kit_ui::knob::Knob::new(&mut value, -range..=range).default(0.0).origin(0.0).diameter(22.0).label(format!(
                "{} {} amount",
                channel.name,
                name.to_lowercase()
            )));
            control::record(ui, &knob, &format!("{} {} amount", channel.name, name.to_lowercase()), "knob", None);
            app.hint(&knob, format!("How far the envelope moves the {}: {value:+.1} {unit}", name.to_lowercase()));
            if knob.changed() {
                app.run_gesture(&knob, "sampler.set", with_channel(channel, json!({ key: { "amount": (value * 10.0).round() / 10.0 } })));
            }
            ui.label(egui::RichText::new(format!("{value:+.1} {unit}")).size(size::SMALL).color(tk.text_dim));
        });
    }
    // The curve: each stage's width grows with the square root of its time, up to ten seconds.
    let rect = canvas(ui, 64.0);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tk.field);
    let unit = rect.width() / 3.6;
    let width_of = |seconds: f64| ((seconds / 10.0).sqrt() as f32).min(1.0) * unit;
    let seconds_of = |width: f32| (10.0 * f64::from((width / unit).clamp(0.0, 1.0)).powi(2) * 1000.0).round() / 1000.0;
    let y_of = |level: f32| rect.bottom() - 4.0 - level.clamp(0.0, 1.0) * (rect.height() - 8.0);
    let x_attack = rect.left() + 4.0 + width_of(adsr.attack.0);
    let x_decay = x_attack + width_of(adsr.decay.0);
    let x_hold = x_decay + unit * 0.5;
    let x_release = x_hold + width_of(adsr.release.0);
    let points = [
        pos2(rect.left() + 4.0, y_of(0.0)),
        pos2(x_attack, y_of(1.0)),
        pos2(x_decay, y_of(adsr.sustain)),
        pos2(x_hold, y_of(adsr.sustain)),
        pos2(x_release, y_of(0.0)),
    ];
    let line = if enabled { theme::color(channel.color) } else { tk.text_faint };
    painter.add(egui::Shape::line(points.to_vec(), Stroke::new(1.5, line)));
    // Later handles take the pointer first. The attack point comes last, so when there is no attack and no decay and
    // both points sit in the corner, a drag pulls the attack out before the decay.
    for (index, (stage, point)) in [("release", points[4]), ("decay", points[2]), ("attack", points[1])].into_iter().enumerate() {
        painter.circle_filled(point, 4.0, line);
        let label = format!("{} {} envelope {stage} point", channel.name, name.to_lowercase());
        let response =
            ui.interact(Rect::from_center_size(point, vec2(14.0, 14.0)), ui.id().with(("envelope-point", channel.id, key, index)), Sense::drag());
        control::record(ui, &response, &label, "point", None);
        app.hint(
            &response,
            match stage {
                "attack" => "Drag sideways to set the attack",
                "decay" => "Drag sideways to set the decay, up and down for the sustain level",
                _ => "Drag sideways to set the release",
            },
        );
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let change = match stage {
                "attack" => json!({ "attack": seconds_of(pointer.x - points[0].x) }),
                "decay" => json!({
                    "decay": seconds_of(pointer.x - points[1].x),
                    "sustain": round3(((rect.bottom() - 4.0 - pointer.y) / (rect.height() - 8.0)).clamp(0.0, 1.0)),
                }),
                _ => json!({ "release": seconds_of(pointer.x - points[3].x) }),
            };
            app.run_gesture(&response, "sampler.set", with_channel(channel, json!({ key: change })));
        }
    }
    ui.horizontal(|ui| {
        for (label, seconds, stage) in [("Attack", adsr.attack.0, "attack"), ("Decay", adsr.decay.0, "decay"), ("Release", adsr.release.0, "release")]
        {
            let text = format!("{name} {}", label.to_lowercase());
            sampler_knob(
                app,
                ui,
                field(channel, &text, label),
                seconds_knob(seconds as f32),
                0.0..=1.0,
                0.0,
                format_seconds_knob,
                |v| json!({ key: { stage: knob_seconds(v) } }),
            );
        }
        let text = format!("{name} sustain");
        sampler_knob(
            app,
            ui,
            field(channel, &text, "Sustain level"),
            adsr.sustain,
            0.0..=1.0,
            1.0,
            percent,
            |v| json!({ key: { "sustain": round3(v) } }),
        );
    });
}

/// The arpeggiator's settings.
fn arpeggiator(app: &mut BodoApp, ui: &mut Ui, channel: &Channel, params: &SamplerParams) {
    let arp = params.arp;
    ui.horizontal_wrapped(|ui| {
        sampler_check(
            app,
            ui,
            field(channel, "Arpeggiate", "Play held keys one after another, in time with the song"),
            arp.enabled,
            |v| json!({ "arp": { "enabled": v } }),
        );
        let direction = match arp.direction {
            ArpDirection::Up => "Up",
            ArpDirection::Down => "Down",
            ArpDirection::UpDown => "Up and down",
            ArpDirection::DownUp => "Down and up",
            ArpDirection::Random => "Random",
            ArpDirection::AsPlayed => "As played",
        };
        sampler_combo(
            app,
            ui,
            field(channel, "Arp direction", "The order the held keys play in"),
            direction,
            [("Up", "up"), ("Down", "down"), ("Up and down", "upDown"), ("Down and up", "downUp"), ("Random", "random"), ("As played", "asPlayed")]
                .into_iter()
                .map(|(label, id)| (label, json!({ "arp": { "direction": id } })))
                .collect(),
        );
        const RATES: [(&str, i64); 7] =
            [("1/4 step", 60), ("1/3 step", 80), ("1/2 step", 120), ("1 step", 240), ("2 steps", 480), ("1 beat", 960), ("1 bar", 3840)];
        let rate = RATES.iter().find(|(_, ticks)| *ticks == arp.rate).map_or_else(|| format!("{} ticks", arp.rate), |(label, _)| (*label).to_owned());
        sampler_combo(
            app,
            ui,
            field(channel, "Arp rate", "Time from one note to the next"),
            &rate,
            RATES.iter().map(|(label, ticks)| (*label, json!({ "arp": { "rate": ticks } }))).collect(),
        );
        sampler_knob(
            app,
            ui,
            field(channel, "Gate", "How long each note lasts, as a share of the step"),
            arp.gate,
            0.01..=1.0,
            0.5,
            percent,
            |v| json!({ "arp": { "gate": round3(v.max(0.01)) } }),
        );
        for (label, value, key, max, hint) in [
            ("Octaves", arp.octaves, "octaves", 4.0, "How many octaves up the pattern climbs"),
            ("Repeat", arp.repeat, "repeat", 8.0, "How many times each note plays before the next"),
        ] {
            let mut number = f32::from(value);
            labeled(ui, label, |ui| {
                let drag = ui.add(egui::DragValue::new(&mut number).range(1.0..=max).speed(0.1).fixed_decimals(0));
                control::record(ui, &drag, &format!("{} arp {}", channel.name, label.to_lowercase()), "number", None);
                app.hint(&drag, hint);
                if drag.changed() {
                    app.run_gesture(&drag, "sampler.set", with_channel(channel, json!({ "arp": { key: number.round() as u8 } })));
                }
            });
        }
    });
}
