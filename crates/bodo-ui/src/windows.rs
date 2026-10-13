//! Free windows inside the workspace (project.md section 14.1, ADR 0012).
//!
//! The Channel Rack, Playlist, Piano Roll and Mixer are windows inside the workspace between the side panels. Each
//! one moves by its title bar, resizes from its edges, maximizes to fill the workspace (double-click the title bar),
//! minimizes to its title bar and closes. Where they sit is kept as fractions of the workspace, so the arrangement
//! survives resizing the main window. A [`Layout`] records it all and belongs to the user, not the project.
//!
//! Every free window but the Channel Rack can also be detached into its own OS window: an egui viewport, which
//! eframe shows as a native window that can go to another monitor or full screen (ADR 0032). A detached window draws
//! with the same app state and handles the keyboard there too, so shortcuts work in it. Its Attach button puts it
//! back. Where the backend can't show more than one native window, as in tests, a detached window stays in the
//! workspace with its Attach button.

use std::collections::BTreeMap;

use bodo_kit_ui::{Icon, icons};
use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Align2, CornerRadius, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::i18n::en;
use crate::theme::{Tokens, rem, size};

/// The smallest a window gets, in points.
const MIN_SIZE: Vec2 = vec2(rem(20.8), rem(9.6));
/// How far from an edge a resize drag starts, in points.
const GRIP: f32 = rem(0.48);

/// A free window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowKind {
    /// The step sequencer (it floats, but never leaves the main window).
    ChannelRack,
    /// The arrangement.
    #[default]
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
            Self::ChannelRack => en("Channel rack"),
            Self::Playlist => en("Playlist"),
            Self::PianoRoll => en("Piano roll"),
            Self::Mixer => en("Mixer"),
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
    /// Whether it is detached into its own OS window.
    #[serde(default)]
    pub detached: bool,
    /// Where its OS window was last while detached: left, top, width and height in points on the desktop, so it
    /// comes back on the same monitor.
    #[serde(default)]
    pub screen: Option<[f32; 4]>,
}

impl WindowState {
    /// A window at `frame` (fractions of the workspace), open or not, attached.
    pub const fn at(open: bool, frame: [f32; 4]) -> Self {
        Self { open, frame, maximized: false, minimized: false, detached: false, screen: None }
    }
}

/// What a free window's host does for it beyond drawing: handling the keyboard of a detached window, which the main
/// window's frame never sees.
pub trait WindowHost {
    /// Runs the shortcuts and the typing keyboard for the keys pressed in a detached window this frame.
    fn detached_keys(&mut self, ctx: &egui::Context);
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

/// A layout the user saved under a name.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLayout {
    /// Its name, unique among the saved layouts and different from every preset's.
    pub name: String,
    /// The arrangement.
    pub layout: Layout,
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
    /// The mixer filling the workspace with the master selected, for mastering.
    Master,
}

impl Preset {
    /// Every preset, in menu order.
    pub const ALL: [Self; 5] = [Self::Default, Self::Compose, Self::Arrange, Self::Mix, Self::Master];

    /// Its name in menus.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => en("Default"),
            Self::Compose => en("Compose"),
            Self::Arrange => en("Arrange"),
            Self::Mix => en("Mix"),
            Self::Master => en("Master"),
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
            Preset::Master => [
                (Mixer, WindowState { maximized: true, ..WindowState::at(true, [0.0, 0.0, 1.0, 1.0]) }),
                (Playlist, WindowState::at(false, [0.0, 0.0, 1.0, 0.3])),
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

/// Where a window waiting for keyboard focus is noted, until its title bar takes it.
fn focus_key() -> Id {
    Id::new("bodo-window-focus")
}

/// Brings the open window after the one in front to the front, in the order of the top bar's buttons, and moves
/// keyboard focus to its title bar, from where Tab goes on into it. The playlist and the piano roll become the editor
/// the tool keys act on. Returns the window, or `None` when none is open.
///
/// Complexity: O(w + l) in windows and egui's layers.
pub fn cycle(app: &mut crate::BodoApp, ctx: Option<&egui::Context>) -> Option<WindowKind> {
    let open: Vec<WindowKind> = WindowKind::ALL.into_iter().filter(|k| app.prefs.layout.is_open(*k)).collect();
    if open.is_empty() {
        return None;
    }
    // The window in front: the last of them in egui's layer order, else the editor used last.
    let front = ctx
        .and_then(|ctx| ctx.memory(|m| m.layer_ids().filter_map(|layer| open.iter().copied().find(|k| layer.id == area_id(*k))).last()))
        .unwrap_or(app.view.editor);
    let next = open.iter().position(|k| *k == front).map_or(open[0], |i| open[(i + 1) % open.len()]);
    match ctx {
        Some(ctx) => {
            app.prefs.layout.open(ctx, next);
            ctx.data_mut(|d| d.insert_temp(focus_key(), area_id(next)));
        }
        None => {
            let mut state = app.prefs.layout.state(next);
            state.open = true;
            state.minimized = false;
            app.prefs.layout.windows.insert(next, state);
        }
    }
    if matches!(next, WindowKind::Playlist | WindowKind::PianoRoll) {
        app.view.editor = next;
    }
    Some(next)
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
///
/// `label` is English: the control channel records it as it is, so agents find the button in any language, while
/// screen readers and the tooltip get it translated.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    icon_button_as(ui, icon, label, crate::i18n::lookup(label).unwrap_or(label), selected)
}

/// [`icon_button`] whose shown text is `shown`, already in the interface's language, for labels with values in
/// them; `label` is the English one the control channel records.
pub fn icon_button_as(ui: &mut Ui, icon: Icon, label: &str, shown: &str, selected: bool) -> egui::Response {
    painted_button_as(ui, label, shown, selected, |painter, rect, color| {
        let side = 14.0;
        icon.paint(painter, Rect::from_center_size(rect.center(), vec2(side, side)), color);
    })
}

/// A small button in the interface's style whose face `paint` draws into its rect, in the color its state gives.
/// `label` is English, translated for screen readers and the tooltip as in [`icon_button`].
pub fn painted_button(ui: &mut Ui, label: &str, selected: bool, paint: impl FnOnce(&egui::Painter, Rect, egui::Color32)) -> egui::Response {
    painted_button_as(ui, label, crate::i18n::lookup(label).unwrap_or(label), selected, paint)
}

/// [`painted_button`] whose shown text is `shown`, already translated; `label` is the English one recorded.
pub fn painted_button_as(
    ui: &mut Ui,
    label: &str,
    shown: &str,
    selected: bool,
    paint: impl FnOnce(&egui::Painter, Rect, egui::Color32),
) -> egui::Response {
    let tk = Tokens::get();
    let style = button_style(&tk);
    let response =
        IconButton::new(shown, vec2(rem(1.76), rem(1.6)), &style).selected(selected).show(ui, |ui, rect, color| paint(ui.painter(), rect, color));
    crate::control::record(ui, &response, label, "button", Some(selected));
    response.on_hover_text(shown)
}

/// Shows window `kind` inside `workspace` with `title`. `header` draws extra controls into the title bar after the
/// title; `body` draws the contents. Both get `state`, usually the app.
#[allow(clippy::too_many_arguments, reason = "a window needs its place, title, state and both parts")]
pub fn show<T: WindowHost>(
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
    // The Channel Rack never leaves the main window (ADR 0012).
    let detachable = kind != WindowKind::ChannelRack;
    let (new_state, events) = show_frame(ctx, area_id(kind), kind.icon(), state, workspace, title, detachable, state_of, header, body);
    if new_state != state {
        layout.windows.insert(kind, new_state);
    }
    events
}

/// Shows a free window with egui id `id`, its title bar showing `icon` and `title`, at `state` inside `workspace`.
/// `header` draws extra controls into the title bar; `body` draws the contents. A `detachable` window has a button
/// that detaches it into its own OS window, and while detached it shows there. Returns the window's state after
/// this frame's moves, resizes and button clicks, and what happened. Windows that come and go, such as channel
/// windows, keep their own state; the main windows keep theirs in the [`Layout`].
#[allow(clippy::too_many_arguments, reason = "a window needs its place, title, state and both parts")]
pub fn show_frame<T: WindowHost>(
    ctx: &egui::Context,
    id: Id,
    icon: Icon,
    state: WindowState,
    workspace: Rect,
    title: &str,
    detachable: bool,
    state_of: &mut T,
    header: impl FnOnce(&mut T, &mut Ui),
    body: impl FnOnce(&mut T, &mut Ui),
) -> (WindowState, WindowEvents) {
    show_frame_as(ctx, id, icon, state, workspace, title, &shown_title(title), detachable, state_of, header, body)
}

/// [`show_frame`] with the title people see given apart from `title`, the English one the control channel names the
/// window by: for titles built from values, which [`shown_title`] can't translate piece by piece.
#[allow(clippy::too_many_arguments, reason = "a window needs its place, title, state and both parts")]
pub fn show_frame_as<T: WindowHost>(
    ctx: &egui::Context,
    id: Id,
    icon: Icon,
    state: WindowState,
    workspace: Rect,
    title: &str,
    shown: &str,
    detachable: bool,
    state_of: &mut T,
    header: impl FnOnce(&mut T, &mut Ui),
    body: impl FnOnce(&mut T, &mut Ui),
) -> (WindowState, WindowEvents) {
    if state.detached && detachable && !ctx.embed_viewports() {
        return show_detached(ctx, id, icon, state, workspace, title, shown, state_of, header, body);
    }
    let tk = Tokens::get();
    let full = if state.maximized { workspace } else { frame_rect(state.frame, workspace) };
    let rect = if state.minimized { Rect::from_min_size(full.min, vec2(full.width().min(300.0), size::TITLE_BAR)) } else { full };
    // A window that wasn't showing comes to the front when it appears, however it was opened.
    let layer = egui::LayerId::new(Order::Middle, id);
    if !ctx.memory(|m| m.areas().visible_last_frame(&layer)) {
        ctx.move_to_top(layer);
    }
    let mut new_state = state;
    let mut events = WindowEvents::default();
    egui::Area::new(id).order(Order::Middle).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        // Clicking the body brings the window forward; it is no stop for Tab, which goes to the controls in it.
        let (rect, background) = ui.allocate_exact_size(rect.size(), Sense::CLICK);
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

        // Moving: drag the title bar; double-click maximizes. The title bar also takes keyboard focus when the window
        // is brought forward from the keyboard, and shows it.
        let drag = ui.interact(title_rect, id.with("title"), Sense::click_and_drag());
        drag.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, title));
        if ctx.data(|d| d.get_temp::<Id>(focus_key())) == Some(id) {
            drag.request_focus();
            ctx.data_mut(|d| d.remove::<Id>(focus_key()));
        }
        if drag.has_focus() {
            painter.rect_stroke(title_rect.shrink(1.0), CornerRadius::same(7), Stroke::new(2.0, tk.accent), StrokeKind::Inside);
            // Enter maximizes or restores it, as a double-click does.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                new_state.maximized = !state.maximized;
                new_state.minimized = false;
            }
        }
        if drag.double_clicked() {
            new_state.maximized = !state.maximized;
            new_state.minimized = false;
        } else if drag.dragged() && !state.maximized {
            let delta = drag.drag_delta();
            let moved = full.translate(delta);
            new_state.frame = to_frame(keep_inside(moved, workspace), workspace);
        }

        let buttons_width = title_buttons_width(if detachable { 4 } else { 3 });
        let mut bar = title_bar(ui, title_rect, buttons_width);
        icon.paint(bar.painter(), Rect::from_min_size(bar.cursor().min + vec2(rem(0.0), rem(0.24)), vec2(rem(1.12), rem(1.12))), tk.text_dim);
        bar.add_space(rem(1.44));
        title_label(&mut bar, shown);
        bar.add_space(rem(0.8));
        header(state_of, &mut bar);
        let mut buttons = ui.new_child(
            UiBuilder::new().max_rect(title_rect.shrink2(vec2(rem(0.32), rem(0.24)))).layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        // The control channel names the buttons in English, with the English title; people see them translated.
        if icon_button_as(&mut buttons, icons::X, &format!("Close {title}"), &tf!("Close {window}", window = shown), false).clicked() {
            new_state.open = false;
            events.closed = true;
        }
        let (max_icon, max_label, max_shown) = if state.maximized {
            (icons::WINDOW_RESTORE, "Restore", tf!("Restore {window}", window = shown))
        } else {
            (icons::SQUARE, "Maximize", tf!("Maximize {window}", window = shown))
        };
        if icon_button_as(&mut buttons, max_icon, &format!("{max_label} {title}"), &max_shown, false).clicked() {
            new_state.maximized = !state.maximized;
            new_state.minimized = false;
        }
        if icon_button_as(&mut buttons, icons::MINUS, &format!("Minimize {title}"), &tf!("Minimize {window}", window = shown), state.minimized)
            .clicked()
        {
            new_state.minimized = !state.minimized;
        }
        if detachable {
            if state.detached {
                // Only where the backend can't open more windows: the window stays here, ready to go back.
                if icon_button_as(&mut buttons, icons::ARROW_BACK_UP, &format!("Attach {title}"), &tf!("Attach {window}", window = shown), true)
                    .clicked()
                {
                    new_state.detached = false;
                }
            } else if icon_button_as(&mut buttons, icons::EXTERNAL_LINK, &format!("Detach {title}"), &tf!("Detach {window}", window = shown), false)
                .clicked()
            {
                new_state = detach(ctx, new_state, full);
            }
        }

        if !state.minimized {
            let body_rect = Rect::from_min_max(pos2(rect.left() + 1.0, title_rect.bottom()), rect.max - vec2(rem(0.08), rem(0.08)));
            let mut body_ui =
                ui.new_child(UiBuilder::new().id_salt(id.with("body")).max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
            body_ui.set_clip_rect(body_rect.intersect(workspace));
            body(state_of, &mut body_ui);
            if !state.maximized {
                resize_grips(ui, id, rect, full, workspace, &mut new_state);
            }
        }
    });
    (new_state, events)
}

/// `state` detached into its own OS window, placed where the window is now: `rect`, in the main window's points.
/// A window detached before goes back where it was.
fn detach(ctx: &egui::Context, state: WindowState, rect: Rect) -> WindowState {
    let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
    let here = [origin.x + rect.left(), origin.y + rect.top(), rect.width(), (rect.height() - size::TITLE_BAR).max(MIN_SIZE.y)];
    WindowState { detached: true, minimized: false, maximized: false, screen: Some(state.screen.unwrap_or(here)), ..state }
}

/// Shows a detached window in its own OS window. It opens where `state.screen` says, keeps track of where the user
/// moves it, closes with its OS window, and goes back into the main window with its Attach button.
#[allow(clippy::too_many_arguments, reason = "a window needs its place, title, state and both parts")]
fn show_detached<T: WindowHost>(
    ctx: &egui::Context,
    id: Id,
    icon: Icon,
    state: WindowState,
    workspace: Rect,
    title: &str,
    shown: &str,
    state_of: &mut T,
    header: impl FnOnce(&mut T, &mut Ui),
    body: impl FnOnce(&mut T, &mut Ui),
) -> (WindowState, WindowEvents) {
    let tk = Tokens::get();
    // The OS window opens where it was saved. After that the user moves it, and the builder stays as it was, or
    // egui would move the window back.
    let fallback = {
        let rect = frame_rect(state.frame, workspace);
        [rect.left(), rect.top(), rect.width(), rect.height()]
    };
    let spawn_key = id.with("spawn");
    let [x, y, w, h] = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(spawn_key, || state.screen.unwrap_or(fallback)));
    let builder = egui::ViewportBuilder::default()
        .with_title(format!("{shown} - Bodo"))
        .with_position([x, y])
        .with_inner_size([w.max(MIN_SIZE.x), h.max(MIN_SIZE.y)])
        .with_min_inner_size(MIN_SIZE);
    let mut new_state = state;
    let mut events = WindowEvents::default();
    let (mut header, mut body) = (Some(header), Some(body));
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(id), builder, |ui, _class| {
        let ctx = ui.ctx().clone();
        state_of.detached_keys(&ctx);
        let (close, outer, inner) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
        if close {
            new_state.open = false;
            events.closed = true;
        }
        if let (Some(outer), Some(inner)) = (outer, inner) {
            new_state.screen = Some([outer.left(), outer.top(), inner.width(), inner.height()]);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tk.panel)).show(ui, |ui| {
            let rect = ui.max_rect();
            events.hovered = ui.rect_contains_pointer(rect);
            let title_rect = Rect::from_min_size(rect.min, vec2(rect.width(), size::TITLE_BAR));
            ui.painter().rect_filled(title_rect, 0.0, tk.chrome);
            let mut bar = title_bar(ui, title_rect, title_buttons_width(1));
            icon.paint(bar.painter(), Rect::from_min_size(bar.cursor().min + vec2(rem(0.0), rem(0.24)), vec2(rem(1.12), rem(1.12))), tk.text_dim);
            bar.add_space(rem(1.44));
            title_label(&mut bar, shown);
            bar.add_space(rem(0.8));
            if let Some(header) = header.take() {
                header(state_of, &mut bar);
            }
            let mut buttons = ui.new_child(
                UiBuilder::new().max_rect(title_rect.shrink2(vec2(rem(0.32), rem(0.24)))).layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if icon_button_as(&mut buttons, icons::ARROW_BACK_UP, &format!("Attach {title}"), &tf!("Attach {window}", window = shown), true).clicked()
            {
                new_state.detached = false;
            }
            let body_rect = Rect::from_min_max(pos2(rect.left(), title_rect.bottom()), rect.max);
            let mut body_ui =
                ui.new_child(UiBuilder::new().id_salt(id.with("body")).max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)));
            body_ui.set_clip_rect(body_rect);
            if let Some(body) = body.take() {
                body(state_of, &mut body_ui);
            }
        });
    });
    if !new_state.detached || !new_state.open {
        // The next detach opens where the window was last.
        ctx.data_mut(|d| d.remove::<[f32; 4]>(spawn_key));
    }
    (new_state, events)
}

/// The edges and corner that resize a window.
fn resize_grips(ui: &mut Ui, id: Id, rect: Rect, full: Rect, workspace: Rect, state: &mut WindowState) {
    let right = Rect::from_min_max(pos2(rect.right() - GRIP, rect.top() + size::TITLE_BAR), pos2(rect.right(), rect.bottom() - GRIP));
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - GRIP), pos2(rect.right() - GRIP, rect.bottom()));
    let corner = Rect::from_min_max(rect.max - vec2(GRIP * 2.0, GRIP * 2.0), rect.max);
    let mut size = full.size();
    for (rect, name, cursor, axes) in [
        (right, "right", egui::CursorIcon::ResizeHorizontal, vec2(rem(0.08), rem(0.0))),
        (bottom, "bottom", egui::CursorIcon::ResizeVertical, vec2(rem(0.0), rem(0.08))),
        (corner, "corner", egui::CursorIcon::ResizeNwSe, vec2(rem(0.08), rem(0.08))),
    ] {
        // The pointer resizes; the keyboard maximizes with Enter on the title bar, so the edges are no stops for Tab.
        let response = ui.interact(rect, id.with(name), Sense::DRAG).on_hover_cursor(cursor);
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

/// The width the title bar's `count` buttons take at its right end.
fn title_buttons_width(count: usize) -> f32 {
    count as f32 * (rem(1.76) + rem(0.48)) + rem(0.32)
}

/// The left part of a title bar in `title_rect`, for the icon, the title and the header, leaving `buttons_width` at
/// the right for the window's buttons. Whatever doesn't fit is cut off rather than drawn under them.
fn title_bar(ui: &mut Ui, title_rect: Rect, buttons_width: f32) -> Ui {
    let left = Rect::from_min_max(title_rect.min, pos2((title_rect.right() - buttons_width).max(title_rect.left()), title_rect.bottom()));
    let mut bar =
        ui.new_child(UiBuilder::new().max_rect(left.shrink2(vec2(rem(0.48), rem(0.24)))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bar.set_clip_rect(left.intersect(ui.clip_rect()));
    bar
}

/// The window's title, at most about a third of the bar so the header's controls keep their room, shortened with an
/// ellipsis when longer; hovering it shows it whole.
fn title_label(bar: &mut Ui, shown: &str) {
    let room = (bar.available_width() * 0.32).max(rem(6.0));
    bar.scope(|ui| {
        ui.set_max_width(room);
        ui.add(egui::Label::new(egui::RichText::new(shown).color(Tokens::get().text).size(size::BODY)).truncate());
    });
}

/// The words window titles are built from, such as "Piano roll" in "Piano roll - Verse > Kick". Titles are written in
/// English and shown with each of these pieces translated, so the names in them stay as they are.
pub const TITLE_WORDS: [&str; 14] = [
    en("Channel rack"),
    en("Playlist"),
    en("Arrangement"),
    en("Piano roll"),
    en("Mixer"),
    en("Sampler"),
    en("Audio clip"),
    en("Automation clip"),
    en("Layer"),
    en("MIDI Out"),
    en("Instrument"),
    en("Insert"),
    en("Master"),
    en("No pattern"),
];

/// Window title `title`, written in English, in the interface's language: each piece between " - " and " > " that
/// the catalog has is translated, and names stay as they are.
///
/// Complexity: O(n) in the title's length.
pub fn shown_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for (i, part) in title.split(" - ").enumerate() {
        if i > 0 {
            out.push_str(" - ");
        }
        for (j, piece) in part.split(" > ").enumerate() {
            if j > 0 {
                out.push_str(" > ");
            }
            out.push_str(crate::i18n::lookup(piece).unwrap_or(piece));
        }
    }
    out
}

/// Draws `text`, English, centered in `rect` in the interface's language, for empty states.
pub fn placeholder(ui: &Ui, rect: Rect, text: &str) {
    let tk = Tokens::get();
    let text = crate::i18n::lookup(text).unwrap_or(text);
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
        assert_eq!(rect, Rect::from_min_size(pos2(600.0, 200.0), vec2(MIN_SIZE.x, 300.0)), "at least the minimum width");
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
