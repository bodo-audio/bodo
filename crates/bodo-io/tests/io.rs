//! Saving and loading projects in both containers: round trips, determinism, rebasing, plugin states, refusals,
//! atomic replacement and backups.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_io::{FORMAT_VERSION, IoError, SaveOptions, backups, convert, from_json, hash_file, load, save, to_json};
use bodo_model::channel::ChannelKind;
use bodo_model::{Project, SampleId, SampleRef};
use common::{full, temp_dir, tree, with_states};

/// Options for saving a project that already lives at `path`, so its relative sample paths stay as they are.
fn in_place(path: &Path, backups: usize) -> SaveOptions {
    SaveOptions { from: Some(path.to_path_buf()), backups }
}

/// The names of the entries of a zip file, in order.
fn zip_names(path: &Path) -> Vec<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned()).collect()
}

fn zip_entry(path: &Path, name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut data = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
    data
}

#[test]
fn zip_projects_round_trip_and_save_deterministically() {
    let dir = temp_dir("zip");
    let project = with_states();
    let path = dir.join("Song.bodo");
    let saved = save(&project, &path, &in_place(&path, 0)).unwrap();
    assert_eq!(saved, project, "saving in place changes nothing");
    assert_eq!(load(&path).unwrap(), saved);

    let first = std::fs::read(&path).unwrap();
    save(&saved, &path, &in_place(&path, 0)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first, "saving again gives the same bytes");
    let other = dir.join("Other.bodo");
    save(&project, &other, &in_place(&path, 0)).unwrap();
    assert_eq!(std::fs::read(&other).unwrap(), first, "the name and time of a save don't change its bytes");

    let json = String::from_utf8(zip_entry(&path, "project.json")).unwrap();
    assert!(json.starts_with("{\n  \"format\": \"bodo\",\n  \"format_version\": 1,\n  \"id\": \"proj_"), "{}", &json[..80]);
    assert!(json.ends_with("}\n"));
    assert_eq!(json, to_json(&project).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plugin_states_live_outside_the_json() {
    let dir = temp_dir("states");
    let project = with_states();
    let path = dir.join("Song.bodo");
    save(&project, &path, &in_place(&path, 0)).unwrap();
    let names = zip_names(&path);
    let states: Vec<&String> = names.iter().filter(|n| n.starts_with("plugins/")).collect();
    assert_eq!(states.len(), 2, "the slot plugin without a state has no entry: {names:?}");
    assert_eq!(names[0], "project.json");
    let json = String::from_utf8(zip_entry(&path, "project.json")).unwrap();
    assert!(!json.contains("\"state\""), "states never go into the JSON");

    let loaded = load(&path).unwrap();
    let synth = loaded.rack_order[2];
    let ChannelKind::Instrument { plugin } = &loaded.channels[&synth].kind else { panic!("not an instrument") };
    assert_eq!(&*plugin.state, &[1u8, 2, 3, 0, 255]);
    assert_eq!(zip_entry(&path, &format!("plugins/{}.state", plugin.instance)), [1u8, 2, 3, 0, 255]);
    let master = loaded.mixer.master.slots[9].plugin.as_ref().unwrap();
    assert_eq!(&*master.state, b"master state");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn folder_projects_round_trip_and_convert_both_ways() {
    let dir = temp_dir("folder");
    let project = with_states();
    let folder = dir.join("Song.bodoproj");
    let saved = save(&project, &folder, &in_place(&folder, 0)).unwrap();
    assert_eq!(saved, project);
    assert_eq!(load(&folder).unwrap(), saved);
    let files = tree(&folder);
    assert!(files.contains(&String::from("project.json")), "{files:?}");
    assert_eq!(files.iter().filter(|f| f.ends_with(".state")).count(), 2, "{files:?}");
    let json = std::fs::read_to_string(folder.join("project.json")).unwrap();
    assert_eq!(json, to_json(&project).unwrap());

    // Entries the project doesn't own come along both ways.
    std::fs::write(folder.join("thumbnail.png"), b"png").unwrap();
    std::fs::create_dir_all(folder.join("samples")).unwrap();
    std::fs::write(folder.join("samples").join("kick.wav"), b"RIFF").unwrap();
    let packed = dir.join("Packed.bodo");
    convert(&folder, &packed).unwrap();
    let mut names = zip_names(&packed);
    names.sort();
    assert!(names.contains(&String::from("thumbnail.png")) && names.contains(&String::from("samples/kick.wav")), "{names:?}");
    assert_eq!(zip_entry(&packed, "samples/kick.wav"), b"RIFF");
    assert_eq!(load(&packed).unwrap(), project);

    let unpacked = dir.join("Unpacked.bodoproj");
    convert(&packed, &unpacked).unwrap();
    assert_eq!(tree(&unpacked), tree(&folder));
    assert_eq!(std::fs::read(unpacked.join("thumbnail.png")).unwrap(), b"png");
    assert_eq!(load(&unpacked).unwrap(), project);

    // Conversions never overwrite.
    assert!(matches!(convert(&folder, &packed), Err(IoError::Exists { .. })));

    // Saving a zip in place keeps what it carries.
    save(&project, &packed, &in_place(&packed, 0)).unwrap();
    assert_eq!(zip_entry(&packed, "thumbnail.png"), b"png");

    // A plugin that loses its state loses its file.
    let mut stateless = project.clone();
    let mut mixer = (*stateless.mixer).clone();
    let mut master = (*mixer.master).clone();
    master.slots[9].plugin.as_mut().unwrap().state = Arc::from(Vec::new());
    mixer.master = Arc::new(master);
    stateless.mixer = Arc::new(mixer);
    save(&stateless, &folder, &in_place(&folder, 0)).unwrap();
    assert_eq!(tree(&folder).iter().filter(|f| f.ends_with(".state")).count(), 1);
    assert_eq!(load(&folder).unwrap(), stateless);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A project with one sample at each stored path.
fn with_samples(paths: &[&str]) -> Project {
    let (mut project, _) = full();
    for (i, path) in paths.iter().enumerate() {
        let id = SampleId::from_u128(9000 + i as u128);
        project.samples.insert(id, Arc::new(SampleRef { id, path: (*path).into(), hash: None, market: None }));
    }
    project
}

fn path_of(project: &Project, index: u128) -> String {
    project.samples[&SampleId::from_u128(9000 + index)].path.clone()
}

#[test]
fn sample_paths_are_rebased_to_the_new_folder() {
    let dir = temp_dir("rebase");
    let old = dir.join("old").join("Song.bodo");
    let inside_new = dir.join("Loops").join("c.wav");
    let project =
        with_samples(&["Samples/a.wav", "../../outside.wav", "factory:drums/kick.wav", &inside_new.to_string_lossy(), "Samples/../Samples/./d.wav"]);
    // Moving from `<dir>/old` up to `<dir>`.
    let new = dir.join("Song.bodo");
    let saved = save(&project, &new, &SaveOptions { from: Some(old), backups: 0 }).unwrap();
    assert_eq!(path_of(&saved, 0), "old/Samples/a.wav", "relative and inside the new folder: relative to it");
    let outside = bodo_io::paths::resolve_sample_path(Some(&dir.join("old")), "../../outside.wav").unwrap();
    assert_eq!(path_of(&saved, 1), outside.to_string_lossy(), "outside the new folder: absolute");
    assert!(Path::new(&path_of(&saved, 1)).is_absolute());
    assert_eq!(path_of(&saved, 2), "factory:drums/kick.wav", "factory content is never rebased");
    assert_eq!(path_of(&saved, 3), "Loops/c.wav", "absolute but inside the new folder: relative");
    assert_eq!(path_of(&saved, 4), "old/Samples/d.wav", "dots are folded");
    assert_eq!(load(&new).unwrap(), saved);

    // An unsaved project's relative paths start at the current directory.
    let unsaved = with_samples(&["x.wav"]);
    let saved = save(&unsaved, &dir.join("Fresh.bodo"), &SaveOptions { from: None, backups: 0 }).unwrap();
    let expected = std::env::current_dir().unwrap().join("x.wav");
    assert_eq!(Path::new(&path_of(&saved, 0)), expected);

    // The helper the engine uses for new samples follows the same rule.
    assert_eq!(bodo_io::sample_path(Some(&dir), &dir.join("Recorded").join("take.wav")), "Recorded/take.wav");
    assert_eq!(bodo_io::project_folder(&new), dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_files_are_refused_with_clear_errors() {
    let (project, _) = full();
    let json = to_json(&project).unwrap();

    let newer = json.replacen("\"format_version\": 1", "\"format_version\": 7", 1);
    let err = from_json(&newer).unwrap_err();
    assert!(matches!(err, IoError::TooNew { found: 7, .. }), "{err}");
    let message = err.to_string();
    assert!(message.contains("format version 7") && message.contains(&format!("up to {FORMAT_VERSION}")), "{message}");

    let other = json.replacen("\"format\": \"bodo\"", "\"format\": \"flp\"", 1);
    assert!(matches!(from_json(&other), Err(IoError::NotBodo { found, .. }) if found == "\"flp\""));
    assert!(matches!(from_json("{}"), Err(IoError::NotBodo { .. })));
    let unversioned = json.replacen("\"format_version\": 1", "\"format_version\": \"one\"", 1);
    assert!(matches!(from_json(&unversioned), Err(IoError::BadVersion { .. })));
    let zero = json.replacen("\"format_version\": 1", "\"format_version\": 0", 1);
    assert!(matches!(from_json(&zero), Err(IoError::BadVersion { .. })));

    let broken = &json[..json.len() / 2];
    let err = from_json(broken).unwrap_err();
    assert!(matches!(err, IoError::Json { line, .. } if line > 1), "{err}");
    let missing = json.replacen("\"rackOrder\"", "\"rackOrderX\"", 1).replacen("\"mixer\"", "\"mixerX\"", 1);
    let err = from_json(&missing).unwrap_err();
    assert!(matches!(&err, IoError::Json { message, .. } if message.contains("mixer")), "{err}");

    // A project that breaks invariants: the error lists every violation.
    let mut invalid = project.clone();
    invalid.rack_order.push_back(invalid.rack_order[0]);
    invalid.track_order.push_back(invalid.track_order[0]);
    let err = from_json(&to_json(&invalid).unwrap()).unwrap_err();
    let message = err.to_string();
    assert!(matches!(&err, IoError::Invalid { error, .. } if error.0.len() == 2), "{message}");
    assert!(message.contains("rackOrder") && message.contains("trackOrder"), "{message}");

    let dir = temp_dir("refuse");
    let path = dir.join("Bad.bodo");
    let err = save(&invalid, &path, &SaveOptions::default()).unwrap_err();
    assert!(matches!(&err, IoError::Invalid { path: Some(p), .. } if *p == path), "{err}");
    assert!(!path.exists());
    let err = load(&dir.join("Missing.bodo")).unwrap_err();
    assert!(matches!(err, IoError::Io { action: "open", .. }), "{err}");
    std::fs::write(&path, b"not a zip").unwrap();
    assert!(matches!(load(&path), Err(IoError::Zip { .. })));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
    zip.finish().unwrap();
    assert!(matches!(load(&path), Err(IoError::Missing { entry: "project.json", .. })));
    std::fs::create_dir_all(dir.join("Empty.bodoproj")).unwrap();
    assert!(matches!(load(&dir.join("Empty.bodoproj")), Err(IoError::Missing { .. })));
    // Errors from inside a file name the file.
    let named = dir.join("Newer.bodoproj");
    std::fs::create_dir_all(&named).unwrap();
    std::fs::write(named.join("project.json"), &newer).unwrap();
    assert!(load(&named).unwrap_err().to_string().starts_with(&named.display().to_string()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The hidden temporary files left in `dir` and its `Backup` folder.
fn temp_files(dir: &Path) -> Vec<String> {
    tree(dir).into_iter().filter(|name| name.rsplit('/').find(|p| !p.is_empty()).is_some_and(|p| p.starts_with('.') && p.ends_with(".tmp"))).collect()
}

#[test]
fn saves_are_atomic_and_leave_nothing_behind() {
    let dir = temp_dir("atomic");
    let (project, _) = full();
    let path = dir.join("Song.bodo");
    save(&project, &path, &in_place(&path, 2)).unwrap();
    let mut renamed = project.clone();
    Arc::make_mut(&mut renamed.meta).title = "Renamed".into();
    save(&renamed, &path, &in_place(&path, 2)).unwrap();
    assert_eq!(load(&path).unwrap().meta.title, "Renamed");
    assert!(temp_files(&dir).is_empty(), "{:?}", tree(&dir));

    // A save that fails at the last step leaves the old file and no temporary file.
    let blocked = dir.join("Blocked.bodo");
    std::fs::create_dir_all(blocked.join("in the way")).unwrap();
    assert!(save(&project, &blocked, &SaveOptions { from: None, backups: 0 }).is_err());
    assert!(blocked.is_dir());
    assert!(temp_files(&dir).is_empty(), "{:?}", tree(&dir));

    // Folder mode too.
    let folder = dir.join("Song.bodoproj");
    save(&project, &folder, &in_place(&folder, 0)).unwrap();
    save(&renamed, &folder, &in_place(&folder, 0)).unwrap();
    assert_eq!(load(&folder).unwrap().meta.title, "Renamed");
    assert!(temp_files(&dir).is_empty(), "{:?}", tree(&dir));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn backups_rotate_and_keep_the_newest() {
    let dir = temp_dir("backups");
    let (project, _) = full();
    let path = dir.join("My Song.bodo");
    let titled = |n: usize| {
        let mut p = project.clone();
        Arc::make_mut(&mut p.meta).title = format!("Save {n}");
        p
    };
    for n in 0..8 {
        save(&titled(n), &path, &in_place(&path, 3)).unwrap();
    }
    let found = backups(&path).unwrap();
    assert_eq!(found.len(), 3, "{found:?}");
    // Newest first: the backup made by the last save holds the save before it.
    let titles: Vec<String> = found.iter().map(|b| load(b).unwrap().meta.title.clone()).collect();
    assert_eq!(titles, ["Save 6", "Save 5", "Save 4"]);
    assert!(found.iter().all(|b| b.parent() == Some(&dir.join("Backup"))));
    let name = found[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("My Song 20") && name.ends_with("Z.bodo"), "{name}");

    // Another project's backups in the same folder are left alone, and zero keeps none.
    let other = dir.join("My.bodo");
    save(&project, &other, &in_place(&other, 0)).unwrap();
    save(&project, &other, &in_place(&other, 0)).unwrap();
    assert!(backups(&other).unwrap().is_empty());
    assert_eq!(backups(&path).unwrap().len(), 3);
    assert!(backups(&dir.join("Never saved.bodo")).unwrap().is_empty());

    // Folder projects are backed up as folders.
    let folder = dir.join("Song.bodoproj");
    save(&project, &folder, &in_place(&folder, 2)).unwrap();
    save(&titled(1), &folder, &in_place(&folder, 2)).unwrap();
    let found = backups(&folder).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].is_dir());
    assert_eq!(load(&found[0]).unwrap().meta.title, project.meta.title);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_hash_with_blake3() {
    let dir = temp_dir("hash");
    let path: PathBuf = dir.join("abc.txt");
    std::fs::write(&path, b"abc").unwrap();
    assert_eq!(hash_file(&path).unwrap(), "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85");
    assert!(matches!(hash_file(&dir.join("none")), Err(IoError::Io { .. })));
    std::fs::remove_dir_all(&dir).unwrap();
}
