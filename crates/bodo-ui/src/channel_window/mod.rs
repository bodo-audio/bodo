//! Channel windows (project.md sections 10.2 and 14.1): a channel's name button opens its window, a free window of
//! its own. Several can be open at once, each for one channel.
//!
//! Every window but the automation channel's starts with the channel's header: pan, volume, pitch with its range,
//! and the mixer insert it plays into. Below it, the window for the channel's kind: the Sampler window with its
//! Sample and Envelopes tabs (in `sampler`), and the windows of audio clip, automation, layer, MIDI Out and instrument
//! channels (in `kinds`). Every control changes the project through commands; knob drags are one undo step each.

mod kinds;
mod preview;
mod sampler;

use std::ops::RangeInclusive;
use std::sync::Arc;

use bodo_kit_ui::icons;
use bodo_kit_ui::knob::Knob;
use bodo_model::channel::ChannelKind;
use bodo_model::{Channel, ChannelId, Project};
use egui::{Rect, RichText, Ui, vec2};
use serde_json::{Value, json};

pub use self::sampler::SamplerTab;
use crate::i18n::t;
use crate::theme::{Tokens, rem, size};
use crate::windows::{WindowState, show_frame};
use crate::{BodoApp, control};

/// One open channel window.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelWindow {
    /// The channel it shows.
    pub channel: ChannelId,
    /// Where it is and how it shows.
    pub state: WindowState,
    /// The Sampler window's tab.
    pub tab: SamplerTab,
}

/// The open channel windows, oldest first, and the previews of the samplers' samples.
#[derive(Debug, Default)]
pub struct ChannelWindows {
    /// The windows.
    pub open: Vec<ChannelWindow>,
    previews: preview::Previews,
}

impl ChannelWindows {
    /// Whether channel `channel`'s window is open.
    pub fn is_open(&self, channel: ChannelId) -> bool {
        self.open.iter().any(|w| w.channel == channel)
    }

    /// How many sample previews are still being rendered, for tests that wait for them.
    pub fn previews_loading(&mut self) -> usize {
        self.previews.pending()
    }
}

/// The egui id of channel `channel`'s window.
pub fn area_id(channel: ChannelId) -> egui::Id {
    egui::Id::new(("bodo-channel-window", channel))
}

/// Opens channel `channel`'s window, or brings it to the front when it is open.
pub(crate) fn open(app: &mut BodoApp, ctx: Option<&egui::Context>, channel: ChannelId) {
    let windows = &mut app.view.channel_windows;
    if let Some(window) = windows.open.iter_mut().find(|w| w.channel == channel) {
        window.state.open = true;
        window.state.minimized = false;
    } else {
        // Each new window cascades a little from the last, right of the channel rack so its name buttons stay in reach.
        let n = windows.open.len() as f32;
        let frame = [0.44 + 0.02 * (n % 5.0), 0.04 + 0.03 * (n % 5.0), 0.5, 0.82];
        windows.open.push(ChannelWindow { channel, state: WindowState::at(true, frame), tab: SamplerTab::Sample });
    }
    if let Some(ctx) = ctx {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, area_id(channel)));
    }
}

/// Draws every open channel window inside `workspace`, and forgets the ones closed or whose channel is gone.
pub(crate) fn show_all(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let windows = std::mem::take(&mut app.view.channel_windows.open);
    let mut kept = Vec::with_capacity(windows.len());
    for mut window in windows {
        let Some(channel) = project.channels.get(&window.channel).cloned() else { continue };
        let (icon, kind) = match &channel.kind {
            ChannelKind::Sampler(_) => (icons::WAVE_SINE, "Sampler"),
            ChannelKind::Instrument { .. } => (icons::PIANO, "Instrument"),
            ChannelKind::AudioClip { .. } => (icons::FILE_MUSIC, "Audio clip"),
            ChannelKind::Automation(_) => (icons::ADJUSTMENTS_HORIZONTAL, "Automation"),
            ChannelKind::Layer { .. } => (icons::COPY, "Layer"),
            ChannelKind::MidiOut { .. } => (icons::EXTERNAL_LINK, "MIDI Out"),
        };
        let title = format!("{kind} - {}", channel.name);
        // A tab chosen in the title bar shows from the next frame on.
        let shown = window.tab;
        let mut tab = window.tab;
        let (state, events) = show_frame(
            ctx,
            area_id(channel.id),
            icon,
            window.state,
            workspace,
            &title,
            true,
            app,
            |app, ui| {
                if matches!(channel.kind, ChannelKind::Sampler(_)) {
                    sampler::tabs(app, ui, &channel, &mut tab);
                }
            },
            |app, ui| body(app, ui, project, &channel, shown),
        );
        window.state = state;
        window.tab = tab;
        if !events.closed && window.state.open {
            kept.push(window);
        }
    }
    // Windows opened while these drew come after them.
    kept.append(&mut app.view.channel_windows.open);
    app.view.channel_windows.open = kept;
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>, channel: &Channel, tab: SamplerTab) {
    egui::ScrollArea::vertical().id_salt(("channel-window", channel.id)).auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(rem(0.48));
        ui.horizontal(|ui| {
            ui.add_space(rem(0.64));
            ui.vertical(|ui| {
                if !matches!(channel.kind, ChannelKind::Automation(_)) {
                    header(app, ui, project, channel);
                    ui.separator();
                }
                match &channel.kind {
                    ChannelKind::Sampler(params) => sampler::show(app, ui, project, channel, params, tab),
                    ChannelKind::AudioClip { .. } => kinds::audio_clip(app, ui, project, channel),
                    ChannelKind::Automation(lane) => kinds::automation(app, ui, project, channel, lane),
                    ChannelKind::Layer { children } => kinds::layer(app, ui, project, channel, children),
                    ChannelKind::MidiOut { port, channel: midi } => kinds::midi_out(app, ui, channel, port, *midi),
                    ChannelKind::Instrument { plugin } => kinds::instrument(ui, plugin),
                }
                ui.add_space(rem(0.64));
            });
        });
    });
}

/// The channel's header: pan, volume, pitch with its range, and the insert it plays into.
fn header(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    let (id, name) = (channel.id, channel.name.as_str());
    ui.horizontal(|ui| {
        let mut pan = channel.pan;
        labeled(ui, "Pan", |ui| {
            let knob = ui.add(bodo_kit_ui::knob::pan(&mut pan).diameter(rem(2.08)).label(format!("{name} window pan")));
            control::record(ui, &knob, &format!("{name} window pan"), "knob", None);
            app.hint(&knob, format!("Pan: {}", bodo_kit_ui::knob::format_pan(pan)));
            if knob.changed() {
                app.run_gesture(&knob, "channel.set_pan", json!({ "channel": id, "pan": pan }));
            }
        });
        let mut volume = channel.volume;
        labeled(ui, "Volume", |ui| {
            let knob = ui.add(
                Knob::new(&mut volume, 0.0..=2.0)
                    .default(1.0)
                    .diameter(rem(2.08))
                    .label(format!("{name} window volume"))
                    .format(|v| format!("{:.0}%", v * 100.0)),
            );
            control::record(ui, &knob, &format!("{name} window volume"), "knob", None);
            app.hint(&knob, format!("Volume: {:.0}%", volume * 100.0));
            if knob.changed() {
                app.run_gesture(&knob, "channel.set_volume", json!({ "channel": id, "volume": volume }));
            }
        });
        // The pitch knob reaches as far as the sampler's pitch range; other channels use two semitones.
        let range = match &channel.kind {
            ChannelKind::Sampler(params) => params.pitch_range,
            _ => 2.0,
        };
        let mut semitones = channel.pitch_cents / 100.0;
        labeled(ui, "Pitch", |ui| {
            let knob = ui.add(
                Knob::new(&mut semitones, -range..=range)
                    .default(0.0)
                    .origin(0.0)
                    .diameter(rem(2.08))
                    .label(format!("{name} window pitch"))
                    .format(|v| format!("{v:+.2} st")),
            );
            control::record(ui, &knob, &format!("{name} window pitch"), "knob", None);
            app.hint(&knob, format!("Pitch: {semitones:+.2} semitones, up to {range} either way"));
            if knob.changed() {
                app.run_gesture(&knob, "channel.set_pitch", json!({ "channel": id, "cents": (semitones * 100.0).round() }));
            }
        });
        if let ChannelKind::Sampler(params) = &channel.kind {
            labeled(ui, "Range", |ui| {
                let mut pitch_range = params.pitch_range;
                let drag = ui.add(egui::DragValue::new(&mut pitch_range).range(1.0..=48.0).speed(0.2).fixed_decimals(0).suffix(" st"));
                control::record(ui, &drag, &format!("{name} pitch range"), "number", None);
                app.hint(&drag, "How far the pitch knob reaches, in semitones either way");
                if drag.changed() {
                    app.run_gesture(&drag, "sampler.set", json!({ "channel": id, "pitchRange": pitch_range.round() }));
                }
            });
        }
        if !matches!(channel.kind, ChannelKind::Automation(_) | ChannelKind::MidiOut { .. }) {
            labeled(ui, "Insert", |ui| insert_combo(app, ui, project, channel));
        }
    });
}

/// The mixer insert the channel plays into.
fn insert_combo(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    let number = channel.target.and_then(|t| bodo_engine::insert_number(project, t)).unwrap_or(0);
    let shown = if number == 0 { String::from("Master") } else { format!("Insert {number}") };
    let combo = egui::ComboBox::from_id_salt(("channel-window-insert", channel.id)).width(rem(7.68)).selected_text(shown).show_ui(ui, |ui| {
        if ui.selectable_label(number == 0, t("Master")).clicked() {
            let _ = app.run("channel.set_target_insert", json!({ "channel": channel.id, "insert": 0 }));
        }
        for (n, insert) in project.mixer.order.iter().filter_map(|i| project.mixer.inserts.get(i)).enumerate() {
            if ui.selectable_label(number == n + 1, format!("{} {}", n + 1, insert.name)).clicked() {
                let _ = app.run("channel.set_target_insert", json!({ "channel": channel.id, "insert": n + 1 }));
            }
        }
    });
    control::record(ui, &combo.response, &format!("{} window insert", channel.name), "combo", None);
    app.hint(&combo.response, "The mixer insert this channel plays into");
}

/// `add` above a small caption.
fn labeled(ui: &mut Ui, caption: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.set_min_width(rem(3.52));
        add(ui);
        ui.label(RichText::new(caption).size(size::SMALL).color(Tokens::get().text_dim));
    });
}

/// A section heading.
fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(rem(0.32));
    ui.label(RichText::new(text).size(size::BODY).strong().color(Tokens::get().text));
}

/// What a sampler control edits: the channel and the name of the control, for labels and hints.
struct Field<'a> {
    channel: &'a Channel,
    label: &'a str,
    hint: &'a str,
}

impl Field<'_> {
    fn element(&self) -> String {
        format!("{} {}", self.channel.name, self.label)
    }
}

/// A knob for a sampler setting: a drag sends `sampler.set` with `params(value)`, one undo step a drag.
#[allow(clippy::too_many_arguments, reason = "a knob's value, range, default and display, and what it sets")]
fn sampler_knob(
    app: &mut BodoApp,
    ui: &mut Ui,
    field: Field<'_>,
    value: f32,
    range: RangeInclusive<f32>,
    default: f32,
    format: fn(f32) -> String,
    params: impl FnOnce(f32) -> Value,
) {
    let mut value = value;
    labeled(ui, field.label, |ui| {
        let element = field.element();
        let knob = ui.add(Knob::new(&mut value, range).default(default).diameter(rem(2.08)).label(element.clone()).format(format));
        control::record(ui, &knob, &element, "knob", None);
        app.hint(&knob, format!("{}: {}", field.hint, format(value)));
        if knob.changed() {
            app.run_gesture(&knob, "sampler.set", with_channel(field.channel, params(value)));
        }
    });
}

/// A checkbox for a sampler setting.
fn sampler_check(app: &mut BodoApp, ui: &mut Ui, field: Field<'_>, value: bool, params: impl FnOnce(bool) -> Value) {
    let mut value = value;
    let check = ui.checkbox(&mut value, field.label);
    control::record(ui, &check, &field.element(), "checkbox", Some(value));
    app.hint(&check, field.hint);
    if check.changed() {
        let _ = app.run("sampler.set", with_channel(field.channel, params(value)));
    }
}

/// A list of choices for a sampler setting: `options` pairs each choice's label with the params it sends.
fn sampler_combo(app: &mut BodoApp, ui: &mut Ui, field: Field<'_>, current: &str, options: Vec<(&str, Value)>) {
    let mut chosen = None;
    let combo = egui::ComboBox::from_id_salt(field.element()).width(rem(8.8)).selected_text(current).show_ui(ui, |ui| {
        for (label, params) in options {
            if ui.selectable_label(label == current, label).clicked() {
                chosen = Some(params);
            }
        }
    });
    control::record(ui, &combo.response, &field.element(), "combo", None);
    app.hint(&combo.response, field.hint);
    if let Some(params) = chosen {
        let _ = app.run("sampler.set", with_channel(field.channel, params));
    }
}

/// `params` with the channel added.
fn with_channel(channel: &Channel, mut params: Value) -> Value {
    if let Value::Object(map) = &mut params {
        map.insert(String::from("channel"), json!(channel.id));
    }
    params
}

/// The width of the window's controls.
fn width(ui: &Ui) -> f32 {
    (ui.available_width() - 8.0).max(200.0)
}

/// A drawing area `height` tall across the window.
fn canvas(ui: &mut Ui, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(width(ui), height), egui::Sense::hover());
    rect
}
