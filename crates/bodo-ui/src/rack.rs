//! The Channel Rack (project.md section 10.2): one row per channel with its mute LED, pan and volume knobs, mixer
//! insert, name button and the step sequencer for the current pattern. Steps paint by dragging, and one stroke is one
//! undo step. Drop a sound from the browser to add a channel. Audio clip and automation channels play from the
//! playlist, so their rows have no steps; an automation channel has no knobs either. Right-click a knob to automate it.
//!
//! The header holds the options menu (rack-wide actions such as filling steps or moving channels to a group), the
//! group filter, the swing knob for every channel shown, the graph editor and the compact view. Rows reorder by
//! dragging their name buttons. The graph editor, under the rows, edits the velocity, pitch, pan or release of each
//! step of the selected channel with brush strokes, one undo step each.

use std::sync::Arc;

use bodo_engine::Call;
use bodo_engine::params::STEP;
use bodo_kit_ui::{Knob, Led, StepRow, icons};
use bodo_model::{Channel, ChannelId, ChannelKind, Note, PatternId, Project};
use egui::{Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::browser::SoundPayload;
use crate::i18n::{en, t};
use crate::theme::{self, Tokens, rem, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control, dialogs, topbar};

/// The height of a channel row, and of a compact one.
const ROW: f32 = rem(2.08);
const ROW_COMPACT: f32 = rem(1.44);
/// The size of one step, and of a compact one.
const STEP_SIZE: egui::Vec2 = vec2(rem(1.2), rem(1.76));
const STEP_COMPACT: egui::Vec2 = vec2(rem(1.04), rem(1.2));
/// The graph editor's height.
const GRAPH: f32 = rem(6.72);

/// Which channels the rack shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GroupFilter {
    /// Every channel.
    #[default]
    All,
    /// Channels in no group.
    Unsorted,
    /// The channels of one group.
    Group(String),
}

impl GroupFilter {
    /// Whether `channel` shows under this filter.
    pub fn shows(&self, channel: &Channel) -> bool {
        match self {
            Self::All => true,
            Self::Unsorted => channel.group.is_none(),
            Self::Group(name) => channel.group.as_deref() == Some(name.as_str()),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::All => String::from(t("All channels")),
            Self::Unsorted => String::from(t("Unsorted")),
            Self::Group(name) => name.clone(),
        }
    }
}

/// What the graph editor edits on each step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepParam {
    /// How hard the step plays.
    #[default]
    Velocity,
    /// The step's key, up or down from the channel's root key.
    Pitch,
    /// The step's pan.
    Pan,
    /// The step's release velocity.
    Release,
}

impl StepParam {
    /// Every parameter, in the menu's order.
    pub const ALL: [Self; 4] = [Self::Velocity, Self::Pitch, Self::Pan, Self::Release];

    /// Its name, in English.
    pub fn label(self) -> &'static str {
        match self {
            Self::Velocity => en("Velocity"),
            Self::Pitch => en("Pitch"),
            Self::Pan => en("Pan"),
            Self::Release => en("Release"),
        }
    }

    /// The graph editor's hint while it edits this parameter, in English.
    fn hint(self) -> &'static str {
        match self {
            Self::Velocity => en("Drag across the bars to set each step's velocity"),
            Self::Pitch => en("Drag across the bars to set each step's pitch"),
            Self::Pan => en("Drag across the bars to set each step's pan"),
            Self::Release => en("Drag across the bars to set each step's release"),
        }
    }

    /// Whether its bars grow from the middle.
    fn centered(self) -> bool {
        matches!(self, Self::Pitch | Self::Pan)
    }

    /// `note`'s value as a height from 0 to 1, with `root` the channel's step key.
    fn height(self, note: &Note, root: u8) -> f32 {
        match self {
            Self::Velocity => f32::from(note.velocity) / f32::from(u16::MAX),
            Self::Pitch => (f32::from(note.key) - f32::from(root) + 12.0) / 24.0,
            Self::Pan => (note.pan + 1.0) / 2.0,
            Self::Release => f32::from(note.release_velocity) / f32::from(u16::MAX),
        }
        .clamp(0.0, 1.0)
    }

    /// The `note.set_props` parameters for height `h`.
    fn params(self, h: f32, root: u8) -> Value {
        let h = h.clamp(0.0, 1.0);
        match self {
            Self::Velocity => json!({ "velocity": (h * 100.0).round() / 100.0 }),
            Self::Pitch => json!({ "key": (i32::from(root) + (h * 24.0 - 12.0).round() as i32).clamp(0, 127) }),
            Self::Pan => json!({ "pan": ((h * 2.0 - 1.0) * 100.0).round() / 100.0 }),
            Self::Release => json!({ "releaseVelocity": (h * 100.0).round() / 100.0 }),
        }
    }
}

/// A group name being typed: a new group for the selected channels, or a new name for a group.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Naming {
    renaming: Option<String>,
    text: String,
}

/// The channel rack's filter, view options, graph editor and the gestures in progress.
#[derive(Debug, Default)]
pub struct RackView {
    /// The group shown.
    pub filter: GroupFilter,
    /// Smaller rows.
    pub compact: bool,
    /// Whether the graph editor shows under the rows.
    pub graph: bool,
    /// What the graph editor edits.
    pub graph_param: StepParam,
    /// The MIDI input ports the channel menus offer, listed when a menu first needs them.
    pub midi_ports: Option<Vec<String>>,
    /// The channel being dragged to a new place.
    dragging: Option<ChannelId>,
    /// A group name being typed.
    naming: Option<Naming>,
    /// The graph editor's stroke in progress: its gesture and where the pointer was last frame.
    stroke: Option<(u64, Pos2)>,
    /// Where the steps start, measured on last frame's rows.
    steps_left: Option<f32>,
}

/// Draws the Channel Rack window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let pattern = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).map(|p| (p.id, p.name.clone()));
    let mut layout = std::mem::take(&mut app.prefs.layout);
    let title = match &pattern {
        Some((_, name)) => format!("Channel rack - {name}"),
        None => String::from("Channel rack"),
    };
    let pattern = pattern.map(|(id, _)| id);
    windows::show(
        ctx,
        &mut layout,
        WindowKind::ChannelRack,
        workspace,
        &title,
        app,
        |app, ui| header(app, ui, project, pattern),
        |app, ui| body(app, ui, project, pattern),
    );
    app.prefs.layout = layout;
    naming_dialog(app, ctx);
}

/// The channels the rack shows, in rack order.
///
/// Complexity: O(c) in channels.
fn shown(app: &BodoApp, project: &Project) -> Vec<ChannelId> {
    project.rack_order.iter().filter(|id| project.channels.get(*id).is_some_and(|c| app.view.rack.filter.shows(c))).copied().collect()
}

/// The channels rack-wide actions act on: the selected ones that show, or every channel shown.
fn targets(app: &BodoApp, project: &Project) -> Vec<ChannelId> {
    let all = shown(app, project);
    let selected: Vec<ChannelId> = all.iter().filter(|c| app.session.editor().channels.contains(c)).copied().collect();
    if selected.is_empty() { all } else { selected }
}

/// The group names channels use, sorted.
fn groups(project: &Project) -> Vec<String> {
    let mut names: Vec<String> = project.channels.values().filter_map(|c| c.group.clone()).collect();
    names.sort();
    names.dedup();
    names
}

/// How many steps the rack shows for `pattern`.
fn step_count(project: &Project, pattern: Option<PatternId>) -> usize {
    pattern.map_or(16, |p| (bodo_engine::pattern_length(project, p).0 / STEP.0).max(16) as usize)
}

fn header(app: &mut BodoApp, ui: &mut Ui, project: &Project, pattern: Option<PatternId>) {
    let options = icon_button(ui, icons::MENU_2, "Channel rack options", false);
    app.hint(&options, "Channel rack options: add, clone, delete, select, fill steps, groups");
    egui::Popup::menu(&options).show(|ui| options_menu(app, ui, project, pattern));

    // The group filter.
    let current = app.view.rack.filter.label();
    let combo = egui::ComboBox::from_id_salt("bodo-rack-filter").width(rem(7.68)).selected_text(current).show_ui(ui, |ui| {
        let mut choices = vec![GroupFilter::All, GroupFilter::Unsorted];
        choices.extend(groups(project).into_iter().map(GroupFilter::Group));
        for choice in choices {
            if ui.selectable_label(app.view.rack.filter == choice, choice.label()).clicked() {
                app.view.rack.filter = choice;
            }
        }
        if let GroupFilter::Group(name) = app.view.rack.filter.clone() {
            ui.separator();
            if ui.button(tf!("Rename {name}...", name = name)).clicked() {
                app.view.rack.naming = Some(Naming { renaming: Some(name.clone()), text: name });
            }
        }
    });
    control::record(ui, &combo.response, "Channel rack group", "combo", None);
    app.hint(&combo.response, "Which channels the rack shows: all, those in no group, or one group");

    // Swing for every channel shown.
    let ids = shown(app, project);
    let mut swing = ids.first().and_then(|c| project.channels.get(c)).map_or(0.0, |c| c.swing);
    let knob = ui.add(Knob::new(&mut swing, 0.0..=1.0).default(0.0).diameter(rem(1.6)).label(t("Swing")).format(|v| format!("{:.0}%", v * 100.0)));
    control::record(ui, &knob, "Channel rack swing", "knob", None);
    app.hint(&knob, tf!("Swing for the channels shown: {percent}%", percent = format!("{:.0}", swing * 100.0)));
    if knob.changed() && !ids.is_empty() {
        app.run_gesture(&knob, "channel.set_swing", json!({ "channels": ids, "swing": swing }));
    }

    let graph = icon_button(ui, icons::ADJUSTMENTS_HORIZONTAL, "Graph editor", app.view.rack.graph);
    app.hint(&graph, "Graph editor: the velocity, pitch, pan or release of each step of the selected channel");
    if graph.clicked() {
        app.view.rack.graph = !app.view.rack.graph;
    }
    let compact = icon_button(ui, icons::LAYOUT_COLUMNS, "Compact rows", app.view.rack.compact);
    app.hint(&compact, "Compact rows: smaller rows and steps");
    if compact.clicked() {
        app.view.rack.compact = !app.view.rack.compact;
    }
}

/// The options menu: rack-wide actions on the selected channels, or on every channel shown.
fn options_menu(app: &mut BodoApp, ui: &mut Ui, project: &Project, pattern: Option<PatternId>) {
    ui.set_min_width(rem(16.0));
    ui.menu_button(t("Add channel"), |ui| topbar::add_channel_menu(app, ui));
    ui.separator();
    let chosen = targets(app, project);
    if ui.button(t("Select all")).clicked() {
        let _ = app.run("edit.select", json!({ "channels": shown(app, project) }));
        ui.close();
    }
    if ui.button(t("Clone selected")).clicked() {
        let selected: Vec<ChannelId> = shown(app, project).into_iter().filter(|c| app.session.editor().channels.contains(c)).collect();
        let calls: Vec<Call> = selected.iter().map(|c| Call::new("channel.duplicate", json!({ "channel": c }))).collect();
        if !calls.is_empty() {
            let _ = app.session.batch(&calls, Some(en("Clone channels")));
        }
        ui.close();
    }
    if ui.button(t("Delete selected")).clicked() {
        let _ = app.run("channel.remove", json!({}));
        ui.close();
    }
    ui.separator();
    for (every, label) in FILLS {
        if ui.button(t(label)).clicked() {
            fill(app, project, pattern, &chosen, Some(every));
            ui.close();
        }
    }
    if ui.button(t("Clear steps")).clicked() {
        fill(app, project, pattern, &chosen, None);
        ui.close();
    }
    ui.separator();
    ui.menu_button(t("Move to group"), |ui| group_menu(app, ui, project, &chosen));
}

/// The groups the channels `ids` can move to: unsorted, an existing group, or a new one.
fn group_menu(app: &mut BodoApp, ui: &mut Ui, project: &Project, ids: &[ChannelId]) {
    if ui.button(t("Unsorted")).clicked() {
        let _ = app.run("channel.set_group", json!({ "channels": ids, "group": null }));
        ui.close();
    }
    for name in groups(project) {
        if ui.button(&name).clicked() {
            let _ = app.run("channel.set_group", json!({ "channels": ids, "group": name }));
            ui.close();
        }
    }
    if ui.button(t("New group...")).clicked() {
        if !ids.is_empty() {
            let _ = app.run("edit.select", json!({ "channels": ids }));
        }
        app.view.rack.naming = Some(Naming { renaming: None, text: String::from(t("Group")) });
        ui.close();
    }
}

/// The ways to fill steps, every second, fourth or eighth step, with their names, which also name the undo step.
const FILLS: [(usize, &str); 3] = [(2, en("Fill each 2 steps")), (4, en("Fill each 4 steps")), (8, en("Fill each 8 steps"))];

/// Turns on every `every`th step of `channels` and turns the others off, or clears them all with `None`, as one undo
/// step. Without a current pattern, it makes one first.
fn fill(app: &mut BodoApp, project: &Project, pattern: Option<PatternId>, channels: &[ChannelId], every: Option<usize>) {
    if pattern.is_none() && app.run("pattern.new", json!({})).is_err() {
        return;
    }
    let steps = step_count(project, app.session.editor().pattern);
    let text: String = (0..steps).map(|i| if every.is_some_and(|n| i % n == 0) { 'x' } else { '.' }).collect();
    let calls: Vec<Call> = channels
        .iter()
        .filter(|c| project.channels.get(*c).is_some_and(|c| plays_steps(c)))
        .map(|c| Call::new("pattern.set_steps", json!({ "channel": c, "steps": text })))
        .collect();
    if !calls.is_empty() {
        let label = every.and_then(|n| FILLS.iter().find(|(every, _)| *every == n)).map_or(en("Clear steps"), |(_, label)| *label);
        let _ = app.session.batch(&calls, Some(label));
    }
}

/// Whether `channel` has steps: audio clip and automation channels play from the playlist.
fn plays_steps(channel: &Channel) -> bool {
    !matches!(channel.kind, ChannelKind::AudioClip { .. } | ChannelKind::Automation(_))
}

/// The dialog that names a new group or renames one.
fn naming_dialog(app: &mut BodoApp, ctx: &egui::Context) {
    let Some(naming) = app.view.rack.naming.as_mut() else { return };
    let title = if naming.renaming.is_some() { t("Rename group") } else { t("New group") };
    let mut done = None;
    egui::Modal::new(egui::Id::new("bodo-rack-naming")).show(ctx, |ui| {
        ui.label(RichText::new(title).strong());
        let edit = ui.text_edit_singleline(&mut naming.text);
        control::record(ui, &edit, "Group name", "text", None);
        edit.request_focus();
        ui.horizontal(|ui| {
            if ui.button(t("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = Some(true);
            }
            if ui.button(t("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
    });
    let Some(ok) = done else { return };
    let Some(naming) = app.view.rack.naming.take() else { return };
    let name = naming.text.trim().to_owned();
    if !ok || name.is_empty() {
        return;
    }
    let result = match &naming.renaming {
        Some(old) => app.run("channel.rename_group", json!({ "group": old, "name": name })),
        None => app.run("channel.set_group", json!({ "group": name })),
    };
    if result.is_ok() {
        app.view.rack.filter = GroupFilter::Group(name);
    }
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>, pattern: Option<PatternId>) {
    let tk = Tokens::get();
    let steps = step_count(project, pattern);
    let paint_id = ui.id().with("step-paint");
    let rect = ui.max_rect();
    let ids = shown(app, project);
    let mut rows: Vec<(ChannelId, Rect)> = Vec::with_capacity(ids.len());
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(rem(0.48));
        ui.spacing_mut().item_spacing = vec2(rem(0.48), if app.view.rack.compact { rem(0.16) } else { rem(0.32) });
        for id in &ids {
            if let Some(channel) = project.channels.get(id) {
                let rect = row(app, ui, project, channel, pattern, steps, paint_id);
                rows.push((*id, rect));
            }
        }
        if ids.is_empty() {
            ui.add_space(rem(0.96));
            ui.horizontal(|ui| {
                ui.add_space(rem(0.96));
                let text = if project.rack_order.is_empty() {
                    t("No channels yet. Drag a sound here from the browser, or press +.")
                } else {
                    t("No channels in this group. Choose another in the header, or move channels here from the options menu.")
                };
                ui.label(RichText::new(text).color(tk.text_faint));
            });
        }
        ui.add_space(rem(0.32));
        ui.horizontal(|ui| {
            ui.add_space(rem(0.64));
            let add = icon_button(ui, icons::PLUS, "Add channel", false);
            app.hint(&add, "Add a channel: a factory sound, an empty sampler, or a file");
            egui::Popup::menu(&add).show(|ui| topbar::add_channel_menu(app, ui));
        });
        if app.view.rack.graph {
            ui.add_space(rem(0.48));
            graph_editor(app, ui, project, pattern, steps);
        }
        ui.add_space(rem(0.64));
    });
    reorder(app, ui, project, &rows);
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

/// While a name button is dragged, draws where the channel would land; on release, moves it there.
fn reorder(app: &mut BodoApp, ui: &Ui, project: &Project, rows: &[(ChannelId, Rect)]) {
    let Some(dragged) = app.view.rack.dragging else { return };
    let (pointer, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_down()));
    let Some(pointer) = pointer else { return };
    // The row the pointer is over, and whether it is in that row's lower half.
    let target = rows.iter().find(|(_, r)| pointer.y < r.bottom()).or(rows.last()).map(|(id, r)| (*id, pointer.y > r.center().y));
    if let Some((over, after)) = target
        && let Some((_, rect)) = rows.iter().find(|(id, _)| *id == over)
    {
        let y = if after { rect.bottom() + 1.0 } else { rect.top() - 1.0 };
        ui.painter().hline(rect.x_range(), y, Stroke::new(2.0, Tokens::get().accent));
    }
    if down {
        return;
    }
    app.view.rack.dragging = None;
    let Some((over, after)) = target else { return };
    let order: Vec<ChannelId> = project.rack_order.iter().copied().filter(|c| *c != dragged).collect();
    let Some(index) = order.iter().position(|c| *c == over).map(|i| if after { i + 1 } else { i }) else { return };
    if project.rack_order.index_of(&dragged) != Some(index) {
        let _ = app.run("channel.reorder", json!({ "channel": dragged, "to": index + 1 }));
    }
}

#[allow(clippy::too_many_arguments, reason = "a row draws from the project, the channel and the pattern at once")]
fn row(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, pattern: Option<PatternId>, steps: usize, paint_id: egui::Id) -> Rect {
    let id = channel.id;
    let name = channel.name.clone();
    let compact = app.view.rack.compact;
    let step_size = if compact { STEP_COMPACT } else { STEP_SIZE };
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(if compact { ROW_COMPACT } else { ROW });
            ui.add_space(rem(0.64));
            // Mute: the LED is lit while the channel plays.
            let mut on = !channel.muted;
            let led = ui.add(Led::new(&mut on).label(tf!("{name} on", name = name)));
            control::record(ui, &led, &format!("{name} on"), "led", Some(!channel.muted));
            app.hint(&led, tf!("{name}: click to mute or unmute", name = name));
            if led.changed() {
                let _ = app.run("channel.mute", json!({ "channel": id, "muted": !on }));
            }
            if matches!(channel.kind, ChannelKind::Automation(_)) {
                // An automation channel plays no sound: room where the knobs and insert would be keeps the rows lined up.
                for width in [18.0, 18.0, 26.0] {
                    ui.allocate_exact_size(vec2(width, rem(1.44)), Sense::hover());
                }
            } else {
                knobs(app, ui, channel, compact);
                insert_picker(app, ui, project, channel);
            }
            name_button(app, ui, project, channel, compact);
            if !plays_steps(channel) {
                let text = if matches!(channel.kind, ChannelKind::AudioClip { .. }) {
                    t("Audio clip: place it on the playlist")
                } else {
                    t("Automation clip: edit it on the playlist")
                };
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
            let step_row = StepRow::new(name.clone(), &on)
                .paint_id(paint_id)
                .step_size(step_size)
                .step_names(|n| tf!("{name} step {number}", name = name, number = n));
            let offsets: Vec<f32> = (0..on.len()).map(|i| step_row.offset(i)).collect();
            let shown = step_row.show(ui);
            app.view.rack.steps_left = Some(shown.response.rect.left());
            for (index, offset) in offsets.iter().enumerate() {
                let rect = Rect::from_min_size(shown.response.rect.min + vec2(*offset, 0.0), step_size);
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
                app.view.hint = Some(tf!("{name} steps: click to toggle, drag to paint, right-drag to clear", name = name));
            }
            if !shown.edits.is_empty() {
                paint(app, id, pattern, &shown.edits);
            }
        })
        .response
        .rect
    })
    .inner
}

/// The channel's pan and volume knobs. Right-clicking one offers an automation clip for it.
fn knobs(app: &mut BodoApp, ui: &mut Ui, channel: &Channel, compact: bool) {
    let (id, name) = (channel.id, channel.name.as_str());
    let diameter = if compact { 14.0 } else { 18.0 };
    let mut pan = channel.pan;
    let knob = ui.add(bodo_kit_ui::knob::pan(&mut pan).diameter(diameter).label(tf!("{name} pan", name = name)));
    control::record(ui, &knob, &format!("{name} pan"), "knob", None);
    app.hint(&knob, tf!("{name} pan: {value}; right-click to automate it", name = name, value = bodo_kit_ui::knob::format_pan(pan)));
    if knob.changed() {
        app.run_gesture(&knob, "channel.set_pan", json!({ "channel": id, "pan": pan }));
    }
    knob.context_menu(|ui| automate_menu(app, ui, id, "pan"));
    let mut volume = channel.volume;
    let knob = ui.add(
        Knob::new(&mut volume, 0.0..=2.0)
            .default(1.0)
            .diameter(diameter)
            .label(tf!("{name} volume", name = name))
            .format(|v| format!("{:.0}%", v * 100.0)),
    );
    control::record(ui, &knob, &format!("{name} volume"), "knob", None);
    app.hint(&knob, tf!("{name} volume: {percent}%; right-click to automate it", name = name, percent = format!("{:.0}", volume * 100.0)));
    if knob.changed() {
        app.run_gesture(&knob, "channel.set_volume", json!({ "channel": id, "volume": volume }));
    }
    knob.context_menu(|ui| automate_menu(app, ui, id, "volume"));
}

/// A knob's menu: an automation clip for `param` of `channel`, which becomes the playlist's clip source.
fn automate_menu(app: &mut BodoApp, ui: &mut Ui, channel: ChannelId, param: &str) {
    let button = ui.button(t("Create automation clip"));
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
        let call = Call::new("pattern.toggle_step", json!({ "channel": channel, "step": index + 1, "on": on })).in_gesture(gesture);
        let _ = app.invoke(call);
    }
}

/// The mixer insert the channel plays into: click for the list.
fn insert_picker(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    let tk = Tokens::get();
    let number = channel.target.and_then(|t| bodo_engine::insert_number(project, t)).unwrap_or(0);
    let text = if number == 0 { String::from("M") } else { number.to_string() };
    let button = ui.add(egui::Button::new(RichText::new(text).monospace().size(size::SMALL).color(tk.text_dim)).min_size(vec2(rem(2.08), rem(1.44))));
    control::record(ui, &button, &format!("{} insert", channel.name), "button", None);
    let hint = if number == 0 {
        tf!("{name} plays into the master; click to choose", name = channel.name)
    } else {
        tf!("{name} plays into insert {number}; click to choose", name = channel.name, number = number)
    };
    app.hint(&button, hint);
    egui::Popup::menu(&button).show(|ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if ui.selectable_label(number == 0, t("Master")).clicked() {
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

/// The channel's name button: click selects, double-click opens the piano roll, drag moves the row, right-click shows
/// its menu.
fn name_button(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel, compact: bool) {
    let tk = Tokens::get();
    let editor = app.session.editor();
    let selected = editor.channels.contains(&channel.id) || editor.channel == Some(channel.id);
    let (rect, response) = ui.allocate_exact_size(vec2(rem(8.0), if compact { rem(1.28) } else { rem(1.76) }), Sense::click_and_drag());
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
    let font = if compact { size::SMALL } else { size::BODY };
    let galley = painter.layout(channel.name.clone(), egui::FontId::proportional(font), text_color, rect.width() - 16.0);
    // A channel assigned a mixer track shows it under its name, small: "Mixer 3 · Kick"; in compact rows, the number
    // at the right.
    let track = channel.target.and_then(|t| Some((bodo_engine::insert_number(project, t)?, project.mixer.inserts.get(&t)?.name.clone())));
    let clip = painter.with_clip_rect(rect.shrink(2.0));
    match &track {
        Some((number, track_name)) if !compact => {
            let sub = painter.layout_no_wrap(
                tf!("Mixer {number} · {name}", number = number, name = track_name),
                egui::FontId::proportional(rem(0.62)),
                tk.text_dim,
            );
            let top = rect.center().y - (galley.size().y + sub.size().y) / 2.0;
            clip.galley(egui::pos2(rect.left() + 11.0, top), galley.clone(), text_color);
            let at = egui::pos2(rect.left() + 11.0, top + galley.size().y - rem(0.1));
            let line = Rect::from_min_size(at, sub.size());
            clip.galley(at, sub, tk.text_dim);
            control::record_rect(
                ui,
                ui.id().with(("mixer track", channel.id)),
                line,
                &format!("Mixer {number} · {track_name}"),
                "readout",
                None,
                false,
            );
        }
        Some((number, _)) => {
            clip.galley(egui::pos2(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley.clone(), text_color);
            clip.text(
                rect.right_center() - vec2(rem(0.35), 0.0),
                egui::Align2::RIGHT_CENTER,
                number.to_string(),
                egui::FontId::monospace(rem(0.62)),
                tk.text_dim,
            );
        }
        None => clip.galley(egui::pos2(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley.clone(), text_color),
    }
    control::record(ui, &response, &channel.name, "channel", Some(selected));
    // Audio clip and automation channels have no notes: double-clicking one makes it the playlist's clip source.
    let plays_notes = plays_steps(channel);
    let hint = if plays_notes {
        tf!("{name}: click to select, double-click for the piano roll, drag to move, right-click for more", name = channel.name)
    } else {
        tf!("{name}: click to select, double-click to draw it on the playlist, drag to move, right-click for more", name = channel.name)
    };
    app.hint(&response, hint);
    if response.drag_started() {
        app.view.rack.dragging = Some(channel.id);
    }
    if response.double_clicked() {
        if plays_notes {
            let _ = app.run("ui.open_piano_roll", json!({ "channel": channel.id }));
        } else {
            app.view.playlist.source = Some(channel.id);
        }
    } else if response.clicked() {
        let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
        let _ = app.run("edit.select", json!({ "channels": [channel.id], "channel": channel.id, "add": add }));
        // A plain click also opens the channel's window, such as the Sampler.
        if !add {
            let _ = app.run("ui.open_channel_window", json!({ "channel": channel.id }));
        }
    }
    response.context_menu(|ui| channel_menu(app, ui, project, channel));
}

/// A channel's menu: rename, open, clone, load a sample, fill its steps, color, MIDI input, group and delete.
fn channel_menu(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    let plays_notes = plays_steps(channel);
    if ui.button(t("Rename...")).clicked() {
        dialogs::rename_channel(app, channel.id);
        ui.close();
    }
    if plays_notes && ui.button(t("Open in piano roll")).clicked() {
        let _ = app.run("ui.open_piano_roll", json!({ "channel": channel.id }));
        ui.close();
    }
    if ui.button(t("Clone")).clicked() {
        let _ = app.run("channel.duplicate", json!({ "channel": channel.id }));
        ui.close();
    }
    mixer_track_items(app, ui, project, channel);
    if plays_notes
        && ui.button(t("Load sample...")).clicked()
        && let Some(file) = app.services.pick_samples.as_ref().map(|pick| pick()).and_then(|files| files.into_iter().next())
    {
        let _ = app.run("channel.set_sample", json!({ "channel": channel.id, "sample": file.display().to_string() }));
        ui.close();
    }
    if plays_notes {
        ui.menu_button(t("Fill steps"), |ui| {
            let pattern = app.session.editor().pattern;
            for (every, label) in [(2, en("Each 2 steps")), (4, en("Each 4 steps")), (8, en("Each 8 steps"))] {
                if ui.button(t(label)).clicked() {
                    fill(app, project, pattern, &[channel.id], Some(every));
                    ui.close();
                }
            }
            if ui.button(t("Clear")).clicked() {
                fill(app, project, pattern, &[channel.id], None);
                ui.close();
            }
        });
    }
    ui.menu_button(t("Color"), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(rem(11.2));
            for hue in bodo_engine::palette::HUES {
                let (rect, swatch) = ui.allocate_exact_size(vec2(rem(1.28), rem(1.28)), Sense::click());
                ui.painter().rect_filled(rect, 3.0, theme::color(hue));
                if hue == channel.color {
                    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.5, Tokens::get().text), StrokeKind::Outside);
                }
                control::record(ui, &swatch, &format!("Color {hue}"), "swatch", Some(hue == channel.color));
                if swatch.clicked() {
                    let _ = app.run("channel.set_color", json!({ "channel": channel.id, "color": hue.to_string() }));
                }
            }
        });
    });
    if plays_notes {
        ui.menu_button(t("MIDI input"), |ui| midi_menu(app, ui, channel));
    }
    ui.menu_button(t("Group"), |ui| group_menu(app, ui, project, &[channel.id]));
    ui.separator();
    if ui.button(t("Delete")).clicked() {
        let _ = app.run("channel.remove", json!({ "channels": [channel.id] }));
        ui.close();
    }
}

/// The channel menu's mixer track items: assign it to the track selected in the mixer, or to a free one when none is
/// selected; to a free one anyway; or unassign it, so it plays into the master.
fn mixer_track_items(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: &Channel) {
    ui.separator();
    let master = project.mixer.master.id;
    let selected = app.session.editor().insert.filter(|i| *i != master && project.mixer.inserts.contains_key(i));
    let assign = ui.button(t("Assign to mixer track"));
    control::record(ui, &assign, &format!("{} assign to mixer track", channel.name), "button", None);
    let hint = match selected.and_then(|i| Some((bodo_engine::insert_number(project, i)?, project.mixer.inserts.get(&i)?.name.clone()))) {
        Some((number, name)) => tf!("Plays into mixer track {number}, {name}, the one selected in the mixer", number = number, name = name),
        None => t("Plays into a free mixer track; one with its default name takes the channel's name").to_owned(),
    };
    app.hint(&assign, hint);
    if assign.clicked() {
        let _ = app.run("channel.assign_insert", json!({ "channel": channel.id }));
        ui.close();
    }
    if selected.is_some() {
        let free = ui.button(t("Assign to a free mixer track"));
        control::record(ui, &free, &format!("{} assign to a free mixer track", channel.name), "button", None);
        if free.clicked() {
            let _ = app.run("channel.assign_insert", json!({ "channel": channel.id, "free": true }));
            ui.close();
        }
    }
    if channel.target.is_some() {
        let unassign = ui.button(t("Unassign from mixer track"));
        control::record(ui, &unassign, &format!("{} unassign from mixer track", channel.name), "button", None);
        app.hint(&unassign, t("The channel plays into the master again"));
        if unassign.clicked() {
            let _ = app.run("channel.unassign_insert", json!({ "channel": channel.id }));
            ui.close();
        }
    }
    ui.separator();
}

/// The MIDI inputs a channel can listen to: none, or a port with every MIDI channel or one of them.
fn midi_menu(app: &mut BodoApp, ui: &mut Ui, channel: &Channel) {
    let ports = app.view.rack.midi_ports.get_or_insert_with(bodo_kit_device::midi::input_ports).clone();
    let bound = channel.midi_in.as_ref();
    if ui.selectable_label(bound.is_none(), t("None")).clicked() {
        let _ = app.run("channel.set_midi_in", json!({ "channel": channel.id, "port": null }));
        ui.close();
    }
    if ports.is_empty() {
        ui.label(RichText::new(t("No MIDI inputs found")).color(Tokens::get().text_faint));
    }
    for port in ports {
        let on_port = bound.is_some_and(|b| b.port == port);
        ui.menu_button(if on_port { tf!("{port} (in use)", port = port) } else { port.clone() }, |ui| {
            if ui.selectable_label(on_port && bound.is_some_and(|b| b.channel.is_none()), t("Every MIDI channel")).clicked() {
                let _ = app.run("channel.set_midi_in", json!({ "channel": channel.id, "port": port, "midiChannel": null }));
                ui.close();
            }
            for n in 1..=16u8 {
                let on = on_port && bound.is_some_and(|b| b.channel == Some(n - 1));
                if ui.selectable_label(on, tf!("MIDI channel {number}", number = n)).clicked() {
                    let _ = app.run("channel.set_midi_in", json!({ "channel": channel.id, "port": port, "midiChannel": n }));
                    ui.close();
                }
            }
        });
    }
}

/// The key a channel's steps play: its sampler's root key, or C4.
fn step_key(channel: &Channel) -> u8 {
    match &channel.kind {
        ChannelKind::Sampler(params) => params.root_key,
        _ => 60,
    }
}

/// The graph editor: under the rows, one bar per step of the selected channel, for the parameter chosen at its left.
/// A brush stroke sets every step it crosses to the height of the pointer there, as one undo step.
///
/// Complexity: O(s) in steps, per frame.
fn graph_editor(app: &mut BodoApp, ui: &mut Ui, project: &Project, pattern: Option<PatternId>, steps: usize) {
    let tk = Tokens::get();
    let channel = app
        .session
        .editor()
        .channel
        .filter(|c| project.channels.get(c).is_some_and(|c| plays_steps(c)))
        .or_else(|| shown(app, project).into_iter().find(|c| project.channels.get(c).is_some_and(|c| plays_steps(c))));
    let Some(channel) = channel.and_then(|c| project.channels.get(&c)).cloned() else { return };
    let step_size = if app.view.rack.compact { STEP_COMPACT } else { STEP_SIZE };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), GRAPH), Sense::hover());
    let left = app.view.rack.steps_left.unwrap_or(rect.left() + 220.0);
    let area = Rect::from_min_max(pos2(left, rect.top() + 4.0), pos2(rect.right().max(left + 40.0), rect.bottom() - 4.0));
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, tk.chrome_dark);

    // The parameter, and the channel it edits, at the left.
    let label_rect = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top() + 6.0), pos2(left - 8.0, rect.top() + 28.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_rect), |ui| {
        let param = app.view.rack.graph_param;
        let combo = egui::ComboBox::from_id_salt("bodo-rack-graph-param").width(rem(8.0)).selected_text(t(param.label())).show_ui(ui, |ui| {
            for choice in StepParam::ALL {
                if ui.selectable_label(choice == param, t(choice.label())).clicked() {
                    app.view.rack.graph_param = choice;
                }
            }
        });
        control::record(ui, &combo.response, "Graph editor parameter", "combo", None);
    });
    painter.text(
        pos2(rect.left() + 10.0, rect.top() + 40.0),
        egui::Align2::LEFT_TOP,
        &channel.name,
        egui::FontId::proportional(size::SMALL),
        tk.text_dim,
    );

    let param = app.view.rack.graph_param;
    if param.centered() {
        painter.hline(area.x_range(), area.center().y, Stroke::new(1.0, tk.grid_beat));
    }
    let root = step_key(&channel);
    let notes: Vec<Note> = pattern
        .and_then(|p| project.patterns.get(&p))
        .and_then(|p| p.notes.get(&channel.id))
        .map(|seq| seq.notes().iter().filter(|n| n.start.0 % STEP.0 == 0 && ((n.start.0 / STEP.0) as usize) < steps).cloned().collect())
        .unwrap_or_default();
    let layout = StepRow::new("", &[]).step_size(step_size);
    let color = theme::color(channel.color);
    let mut bars: Vec<(f32, &Note)> = Vec::with_capacity(notes.len());
    for note in &notes {
        let index = (note.start.0 / STEP.0) as usize;
        let x = left + layout.offset(index);
        let bar = Rect::from_min_max(pos2(x + 2.0, area.top()), pos2(x + step_size.x - 2.0, area.bottom()));
        let h = param.height(note, root);
        let top = area.bottom() - h * area.height();
        let base = if param.centered() { area.center().y } else { area.bottom() };
        painter.rect_filled(
            Rect::from_min_max(pos2(bar.left(), top.min(base)), pos2(bar.right(), top.max(base).max(top.min(base) + 1.0))),
            2.0,
            color,
        );
        control::record_rect(
            ui,
            ui.id().with(("graph", note.id)),
            bar,
            &format!("{} step {} {}", channel.name, index + 1, param.label().to_lowercase()),
            "graph-bar",
            None,
            true,
        );
        bars.push((bar.center().x, note));
    }

    let response = ui.interact(area, ui.id().with("rack-graph"), Sense::click_and_drag());
    control::record(ui, &response, "Graph editor", "graph", None);
    app.hint(&response, param.hint());
    let (pointer, pressed, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
    let Some(pointer) = pointer else { return };
    if pressed && area.contains(pointer) && (response.hovered() || response.is_pointer_button_down_on()) {
        app.view.rack.stroke = Some((app.new_gesture(), pointer));
    }
    let Some((gesture, last)) = app.view.rack.stroke else { return };
    if !down {
        app.view.rack.stroke = None;
        return;
    }
    let (from, to) = if last.x <= pointer.x { (last, pointer) } else { (pointer, last) };
    let half = step_size.x / 2.0;
    let mut calls = Vec::new();
    for (x, note) in bars.iter().filter(|(x, _)| *x >= from.x - half && *x <= to.x + half) {
        let t = if to.x - from.x > 0.5 { ((x - from.x) / (to.x - from.x)).clamp(0.0, 1.0) } else { 1.0 };
        let y = from.y + (to.y - from.y) * t;
        let h = (area.bottom() - y) / area.height();
        if (param.height(note, root) - h.clamp(0.0, 1.0)).abs() > 0.01 {
            let mut params = param.params(h, root);
            params["notes"] = json!([note.id.0]);
            calls.push(Call::new("note.set_props", params).in_gesture(gesture));
        }
    }
    for call in calls {
        let _ = app.invoke(call);
    }
    app.view.rack.stroke = Some((gesture, pointer));
}
