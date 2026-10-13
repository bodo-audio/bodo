//! The two containers of a project (project.md section 18.1, ADR 0007).
//!
//! A `.bodo` file is a zip archive, and a `.bodoproj` folder is the same layout unpacked:
//!
//! - `project.json`: the project ([`crate::to_json`]);
//! - `plugins/<instance>.state`: the saved state of each plugin instance that has one;
//! - anything else, such as `samples/`, `recorded/` or `thumbnail.png`, which is kept as it is.
//!
//! Saving is atomic and deterministic: the same project always gives the same bytes.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_model::channel::{ChannelKind, PluginRef};
use bodo_model::mixer::MixerInsert;
use bodo_model::{ChannelId, InsertId, PluginInstanceId, Project};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipArchive, ZipWriter};

use crate::atomic::{self, TempGuard, is_temp_name, replace, sync_folder, temp_path};
use crate::backup::back_up;
use crate::error::IoError;
use crate::json::{from_json, to_json};
use crate::paths::{absolute, project_folder, rebase};

/// How many backups of each project a save keeps by default.
pub const DEFAULT_BACKUPS: usize = 10;

/// The name of the project entry.
const PROJECT_JSON: &str = "project.json";
/// The folder of plugin state entries.
const PLUGINS: &str = "plugins";
/// The extension of plugin state entries.
const STATE: &str = ".state";

/// How to save a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveOptions {
    /// Where the project was loaded from or last saved to, or `None` for a project never saved. Relative sample
    /// paths start at that project's folder (the current directory for `None`) and are rebased to the new one, and
    /// entries the project doesn't own, such as embedded samples, are carried over from there.
    pub from: Option<PathBuf>,
    /// How many backups of the project to keep in its `Backup` folder; zero keeps none.
    pub backups: usize,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self { from: None, backups: DEFAULT_BACKUPS }
    }
}

/// The two ways a project is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    /// A `.bodo` zip file.
    Zip,
    /// A `.bodoproj` folder.
    Folder,
}

impl Container {
    /// The container a save to `path` writes: a folder for a `.bodoproj` path, a zip file otherwise.
    fn for_save(path: &Path) -> Self {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bodoproj")) { Self::Folder } else { Self::Zip }
    }

    /// The container at `path`.
    fn for_load(path: &Path) -> Self {
        if path.is_dir() { Self::Folder } else { Self::Zip }
    }
}

/// Plugin states read from a container, by instance.
type States = HashMap<PluginInstanceId, Arc<[u8]>>;

/// An entry the project doesn't own, carried from one save to the next.
#[derive(Clone, Debug)]
struct Extra {
    /// Its name inside the container, with forward slashes.
    name: String,
    data: Vec<u8>,
}

/// Saves `project` to `path`: a `.bodoproj` folder when the path ends in `.bodoproj`, a `.bodo` zip file otherwise.
///
/// The save is atomic: a crash leaves the old project or the new one. When it replaces a project, the old one is
/// first copied to the `Backup` folder, keeping the newest [`SaveOptions::backups`]. Relative sample paths are
/// rebased to the new project folder. Returns the project exactly as [`load`] will read it back.
///
/// Refuses a project that breaks the model's invariants.
///
/// Complexity: O(N log N) in the size of the project, plus the size of the entries carried over and the backup.
pub fn save(project: &Project, path: &Path, options: &SaveOptions) -> Result<Project, IoError> {
    project.validate().map_err(|error| IoError::Invalid { path: Some(path.to_path_buf()), error })?;
    let from = options.from.as_deref();
    let saved = rebase(project, from.map(project_folder).as_deref(), &project_folder(path));
    let json = to_json(&saved)?;
    let states = plugin_states(&saved);
    let in_place = from.is_some_and(|from| absolute(from) == absolute(path));
    match Container::for_save(path) {
        Container::Zip => write_zip(path, &json, &states, &carried(from), options.backups)?,
        Container::Folder => {
            let extras = if in_place { Vec::new() } else { carried(from) };
            write_folder(path, &json, &states, &extras, options.backups)?;
        }
    }
    Ok(saved)
}

/// Loads the project at `path`, a `.bodo` file or a `.bodoproj` folder: migrates older versions, refuses projects
/// that break the model's invariants, and attaches each plugin's saved state.
///
/// Complexity: O(N log N) in the size of the project.
pub fn load(path: &Path) -> Result<Project, IoError> {
    let (text, states) = match Container::for_load(path) {
        Container::Folder => read_folder(path)?,
        Container::Zip => read_zip(path)?,
    };
    let project = from_json(&text).map_err(|err| err.at(path))?;
    Ok(attach_states(project, &states))
}

/// Converts the project at `from` into the container `to` names: packs a `.bodoproj` folder into a `.bodo` file,
/// or unpacks a `.bodo` file into a `.bodoproj` folder. Every entry is kept. `project.json` is written fresh, so an
/// older version is migrated and relative sample paths are rebased when `to` is in another folder.
///
/// Never overwrites: fails when `to` exists.
///
/// Complexity: O(size of the project and its entries).
pub fn convert(from: &Path, to: &Path) -> Result<(), IoError> {
    if to.exists() {
        return Err(IoError::Exists { path: to.to_path_buf() });
    }
    let project = load(from)?;
    save(&project, to, &SaveOptions { from: Some(from.to_path_buf()), backups: 0 })?;
    Ok(())
}

/// The zip options of every entry: fixed time, permissions and system, so the bytes never depend on when or
/// where the file was saved.
fn entry_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
        .system(System::Unix)
}

fn write_zip(path: &Path, json: &str, states: &BTreeMap<String, Arc<[u8]>>, extras: &[Extra], keep: usize) -> Result<(), IoError> {
    create_parent(path)?;
    let temp = temp_path(path);
    let guard = TempGuard::new(&temp);
    let file = File::create(&temp).map_err(|err| IoError::io("create", &temp, err))?;
    let mut zip = ZipWriter::new(file);
    let mut add = |name: &str, data: &[u8]| -> Result<(), IoError> {
        zip.start_file(name, entry_options()).map_err(|err| IoError::zip(path, err))?;
        zip.write_all(data).map_err(|err| IoError::io("write", &temp, err))
    };
    add(PROJECT_JSON, json.as_bytes())?;
    for (name, data) in states {
        add(name, data)?;
    }
    for extra in extras {
        add(&extra.name, &extra.data)?;
    }
    let file = zip.finish().map_err(|err| IoError::zip(path, err))?;
    file.sync_all().map_err(|err| IoError::io("flush", &temp, err))?;
    drop(file);
    back_up(path, keep)?;
    replace(&temp, path)?;
    guard.keep();
    Ok(())
}

fn write_folder(path: &Path, json: &str, states: &BTreeMap<String, Arc<[u8]>>, extras: &[Extra], keep: usize) -> Result<(), IoError> {
    if path.join(PROJECT_JSON).is_file() {
        back_up(path, keep)?;
    }
    if path.exists() {
        // Update in place, one atomic file at a time, with `project.json` last.
        fill_folder(path, json, states, extras)?;
        remove_stale_states(path, states)?;
        return sync_folder(path);
    }
    // A new folder is built next to its place, then renamed there in one step.
    create_parent(path)?;
    let temp = temp_path(path);
    let guard = TempGuard::new(&temp);
    std::fs::create_dir(&temp).map_err(|err| IoError::io("create", &temp, err))?;
    fill_folder(&temp, json, states, extras)?;
    replace(&temp, path)?;
    guard.keep();
    Ok(())
}

/// Writes the entries into the folder `root`, with `project.json` last.
fn fill_folder(root: &Path, json: &str, states: &BTreeMap<String, Arc<[u8]>>, extras: &[Extra]) -> Result<(), IoError> {
    for extra in extras {
        write_entry(root, &extra.name, &extra.data)?;
    }
    for (name, data) in states {
        write_entry(root, name, data)?;
    }
    write_entry(root, PROJECT_JSON, json.as_bytes())?;
    sync_folder(root)
}

/// Writes one entry, named with forward slashes, below `root`, creating its folders.
fn write_entry(root: &Path, name: &str, data: &[u8]) -> Result<(), IoError> {
    let Some(relative) = safe_relative(name) else {
        tracing::warn!(entry = name, "skipping an entry whose name leaves the project");
        return Ok(());
    };
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| IoError::io("create", parent, err))?;
    }
    atomic::write(&target, data)
}

/// Deletes plugin state files the project no longer has.
fn remove_stale_states(root: &Path, states: &BTreeMap<String, Arc<[u8]>>) -> Result<(), IoError> {
    let folder = root.join(PLUGINS);
    let Ok(entries) = std::fs::read_dir(&folder) else { return Ok(()) };
    for entry in entries {
        let entry = entry.map_err(|err| IoError::io("list", &folder, err))?;
        let name = format!("{PLUGINS}/{}", entry.file_name().to_string_lossy());
        if state_instance(&name).is_some() && !states.contains_key(&name) {
            let path = entry.path();
            std::fs::remove_file(&path).map_err(|err| IoError::io("remove", &path, err))?;
        }
    }
    Ok(())
}

fn create_parent(path: &Path) -> Result<(), IoError> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => std::fs::create_dir_all(parent).map_err(|err| IoError::io("create", parent, err)),
        _ => Ok(()),
    }
}

/// `name` as a relative path, or `None` when it is empty, absolute, or climbs out with `..`.
fn safe_relative(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') || part.contains(':') {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// The plugin instance a container entry holds the state of, if it is a plugin state entry.
fn state_instance(name: &str) -> Option<PluginInstanceId> {
    name.strip_prefix(PLUGINS)?.strip_prefix('/')?.strip_suffix(STATE)?.parse().ok()
}

/// Every plugin of the project: instrument channels, then the FX slots of the master and the inserts.
fn plugins(project: &Project) -> impl Iterator<Item = &PluginRef> {
    let channels = project.channels.values().filter_map(|channel| match &channel.kind {
        ChannelKind::Instrument { plugin } => Some(plugin),
        _ => None,
    });
    let strips = std::iter::once(&project.mixer.master).chain(project.mixer.inserts.values());
    channels.chain(strips.flat_map(|insert| insert.slots.iter().filter_map(|slot| slot.plugin.as_ref())))
}

/// The state entries of the project's plugins, by entry name. Plugins without a state have no entry.
fn plugin_states(project: &Project) -> BTreeMap<String, Arc<[u8]>> {
    plugins(project)
        .filter(|plugin| !plugin.state.is_empty())
        .map(|plugin| (format!("{PLUGINS}/{}{STATE}", plugin.instance), Arc::clone(&plugin.state)))
        .collect()
}

/// `project` with each plugin's state taken from `states`.
fn attach_states(mut project: Project, states: &States) -> Project {
    if states.is_empty() {
        return project;
    }
    let channels: Vec<ChannelId> = project
        .channels
        .iter()
        .filter(|(_, channel)| matches!(&channel.kind, ChannelKind::Instrument { plugin } if states.contains_key(&plugin.instance)))
        .map(|(id, _)| *id)
        .collect();
    for id in channels {
        if let Some(channel) = project.channels.get_mut(&id)
            && let ChannelKind::Instrument { plugin } = &mut Arc::make_mut(channel).kind
            && let Some(state) = states.get(&plugin.instance)
        {
            plugin.state = Arc::clone(state);
        }
    }
    let needs = |insert: &MixerInsert| insert.slots.iter().any(|slot| slot.plugin.as_ref().is_some_and(|p| states.contains_key(&p.instance)));
    let fill = |insert: &mut MixerInsert| {
        for plugin in insert.slots.iter_mut().filter_map(|slot| slot.plugin.as_mut()) {
            if let Some(state) = states.get(&plugin.instance) {
                plugin.state = Arc::clone(state);
            }
        }
    };
    if needs(&project.mixer.master) || project.mixer.inserts.values().any(|insert| needs(insert)) {
        let mixer = Arc::make_mut(&mut project.mixer);
        if needs(&mixer.master) {
            fill(Arc::make_mut(&mut mixer.master));
        }
        let inserts: Vec<InsertId> = mixer.inserts.iter().filter(|(_, insert)| needs(insert)).map(|(id, _)| *id).collect();
        for id in inserts {
            if let Some(insert) = mixer.inserts.get_mut(&id) {
                fill(Arc::make_mut(insert));
            }
        }
    }
    project
}

/// Reads `project.json` and the plugin states of a `.bodoproj` folder.
fn read_folder(root: &Path) -> Result<(String, States), IoError> {
    let json = root.join(PROJECT_JSON);
    let text = match std::fs::read_to_string(&json) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(IoError::Missing { path: root.to_path_buf(), entry: PROJECT_JSON }),
        Err(err) => return Err(IoError::io("read", &json, err)),
    };
    let mut states = HashMap::new();
    let folder = root.join(PLUGINS);
    if let Ok(entries) = std::fs::read_dir(&folder) {
        for entry in entries {
            let entry = entry.map_err(|err| IoError::io("list", &folder, err))?;
            let name = format!("{PLUGINS}/{}", entry.file_name().to_string_lossy());
            if let Some(instance) = state_instance(&name) {
                let path = entry.path();
                let data = std::fs::read(&path).map_err(|err| IoError::io("read", &path, err))?;
                states.insert(instance, Arc::from(data));
            }
        }
    }
    Ok((text, states))
}

/// Reads `project.json` and the plugin states of a `.bodo` file.
fn read_zip(path: &Path) -> Result<(String, States), IoError> {
    let file = File::open(path).map_err(|err| IoError::io("open", path, err))?;
    let mut zip = ZipArchive::new(file).map_err(|err| IoError::zip(path, err))?;
    let mut text = String::new();
    match zip.by_name(PROJECT_JSON) {
        Ok(mut entry) => entry.read_to_string(&mut text).map_err(|err| IoError::io("read", path, err))?,
        Err(zip::result::ZipError::FileNotFound) => return Err(IoError::Missing { path: path.to_path_buf(), entry: PROJECT_JSON }),
        Err(err) => return Err(IoError::zip(path, err)),
    };
    let mut states = HashMap::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|err| IoError::zip(path, err))?;
        let Some(instance) = entry.name().ok().and_then(|name| state_instance(&name)) else { continue };
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|err| IoError::io("read", path, err))?;
        states.insert(instance, Arc::from(data));
    }
    Ok((text, states))
}

/// The entries of the project at `from` that it doesn't own: everything but `project.json` and plugin states,
/// sorted by name. A missing or unreadable project gives none, after a warning: the old file still has them.
fn carried(from: Option<&Path>) -> Vec<Extra> {
    let Some(from) = from.filter(|from| from.exists()) else { return Vec::new() };
    let read = match Container::for_load(from) {
        Container::Folder => folder_extras(from),
        Container::Zip => zip_extras(from),
    };
    match read {
        Ok(mut extras) => {
            extras.sort_by(|a, b| a.name.cmp(&b.name));
            extras
        }
        Err(err) => {
            tracing::warn!(%err, "could not carry the entries of the previous project file over");
            Vec::new()
        }
    }
}

/// Whether a container entry belongs to the project itself rather than being carried over.
fn is_owned(name: &str) -> bool {
    name == PROJECT_JSON || state_instance(name).is_some()
}

fn zip_extras(path: &Path) -> Result<Vec<Extra>, IoError> {
    let file = File::open(path).map_err(|err| IoError::io("open", path, err))?;
    let mut zip = ZipArchive::new(file).map_err(|err| IoError::zip(path, err))?;
    let mut extras = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|err| IoError::zip(path, err))?;
        let Ok(name) = entry.name().map(|name| name.into_owned()) else { continue };
        if entry.is_dir() || is_owned(&name) || safe_relative(&name).is_none() {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|err| IoError::io("read", path, err))?;
        extras.push(Extra { name, data });
    }
    Ok(extras)
}

fn folder_extras(root: &Path) -> Result<Vec<Extra>, IoError> {
    let mut extras = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        let entries = std::fs::read_dir(&folder).map_err(|err| IoError::io("list", &folder, err))?;
        for entry in entries {
            let entry = entry.map_err(|err| IoError::io("list", &folder, err))?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if is_temp_name(&file_name) {
                continue;
            }
            let name = format!("{prefix}{file_name}");
            let path = entry.path();
            if path.is_dir() {
                pending.push((path, format!("{name}/")));
            } else if !is_owned(&name) {
                let data = std::fs::read(&path).map_err(|err| IoError::io("read", &path, err))?;
                extras.push(Extra { name, data });
            }
        }
    }
    Ok(extras)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_must_stay_inside() {
        assert_eq!(safe_relative("samples/kick.wav"), Some(PathBuf::from("samples").join("kick.wav")));
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a//b", "c:\\x", "a\\b"] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
    }

    #[test]
    fn plugin_state_names_parse() {
        let id = PluginInstanceId::from_u128(7);
        assert_eq!(state_instance(&format!("plugins/{id}.state")), Some(id));
        assert_eq!(state_instance(&format!("plugins/x/{id}.state")), None);
        assert_eq!(state_instance("plugins/readme.txt"), None);
        assert!(is_owned("project.json") && !is_owned("thumbnail.png"));
    }
}
