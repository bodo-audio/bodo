//! The Piano Roll (project.md section 10.4): the notes of one channel in the current pattern, with the other channels'
//! notes shown faintly behind them as ghost notes.
//!
//! The header holds the tools (the playlist's, less Slip, with the same keys), the note tools menu, the chord the
//! stamp lays down, and the scale whose keys are highlighted. Above the keyboard, the chord readout names the chord of
//! the selected notes, or of the notes under the pointer. Below the notes, the lanes edit one value of each note with
//! brush strokes. [`tools`] handles the grid's input and [`lanes`] the lanes'. Ctrl/Cmd+wheel zooms, Shift+wheel
//! scrolls sideways, the wheel scrolls keys; the arrow keys transpose the selection, by an octave with Shift.
//!
//! The ghost notes button hides or shows the other channels' notes. Double-clicking a note, or Tools > Properties,
//! opens [`properties`], which edits every property of one note at once. With auto scroll on, the view follows the
//! pattern's playhead.
//!
//! Drawing is culled: only visible keys are drawn, and notes come from the sequence's overlap query, O(log n + k).

mod lanes;
mod properties;
mod tools;

use std::collections::BTreeSet;
use std::sync::Arc;

use bodo_engine::PlayMode;
use bodo_engine::params::{STEP, key_name};
use bodo_kit_core::Tick;
use bodo_kit_music::{Chord, PitchClass, Scale, ScaleKind};
use bodo_model::{ChannelId, Note, NoteId, Pattern, Project};
use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

pub use self::lanes::Lane;
use crate::playlist::Tool;
use crate::theme::{self, Tokens, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control};

/// The keyboard's width.
const KEYS: f32 = 46.0;
/// The ruler's height.
const RULER: f32 = 20.0;
/// How close to a note's right edge a drag resizes it.
const EDGE: f32 = 5.0;
/// The height of the bar between the notes and the lanes.
const SPLIT: f32 = 5.0;

/// The tools the piano roll has: the playlist's, less Slip.
pub const TOOLS: [Tool; 8] = [Tool::Draw, Tool::Paint, Tool::Delete, Tool::Mute, Tool::Slice, Tool::Select, Tool::Zoom, Tool::Playback];

/// Chord qualities the stamp lays down, with the names the menu shows.
pub const STAMPS: [(&str, &str); 15] = [
    ("", "Major"),
    ("m", "Minor"),
    ("7", "Dominant 7th"),
    ("maj7", "Major 7th"),
    ("m7", "Minor 7th"),
    ("dim", "Diminished"),
    ("aug", "Augmented"),
    ("sus2", "Suspended 2nd"),
    ("sus4", "Suspended 4th"),
    ("add9", "Added 9th"),
    ("6", "Major 6th"),
    ("m6", "Minor 6th"),
    ("9", "Dominant 9th"),
    ("m9", "Minor 9th"),
    ("m7b5", "Half-diminished"),
];

/// The piano roll's tool, zoom, scroll, lanes, scale, stamp and the gesture in progress.
#[derive(Debug)]
pub struct PianoView {
    /// The tool.
    pub tool: Tool,
    /// Points per tick.
    pub zoom: f32,
    /// The tick at the grid's left edge.
    pub scroll: f64,
    /// The key at the top edge, fractional while scrolling.
    pub top_key: f32,
    /// Points per key.
    pub key_height: f32,
    /// The length of new notes: the last note drawn, resized or clicked.
    pub note_length: Tick,
    /// The value the lanes edit.
    pub lane: Lane,
    /// The lanes' height in points.
    pub lane_height: f32,
    /// The scale whose keys are highlighted, and that "Limit to scale" uses.
    pub scale: Option<Scale>,
    /// The chord quality Draw lays down, such as `m7`, or a single note when `None`.
    pub stamp: Option<&'static str>,
    drag: Option<tools::Drag>,
    brush: Option<lanes::Brush>,
    resizing_lanes: bool,
    /// The key sounding from a click on the keyboard.
    sounding: Option<(ChannelId, u8)>,
    /// Whether the other channels' notes show behind this channel's.
    pub ghosts: bool,
    /// The note whose properties dialog is open.
    pub properties: Option<NoteId>,
    /// The properties dialog's values while they are edited.
    editing: Option<properties::Editing>,
}

impl Default for PianoView {
    fn default() -> Self {
        Self {
            tool: Tool::Draw,
            zoom: 160.0 / 3840.0 * 4.0,
            scroll: 0.0,
            top_key: 76.0,
            key_height: 14.0,
            note_length: STEP,
            lane: Lane::Velocity,
            lane_height: 90.0,
            scale: None,
            stamp: None,
            drag: None,
            brush: None,
            resizing_lanes: false,
            sounding: None,
            ghosts: true,
            properties: None,
            editing: None,
        }
    }
}

/// Sets the piano roll's tool from its id; Slip belongs to the playlist only.
pub(crate) fn set_tool(app: &mut BodoApp, id: &str) -> Result<Value, String> {
    let tool = TOOLS
        .into_iter()
        .find(|t| t.label().eq_ignore_ascii_case(id))
        .ok_or("the piano roll's tool must be draw, paint, delete, mute, slice, select, zoom or playback")?;
    app.view.piano.tool = tool;
    Ok(json!({ "tool": tool.label(), "window": "pianoRoll" }))
}

fn tool_hint(tool: Tool) -> &'static str {
    match tool {
        Tool::Draw => {
            "Draw (P): click to add a note or the stamp's chord, drag to move or resize, right-click to delete; Alt turns snap off; double-click a ghost note to edit its channel"
        }
        Tool::Paint => "Paint (B): drag to lay down notes along the path; right-drag deletes",
        Tool::Delete => "Delete (D): click or drag over notes to delete them",
        Tool::Mute => "Mute (T): click or drag over notes to mute or unmute them",
        Tool::Slice => "Slice (C): drag a line to cut the notes it crosses; Shift slants it",
        Tool::Select => "Select (E): drag a box to select notes; Shift adds or removes",
        Tool::Zoom => "Zoom (Z): drag a box to zoom to it, click to zoom in, right-click to zoom out",
        Tool::Playback => "Playback (Y): hold to play the pattern from the pointer; drag to scrub",
        Tool::Slip => "",
    }
}

fn is_black(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

/// Draws the Piano Roll window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let mut layout = std::mem::take(&mut app.prefs.layout);
    let channel = app.session.editor().channel.filter(|c| project.channels.contains_key(c)).or_else(|| project.rack_order.front().copied());
    let pattern = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).cloned();
    let channel_name = channel.and_then(|c| project.channels.get(&c)).map(|c| c.name.clone()).unwrap_or_default();
    let title = match &pattern {
        Some(p) => format!("Piano roll - {} > {channel_name}", p.name),
        None => String::from("Piano roll"),
    };
    windows::show(
        ctx,
        &mut layout,
        WindowKind::PianoRoll,
        workspace,
        &title,
        app,
        |app, ui| header(app, ui, project, pattern.as_deref(), channel),
        |app, ui| match (&pattern, channel) {
            (Some(pattern), Some(channel)) => body(app, ui, project, pattern, channel),
            _ => windows::placeholder(ui, ui.max_rect(), "Make a pattern and a channel to write notes"),
        },
    );
    app.prefs.layout = layout;
}

/// The notes the note tools act on: the channel's selected notes, or all of its notes when none are selected.
fn targets(app: &BodoApp, pattern: &Pattern, channel: ChannelId) -> Vec<u32> {
    let Some(seq) = pattern.notes.get(&channel) else { return Vec::new() };
    let selected = &app.session.editor().notes;
    let chosen: Vec<u32> = seq.notes().iter().filter(|n| selected.contains(&n.id)).map(|n| n.id.0).collect();
    if chosen.is_empty() { seq.notes().iter().map(|n| n.id.0).collect() } else { chosen }
}

fn header(app: &mut BodoApp, ui: &mut Ui, project: &Project, pattern: Option<&Pattern>, channel: Option<ChannelId>) {
    for tool in TOOLS {
        let button = icon_button(ui, tool.icon(), &format!("Piano roll tool {}", tool.label()), app.view.piano.tool == tool);
        let hint = crate::shortcuts::tool_hint(app, ui.ctx(), tool, tool_hint(tool));
        app.hint(&button, hint);
        if button.clicked() {
            app.view.piano.tool = tool;
            app.view.editor = WindowKind::PianoRoll;
        }
    }
    ui.separator();
    if let (Some(pattern), Some(channel)) = (pattern, channel) {
        note_tools_menu(app, ui, pattern, channel);
    }
    chips(app, ui);
    // Two notes, one faint behind the other.
    let ghosts = windows::painted_button(ui, "Ghost notes", app.view.piano.ghosts, |painter, rect, color| {
        let c = rect.center();
        painter.rect_filled(Rect::from_center_size(c + vec2(-2.0, -2.5), vec2(9.0, 3.5)), 1.0, theme::faded(color, 0.4));
        painter.rect_filled(Rect::from_center_size(c + vec2(2.0, 2.5), vec2(9.0, 3.5)), 1.0, color);
    });
    app.hint(&ghosts, "Ghost notes: show the other channels' notes faintly behind this channel's");
    if ghosts.clicked() {
        app.view.piano.ghosts = !app.view.piano.ghosts;
    }
    let name = channel.and_then(|c| project.channels.get(&c)).map(|c| c.name.clone()).unwrap_or_else(|| String::from("No channel"));
    let combo = egui::ComboBox::from_id_salt("bodo-piano-channel").width(90.0).selected_text(name).show_ui(ui, |ui| {
        for id in project.rack_order.iter() {
            if let Some(c) = project.channels.get(id)
                && ui.selectable_label(channel == Some(*id), &c.name).clicked()
            {
                let _ = app.run("edit.select", json!({ "channel": id }));
            }
        }
    });
    control::record(ui, &combo.response, "Piano roll channel", "combo", None);
    app.hint(&combo.response, "The channel whose notes the piano roll edits");
}

/// The note tools, each acting on the selected notes, or on all of the channel's notes when none are selected.
fn note_tools_menu(app: &mut BodoApp, ui: &mut Ui, pattern: &Pattern, channel: ChannelId) {
    let notes = targets(app, pattern, channel);
    let step = app.view.snap.ticks(app.session.project()).max(1);
    let scale = app.view.piano.scale;
    // Each randomize gives new notes, but the same ones for the same project history, so tests can repeat it.
    let seed = app.session.revision() + 1;
    let mut chosen: Option<(&str, Value)> = None;
    let menu = ui.menu_button("Tools", |ui| {
        ui.set_min_width(180.0);
        let mut item = |ui: &mut Ui, label: &str, id: &'static str, params: Value| {
            if ui.button(label).clicked() {
                chosen = Some((id, params));
                ui.close();
            }
        };
        item(ui, "Quantize starts", "note.quantize", json!({ "notes": notes, "grid": step }));
        item(ui, "Quantize starts and ends", "note.quantize", json!({ "notes": notes, "grid": step, "ends": true }));
        item(ui, "Un-humanize", "note.unhumanize", json!({ "notes": notes }));
        item(ui, "Humanize", "note.humanize", json!({ "notes": notes }));
        let (root, kind) = scale.map_or(("C", "major"), |s| (s.root.name(), s.kind.name()));
        let params = json!({ "pattern": pattern.id, "channel": channel, "root": root, "scale": kind, "step": step, "seed": seed });
        item(ui, &format!("Randomize in {root} {kind}"), "note.randomize", params);
        ui.separator();
        item(ui, "Legato", "note.legato", json!({ "notes": notes }));
        item(ui, "Chop to the snap", "note.chop", json!({ "notes": notes, "length": step }));
        item(ui, "Glue", "note.glue", json!({ "notes": notes }));
        ui.menu_button("Articulate", |ui| {
            for (label, amount) in [("Staccato", 0.25), ("Half", 0.5), ("Tenuto", 0.9), ("Full length", 1.0)] {
                item(ui, label, "note.articulate", json!({ "notes": notes, "amount": amount }));
            }
        });
        item(ui, "Strum up", "note.strum", json!({ "notes": notes, "direction": "up" }));
        item(ui, "Strum down", "note.strum", json!({ "notes": notes, "direction": "down" }));
        ui.menu_button("Arpeggiate", |ui| {
            for (label, order) in [
                ("Up", "up"),
                ("Down", "down"),
                ("Up and down", "upDown"),
                ("Down and up", "downUp"),
                ("Random", "random"),
                ("As played", "asPlayed"),
            ] {
                item(ui, label, "note.arpeggiate", json!({ "notes": notes, "order": order, "rate": step }));
            }
        });
        ui.separator();
        item(ui, "Flip in time", "note.flip", json!({ "notes": notes, "axis": "time" }));
        item(ui, "Flip keys", "note.flip", json!({ "notes": notes, "axis": "keys" }));
        item(ui, "Transpose up an octave", "note.transpose", json!({ "notes": notes, "semitones": 12 }));
        item(ui, "Transpose down an octave", "note.transpose", json!({ "notes": notes, "semitones": -12 }));
        match scale {
            Some(scale) => {
                let params = json!({ "notes": notes, "root": scale.root.name(), "scale": scale.kind.name() });
                item(ui, &format!("Limit to {} {}", scale.root.name(), scale.kind.name()), "note.limit_to_scale", params);
            }
            None => {
                ui.add_enabled(false, egui::Button::new("Limit to scale")).on_disabled_hover_text("Choose a scale first");
            }
        }
        ui.separator();
        item(ui, "Make slides", "note.set_props", json!({ "notes": notes, "slide": true }));
        item(ui, "Make portamento", "note.set_props", json!({ "notes": notes, "porta": true }));
        item(ui, "Make plain notes", "note.set_props", json!({ "notes": notes, "slide": false, "porta": false }));
        ui.separator();
        ui.menu_button("Stamp", |ui| stamp_menu(app, ui));
        ui.menu_button("Scale", |ui| scale_menu(app, ui));
        ui.separator();
        // The properties of the first selected note, or of the channel's first note.
        if let Some(first) = notes.first()
            && ui.button("Properties...").clicked()
        {
            app.view.piano.properties = Some(NoteId(*first));
            app.view.piano.editing = None;
            ui.close();
        }
    });
    control::record(ui, &menu.response, "Piano roll tools", "menu", None);
    app.hint(&menu.response, "Note tools: they act on the selected notes, or on all of the channel's notes when none are selected");
    if let Some((id, params)) = chosen {
        let _ = app.run(id, params);
    }
}

/// The stamp's chords: Draw lays down the chosen one, its root on the key clicked.
fn stamp_menu(app: &mut BodoApp, ui: &mut Ui) {
    let current = app.view.piano.stamp;
    if ui.selectable_label(current.is_none(), "Single note").clicked() {
        app.view.piano.stamp = None;
    }
    for (quality, name) in STAMPS {
        if ui.selectable_label(current == Some(quality), name).clicked() {
            app.view.piano.stamp = Some(quality);
        }
    }
}

/// The scale whose keys are highlighted, and that "Limit to scale" uses: its root, then its kind.
fn scale_menu(app: &mut BodoApp, ui: &mut Ui) {
    let current = app.view.piano.scale;
    if ui.selectable_label(current.is_none(), "No scale").clicked() {
        app.view.piano.scale = None;
    }
    let root = current.map_or(PitchClass::new(0), |s| s.root);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(190.0);
        for n in 0..12 {
            let class = PitchClass::new(n);
            if ui.selectable_label(current.is_some_and(|s| s.root == class), class.name()).clicked() {
                app.view.piano.scale = Some(Scale::new(class, current.map_or(ScaleKind::Major, |s| s.kind)));
            }
        }
    });
    ui.separator();
    for kind in ScaleKind::ALL {
        if ui.selectable_label(current.is_some_and(|s| s.kind == kind), kind.name()).clicked() {
            app.view.piano.scale = Some(Scale::new(root, kind));
        }
    }
}

/// Small labels for the stamp and the scale while they are on; clicking one turns it off.
fn chips(app: &mut BodoApp, ui: &mut Ui) {
    let tk = Tokens::get();
    let chip = |ui: &mut Ui, text: String| ui.add(egui::Button::new(egui::RichText::new(text).size(size::SMALL).color(tk.accent)).frame(false));
    if let Some(quality) = app.view.piano.stamp {
        let name = STAMPS.iter().find(|(q, _)| *q == quality).map_or(quality, |(_, n)| n);
        let response = chip(ui, format!("Stamp: {name}"));
        control::record(ui, &response, "Piano roll stamp", "chip", None);
        app.hint(&response, "Draw lays down this chord; click to draw single notes again");
        if response.clicked() {
            app.view.piano.stamp = None;
        }
    }
    if let Some(scale) = app.view.piano.scale {
        let response = chip(ui, format!("{} {}", scale.root.name(), scale.kind.name()));
        control::record(ui, &response, "Piano roll scale", "chip", None);
        app.hint(&response, "The highlighted scale; click to turn the highlight off");
        if response.clicked() {
            app.view.piano.scale = None;
        }
    }
}

/// Converts between ticks, keys and points.
#[derive(Clone, Copy, Debug)]
struct Grid {
    rect: Rect,
    zoom: f32,
    scroll: f64,
    top_key: f32,
    key_height: f32,
}

impl Grid {
    fn x(&self, tick: f64) -> f32 {
        self.rect.left() + ((tick - self.scroll) as f32) * self.zoom
    }

    fn tick(&self, x: f32) -> f64 {
        self.scroll + f64::from((x - self.rect.left()) / self.zoom)
    }

    /// The top of key `key`'s row.
    fn y(&self, key: u8) -> f32 {
        self.rect.top() + (self.top_key - f32::from(key)) * self.key_height
    }

    fn key(&self, y: f32) -> Option<u8> {
        let key = (self.top_key - (y - self.rect.top()) / self.key_height).ceil();
        (0.0..=127.0).contains(&key).then_some(key as u8)
    }
}

/// A note of the edited channel as drawn this frame.
#[derive(Clone, Debug)]
struct Shown {
    note: Note,
    rect: Rect,
}

/// Where on a note the pointer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Body,
    Edge,
}

/// The grid as drawn this frame, for the tools.
struct Canvas {
    rect: Rect,
    grid: Grid,
    channel: ChannelId,
    /// The edited channel's notes in view, in time order.
    shown: Vec<Shown>,
    /// The other channels' notes in view.
    ghosts: Vec<(ChannelId, Rect)>,
}

impl Canvas {
    /// The topmost note under `at`, and whether `at` is on its right edge.
    fn note_at(&self, at: egui::Pos2) -> Option<(&Shown, Zone)> {
        self.shown.iter().rev().find(|s| s.rect.contains(at)).map(|s| (s, if at.x >= s.rect.right() - EDGE { Zone::Edge } else { Zone::Body }))
    }
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Project, pattern: &Pattern, channel: ChannelId) {
    let tk = Tokens::get();
    let rect = ui.max_rect();
    let lane_height = app.view.piano.lane_height.clamp(0.0, (rect.height() * 0.6).max(0.0));
    let lanes_rect = Rect::from_min_max(pos2(rect.left(), rect.bottom() - lane_height), rect.max);
    let split = Rect::from_min_max(pos2(rect.left(), lanes_rect.top() - SPLIT), pos2(rect.right(), lanes_rect.top()));
    let corner = Rect::from_min_size(rect.min, vec2(KEYS, RULER));
    let keys = Rect::from_min_max(pos2(rect.left(), rect.top() + RULER), pos2(rect.left() + KEYS, split.top()));
    let ruler = Rect::from_min_max(pos2(keys.right(), rect.top()), pos2(rect.right(), rect.top() + RULER));
    let canvas = Rect::from_min_max(pos2(keys.right(), ruler.bottom()), pos2(rect.right(), split.top()));
    navigate(app, ui, canvas.union(ruler).union(keys));
    follow_playhead(app, canvas);
    let view = &app.view.piano;
    let grid = Grid { rect: canvas, zoom: view.zoom, scroll: view.scroll, top_key: view.top_key, key_height: view.key_height };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(canvas, 0.0, tk.panel);
    painter.rect_filled(ruler.union(corner), 0.0, tk.chrome_dark);

    rows_and_keyboard(app, ui, &painter, &grid, canvas, keys, channel);
    time_grid(&painter, project, pattern, &grid, canvas, ruler);
    ruler_input(app, ui, project, ruler, &grid);

    // Notes: other channels faintly, this channel in its color.
    let (low, high) = (grid.key(canvas.bottom()).unwrap_or(0), grid.key(canvas.top()).unwrap_or(127));
    let view_start = Tick(grid.tick(canvas.left()).floor() as i64);
    let view_end = Tick(grid.tick(canvas.right()).ceil() as i64 + 1);
    let note_painter = painter.with_clip_rect(canvas);
    let selected = app.session.editor().notes.clone();
    let mut shown = Vec::new();
    let mut ghosts = Vec::new();
    let mut in_view = Vec::new();
    let show_ghosts = app.view.piano.ghosts;
    for (owner, seq) in pattern.notes.iter().filter(|(owner, _)| show_ghosts || **owner == channel) {
        let own = *owner == channel;
        let color = project.channels.get(owner).map_or(tk.text_faint, |c| theme::color(c.color));
        for note in seq.overlapping(view_start, view_end) {
            if own {
                in_view.push(note.clone());
            }
            if note.key < low || note.key > high {
                continue;
            }
            let y = grid.y(note.key);
            let left = grid.x(note.start.0 as f64);
            let rect = Rect::from_min_max(pos2(left, y + 1.0), pos2(grid.x(note.end().0 as f64).max(left + 3.0), y + grid.key_height - 1.0));
            if !own {
                note_painter.rect_filled(rect, 2.0, theme::faded(color, 0.25));
                ghosts.push((*owner, rect));
                continue;
            }
            draw_note(ui, &note_painter, project, note, rect, color, selected.contains(&note.id));
            shown.push(Shown { note: note.clone(), rect });
        }
    }
    chord_readout(app, ui, &painter, corner, &in_view, &selected, &grid, canvas);
    playhead(app, &painter, canvas.union(ruler), &grid);
    let board = Canvas { rect: canvas, grid, channel, shown, ghosts };
    tools::input(app, ui, project, &board);
    keyboard_shortcuts(app, ui, pattern, channel);

    lanes::split(app, ui, split, rect);
    if lane_height > 0.0 {
        lanes::show(app, ui, project, lanes_rect, &grid, channel, pattern, &in_view);
    }
    properties::dialog(app, ui.ctx(), project, pattern);
}

/// While the pattern plays and auto-scrolling is on, keeps the playhead in view: when it passes the grid's right
/// edge, or wraps back to the start, the view pages to it.
fn follow_playhead(app: &mut BodoApp, canvas: Rect) {
    let transport = app.session.transport();
    if !app.view.auto_scroll || !transport.playing || transport.mode != PlayMode::Pattern {
        return;
    }
    let view = &mut app.view.piano;
    let at = transport.position.0 as f64;
    let width = f64::from(canvas.width() / view.zoom);
    if at > view.scroll + width * 0.92 || at < view.scroll {
        view.scroll = (at - width * 0.05).max(0.0);
    }
}

fn draw_note(ui: &Ui, painter: &egui::Painter, project: &Project, note: &Note, rect: Rect, color: egui::Color32, selected: bool) {
    let tk = Tokens::get();
    // A note in a color group other than the first takes that group's hue instead of the channel's.
    let color = match usize::from(note.color) {
        0 => color,
        group => bodo_engine::palette::HUES.get(group).map_or(color, |hue| theme::color(*hue)),
    };
    let velocity = f32::from(note.velocity) / f32::from(u16::MAX);
    let fill = theme::faded(theme::tint(color, 0.45 + 0.55 * velocity), if note.muted { 0.35 } else { 1.0 });
    painter.rect_filled(rect, 2.0, fill);
    if selected {
        painter.rect_stroke(rect, 2.0, Stroke::new(1.5, tk.accent), StrokeKind::Inside);
    }
    // Slides lean into the next note; portamento glides from the last one.
    let mark = theme::text_on(fill);
    if note.slide && rect.width() > 8.0 {
        let (a, b) = (rect.left_bottom() + vec2(3.0, -3.0), rect.right_top() + vec2(-3.0, 3.0));
        painter.line_segment([a, b], Stroke::new(1.2, mark));
    }
    if note.porta && rect.width() > 8.0 {
        painter.circle_filled(rect.left_center() + vec2(4.0, 0.0), 2.0, mark);
    }
    if rect.width() > 26.0 && rect.height() >= 9.0 {
        painter.text(rect.left_center() + vec2(3.0, 0.0), Align2::LEFT_CENTER, key_name(note.key), FontId::proportional(8.5), mark);
    }
    let what = if note.muted { "Muted note" } else { "Note" };
    control::record_rect(
        ui,
        ui.id().with(("note", note.id)),
        rect,
        &format!("{what} {} at {}", key_name(note.key), bodo_engine::params::format_position(project, note.start)),
        "note",
        Some(selected),
        true,
    );
}

/// The rows behind the notes, with the scale highlighted, and the keyboard, which plays the channel while held.
fn rows_and_keyboard(app: &mut BodoApp, ui: &mut Ui, painter: &egui::Painter, grid: &Grid, canvas: Rect, keys: Rect, channel: ChannelId) {
    let tk = Tokens::get();
    let scale = app.view.piano.scale;
    let (low, high) = (grid.key(canvas.bottom()).unwrap_or(0), grid.key(canvas.top()).unwrap_or(127));
    let rows = painter.with_clip_rect(canvas);
    let board = painter.with_clip_rect(keys);
    for key in low..=high {
        let y = grid.y(key);
        let row = Rect::from_min_size(pos2(canvas.left(), y), vec2(canvas.width(), grid.key_height));
        if is_black(key) {
            rows.rect_filled(row, 0.0, tk.row_black);
        }
        if let Some(scale) = scale
            && scale.contains(key)
        {
            let root = PitchClass::of(key) == scale.root;
            rows.rect_filled(row, 0.0, theme::faded(tk.accent, if root { 0.16 } else { 0.07 }));
        }
        rows.hline(canvas.x_range(), y + grid.key_height, Stroke::new(1.0, if key % 12 == 0 { tk.grid_bar } else { tk.grid_step }));
        let key_rect = Rect::from_min_size(pos2(keys.left(), y), vec2(keys.width(), grid.key_height));
        let base = if is_black(key) { tk.key_black } else { tk.key_white };
        let sounding = app.view.piano.sounding == Some((channel, key));
        board.rect_filled(key_rect.shrink2(vec2(0.0, 0.5)), 0.0, if sounding { tk.accent } else { base });
        if scale.is_some_and(|s| PitchClass::of(key) == s.root) {
            board.rect_filled(Rect::from_min_size(key_rect.min, vec2(3.0, key_rect.height())), 0.0, tk.accent);
        }
        if key % 12 == 0 {
            board.text(
                key_rect.right_center() - vec2(3.0, 0.0),
                Align2::RIGHT_CENTER,
                key_name(key),
                FontId::proportional(9.5),
                theme::text_on(tk.key_white),
            );
        }
    }
    // Holding a key plays the channel.
    let response = ui.interact(keys, ui.id().with("piano-keys"), Sense::click_and_drag());
    control::record(ui, &response, "Piano keyboard", "keyboard", None);
    app.hint(&response, "Hold a key to hear the channel play it");
    let held = response.is_pointer_button_down_on().then(|| response.interact_pointer_pos().and_then(|p| grid.key(p.y))).flatten();
    let now = held.map(|key| (channel, key));
    if now != app.view.piano.sounding {
        if let Some((c, key)) = app.view.piano.sounding {
            app.audio.play_note(c, key, 0.0, false);
        }
        if let Some((c, key)) = now {
            app.audio.play_note(c, key, 0.8, true);
        }
        app.view.piano.sounding = now;
    }
}

fn time_grid(painter: &egui::Painter, project: &Project, pattern: &Pattern, grid: &Grid, canvas: Rect, ruler: Rect) {
    let tk = Tokens::get();
    let length = bodo_engine::pattern_length(project, pattern.id);
    let meter = project.meter.signature_at(Tick::ZERO);
    let (bar, beat, step) = (meter.bar_ticks() as f64, meter.beat_ticks() as f64, STEP.0 as f64);
    let (first, last) = (grid.tick(canvas.left()).max(0.0), grid.tick(canvas.right()));
    let lines = painter.with_clip_rect(canvas.union(ruler));
    for (spacing, color) in [(step, tk.grid_step), (beat, tk.grid_beat), (bar, tk.grid_bar)] {
        if spacing * f64::from(grid.zoom) < 6.0 {
            continue;
        }
        let mut t = (first / spacing).floor() * spacing;
        while t <= last {
            lines.vline(grid.x(t), canvas.y_range(), Stroke::new(1.0, color));
            if spacing == bar {
                let label = format!("{}", (t / bar) as i64 + 1);
                lines.text(pos2(grid.x(t) + 3.0, ruler.center().y), Align2::LEFT_CENTER, label, FontId::monospace(size::SMALL), tk.text_dim);
            }
            t += spacing;
        }
    }
    let end = grid.x(length.0 as f64);
    if end < canvas.right() {
        lines.rect_filled(Rect::from_min_max(pos2(end.max(canvas.left()), canvas.top()), canvas.max), 0.0, theme::faded(tk.border, 0.5));
    }
}

/// A click or drag on the ruler moves the pattern's playhead.
fn ruler_input(app: &mut BodoApp, ui: &mut Ui, project: &Project, ruler: Rect, grid: &Grid) {
    let response = ui.interact(ruler, ui.id().with("piano-ruler"), Sense::click_and_drag());
    control::record(ui, &response, "Piano roll ruler", "ruler", None);
    app.hint(&response, "Click or drag to move the pattern's position");
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let at = app.view.snap.floor(project, grid.tick(p.x)).max(0);
        if app.session.transport().mode != PlayMode::Pattern {
            let _ = app.run("transport.set_mode", json!({ "mode": "pattern" }));
        }
        if app.session.transport().position.0 != at {
            let _ = app.run("transport.set_position", json!({ "at": at }));
        }
    }
}

/// Names the chord of the selected notes, or of the notes sounding under the pointer, above the keyboard.
#[allow(clippy::too_many_arguments, reason = "one drawing step of the body, kept apart for reading")]
fn chord_readout(
    app: &BodoApp,
    ui: &Ui,
    painter: &egui::Painter,
    corner: Rect,
    notes: &[Note],
    selected: &BTreeSet<NoteId>,
    grid: &Grid,
    canvas: Rect,
) {
    let mut keys: Vec<u8> = notes.iter().filter(|n| selected.contains(&n.id)).map(|n| n.key).collect();
    if keys.is_empty() {
        let at = ui.input(|i| i.pointer.hover_pos()).filter(|p| canvas.contains(*p)).map(|p| grid.tick(p.x)).or_else(|| {
            let transport = app.session.transport();
            (transport.playing && transport.mode == PlayMode::Pattern).then_some(transport.position.0 as f64)
        });
        if let Some(at) = at {
            keys = notes.iter().filter(|n| (n.start.0 as f64) <= at && at < n.end().0 as f64).map(|n| n.key).collect();
        }
    }
    let Some(chord) = Chord::detect(&keys) else { return };
    let tk = Tokens::get();
    let name = chord.to_string();
    painter.with_clip_rect(corner).text(corner.center(), Align2::CENTER_CENTER, &name, FontId::proportional(size::SMALL), tk.accent);
    control::record_rect(ui, ui.id().with("piano-chord"), corner, &format!("Chord {name}"), "readout", None, false);
}

/// Arrow keys transpose the selected notes while the piano roll is the editor in front: a semitone, or an octave with
/// Shift.
fn keyboard_shortcuts(app: &mut BodoApp, ui: &Ui, pattern: &Pattern, channel: ChannelId) {
    if app.view.editor != WindowKind::PianoRoll || ui.ctx().egui_wants_keyboard_input() || app.session.editor().notes.is_empty() {
        return;
    }
    let (up, down, shift) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.modifiers.shift));
    let semitones = match (up, down) {
        (true, false) => 1,
        (false, true) => -1,
        _ => return,
    } * if shift { 12 } else { 1 };
    let notes = targets(app, pattern, channel);
    let _ = app.run("note.transpose", json!({ "notes": notes, "semitones": semitones }));
}

fn navigate(app: &mut BodoApp, ui: &Ui, area: Rect) {
    if !ui.rect_contains_pointer(area) {
        return;
    }
    let (delta, modifiers, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    if delta == egui::Vec2::ZERO {
        return;
    }
    let view = &mut app.view.piano;
    let left = area.left() + KEYS;
    if modifiers.command || modifiers.ctrl {
        let anchor = pointer.map_or(left, |p| p.x);
        let before = view.scroll + f64::from((anchor - left) / view.zoom);
        view.zoom = (view.zoom * (1.0 + delta.y * 0.004)).clamp(8.0 / 3840.0, 4.0);
        view.scroll = (before - f64::from((anchor - left) / view.zoom)).max(0.0);
    } else if modifiers.alt {
        view.key_height = (view.key_height * (1.0 + delta.y * 0.004)).clamp(6.0, 40.0);
    } else if modifiers.shift || delta.x != 0.0 {
        let dx = if delta.x != 0.0 { delta.x } else { delta.y };
        view.scroll = (view.scroll - f64::from(dx / view.zoom)).max(0.0);
    } else {
        view.top_key = (view.top_key + delta.y / view.key_height).clamp(12.0, 127.0);
    }
}

fn playhead(app: &BodoApp, painter: &egui::Painter, area: Rect, grid: &Grid) {
    if app.session.transport().mode != PlayMode::Pattern || !app.session.transport().playing {
        return;
    }
    let x = grid.x(app.session.transport().position.0 as f64);
    if x >= area.left() && x <= area.right() {
        painter.with_clip_rect(area).vline(x, area.y_range(), Stroke::new(1.5, Tokens::get().playhead));
    }
}
