//! Autosave and crash recovery (project.md section 18.1).
//!
//! Every few minutes, a project with unsaved changes is copied to `<data>/Autosave/`. While Bodo runs, a lock file
//! sits in the data folder; finding it at startup means Bodo didn't close properly, so it offers the autosaves.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::BodoApp;

/// When the last autosave happened.
#[derive(Debug, Default)]
pub(crate) struct Autosave {
    last: Option<Instant>,
}

/// The folder autosaves go to, inside Bodo's data folder.
fn folder(data: &Path) -> PathBuf {
    data.join("Autosave")
}

fn lock_path(data: &Path) -> PathBuf {
    data.join("running.lock")
}

/// Marks Bodo as running in its data folder `data`, and returns the autosaves to offer if it didn't close properly
/// last time, newest first.
pub fn start_session(data: &Path) -> Vec<PathBuf> {
    let lock = lock_path(data);
    let crashed = lock.exists();
    if let Some(dir) = lock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(error) = std::fs::write(&lock, std::process::id().to_string()) {
        tracing::warn!(%error, "could not write the running lock");
    }
    if !crashed {
        return Vec::new();
    }
    let dir = folder(data);
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "bodo"))
        .filter_map(|p| p.metadata().and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .collect();
    found.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    found.into_iter().map(|(_, p)| p).take(5).collect()
}

/// Marks Bodo as closed properly, and drops the autosave of a project with nothing unsaved.
pub fn end_session(app: &BodoApp) {
    if !app.session.is_dirty()
        && let Some(path) = autosave_path(app)
    {
        let _ = std::fs::remove_file(path);
    }
    if let Some(data) = &app.data_dir {
        let _ = std::fs::remove_file(lock_path(data));
    }
}

/// Where the current project autosaves: named by its title and id, so each project has one autosave.
fn autosave_path(app: &BodoApp) -> Option<PathBuf> {
    let title: String =
        app.title().trim_end_matches(" *").chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    app.data_dir.as_deref().map(|data| folder(data).join(format!("{title} {}.bodo", app.session.project().id)))
}

/// Autosaves when it is time. Call once per frame.
pub(crate) fn tick(app: &mut BodoApp) {
    let minutes = app.prefs.autosave_minutes;
    if minutes == 0 || !app.session.is_dirty() {
        app.autosave.last = None;
        return;
    }
    let started = *app.autosave.last.get_or_insert_with(Instant::now);
    if started.elapsed() < Duration::from_secs(u64::from(minutes) * 60) {
        return;
    }
    app.autosave.last = Some(Instant::now());
    if let Some(path) = autosave_path(app) {
        let id = app.session.project().id.to_string();
        // An older autosave of this project under another title goes.
        if let Some(dir) = path.parent() {
            for old in std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()) {
                if old != path && old.file_name().is_some_and(|n| n.to_string_lossy().contains(&id)) {
                    let _ = std::fs::remove_file(old);
                }
            }
        }
        let options = bodo_io::SaveOptions { from: app.session.file().map(Path::to_path_buf), backups: 0 };
        match bodo_io::save(app.session.project(), &path, &options) {
            Ok(_) => app.set_status("Autosaved", false),
            Err(error) => tracing::warn!(%error, "autosave failed"),
        }
    }
}

/// Opens the autosave at `path` as an unsaved project, so saving asks where to.
pub(crate) fn recover(app: &mut BodoApp, path: &Path) {
    match bodo_io::load(path) {
        Ok(project) => {
            app.session.recover(project);
            app.set_status("Recovered; save it to keep it", false);
        }
        Err(error) => app.set_status(format!("Could not recover: {error}"), true),
    }
}
