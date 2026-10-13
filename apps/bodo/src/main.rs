//! Bodo, the digital audio workstation.
//!
//! Usage: `bodo [options] [project.bodo]`. The window is an eframe app around [`bodo_ui::BodoApp`]: preferences live in
//! eframe's storage, the control channel lets agents and tests drive the app, and `--screenshot` saves a picture of
//! the window and quits.

#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr, reason = "a command-line program reports to the console")]

use std::path::PathBuf;
use std::process::ExitCode;

use bodo_engine::{Env, Session, locate_factory};
use bodo_ui::{BodoApp, PREFS_KEY, Prefs, Services};

const USAGE: &str = "\
usage: bodo [options] [project.bodo]

options:
      --screenshot <png>   save a picture of the window, then quit
      --light              use the light theme for this run
      --fresh              start with default preferences and don't save them
      --no-audio           don't open the audio device
      --no-control         don't let agents connect (bodo-cli mcp --connect)
      --control <port>     let agents connect on this port instead of the default
      --size <w>x<h>       the window's size in points, such as 1600x1000
  -V, --version            print the version
  -h, --help               print this help
";

/// Command-line choices.
#[derive(Debug, Default)]
struct Args {
    project: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    light: bool,
    fresh: bool,
    no_audio: bool,
    no_control: bool,
    control: Option<u16>,
    size: Option<[f32; 2]>,
}

struct App {
    app: BodoApp,
    title: String,
    save_prefs: bool,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.app.quit_requested && self.app.session.is_dirty() && !self.app.quit_after_shot {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.app.request_quit();
        }
        if std::mem::take(&mut self.app.quit_requested) {
            bodo_ui::end_session(&self.app);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let title = format!("{} - Bodo", self.app.title());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.remote.feed_input(raw);
        if self.app.quit_after_shot {
            raw.events.retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.save_prefs && !self.app.quit_after_shot {
            match serde_json::to_string(&self.app.prefs) {
                Ok(text) => storage.set_string(PREFS_KEY, text),
                Err(error) => tracing::warn!(%error, "could not save preferences"),
            }
        }
    }

    fn on_exit(&mut self) {
        bodo_ui::end_session(&self.app);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// The control server, as the interface sees it.
struct ControlServer(bodo_control::Server);

impl bodo_ui::remote::ControlSource for ControlServer {
    fn next(&self) -> Option<bodo_ui::remote::ControlRequest> {
        self.0.next().map(|request| {
            let (method, params) = (request.method.clone(), request.params.clone());
            bodo_ui::remote::ControlRequest::new(method, params, move |result| request.reply(result))
        })
    }

    fn set_waker(&self, wake: Box<dyn Fn() + Send>) {
        self.0.set_waker(wake);
    }

    fn addr(&self) -> std::net::SocketAddr {
        self.0.addr()
    }
}

/// Starts the control channel on `port`, or Bodo's default port, and publishes its token for `bodo-cli mcp --connect`.
fn start_control(port: Option<u16>) -> anyhow::Result<ControlServer> {
    let server = match port {
        Some(port) => bodo_control::Server::start(port)?,
        None => bodo_control::Server::start_default()?,
    };
    server.publish()?;
    Ok(ControlServer(server))
}

/// A save dialog for `suggestion`: a file name, or a path whose folder the dialog opens in.
fn save_dialog(suggestion: &str) -> rfd::FileDialog {
    let path = std::path::Path::new(suggestion);
    let name = path.file_name().map_or_else(|| suggestion.to_owned(), |n| n.to_string_lossy().into_owned());
    let dialog = rfd::FileDialog::new().set_file_name(name);
    match path.parent().filter(|p| !p.as_os_str().is_empty() && p.is_dir()) {
        Some(folder) => dialog.set_directory(folder),
        None => dialog,
    }
}

fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|| rfd::FileDialog::new().add_filter("Bodo project", &["bodo"]).pick_file())),
        pick_save: Some(Box::new(|name: &str| save_dialog(name).add_filter("Bodo project", &["bodo"]).save_file())),
        pick_samples: Some(Box::new(|| {
            rfd::FileDialog::new().add_filter("Audio", &bodo_engine::samples::AUDIO_EXTENSIONS).pick_files().unwrap_or_default()
        })),
        pick_folder: Some(Box::new(|| rfd::FileDialog::new().pick_folder())),
        pick_midi: Some(Box::new(|| rfd::FileDialog::new().add_filter("MIDI file", &["mid", "midi"]).pick_file())),
        pick_midi_save: Some(Box::new(|name: &str| save_dialog(name).add_filter("MIDI file", &["mid"]).save_file())),
        memory: Some(bodo_ui::resident_memory),
    }
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("bodo {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "--light" => args.light = true,
            "--fresh" => args.fresh = true,
            "--no-audio" => args.no_audio = true,
            "--no-control" => args.no_control = true,
            "--control" => args.control = Some(it.next().and_then(|p| p.parse().ok()).ok_or("--control needs a port number")?),
            "--screenshot" => args.screenshot = Some(PathBuf::from(it.next().ok_or("--screenshot needs a file name")?)),
            "--size" => {
                let text = it.next().ok_or("--size needs a size such as 1600x1000")?;
                let (w, h) = text.split_once('x').ok_or("--size needs a size such as 1600x1000")?;
                args.size = Some([w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?]);
            }
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            path => args.project = Some(PathBuf::from(path)),
        }
    }
    Ok(Some(args))
}

fn main() -> ExitCode {
    let level = if std::env::var_os("BODO_LOG").is_some() { tracing::Level::INFO } else { tracing::Level::WARN };
    tracing_subscriber::fmt().with_max_level(level).with_writer(std::io::stderr).init();
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    let size = args.size.unwrap_or([1600.0, 1000.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Bodo")
            .with_inner_size(size)
            .with_min_inner_size([1024.0, 640.0])
            .with_drag_and_drop(true)
            .with_app_id("dev.bodo.Bodo"),
        persist_window: !args.fresh && args.screenshot.is_none(),
        ..Default::default()
    };
    eframe::run_native(
        "Bodo",
        options,
        Box::new(move |cc| {
            let mut prefs = Prefs::default();
            if !args.fresh
                && args.screenshot.is_none()
                && let Some(text) = cc.storage.and_then(|s| s.get_string(PREFS_KEY))
            {
                match serde_json::from_str(&text) {
                    Ok(saved) => prefs = saved,
                    Err(error) => tracing::warn!(%error, "ignoring unreadable preferences"),
                }
            }
            if args.light {
                prefs.theme = bodo_ui::theme::ThemePref::Light;
            }
            let factory = locate_factory();
            if prefs.user_library.is_none() {
                prefs.user_library = bodo_ui::library::default_root();
            }
            if let Some(root) = &prefs.user_library
                && let Err(error) = bodo_ui::library::prepare(root, factory.as_deref())
            {
                tracing::warn!(%error, "the user library is not ready");
            }
            let env = Env { factory, library: prefs.library.clone(), user: prefs.user_library.clone() };
            let session = Session::new(env);
            let screenshot = args.screenshot.is_some();
            let mut app = BodoApp::new(session, prefs, services(), !args.no_audio && !screenshot);
            if let Some(project) = &args.project {
                let _ = app.run("project.open", serde_json::json!({ "path": project.display().to_string(), "discardChanges": true }));
            }
            if !screenshot {
                app.data_dir = bodo_control::data_dir();
                if let Some(data) = &app.data_dir {
                    app.view.dialogs.recovery = bodo_ui::start_session(data, app.prefs.user_library.as_deref());
                }
            }
            // `bodo-cli mcp --connect`: MCP tools run on this app's own session, so agent edits share its undo history.
            app.remote.extension =
                Some(Box::new(|session, method, params| (method == bodo_mcp::connect::METHOD).then(|| bodo_mcp::connect::answer(session, params))));
            if !args.no_control && !screenshot {
                match start_control(args.control) {
                    Ok(server) => {
                        tracing::info!(addr = %server.0.addr(), "agents can connect");
                        app.remote.attach(Box::new(server));
                    }
                    Err(error) => tracing::warn!(%error, "the control channel could not start"),
                }
            }
            if let Some(path) = &args.screenshot {
                app.shots.request(path.clone());
                app.quit_after_shot = true;
            }
            Ok(Box::new(App { app, title: String::new(), save_prefs: !args.fresh }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
