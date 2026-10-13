//! The Bodo interface (project.md section 14, ADRs 0001 and 0012).
//!
//! [`BodoApp`] is the whole main window: the two-row top bar, the Browser on the left, and the workspace with the free
//! windows (Channel Rack, Playlist, Piano Roll, Mixer). It owns the editing session and the audio engine. The UI
//! never owns document data: every change goes through [`BodoApp::run`], which calls the session's commands, and every
//! frame draws from the session's current snapshot.
//!
//! - [`theme`]: design tokens and egui styling.
//! - [`windows`]: free windows and layouts.
//! - [`audio`]: the audio device and engine, kept in step with the session.
//! - [`control`] and [`remote`]: automation through the control channel, for agents and tests.

#![forbid(unsafe_code)]

pub mod audio;
mod autosave;
mod browser;
pub mod control;
mod dialogs;
pub mod library;
mod mixer;
mod palette;
mod piano_roll;
mod playlist;
mod rack;
pub mod remote;
pub mod screenshot;
mod shortcuts;
pub mod theme;
mod topbar;
mod typing;
pub mod windows;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bodo_engine::{Call, CommandError, Session};
use bodo_model::Project;
use serde_json::{Value, json};

pub use crate::autosave::{end_session, start_session};
pub use crate::piano_roll::{Lane, PianoView};
pub use crate::playlist::{PlaylistView, Tool};
pub use crate::rack::{GroupFilter, RackView, StepParam};
use crate::theme::{ThemePref, Tokens};
pub use crate::typing::Typing;
use crate::windows::{Layout, Preset, WindowKind};

/// The storage key of the preferences.
pub const PREFS_KEY: &str = "bodo.prefs";

/// Asks the user for a file or folder; `None` when they cancel.
pub type PickPath = Box<dyn Fn() -> Option<PathBuf>>;
/// Asks the user where to save, suggesting a file name.
pub type PickSave = Box<dyn Fn(&str) -> Option<PathBuf>>;
/// Asks the user for some files.
pub type PickFiles = Box<dyn Fn() -> Vec<PathBuf>>;

/// File dialogs and other services the app shell provides. Tests leave them empty.
#[derive(Default)]
pub struct Services {
    /// Asks for a project to open.
    pub pick_open: Option<PickPath>,
    /// Asks where to save a project, suggesting a file name.
    pub pick_save: Option<PickSave>,
    /// Asks for audio files to load.
    pub pick_samples: Option<PickFiles>,
    /// Asks for a folder of sounds to add to the browser.
    pub pick_folder: Option<PickPath>,
    /// Asks for a MIDI file to import.
    pub pick_midi: Option<PickPath>,
    /// Asks where to write a MIDI file, suggesting a file name.
    pub pick_midi_save: Option<PickSave>,
    /// Measures the memory the app holds, in bytes, for the performance readout. [`resident_memory`] does it.
    pub memory: Option<fn() -> Option<u64>>,
}

/// The memory the process holds in RAM, in bytes, where the platform tells.
pub fn resident_memory() -> Option<u64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as u64)
}

/// What the user chose, kept between runs. Window layouts belong here, not in projects.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// The theme.
    pub theme: ThemePref,
    /// The window layout.
    pub layout: Layout,
    /// Folders of the user's own sounds.
    pub library: Vec<PathBuf>,
    /// Recently opened or saved projects, newest first.
    pub recent: Vec<PathBuf>,
    /// The user library: a Bodo folder in the user's documents, chosen on first run. `None` for no library, as in
    /// tests.
    pub user_library: Option<PathBuf>,
    /// Minutes between autosaves; 0 turns autosave off.
    pub autosave_minutes: u32,
    /// The audio device.
    pub audio: audio::AudioPrefs,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: ThemePref::Dark,
            layout: Layout::default(),
            library: Vec::new(),
            recent: Vec::new(),
            user_library: None,
            autosave_minutes: 5,
            audio: audio::AudioPrefs::default(),
        }
    }
}

/// How positions snap in the editors (project.md section 10.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Snap {
    /// Anywhere.
    None,
    /// A sixth of a step.
    SixthStep,
    /// A quarter of a step.
    QuarterStep,
    /// A third of a step.
    ThirdStep,
    /// Half a step.
    HalfStep,
    /// A sixteenth note.
    Step,
    /// A sixth of a beat.
    SixthBeat,
    /// A third of a beat.
    ThirdBeat,
    /// Half a beat.
    HalfBeat,
    /// A beat.
    #[default]
    Beat,
    /// A bar.
    Bar,
}

impl Snap {
    /// Every choice, in menu order.
    pub const ALL: [Self; 11] = [
        Self::None,
        Self::SixthStep,
        Self::QuarterStep,
        Self::ThirdStep,
        Self::HalfStep,
        Self::Step,
        Self::SixthBeat,
        Self::ThirdBeat,
        Self::HalfBeat,
        Self::Beat,
        Self::Bar,
    ];

    /// Its name in the snap menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::SixthStep => "1/6 step",
            Self::QuarterStep => "1/4 step",
            Self::ThirdStep => "1/3 step",
            Self::HalfStep => "1/2 step",
            Self::Step => "Step",
            Self::SixthBeat => "1/6 beat",
            Self::ThirdBeat => "1/3 beat",
            Self::HalfBeat => "1/2 beat",
            Self::Beat => "Beat",
            Self::Bar => "Bar",
        }
    }

    /// The grid in ticks, for `project`'s meter; 1 for none.
    pub fn ticks(self, project: &Project) -> i64 {
        let meter = project.meter.signature_at(bodo_kit_core::Tick::ZERO);
        let step = bodo_engine::params::STEP.0;
        let beat = meter.beat_ticks();
        match self {
            Self::None => 1,
            Self::SixthStep => step / 6,
            Self::QuarterStep => step / 4,
            Self::ThirdStep => step / 3,
            Self::HalfStep => step / 2,
            Self::Step => step,
            Self::SixthBeat => beat / 6,
            Self::ThirdBeat => beat / 3,
            Self::HalfBeat => beat / 2,
            Self::Beat => beat,
            Self::Bar => meter.bar_ticks(),
        }
    }

    /// `ticks` rounded down to the grid. A position a hair below a grid line, as converting from points can give,
    /// counts as on it.
    pub fn floor(self, project: &Project, ticks: f64) -> i64 {
        let grid = self.ticks(project).max(1) as f64;
        ((ticks / grid + 1e-6).floor() * grid) as i64
    }

    /// `ticks` rounded to the nearest grid line.
    pub fn round(self, project: &Project, ticks: f64) -> i64 {
        let grid = self.ticks(project).max(1) as f64;
        ((ticks / grid).round() * grid) as i64
    }
}

/// What the interface shows besides the project: tools, zoom, scroll, dialogs. Not saved.
#[derive(Debug, Default)]
pub struct View {
    /// The snap of the playlist and piano roll.
    pub snap: Snap,
    /// The playlist's tool, zoom and scroll.
    pub playlist: playlist::PlaylistView,
    /// The piano roll's tool, zoom and scroll.
    pub piano: piano_roll::PianoView,
    /// The mixer's scroll.
    pub mixer: mixer::MixerView,
    /// The browser's tab, open folders and search.
    pub browser: browser::BrowserView,
    /// The command palette, while it is open.
    pub palette: Option<palette::PaletteState>,
    /// Dialogs and popups.
    pub dialogs: dialogs::Dialogs,
    /// What the hint bar says about the control under the pointer.
    pub hint: Option<String>,
    /// The gesture of the step stroke being painted, so it is one undo step.
    pub step_gesture: Option<u64>,
    /// The editor the tool keys and arrow keys act on: the playlist or the piano roll, whichever was used last.
    pub editor: WindowKind,
    /// The typing keyboard: whether the computer keyboard plays notes, and the notes it holds.
    pub typing: typing::Typing,
    /// The channel rack's group filter, view options and graph editor.
    pub rack: rack::RackView,
    /// Whether the playlist and the piano roll follow the playhead while it plays.
    pub auto_scroll: bool,
}

/// The main window.
pub struct BodoApp {
    /// The editing session: the project, its history and the selection.
    pub session: Session,
    /// The user's preferences.
    pub prefs: Prefs,
    /// Tools, zoom, scroll and dialogs.
    pub view: View,
    /// The audio device and engine.
    pub audio: audio::Audio,
    /// File dialogs.
    pub services: Services,
    /// The control channel for agents and tests.
    pub remote: remote::Remote,
    /// Screenshots being taken.
    pub shots: screenshot::Shots,
    /// Bodo's data folder, for autosaves; `None` turns autosave off.
    pub data_dir: Option<PathBuf>,
    /// Set when the app should close.
    pub quit_requested: bool,
    /// Quit once the requested screenshot is saved (`--screenshot`).
    pub quit_after_shot: bool,
    autosave: autosave::Autosave,
    shown_theme: Option<egui::Theme>,
    status: Option<(String, bool, Instant)>,
    gestures: std::collections::HashMap<egui::Id, u64>,
    next_gesture: u64,
}

/// How long a status message stays in the hint bar.
const STATUS_TIME: Duration = Duration::from_secs(6);

impl BodoApp {
    /// The app around `session`, with `prefs`. `audio` opens the audio device; tests leave it off.
    pub fn new(session: Session, prefs: Prefs, services: Services, audio: bool) -> Self {
        let audio = if audio { audio::Audio::start(&session, &prefs.audio) } else { audio::Audio::off() };
        let mut app = Self {
            session,
            prefs,
            view: View::default(),
            audio,
            services,
            remote: remote::Remote::default(),
            shots: screenshot::Shots::default(),
            data_dir: None,
            quit_requested: false,
            quit_after_shot: false,
            autosave: autosave::Autosave::default(),
            shown_theme: None,
            status: None,
            gestures: std::collections::HashMap::new(),
            next_gesture: 1,
        };
        app.sync_library();
        app
    }

    /// The window title: the project's name and whether it has unsaved changes.
    pub fn title(&self) -> String {
        let project = self.session.project();
        let name = if !project.meta.title.is_empty() {
            project.meta.title.clone()
        } else {
            self.session.file().and_then(|f| f.file_stem()).map_or_else(|| String::from("Untitled"), |s| s.to_string_lossy().into_owned())
        };
        format!("{name}{}", if self.session.is_dirty() { " *" } else { "" })
    }

    /// Runs command `id` with `params`: a command of the session, or one of the interface's own (`ui.*` commands that
    /// the session doesn't know). Shows the outcome in the hint bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.invoke(Call::new(id, params))
    }

    /// Runs `call`, which may belong to a gesture so a whole drag is one undo step.
    pub fn invoke(&mut self, call: Call) -> Result<Value, String> {
        if self.session.registry().get(&call.id).is_none()
            && let Some(result) = self.ui_command(&call.id, &call.params)
        {
            return result;
        }
        match self.session.invoke(&call) {
            Ok(output) => {
                if let Some(undo) = &output.undo {
                    self.set_status(undo.clone(), false);
                }
                self.after_command();
                Ok(output.result)
            }
            Err(error) => {
                self.report(&error);
                Err(error.to_string())
            }
        }
    }

    /// Runs command `id` as part of the gesture of the widget with `response`: one undo step from the press to the
    /// release.
    pub fn run_gesture(&mut self, response: &egui::Response, id: &str, params: Value) {
        if response.drag_started() || !self.gestures.contains_key(&response.id) {
            self.next_gesture += 1;
            self.gestures.insert(response.id, self.next_gesture);
        }
        let gesture = self.gestures.get(&response.id).copied();
        let call = Call { id: id.to_owned(), params, gesture: gesture.filter(|_| response.dragged() || response.drag_stopped()) };
        let _ = self.invoke(call);
        if !response.dragged() {
            self.gestures.remove(&response.id);
        }
    }

    /// A new gesture id, for gestures that span several widgets, such as painting steps.
    pub fn new_gesture(&mut self) -> u64 {
        self.next_gesture += 1;
        self.next_gesture
    }

    fn report(&mut self, error: &CommandError) {
        let mut text = error.message.clone();
        if let Some(hint) = &error.hint {
            text.push_str(&format!(" ({hint})"));
        }
        self.set_status(text, true);
    }

    /// Shows `text` in the hint bar for a few seconds; `error` shows it in the error color.
    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), error, Instant::now()));
    }

    /// The status message, if one is still showing, and whether it is an error.
    pub fn status(&self) -> Option<(&str, bool)> {
        self.status.as_ref().filter(|(_, _, at)| at.elapsed() < STATUS_TIME).map(|(text, error, _)| (text.as_str(), *error))
    }

    /// Commands of the interface itself, which aren't project edits. Returns `None` for an id it doesn't know.
    fn ui_command(&mut self, id: &str, params: &Value) -> Option<Result<Value, String>> {
        let text = |key: &str| params.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let result = match id {
            "ui.toggle_window" | "ui.open_window" => match WindowKind::from_id(&text("window")) {
                Some(kind) => {
                    let ctx = self.remote.ctx();
                    match (id, ctx) {
                        ("ui.open_window", Some(ctx)) => self.prefs.layout.open(&ctx, kind),
                        (_, Some(ctx)) => self.prefs.layout.toggle(&ctx, kind),
                        (_, None) => {
                            let mut state = self.prefs.layout.state(kind);
                            state.open = id == "ui.open_window" || !state.open;
                            self.prefs.layout.windows.insert(kind, state);
                        }
                    }
                    if matches!(kind, WindowKind::Playlist | WindowKind::PianoRoll) && self.prefs.layout.is_open(kind) {
                        self.view.editor = kind;
                    }
                    Ok(json!({ "window": kind.id(), "open": self.prefs.layout.is_open(kind) }))
                }
                None => Err(String::from("window must be channelRack, playlist, pianoRoll or mixer")),
            },
            "ui.toggle_browser" => {
                self.prefs.layout.browser = !self.prefs.layout.browser;
                Ok(json!({ "browser": self.prefs.layout.browser }))
            }
            "ui.set_layout" => match Preset::from_id(&text("layout")) {
                Some(preset) => {
                    self.prefs.layout =
                        Layout { browser: self.prefs.layout.browser, browser_width: self.prefs.layout.browser_width, ..Layout::preset(preset) };
                    Ok(json!({ "layout": preset.label() }))
                }
                None => Err(String::from("layout must be Default, Compose, Arrange or Mix")),
            },
            "ui.set_snap" => match Snap::ALL.into_iter().find(|s| s.label().eq_ignore_ascii_case(&text("snap"))) {
                Some(snap) => {
                    self.view.snap = snap;
                    Ok(json!({ "snap": snap.label() }))
                }
                None => Err(format!("snap must be one of {}", Snap::ALL.iter().map(|s| s.label()).collect::<Vec<_>>().join(", "))),
            },
            "ui.set_theme" => match ThemePref::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(&text("theme"))) {
                Some(theme) => {
                    self.prefs.theme = theme;
                    Ok(json!({ "theme": theme.label() }))
                }
                None => Err(String::from("theme must be Dark, Light or System")),
            },
            "ui.set_tool" => {
                let target = match params.get("window").and_then(Value::as_str) {
                    Some(window) => WindowKind::from_id(window),
                    None if self.view.editor == WindowKind::PianoRoll && self.prefs.layout.is_open(WindowKind::PianoRoll) => {
                        Some(WindowKind::PianoRoll)
                    }
                    None => Some(WindowKind::Playlist),
                };
                match target {
                    Some(WindowKind::PianoRoll) => piano_roll::set_tool(self, &text("tool")),
                    Some(WindowKind::Playlist) => playlist::set_tool(self, &text("tool")),
                    _ => Err(String::from("window must be playlist or pianoRoll")),
                }
            }
            "ui.open_piano_roll" => {
                if let Some(channel) = params.get("channel").filter(|c| !c.is_null())
                    && let Err(e) = self.session.execute("edit.select", json!({ "channel": channel }))
                {
                    return Some(Err(e.to_string()));
                }
                if let Some(ctx) = self.remote.ctx() {
                    self.prefs.layout.open(&ctx, WindowKind::PianoRoll);
                } else {
                    let mut state = self.prefs.layout.state(WindowKind::PianoRoll);
                    state.open = true;
                    self.prefs.layout.windows.insert(WindowKind::PianoRoll, state);
                }
                self.view.editor = WindowKind::PianoRoll;
                Ok(json!({ "channel": self.session.editor().channel }))
            }
            "ui.palette" => {
                self.view.palette = Some(palette::PaletteState::default());
                Ok(json!({ "palette": true }))
            }
            _ => return None,
        };
        if let Err(error) = &result {
            self.set_status(error.clone(), true);
        }
        Some(result)
    }

    /// The interface's own commands, for the palette and the control channel: id, title and summary.
    pub fn ui_commands() -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("ui.toggle_window", "Show or hide a window", "Shows or hides the channel rack, playlist, piano roll or mixer: {\"window\": \"mixer\"}."),
            ("ui.open_window", "Open a window", "Opens and brings a window to the front: {\"window\": \"playlist\"}."),
            ("ui.toggle_browser", "Show or hide the browser", "Shows or hides the browser on the left."),
            ("ui.set_layout", "Switch layout", "Arranges the windows: {\"layout\": \"Default\" | \"Compose\" | \"Arrange\" | \"Mix\"}."),
            (
                "ui.set_snap",
                "Set snap",
                "Sets the editors' snap: {\"snap\": \"None\" | \"1/4 step\" | \"Step\" | \"1/2 beat\" | \"Beat\" | \"Bar\" ...}.",
            ),
            ("ui.set_theme", "Set theme", "Sets the theme: {\"theme\": \"Dark\" | \"Light\" | \"System\"}."),
            (
                "ui.set_tool",
                "Set tool",
                "Sets the tool of the editor used last, or of {\"window\": \"playlist\" or \"pianoRoll\"}: draw, paint, delete, mute, slip (playlist only), slice, select, zoom or playback.",
            ),
            ("ui.open_piano_roll", "Open piano roll", "Opens the piano roll, optionally for a channel: {\"channel\": \"Kick\"}."),
            ("ui.palette", "Command palette", "Opens the command palette."),
        ]
    }

    /// Keeps the session's environment in step with the user's sample folders.
    fn sync_library(&mut self) {
        let mut env = self.session.env().clone();
        if env.library != self.prefs.library || env.user != self.prefs.user_library {
            env.library.clone_from(&self.prefs.library);
            env.user.clone_from(&self.prefs.user_library);
            self.session.set_env(env);
        }
    }

    /// The user library, when there is one.
    pub fn user_library(&self) -> Option<bodo_engine::library::UserLibrary> {
        self.prefs.user_library.as_ref().map(bodo_engine::library::UserLibrary::new)
    }

    /// Bookkeeping after any command: the audio engine, recent files.
    fn after_command(&mut self) {
        if let Some(file) = self.session.file().map(PathBuf::from)
            && self.prefs.recent.first() != Some(&file)
        {
            self.prefs.recent.retain(|f| f != &file);
            self.prefs.recent.insert(0, file);
            self.prefs.recent.truncate(10);
        }
    }

    /// Everything the frame needs before drawing: theme, shortcuts, the control channel, audio and autosave.
    pub fn logic(&mut self, ctx: &egui::Context) {
        let theme = self.prefs.theme.resolve(ctx.system_theme());
        if self.shown_theme != Some(theme) {
            theme::apply(ctx, theme);
            self.shown_theme = Some(theme);
        }
        self.remote.set_ctx(ctx);
        typing::handle(self, ctx);
        shortcuts::handle(self, ctx);
        remote::drain(self, ctx);
        self.audio.sync(&mut self.session);
        if self.audio.is_playing() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        autosave::tick(self);
        self.shots.tick(ctx);
        if self.quit_after_shot
            && let Some(done) = &self.shots.done
        {
            if let Err(error) = done {
                tracing::error!(%error, "screenshot failed");
            }
            self.quit_requested = true;
        }
        if let Some((_, _, at)) = &self.status {
            let left = STATUS_TIME.saturating_sub(at.elapsed());
            if !left.is_zero() {
                ctx.request_repaint_after(left);
            }
        }
        self.sync_library();
    }

    /// Draws the main window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        control::begin_frame(ui.ctx());
        self.view.hint = None;
        let tk = Tokens::get();
        topbar::show(self, ui);
        if self.prefs.layout.browser {
            let panel = egui::Panel::left("bodo-browser")
                .resizable(true)
                .default_size(self.prefs.layout.browser_width)
                .size_range(160.0..=480.0)
                .frame(egui::Frame::NONE.fill(tk.panel).inner_margin(egui::Margin::same(0)))
                .show(ui, |ui| browser::show(self, ui));
            self.prefs.layout.browser_width = panel.response.rect.width();
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tk.border)).show(ui, |ui| {
            let workspace = ui.max_rect();
            let project = Arc::clone(self.session.project());
            // Windows that appear together stack in this order, the last on top; after that, the one used last is.
            playlist::window(self, ui.ctx(), workspace, &project);
            mixer::window(self, ui.ctx(), workspace, &project);
            rack::window(self, ui.ctx(), workspace, &project);
            piano_roll::window(self, ui.ctx(), workspace, &project);
            // A press on the playlist or the piano roll makes it the editor the tool keys act on.
            let ctx = ui.ctx();
            let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
            if let Some(layer) = pressed.and_then(|at| ctx.layer_id_at(at)) {
                for kind in [WindowKind::Playlist, WindowKind::PianoRoll] {
                    if layer.id == windows::area_id(kind) {
                        self.view.editor = kind;
                    }
                }
            }
        });
        dialogs::show(self, ui.ctx());
        palette::show(self, ui.ctx());
        browser::handle_drops(self, ui.ctx());
    }

    /// Closes Bodo, asking first when there are unsaved changes.
    pub fn request_quit(&mut self) {
        dialogs::guard_unsaved(self, dialogs::Pending::Quit);
    }

    /// Sets the hint bar's text for this frame, for the control under the pointer.
    pub fn hint(&mut self, response: &egui::Response, text: impl Into<String>) {
        if response.hovered() {
            self.view.hint = Some(text.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_treats_a_hair_below_a_line_as_on_it() {
        let project = Project::new(120.0).unwrap();
        let zoom = 72.0f64 / 3840.0;
        assert_eq!(Snap::Beat.floor(&project, 36.0 / zoom), 1920);
        assert_eq!(Snap::Beat.floor(&project, 1919.0), 960);
        assert_eq!(Snap::Bar.round(&project, 2000.0), 3840);
        assert_eq!(Snap::None.floor(&project, 123.7), 123);
    }
}
