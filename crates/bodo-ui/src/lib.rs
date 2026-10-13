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

#[macro_use]
pub mod i18n;
pub mod audio;
mod autosave;
mod browser;
pub mod channel_window;
pub mod control;
mod dialogs;
pub mod effect_editor;
pub mod layouts;
pub mod library;
pub mod menu;
mod midi_out;
pub mod mixer;
mod palette;
mod piano_roll;
mod playlist;
mod rack;
pub mod remote;
pub mod screenshot;
mod shortcut_editor;
pub mod shortcuts;
pub mod theme;
mod topbar;
mod typing;
pub mod windows;

use std::collections::BTreeMap;
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
use crate::windows::{Layout, Preset, SavedLayout, WindowKind};

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

/// The interface sizes the zoom keys step through, from 75% to 200% (project.md section 14.7).
pub const UI_SCALES: [f32; 9] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.4, 1.5, 1.75, 2.0];

/// `scale` kept between 75% and 200%; 100% for nonsense.
fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() { scale.clamp(0.75, 2.0) } else { 1.0 }
}

/// The screen width, in points at the system's own scaling, at which Auto sizes the interface at 100%.
pub const AUTO_BASE_WIDTH: f32 = 1440.0;

/// How strongly Auto follows the screen's width: the size grows as the width's ratio to [`AUTO_BASE_WIDTH`] to this
/// power. A wider screen shows more, and sits farther from the eyes, so its interface grows, but much less than in
/// proportion: in proportion, a 3360-point display made everything twice as big, far too big to work with.
pub const AUTO_GROWTH: f32 = 0.3;

/// The interface size Auto picks for a screen `width` points wide (at the system's own scaling), the way a responsive
/// web page sizes `rem` from the viewport: 100% at [`AUTO_BASE_WIDTH`], growing gently with the width (see
/// [`AUTO_GROWTH`]), kept between 75% and 200%, in steps of 5% so it never jitters.
pub fn auto_scale(width: f32) -> f32 {
    if !width.is_finite() || width <= 0.0 {
        return 1.0;
    }
    ((width / AUTO_BASE_WIDTH).powf(AUTO_GROWTH).clamp(0.75, 2.0) * 20.0).round() / 20.0
}

/// The width Auto sizes from: the monitor's, else the window's, in points at the system's own scaling. egui measures
/// both in points after the interface's own zoom, so the zoom is taken back out, or the size would chase itself.
fn screen_width(ctx: &egui::Context) -> f32 {
    let zoom = ctx.zoom_factor();
    let monitor = ctx.input(|i| i.viewport().monitor_size).map(|size| size.x).filter(|w| *w > 0.0);
    monitor.unwrap_or_else(|| ctx.content_rect().width()) * zoom
}

/// The next interface size up (`up`) or down from `scale` among [`UI_SCALES`], or `scale` at the end of the range.
pub fn step_scale(scale: f32, up: bool) -> f32 {
    let scale = clamp_scale(scale);
    if up {
        UI_SCALES.iter().copied().find(|s| *s > scale + 1e-3).unwrap_or(scale)
    } else {
        UI_SCALES.iter().rev().copied().find(|s| *s < scale - 1e-3).unwrap_or(scale)
    }
}

/// What the user chose, kept between runs. Window layouts belong here, not in projects.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// The theme.
    pub theme: ThemePref,
    /// Color-blind-safe colors for meters, clips, patterns and channels (project.md section 14.7).
    pub color_safe: bool,
    /// The interface's size, from 0.75 to 2 (75% to 200%), or `None` for Auto: from the screen's width.
    pub ui_scale: Option<f32>,
    /// The language of the interface.
    pub language: i18n::LanguagePref,
    /// The window layout.
    pub layout: Layout,
    /// The layouts the user saved, by name, besides the presets.
    pub layouts: Vec<SavedLayout>,
    /// The name of the layout chosen or saved last, a preset's or the user's, which the menus mark.
    pub layout_name: Option<String>,
    /// Keyboard shortcuts the user changed: binding id, such as `command:project.save`, to keys such as `Mod+S`, or
    /// an empty string for no shortcut. Everything else keeps its default.
    pub shortcuts: BTreeMap<String, String>,
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
            color_safe: false,
            ui_scale: None,
            language: i18n::LanguagePref::default(),
            layout: Layout::default(),
            layouts: Vec::new(),
            layout_name: Some(String::from(Preset::Default.label())),
            shortcuts: BTreeMap::new(),
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

    /// Its name in `ui.set_snap` and, translated, in the snap menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => i18n::en("None"),
            Self::SixthStep => i18n::en("1/6 step"),
            Self::QuarterStep => i18n::en("1/4 step"),
            Self::ThirdStep => i18n::en("1/3 step"),
            Self::HalfStep => i18n::en("1/2 step"),
            Self::Step => i18n::en("Step"),
            Self::SixthBeat => i18n::en("1/6 beat"),
            Self::ThirdBeat => i18n::en("1/3 beat"),
            Self::HalfBeat => i18n::en("1/2 beat"),
            Self::Beat => i18n::en("Beat"),
            Self::Bar => i18n::en("Bar"),
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
    /// The open channel windows, such as the Sampler.
    pub channel_windows: channel_window::ChannelWindows,
    /// The open effect editors.
    pub effect_editors: effect_editor::EffectEditors,
    /// Set by "Go to the menus" (F10): the first menu takes keyboard focus on the next frame.
    pub focus_menu: bool,
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
    /// Set once the user has confirmed quitting: the window's close requests then go through.
    closing: bool,
    autosave: autosave::Autosave,
    shown_theme: Option<(theme::Look, bool)>,
    /// The interface size showing: the preference, or what Auto picked.
    shown_scale: f32,
    status: Option<(String, bool, Instant)>,
    /// The commands that made recent undo steps, by the steps' labels, newest last, so a step whose label the engine
    /// built shows by its command's translated title.
    step_titles: std::collections::VecDeque<(String, &'static str)>,
    gestures: std::collections::HashMap<egui::Id, u64>,
    next_gesture: u64,
}

/// How many recent undo steps remember the command that made them, for [`BodoApp::step_name`].
const STEP_TITLES: usize = 256;

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
            closing: false,
            autosave: autosave::Autosave::default(),
            shown_theme: None,
            shown_scale: 1.0,
            status: None,
            step_titles: std::collections::VecDeque::new(),
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
            self.session.file().and_then(|f| f.file_stem()).map_or_else(|| String::from(i18n::t("Untitled")), |s| s.to_string_lossy().into_owned())
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
                    if let Some(spec) = self.session.registry().get(&call.id) {
                        if self.step_titles.len() >= STEP_TITLES {
                            self.step_titles.pop_front();
                        }
                        self.step_titles.push_back((undo.clone(), spec.title));
                    }
                    let shown = self.step_name(undo);
                    self.set_status(shown, false);
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

    /// Undo step `label` in the interface's language: its translation when the catalog has the label, or else, when
    /// the interface isn't in English, the translated title of the command that made it, such as "Örnekleyiciyi
    /// düzenle" for "Edit sampler Kick". Steps made elsewhere, such as by an agent, keep their English label.
    ///
    /// Complexity: O(s) in the remembered steps, at most a few hundred.
    pub fn step_name(&self, label: &str) -> String {
        if let Some(shown) = i18n::lookup(label) {
            return shown.to_owned();
        }
        if i18n::language() == i18n::Language::English {
            return label.to_owned();
        }
        self.step_titles.iter().rev().find(|(step, _)| step == label).map_or_else(|| label.to_owned(), |(_, title)| i18n::tr(title))
    }

    /// Shows `text` in the hint bar for a few seconds; `error` shows it in the error color.
    ///
    /// English text is translated here, as with [`Self::hint`]; messages from commands show as they come.
    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        let text = i18n::lookup(&text).map_or(text, str::to_owned);
        self.status = Some((text, error, Instant::now()));
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
            "ui.set_layout" => layouts::switch(self, &text("layout")),
            "ui.save_layout" => layouts::save(self, &text("name")),
            "ui.rename_layout" => layouts::rename(self, &text("layout"), &text("name")),
            "ui.delete_layout" => layouts::delete(self, &text("layout")),
            "ui.detach_window" => match WindowKind::from_id(&text("window")) {
                Some(WindowKind::ChannelRack) => Err(String::from("the channel rack stays in the main window")),
                Some(kind) => {
                    let mut state = self.prefs.layout.state(kind);
                    state.detached = params.get("detached").and_then(Value::as_bool).unwrap_or(!state.detached);
                    state.open = true;
                    state.minimized = false;
                    self.prefs.layout.windows.insert(kind, state);
                    Ok(json!({ "window": kind.id(), "detached": state.detached }))
                }
                None => Err(String::from("window must be playlist, pianoRoll or mixer")),
            },
            "ui.set_shortcut" => {
                let steal = params.get("replace").and_then(Value::as_bool).unwrap_or(false);
                shortcuts::assign(self, &text("binding"), &text("keys"), steal)
                    .map(|unbound| json!({ "binding": text("binding"), "unbound": unbound }))
            }
            "ui.reset_shortcuts" => {
                let binding = params.get("binding").and_then(Value::as_str);
                shortcuts::reset(self, binding);
                Ok(json!({ "reset": binding.unwrap_or("all") }))
            }
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
                None => Err(String::from("theme must be Dark, Light, High contrast or System")),
            },
            "ui.set_color_safe" => {
                self.prefs.color_safe = params.get("on").and_then(Value::as_bool).unwrap_or(!self.prefs.color_safe);
                Ok(json!({ "colorSafe": self.prefs.color_safe }))
            }
            "ui.set_ui_scale" => {
                let scale = match params.get("scale") {
                    Some(Value::Number(n)) => n.as_f64().map(|n| n as f32),
                    Some(Value::String(s)) => s.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|p| if p > 3.0 { p / 100.0 } else { p }),
                    _ => None,
                };
                let auto = params.get("scale").and_then(Value::as_str).is_some_and(|s| s.trim().eq_ignore_ascii_case("auto"));
                match scale.filter(|s| (0.75..=2.0).contains(s)) {
                    _ if auto => {
                        self.prefs.ui_scale = None;
                        Ok(json!({ "scale": "auto", "now": self.shown_scale }))
                    }
                    Some(scale) => {
                        self.prefs.ui_scale = Some(scale);
                        Ok(json!({ "scale": scale }))
                    }
                    None => Err(String::from("scale must be \"auto\" or from 0.75 to 2 (75% to 200%)")),
                }
            }
            "ui.zoom_in" | "ui.zoom_out" | "ui.zoom_reset" => {
                // Larger and smaller step from the size showing now, Auto's too; reset goes back to Auto.
                self.prefs.ui_scale = match id {
                    "ui.zoom_reset" => None,
                    _ => Some(step_scale(self.shown_scale, id == "ui.zoom_in")),
                };
                Ok(json!({ "scale": self.prefs.ui_scale.map_or_else(|| json!("auto"), |s| json!(s)) }))
            }
            "ui.set_language" => match i18n::LanguagePref::from_id(&text("language")) {
                Some(language) => {
                    self.prefs.language = language;
                    i18n::set_language(language.resolve());
                    Ok(json!({ "language": language.id() }))
                }
                None => Err(String::from("language must be system, en or tr")),
            },
            "ui.next_window" => {
                let kind = windows::cycle(self, self.remote.ctx().as_ref());
                Ok(json!({ "window": kind.map(|k| k.id()) }))
            }
            "ui.focus_menu" => {
                self.view.focus_menu = true;
                Ok(json!({ "menu": "File" }))
            }
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
            "ui.open_channel_window" => {
                // Selecting the channel resolves the reference the way every command does.
                if let Some(channel) = params.get("channel").filter(|c| !c.is_null())
                    && let Err(e) = self.session.execute("edit.select", json!({ "channel": channel }))
                {
                    return Some(Err(e.to_string()));
                }
                match self.session.editor().channel {
                    Some(channel) => {
                        let ctx = self.remote.ctx();
                        channel_window::open(self, ctx.as_ref(), channel);
                        Ok(json!({ "channel": channel, "open": true }))
                    }
                    None => Err(String::from("there is no channel to open")),
                }
            }
            "ui.palette" => {
                self.view.palette = Some(palette::PaletteState::default());
                Ok(json!({ "palette": true }))
            }
            "ui.export" => {
                let what = match text("what").as_str() {
                    "" | "mix" => Some(dialogs::ExportWhat::Mix),
                    "stems" => Some(dialogs::ExportWhat::Stems),
                    "both" => Some(dialogs::ExportWhat::Both),
                    _ => None,
                };
                match what {
                    Some(what) => {
                        self.view.dialogs.export = Some(dialogs::Export::new(self, what));
                        Ok(json!({ "export": true }))
                    }
                    None => Err(String::from("what must be mix, stems or both")),
                }
            }
            "ui.open_effect_editor" => {
                let number = params.get("insert").and_then(Value::as_u64).map_or(0, |n| n as usize);
                let slot = params.get("slot").and_then(Value::as_u64).map_or(1, |n| n as usize).max(1);
                let project = Arc::clone(self.session.project());
                match mixer::insert_at(&project, number).filter(|i| i.slots.get(slot - 1).is_some_and(|s| s.plugin.is_some())) {
                    Some(insert) => {
                        let ctx = self.remote.ctx();
                        effect_editor::open(self, ctx.as_ref(), insert.id, slot - 1);
                        Ok(json!({ "insert": number, "slot": slot }))
                    }
                    None => Err(String::from("there is no effect in that slot")),
                }
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
            (
                "ui.set_layout",
                "Switch layout",
                "Arranges the windows: {\"layout\": \"Default\" | \"Compose\" | \"Arrange\" | \"Mix\" | \"Master\"}, or a layout the user saved by its name.",
            ),
            ("ui.save_layout", "Save layout", "Saves the windows as they are now as a layout: {\"name\": \"Two screens\"}."),
            ("ui.rename_layout", "Rename layout", "Renames a saved layout: {\"layout\": \"Two screens\", \"name\": \"Studio\"}."),
            ("ui.delete_layout", "Delete layout", "Deletes a saved layout: {\"layout\": \"Studio\"}."),
            (
                "ui.detach_window",
                "Detach window",
                "Moves a window into its own OS window, or back: {\"window\": \"mixer\", \"detached\": true}. The channel rack stays in.",
            ),
            (
                "ui.set_shortcut",
                "Set shortcut",
                "Gives a shortcut new keys: {\"binding\": \"command:project.save\", \"keys\": \"Mod+Shift+S\"}; \"\" removes it, and \"replace\": true takes the keys from another.",
            ),
            ("ui.reset_shortcuts", "Reset shortcuts", "Puts a shortcut back to Bodo's keys, {\"binding\": \"tool:draw\"}, or every one."),
            (
                "ui.set_snap",
                "Set snap",
                "Sets the editors' snap: {\"snap\": \"None\" | \"1/4 step\" | \"Step\" | \"1/2 beat\" | \"Beat\" | \"Bar\" ...}.",
            ),
            ("ui.set_theme", "Set theme", "Sets the theme: {\"theme\": \"Dark\" | \"Light\" | \"High contrast\" | \"System\"}."),
            (
                "ui.set_color_safe",
                "Color-blind-safe colors",
                "Turns the color-blind-safe meter and clip colors on or off: {\"on\": true}, or toggles them.",
            ),
            (
                "ui.set_ui_scale",
                "Set interface size",
                "Sets the interface's size: {\"scale\": \"auto\"} to follow the screen's width, or {\"scale\": 1.25} or \"125%\", from 75% to 200%.",
            ),
            ("ui.zoom_in", "Make the interface larger", "Makes the interface one step larger, up to 200%."),
            ("ui.zoom_out", "Make the interface smaller", "Makes the interface one step smaller, down to 75%."),
            ("ui.zoom_reset", "Automatic interface size", "Puts the interface size back to Auto, which follows the screen's width."),
            ("ui.set_language", "Set language", "Sets the interface's language: {\"language\": \"en\" | \"tr\" | \"system\"}."),
            ("ui.next_window", "Next window", "Brings the next open window to the front and moves keyboard focus into it."),
            ("ui.focus_menu", "Go to the menus", "Moves keyboard focus to the File menu; the arrow keys and Tab move on from there."),
            (
                "ui.set_tool",
                "Set tool",
                "Sets the tool of the editor used last, or of {\"window\": \"playlist\" or \"pianoRoll\"}: draw, paint, delete, mute, slip (playlist only), slice, select, zoom or playback.",
            ),
            ("ui.open_piano_roll", "Open piano roll", "Opens the piano roll, optionally for a channel: {\"channel\": \"Kick\"}."),
            (
                "ui.open_channel_window",
                "Open channel window",
                "Opens a channel's window, such as the Sampler, for the selected channel or {\"channel\": \"Kick\"}.",
            ),
            ("ui.palette", "Command palette", "Opens the command palette."),
            ("ui.export", "Export", "Opens the export dialog for the mix, the stems or both: {\"what\": \"mix\" | \"stems\" | \"both\"}."),
            (
                "ui.open_effect_editor",
                "Open effect editor",
                "Opens the editor of the effect in an FX slot: {\"insert\": 1, \"slot\": 2}, the master as insert 0.",
            ),
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
        let look = (self.prefs.theme.resolve(ctx.system_theme()), self.prefs.color_safe);
        if self.shown_theme != Some(look) {
            theme::apply(ctx, look.0, look.1);
            self.shown_theme = Some(look);
        }
        let language = self.prefs.language.resolve();
        if i18n::language() != language {
            i18n::set_language(language);
        }
        // The interface's size: the preference, kept between 75% and 200%. egui's own zoom keys are off, since
        // Bodo's zoom shortcuts can be changed like any other.
        self.prefs.ui_scale = self.prefs.ui_scale.map(clamp_scale);
        if ctx.options(|o| o.zoom_with_keyboard) {
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
        }
        self.shown_scale = self.prefs.ui_scale.unwrap_or_else(|| auto_scale(screen_width(ctx)));
        if (ctx.zoom_factor() - self.shown_scale).abs() > 1e-4 {
            ctx.set_zoom_factor(self.shown_scale);
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
            channel_window::show_all(self, ui.ctx(), workspace, &project);
            effect_editor::show_all(self, ui.ctx(), workspace, &project);
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

    /// Whether the window may close now that it was asked to. With unsaved changes, the first request opens the save
    /// dialog and is refused; once the user has answered it, by saving or by choosing not to, close requests go through.
    pub fn allow_close(&mut self) -> bool {
        if self.closing || self.quit_after_shot || !self.session.is_dirty() {
            return true;
        }
        self.request_quit();
        false
    }

    /// Takes a quit the user confirmed, if there is one: the app should close its window now, and later close
    /// requests go through.
    pub fn take_quit(&mut self) -> bool {
        if std::mem::take(&mut self.quit_requested) {
            self.closing = true;
            return true;
        }
        false
    }

    /// The interface size showing now, from 0.75 to 2: the preference, or what Auto picked for the screen.
    pub fn ui_scale(&self) -> f32 {
        self.shown_scale
    }

    /// Sets the hint bar's text for this frame, for the control under the pointer.
    ///
    /// `text` is English and is translated here: pass a literal, or text from [`tf!`] when it has values in it.
    pub fn hint(&mut self, response: &egui::Response, text: impl Into<String>) {
        if response.hovered() || response.has_focus() {
            let text = text.into();
            self.view.hint = Some(i18n::lookup(&text).map_or(text, str::to_owned));
        }
    }
}

impl windows::WindowHost for BodoApp {
    fn detached_keys(&mut self, ctx: &egui::Context) {
        typing::handle(self, ctx);
        shortcuts::handle(self, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_sizes_the_interface_from_the_screen_width() {
        assert_eq!(auto_scale(1440.0), 1.0, "the base width");
        assert_eq!(auto_scale(1512.0), 1.0, "a 14-inch laptop");
        assert_eq!(auto_scale(1920.0), 1.1, "a full HD monitor");
        assert_eq!(auto_scale(2560.0), 1.2, "a 27-inch monitor");
        assert_eq!(auto_scale(3360.0), 1.3, "a 32-inch 6K display");
        assert_eq!(auto_scale(3840.0), 1.35, "a 4K monitor at its own size");
        assert_eq!(auto_scale(800.0), 0.85, "a small window");
        assert_eq!(auto_scale(0.0), 1.0, "no screen size gives 100%");
        for width in [1000.0, 1333.0, 1777.7, 2222.2] {
            let scale = auto_scale(width);
            assert!(((scale * 20.0).round() - scale * 20.0).abs() < 1e-4, "{scale} is a 5% step");
        }
    }

    #[test]
    fn the_interface_size_preference_round_trips() {
        for size in [None, Some(1.25)] {
            let prefs = Prefs { ui_scale: size, ..Prefs::default() };
            let back: Prefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
            assert_eq!(back.ui_scale, size);
        }
        let old: Prefs = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(old.ui_scale, None, "preferences from before the setting are on Auto");
        assert_eq!(step_scale(1.05, true), 1.1, "larger steps up from Auto's size");
        assert_eq!(step_scale(1.05, false), 1.0);
        assert_eq!(step_scale(2.0, true), 2.0, "the top of the range stays");
    }

    #[test]
    fn rem_sizes_are_whole_points_from_one_base() {
        assert_eq!(theme::rem(1.0), theme::REM);
        assert_eq!(theme::size::BODY, theme::REM);
        const { assert!(theme::size::SMALL >= 12.5, "small text stays readable") };
        assert_eq!(theme::rem(0.48).fract(), 0.0, "whole points, so edges stay on pixels");
    }

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
