//! The Mixer window (project.md section 9.7, ADR 0031): a header, a dB scale down the left, the master and the
//! inserts as strips, and the insert panel on the right. The routing graph can take the strips' place, drawing the
//! inserts as nodes and the routes between them.
//!
//! - **The header** has the options menu, the view buttons (strips or graph, the insert panel) and `+` to add an
//!   insert, and the view size: Wide, Compact or Extra compact.
//! - **Strips** (in `strip.rs`) show, from the top, the name (written upward) with its color and icon, the mute LED, the
//!   pan and stereo separation knobs, solo, polarity, swap and arm, the fader with a peak meter and its clip LED, the
//!   route control, and the input it records.
//! - **The insert panel** (in `panel.rs`) shows the selected insert: its input, the ten FX slots, the strip EQ
//!   (in `eq.rs`), its outputs, its latency and CPU use, its track latency and delay compensation bypass.
//! - **Selection:** click a strip to select it; Shift or Cmd/Ctrl click adds or removes it from a group. Dragging a
//!   fader of a group, or any fader with Alt, moves every selected fader by the same amount, one undo step.
//!
//! Meters are polled from the engine each frame and fall back slowly. Everything is drawn for the strips in view only.

mod eq;
mod graph;
mod panel;
mod strip;

use std::collections::HashMap;
use std::sync::Arc;

use bodo_kit_ui::{Icon, icons};
use bodo_model::{InsertId, Project};
use egui::{Rect, RichText, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::{en, t};
use crate::theme::{Tokens, rem, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control};

pub(crate) use self::panel::{EffectDrag, category_name, effects_by_category};

/// How wide the strips are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StripSize {
    /// Every control on the strip.
    #[default]
    Wide,
    /// Narrower strips: the stereo separation knob and the small switches move to the insert panel.
    Compact,
    /// The narrowest strips: the name, the mute LED, the fader and its meter.
    ExtraCompact,
}

impl StripSize {
    /// Every size, in the menu's order.
    pub const ALL: [Self; 3] = [Self::Wide, Self::Compact, Self::ExtraCompact];

    /// Its name in the menu, in English; [`t`] shows it in the interface's language.
    pub fn label(self) -> &'static str {
        match self {
            Self::Wide => en("Wide"),
            Self::Compact => en("Compact"),
            Self::ExtraCompact => en("Extra compact"),
        }
    }

    /// A strip's width in points, in [`rem`] so it scales with the interface.
    pub fn width(self) -> f32 {
        match self {
            Self::Wide => rem(5.12),
            Self::Compact => rem(3.68),
            Self::ExtraCompact => rem(2.4),
        }
    }

    /// Its one- or two-letter mark on the header's size buttons, in the interface's language.
    fn mark(self) -> &'static str {
        match self {
            Self::Wide => t("W"),
            Self::Compact => t("C"),
            Self::ExtraCompact => t("XC"),
        }
    }
}

/// A meter as shown: peaks that fall back slowly, and the clip latch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shown {
    peak: [f32; 2],
    clipped: bool,
}

/// What the mixer remembers between frames.
#[derive(Debug)]
pub struct MixerView {
    /// How wide the strips are.
    pub size: StripSize,
    /// Whether the insert panel shows on the right.
    pub panel: bool,
    /// Whether the routing graph shows in place of the strips.
    pub graph: bool,
    /// The FX slot chosen in the insert panel, 0 to 9: plugin presets load into it.
    pub slot: Option<usize>,
    /// The input channel count of the audio device in use, and the host and device it was looked up for.
    inputs: Option<(Option<String>, Option<String>, u16)>,
    /// The output channel count of the audio device in use, and the host and device it was looked up for.
    outputs: Option<(Option<String>, Option<String>, u16)>,
    /// Each insert's meter as shown.
    meters: HashMap<InsertId, Shown>,
    /// Each insert's CPU use as shown, refreshed a few times a second, with when it last was.
    cpu: HashMap<InsertId, (f32, f64)>,
    /// Delay compensation's decision for the project revision it was worked out for, when no engine runs.
    latency: Option<(u64, Arc<bodo_mix::MixLatency>)>,
}

impl Default for MixerView {
    fn default() -> Self {
        Self {
            size: StripSize::Wide,
            panel: true,
            graph: false,
            slot: None,
            inputs: None,
            outputs: None,
            meters: HashMap::new(),
            cpu: HashMap::new(),
            latency: None,
        }
    }
}

/// How many input or output channels to offer when the device doesn't say: four stereo pairs.
const UNKNOWN_CHANNELS: u16 = 8;

/// The insert panel's width.
const PANEL: f32 = rem(20.16);

/// The dB scale's width.
const SCALE: f32 = rem(2.08);

/// How an insert is named in commands: its number, or `master`.
pub(crate) fn insert_ref(number: usize) -> Value {
    if number == 0 { json!("master") } else { json!(number) }
}

/// How an insert is named on screen and for agents: `Master` or `Insert N`.
pub(crate) fn insert_label(number: usize) -> String {
    if number == 0 { String::from("Master") } else { format!("Insert {number}") }
}

/// The number of insert `id`: 0 for the master, 1 and up for the inserts in mixer order.
///
/// Complexity: O(i) in inserts.
pub(crate) fn number_of(project: &Project, id: InsertId) -> Option<usize> {
    if project.mixer.master.id == id { Some(0) } else { project.mixer.order.iter().position(|i| *i == id).map(|n| n + 1) }
}

/// The insert with number `number`: 0 is the master.
pub fn insert_at(project: &Project, number: usize) -> Option<&Arc<bodo_model::MixerInsert>> {
    if number == 0 { Some(&project.mixer.master) } else { project.mixer.order.get(number - 1).and_then(|id| project.mixer.inserts.get(id)) }
}

/// The icons an insert can show, by the names `mixer.set_icon` stores.
pub(crate) const INSERT_ICONS: [(&str, Icon); 9] = [
    ("drum", icons::DRUM),
    ("guitar", icons::GUITAR),
    ("piano", icons::PIANO),
    ("keyboard", icons::KEYBOARD),
    ("microphone", icons::MICROPHONE),
    ("music", icons::MUSIC),
    ("wave", icons::WAVE_SINE),
    ("speaker", icons::VOLUME),
    ("headphones", icons::HEADPHONES),
];

/// The icon named `name`, if Bodo draws one by that name.
pub(crate) fn icon_named(name: &str) -> Option<Icon> {
    INSERT_ICONS.iter().find(|(n, _)| *n == name).map(|(_, icon)| *icon)
}

/// How many channels the open audio device has for input (`input`) or output, looked up once per device;
/// [`UNKNOWN_CHANNELS`] without sound.
///
/// Complexity: O(d) in the host's devices when the device changed, O(1) otherwise.
pub(crate) fn device_channels(app: &mut BodoApp, input: bool) -> u16 {
    if !app.audio.is_on() {
        return UNKNOWN_CHANNELS;
    }
    let prefs = app.audio.prefs();
    let (host, device) = (prefs.host.clone(), if input { prefs.input.clone() } else { prefs.output.clone() });
    let cached = if input { &app.view.mixer.inputs } else { &app.view.mixer.outputs };
    if let Some((h, d, count)) = cached
        && *h == host
        && *d == device
    {
        return *count;
    }
    let devices = bodo_kit_device::devices(host.as_deref()).unwrap_or_default();
    let found = match &device {
        Some(name) => devices.iter().find(|d| &d.name == name),
        None => devices.iter().find(|d| if input { d.is_default_input } else { d.is_default_output }),
    };
    let count = found.map_or(UNKNOWN_CHANNELS, |d| if input { d.input_channels } else { d.output_channels });
    let entry = Some((host, device, count));
    if input {
        app.view.mixer.inputs = entry;
    } else {
        app.view.mixer.outputs = entry;
    }
    count
}

/// How device channels read: `-` for none, `1` for a mono channel, `1-2` for a pair, with `prefix` before.
pub(crate) fn port_label(prefix: &str, channels: Option<&[u16]>) -> String {
    match channels {
        None | Some([]) => String::from("-"),
        Some([one]) => format!("{prefix}{}", one + 1),
        Some([first, .., last]) => format!("{prefix}{}-{}", first + 1, last + 1),
    }
}

/// The choices of a port selector: none, each stereo pair and each mono channel of `count` channels.
pub(crate) fn port_choices(count: u16) -> Vec<Option<Vec<u16>>> {
    let mut choices: Vec<Option<Vec<u16>>> = vec![None];
    choices.extend((0..count / 2).map(|pair| Some(vec![pair * 2, pair * 2 + 1])));
    choices.extend((0..count).map(|n| Some(vec![n])));
    choices
}

/// The meter of `insert` as shown this frame: the engine's newest peaks, or the shown ones falling back.
fn meter(app: &mut BodoApp, ui: &Ui, insert: InsertId) -> Shown {
    let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.1);
    // Peaks fall back by half every 150 ms.
    let fall = 0.5f32.powf(dt / 0.15);
    let reading = app.audio.meter(insert);
    let shown = app.view.mixer.meters.entry(insert).or_default();
    if let Some(reading) = reading {
        for side in 0..2 {
            shown.peak[side] = reading.peak[side].max(shown.peak[side] * fall);
        }
        shown.clipped = reading.clipped;
    } else {
        shown.peak = [shown.peak[0] * fall, shown.peak[1] * fall];
    }
    *shown
}

/// The CPU use of `insert` as shown, refreshed four times a second; `None` without sound.
fn cpu(app: &mut BodoApp, ui: &Ui, insert: InsertId) -> Option<f32> {
    let now = ui.input(|i| i.time);
    let due = app.view.mixer.cpu.get(&insert).is_none_or(|(_, at)| now - at > 0.25);
    if due && let Some(load) = app.audio.insert_cpu(insert) {
        app.view.mixer.cpu.insert(insert, (load, now));
    }
    app.view.mixer.cpu.get(&insert).map(|(load, _)| *load)
}

/// The latency of `insert` in frames (its input, its own, its output), from the running engine or else from delay
/// compensation worked out for the project as it stands.
fn insert_latency(app: &mut BodoApp, insert: InsertId) -> Option<bodo_mix::InsertLatency> {
    if let Some(latency) = app.audio.insert_latency(insert) {
        return Some(latency);
    }
    mix_latency(app).and_then(|mix| mix.inserts.iter().find(|(id, _)| *id == insert).map(|(_, l)| *l))
}

/// How late the whole mix is, in frames.
pub(crate) fn total_latency(app: &mut BodoApp) -> Option<u32> {
    app.audio.total_latency().or_else(|| mix_latency(app).map(|mix| mix.total))
}

/// Delay compensation for the project as it stands, worked out once per revision.
fn mix_latency(app: &mut BodoApp) -> Option<Arc<bodo_mix::MixLatency>> {
    let revision = app.session.revision();
    if let Some((seen, mix)) = &app.view.mixer.latency
        && *seen == revision
    {
        return Some(Arc::clone(mix));
    }
    let mix = Arc::new(bodo_mix::mix_latency(app.session.project(), app.session.transport().recording).ok()?);
    app.view.mixer.latency = Some((revision, Arc::clone(&mix)));
    Some(mix)
}

/// Draws the Mixer window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let mut layout = std::mem::take(&mut app.prefs.layout);
    windows::show(ctx, &mut layout, WindowKind::Mixer, workspace, "Mixer", app, header, |app, ui| body(app, ui, project));
    app.prefs.layout = layout;
    if app.audio.is_on() && app.prefs.layout.is_open(WindowKind::Mixer) {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn header(app: &mut BodoApp, ui: &mut Ui) {
    let options = ui.menu_button(RichText::new(t("Options")).size(size::SMALL), |ui| {
        ui.set_min_width(rem(15.2));
        for choice in StripSize::ALL {
            if ui.radio(app.view.mixer.size == choice, t(choice.label())).clicked() {
                app.view.mixer.size = choice;
            }
        }
        ui.separator();
        if ui.button(t("Add insert")).clicked() {
            let _ = app.run("mixer.add_insert", json!({}));
        }
        if ui.button(t("Clear every clip light")).clicked() {
            clear_all_clips(app);
        }
        if ui.button(t("Unsolo all")).clicked() {
            unsolo_all(app);
        }
        ui.separator();
        ui.menu_button(t("Pan law"), |ui| {
            let law = app.session.project().settings.pan_law;
            for (value, id, label) in [
                (bodo_kit_dsp::pan::PanLaw::ConstantPower3dB, "constantPower3dB", t("Constant power (-3 dB)")),
                (bodo_kit_dsp::pan::PanLaw::Compromise4_5dB, "compromise4_5dB", t("Compromise (-4.5 dB)")),
                (bodo_kit_dsp::pan::PanLaw::Linear6dB, "linear6dB", t("Linear (-6 dB)")),
            ] {
                if ui.radio(law == value, label).clicked() {
                    let _ = app.run("project.set_pan_law", json!({ "law": id }));
                }
            }
        });
    });
    control::record(ui, &options.response, "Mixer options", "menu", None);
    app.hint(&options.response, "The mixer's options: strip size, clip lights, solo and the pan law");
    let graph = icon_button(ui, icons::ROUTING, "Routing graph", app.view.mixer.graph);
    app.hint(&graph, "Show the routes as a graph of inserts, or the strips again");
    if graph.clicked() {
        app.view.mixer.graph = !app.view.mixer.graph;
    }
    let panel = icon_button(ui, icons::LAYOUT_COLUMNS, "Insert panel", app.view.mixer.panel);
    app.hint(&panel, "Show or hide the insert panel: the selected insert's effects, EQ and outputs");
    if panel.clicked() {
        app.view.mixer.panel = !app.view.mixer.panel;
    }
    let add = icon_button(ui, icons::PLUS, "Add insert", false);
    app.hint(&add, "Add a mixer insert");
    if add.clicked() {
        let _ = app.run("mixer.add_insert", json!({}));
    }
    ui.separator();
    for choice in StripSize::ALL {
        let selected = app.view.mixer.size == choice;
        let button = ui.selectable_label(selected, RichText::new(choice.mark()).size(size::SMALL));
        control::record(ui, &button, &format!("Mixer size {}", choice.label()), "button", Some(selected));
        app.hint(&button, tf!("{size} strips", size = t(choice.label())));
        if button.clicked() {
            app.view.mixer.size = choice;
        }
    }
}

/// Clears the clip light of every insert.
fn clear_all_clips(app: &mut BodoApp) {
    let project = Arc::clone(app.session.project());
    for insert in project.mixer.inserts.values().chain(std::iter::once(&project.mixer.master)) {
        app.audio.clear_clip(insert.id);
        if let Some(shown) = app.view.mixer.meters.get_mut(&insert.id) {
            shown.clipped = false;
        }
    }
}

/// Unsolos every soloed insert, one undo step.
fn unsolo_all(app: &mut BodoApp) {
    let project = Arc::clone(app.session.project());
    let calls: Vec<bodo_engine::Call> = project
        .mixer
        .order
        .iter()
        .enumerate()
        .filter(|(_, id)| project.mixer.inserts.get(*id).is_some_and(|i| i.solo))
        .map(|(n, _)| bodo_engine::Call::new("mixer.solo", json!({ "insert": n + 1, "on": false })))
        .collect();
    if !calls.is_empty() {
        let _ = app.session.batch(&calls, Some(en("Unsolo all")));
    }
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>) {
    let tk = Tokens::get();
    let rect = ui.max_rect();
    let panel_width = if app.view.mixer.panel { PANEL.min(rect.width() * 0.6) } else { 0.0 };
    let strips_rect = Rect::from_min_max(rect.min, pos2(rect.right() - panel_width, rect.bottom()));
    let panel_rect = Rect::from_min_max(pos2(strips_rect.right(), rect.top()), rect.max);
    if app.view.mixer.graph {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(strips_rect));
        graph::show(app, &mut child, project);
    } else {
        let scale = Rect::from_min_max(strips_rect.min, pos2(strips_rect.left() + SCALE, strips_rect.bottom()));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(scale.right(), strips_rect.top()), strips_rect.max)));
        let faders = strips(app, &mut child, project);
        if let Some((top, bottom)) = faders {
            strip::db_scale(ui, scale, top, bottom);
        }
    }
    if panel_width > 0.0 {
        ui.painter().rect_filled(panel_rect, 0.0, tk.chrome_dark);
        ui.painter().vline(panel_rect.left(), panel_rect.y_range(), egui::Stroke::new(1.0, tk.border));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(panel_rect.shrink2(vec2(rem(0.48), rem(0.32)))));
        panel::show(app, &mut child, project);
    }
}

/// The strips in a sideways scroll area. Returns the top and bottom of the faders, for the dB scale.
fn strips(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>) -> Option<(f32, f32)> {
    let size = app.view.mixer.size;
    let mut faders = None;
    egui::ScrollArea::horizontal().id_salt("bodo-mixer-strips").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = rem(0.16);
            ui.add_space(rem(0.16));
            faders = Some(strip::show(app, ui, project, &project.mixer.master, 0, size));
            ui.add_space(rem(0.48));
            for (n, id) in project.mixer.order.iter().enumerate() {
                if let Some(insert) = project.mixer.inserts.get(id) {
                    strip::show(app, ui, project, insert, n + 1, size);
                }
            }
        });
    });
    faders
}
