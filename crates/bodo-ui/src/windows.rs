//! Free windows inside the workspace (project.md section 14.1, ADR 0012).
//!
//! The Channel Rack, Playlist, Piano Roll and Mixer are windows inside the workspace between the side panels. Each
//! one moves by its title bar, resizes from its edges, maximizes to fill the workspace (double-click the title bar),
//! minimizes to its title bar and closes. Where they sit is kept as fractions of the workspace, so the arrangement
//! survives resizing the main window. A [`Layout`] records it all and belongs to the user, not the project.

use std::collections::BTreeMap;

use bodo_kit_ui::{Icon, icons};
use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Align2, CornerRadius, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::theme::{Tokens, size};

/// The smallest a window gets, in points.
const MIN_SIZE: Vec2 = vec2(260.0, 120.0);
/// How far from an edge a resize drag starts, in points.
const GRIP: f32 = 6.0;

/// A free window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowKind {
    /// The step sequencer (it floats, but never leaves the main window).
    ChannelRack,
    /// The arrangement.
    Playlist,
    /// The note editor.
    PianoRoll,
    /// The mixer.
    Mixer,
}

impl WindowKind {
    /// Every window, in the order of the top bar's buttons.
    pub const ALL: [Self; 4] = [Self::ChannelRack, Self::PianoRoll, Self::Mixer, Self::Playlist];

    /// Its title.
    pub fn title(self) -> &'static str {
        match self {
            Self::ChannelRack => "Channel rack",
            Self::Playlist => "Playlist",
            Self::PianoRoll => "Piano roll",
            Self::Mixer => "Mixer",
        }
    }

    /// Its icon.
    pub fn icon(self) -> Icon {
        match self {
            Self::ChannelRack => icons::CHANNEL_RACK,
            Self::Playlist => icons::PLAYLIST,
            Self::PianoRoll => icons::PIANO,
            Self::Mixer => icons::ADJUSTMENTS_HORIZONTAL,
        }
    }

    /// Its id in commands, such as `channelRack`.
    pub fn id(self) -> &'static str {
        match self {
            Self::ChannelRack => "channelRack",
            Self::Playlist => "playlist",
            Self::PianoRoll => "pianoRoll",
            Self::Mixer => "mixer",
        }
    }

    /// The window an id names.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id().eq_ignore_ascii_case(id) || k.title().eq_ignore_ascii_case(id))
    }

    /// Its function key: F5 Playlist, F6 Channel rack, F7 Piano roll, F9 Mixer, as FL users expect.
    pub fn key(self) -> egui::Key {
        match self {
            Self::Playlist => egui::Key::F5,
            Self::ChannelRack => egui::Key::F6,
            Self::PianoRoll => egui::Key::F7,
            Self::Mixer => egui::Key::F9,
        }
    }
}

/// Where a window is and how it shows.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    /// Whether it is open.
    pub open: bool,
    /// Its left, top, width and height as fractions of the workspace.
    pub frame: [f32; 4],
    /// Whether it fills the workspace.
    pub maximized: bool,
    /// Whether only its title bar shows.
    pub minimized: bool,
}

impl WindowState {
    const fn at(open: bool, frame: [f32; 4]) -> Self {
        Self { open, frame, maximized: false, minimized: false }
    }
}

/// A saved arrangement of the main window.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Layout {
    /// The free windows.
    pub windows: BTreeMap<WindowKind, WindowState>,
    /// Whether the Browser shows on the left.
    pub browser: bool,
    /// The Browser's width in points.
    pub browser_width: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self::preset(Preset::Default)
    }
}

/// The layouts Bodo ships with (project.md section 14.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    /// The owner's sketch: Channel Rack top left, Mixer bottom left, Playlist on the right.
    Default,
    /// Channel Rack and Piano Roll large.
    Compose,
    /// Playlist large.
    Arrange,
    /// Mixer large.
    Mix,
}

impl Preset {
    /// Every preset, in menu order.
    pub const ALL: [Self; 4] = [Self::Default, Self::Compose, Self::Arrange, Self::Mix];

    /// Its name in menus.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Compose => "Compose",
            Self::Arrange => "Arrange",
            Self::Mix => "Mix",
        }
    }

    /// The preset an id names, such as `compose`.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.label().eq_ignore_ascii_case(id))
    }
}

impl Layout {
    /// The arrangement of `preset`.
    pub fn preset(preset: Preset) -> Self {
        use WindowKind::{ChannelRack, Mixer, PianoRoll, Playlist};
        let windows = match preset {
            Preset::Default => [
                (ChannelRack, WindowState::at(true, [0.01, 0.015, 0.47, 0.42])),
                (Mixer, WindowState::at(true, [0.0, 0.47, 0.49, 0.53])),
                (Playlist, WindowState::at(true, [0.5, 0.0, 0.5, 1.0])),
                (PianoRoll, WindowState::at(false, [0.22, 0.2, 0.6, 0.6])),
            ],
            Preset::Compose => [
                (ChannelRack, WindowState::at(true, [0.0, 0.0, 0.45, 0.55])),
                (PianoRoll, WindowState::at(true, [0.0, 0.55, 1.0, 0.45])),
                (Playlist, WindowState::at(true, [0.45, 0.0, 0.55, 0.55])),
                (Mixer, WindowState::at(false, [0.0, 0.47, 0.49, 0.53])),
            ],
            Preset::Arrange => [
                (Playlist, WindowState::at(true, [0.0, 0.0, 1.0, 1.0])),
                (ChannelRack, WindowState::at(false, [0.01, 0.015, 0.47, 0.42])),
                (Mixer, WindowState::at(false, [0.0, 0.47, 0.49, 0.53])),
                (PianoRoll, WindowState::at(false, [0.22, 0.2, 0.6, 0.6])),
            ],
            Preset::Mix => [
                (Mixer, WindowState::at(true, [0.0, 0.3, 1.0, 0.7])),
                (Playlist, WindowState::at(true, [0.0, 0.0, 1.0, 0.3])),
                (ChannelRack, WindowState::at(false, [0.01, 0.015, 0.47, 0.42])),
                (PianoRoll, WindowState::at(false, [0.22, 0.2, 0.6, 0.6])),
            ],
        };
        Self { windows: windows.into_iter().collect(), browser: true, browser_width: 230.0 }
    }

    /// The state of window `kind`.
    pub fn state(&self, kind: WindowKind) -> WindowState {
        self.windows
            .get(&kind)
            .copied()
            .unwrap_or_else(|| Self::preset(Preset::Default).windows.get(&kind).copied().unwrap_or(WindowState::at(false, [0.2, 0.2, 0.5, 0.5])))
    }

    fn state_mut(&mut self, kind: WindowKind) -> &mut WindowState {
        let state = self.state(kind);
        self.windows.entry(kind).or_insert(state)
    }

    /// Whether window `kind` is open.
    pub fn is_open(&self, kind: WindowKind) -> bool {
        self.state(kind).open
    }

    /// Opens window `kind` (restoring it if minimized) and brings it to the front.
    pub fn open(&mut self, ctx: &egui::Context, kind: WindowKind) {
        let state = self.state_mut(kind);
        state.open = true;
        state.minimized = false;
        ctx.move_to_top(egui::LayerId::new(Order::Middle, area_id(kind)));
    }

    /// Closes window `kind` when it shows, and opens it otherwise.
    pub fn toggle(&mut self, ctx: &egui::Context, kind: WindowKind) {
        let state = self.state(kind);
        if state.open && !state.minimized {
            self.state_mut(kind).open = false;
        } else {
            self.open(ctx, kind);
        }
    }
}

/// The egui id of window `kind`'s area.
pub fn area_id(kind: WindowKind) -> Id {
    Id::new(("bodo-window", kind))
}

/// What a window's title bar did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowEvents {
    /// Whether the user closed it.
    pub closed: bool,
    /// Whether the pointer is over it.
    pub hovered: bool,
}

fn button_style(tk: &Tokens) -> IconButtonStyle {
    let plain = ButtonVisuals { fill: egui::Color32::TRANSPARENT, stroke: Stroke::NONE, icon: tk.text_dim };
    IconButtonStyle {
        normal: plain,
        hovered: ButtonVisuals { fill: tk.control_hover, icon: tk.text, ..plain },
        pressed: Some(ButtonVisuals { fill: tk.control, icon: tk.text, ..plain }),
        selected: ButtonVisuals { fill: tk.accent_soft, icon: tk.accent, ..plain },
        corner_radius: CornerRadius::same(5),
        focus_stroke: Stroke::new(1.0, tk.accent),
    }
}

/// A small icon button in the interface's style. Returns its response; the caller acts on clicks.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let tk = Tokens::get();
    let style = button_style(&tk);
    let response = IconButton::new(label, vec2(22.0, 20.0), &style).selected(selected).show(ui, |ui, rect, color| {
        let side = 14.0;
        icon.paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(side, side)), color);
    });
    crate::control::record(ui, &response, label, "button", Some(selected));
    response.on_hover_text(label)
}

/// Shows window `kind` inside `workspace` with `title`. `header` draws extra controls into the title bar after the
/// title; `body` draws the contents. Both get `state`, usually the app.
#[allow(clippy::too_many_arguments, reason = "a window needs its place, title, state and both parts")]
pub fn show<T>(
    ctx: &egui::Context,
    layout: &mut Layout,
    kind: WindowKind,
    workspace: Rect,
    title: &str,
    state_of: &mut T,
    header: impl FnOnce(&mut T, &mut Ui),
    body: impl FnOnce(&mut T, &mut Ui),
) -> WindowEvents {
    let state = layout.state(kind);
    if !state.open {
        return WindowEvents::default();
    }
    let tk = Tokens::get();
    let full = if state.maximized { workspace } else { frame_rect(state.frame, workspace) };
    let rect = if state.minimized { Rect::from_min_size(full.min, vec2(full.width().min(300.0), size::TITLE_BAR)) } else { full };
    let id = area_id(kind);
    let mut new_state = state;
    let mut events = WindowEvents::default();
    egui::Area::new(id).order(Order::Middle).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        let (rect, background) = ui.allocate_exact_size(rect.size(), Sense::click());
        events.hovered = background.hovered() || ui.rect_contains_pointer(rect);
        let painter = ui.painter().clone();
        painter.add(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: tk.shadow }.as_shape(rect, CornerRadius::same(8)));
        painter.rect(rect, CornerRadius::same(8), tk.panel, Stroke::new(1.0, tk.border), StrokeKind::Inside);
        let title_rect = Rect::from_min_size(rect.min, vec2(rect.width(), size::TITLE_BAR));
        painter.rect_filled(
            title_rect,
            CornerRadius { nw: 8, ne: 8, sw: if state.minimized { 8 } else { 0 }, se: if state.minimized { 8 } else { 0 } },
            tk.chrome,
        );

        // Moving: drag the title bar; double-click maximizes.
        let drag = ui.interact(title_rect, id.with("title"), Sense::click_and_drag());
        if drag.double_clicked() {
            new_state.maximized = !state.maximized;
            new_state.minimized = false;
        } else if drag.dragged() && !state.maximized {
            let delta = drag.drag_delta();
            let moved = full.translate(delta);
            new_state.frame = to_frame(keep_inside(moved, workspace), workspace);
        }

        let mut bar =
            ui.new_child(UiBuilder::new().max_rect(title_rect.shrink2(vec2(6.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        kind.icon().paint(bar.painter(), Rect::from_min_size(bar.cursor().min + vec2(0.0, 3.0), vec2(14.0, 14.0)), tk.text_dim);
        bar.add_space(18.0);
        bar.label(egui::RichText::new(title).color(tk.text).size(size::BODY));
        bar.add_space(10.0);
        header(state_of, &mut bar);
        let mut buttons =
            ui.new_child(UiBuilder::new().max_rect(title_rect.shrink2(vec2(4.0, 3.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if icon_button(&mut buttons, icons::X, &format!("Close {title}"), false).clicked() {
            new_state.open = false;
            events.closed = true;
        }
        let (max_icon, max_label) = if state.maximized { (icons::WINDOW_RESTORE, "Restore") } else { (icons::SQUARE, "Maximize") };
        if icon_button(&mut buttons, max_icon, &format!("{max_label} {title}"), false).clicked() {
            new_state.maximized = !state.maximized;
            new_state.minimized = false;
        }
        if icon_button(&mut buttons, icons::MINUS, &format!("Minimize {title}"), state.minimized).clicked() {
            new_state.minimized = !state.minimized;
        }

        if !state.minimized {
            let body_rect = Rect::from_min_max(pos2(rect.left() + 1.0, title_rect.bottom()), rect.max - vec2(1.0, 1.0));
            let mut body_ui =
                ui.new_child(UiBuilder::new().id_salt(id.with("body")).max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
            body_ui.set_clip_rect(body_rect.intersect(workspace));
            body(state_of, &mut body_ui);
            if !state.maximized {
                resize_grips(ui, id, rect, full, workspace, &mut new_state);
            }
        }
    });
    if new_state != state {
        layout.windows.insert(kind, new_state);
    }
    events
}

/// The edges and corner that resize a window.
fn resize_grips(ui: &mut Ui, id: Id, rect: Rect, full: Rect, workspace: Rect, state: &mut WindowState) {
    let right = Rect::from_min_max(pos2(rect.right() - GRIP, rect.top() + size::TITLE_BAR), pos2(rect.right(), rect.bottom() - GRIP));
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - GRIP), pos2(rect.right() - GRIP, rect.bottom()));
    let corner = Rect::from_min_max(rect.max - vec2(GRIP * 2.0, GRIP * 2.0), rect.max);
    let mut size = full.size();
    for (rect, name, cursor, axes) in [
        (right, "right", egui::CursorIcon::ResizeHorizontal, vec2(1.0, 0.0)),
        (bottom, "bottom", egui::CursorIcon::ResizeVertical, vec2(0.0, 1.0)),
        (corner, "corner", egui::CursorIcon::ResizeNwSe, vec2(1.0, 1.0)),
    ] {
        let response = ui.interact(rect, id.with(name), Sense::drag()).on_hover_cursor(cursor);
        if response.dragged() {
            size += response.drag_delta() * axes;
        }
    }
    if size != full.size() {
        let size = size.max(MIN_SIZE).min(workspace.max - full.min);
        state.frame = to_frame(Rect::from_min_size(full.min, size), workspace);
    }
}

/// The screen rectangle of `frame` (fractions) inside `workspace`, kept inside it and at least the minimum size.
pub fn frame_rect(frame: [f32; 4], workspace: Rect) -> Rect {
    let [x, y, w, h] = frame;
    let size = vec2(w * workspace.width(), h * workspace.height()).max(MIN_SIZE).min(workspace.size());
    keep_inside(Rect::from_min_size(workspace.min + vec2(x * workspace.width(), y * workspace.height()), size), workspace)
}

fn to_frame(rect: Rect, workspace: Rect) -> [f32; 4] {
    let w = workspace.width().max(1.0);
    let h = workspace.height().max(1.0);
    [(rect.left() - workspace.left()) / w, (rect.top() - workspace.top()) / h, rect.width() / w, rect.height() / h]
}

/// `rect` moved, not resized, so it lies inside `area` as far as it fits.
fn keep_inside(rect: Rect, area: Rect) -> Rect {
    let x = rect.left().clamp(area.left(), (area.right() - rect.width()).max(area.left()));
    let y = rect.top().clamp(area.top(), (area.bottom() - rect.height()).max(area.top()));
    Rect::from_min_size(Pos2::new(x, y), rect.size())
}

/// Draws `text` centered in `rect`, for empty states.
pub fn placeholder(ui: &Ui, rect: Rect, text: &str) {
    let tk = Tokens::get();
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, egui::FontId::proportional(size::BODY), tk.text_faint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_stay_inside() {
        let workspace = Rect::from_min_size(pos2(100.0, 50.0), vec2(1000.0, 600.0));
        let rect = frame_rect([0.5, 0.25, 0.25, 0.5], workspace);
        assert!(workspace.contains_rect(rect));
        assert_eq!(rect, Rect::from_min_size(pos2(600.0, 200.0), vec2(260.0, 300.0)), "at least the minimum width");
        let back = to_frame(rect, workspace);
        assert!((back[0] - 0.5).abs() < 1e-6 && (back[3] - 0.5).abs() < 1e-6);
        let outside = keep_inside(Rect::from_min_size(pos2(1050.0, 600.0), vec2(200.0, 100.0)), workspace);
        assert!(workspace.contains_rect(outside));
    }

    #[test]
    fn presets_open_the_right_windows() {
        let layout = Layout::preset(Preset::Default);
        assert!(layout.is_open(WindowKind::ChannelRack) && layout.is_open(WindowKind::Playlist) && layout.is_open(WindowKind::Mixer));
        assert!(!layout.is_open(WindowKind::PianoRoll));
        assert!(Layout::preset(Preset::Compose).is_open(WindowKind::PianoRoll));
        assert_eq!(WindowKind::from_id("pianoRoll"), Some(WindowKind::PianoRoll));
        assert_eq!(Preset::from_id("mix"), Some(Preset::Mix));
    }
}
