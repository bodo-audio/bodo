//! Backups of a project, kept in the `Backup` folder of its project folder (project.md section 18.1).
//!
//! Before a save replaces a project, the old version is copied to `Backup/<stem> <UTC timestamp>.<extension>`,
//! such as `Backup/My Song 2026-10-13T14-05-09.120Z.bodo`. The timestamps sort in time order, and only the newest
//! few backups of each project are kept.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::atomic::{TempGuard, is_temp_name, sync_folder, temp_path};
use crate::error::IoError;
use crate::paths::project_folder;

/// The name of the folder backups go in, inside the project folder.
pub(crate) const BACKUP_FOLDER: &str = "Backup";

/// The length of a timestamp such as `2026-10-13T14-05-09.120Z`.
const STAMP_LEN: usize = 24;

/// The backups of the project at `path`, newest first. A project without backups has none.
///
/// A backup's relative sample paths still start at the project folder, not at the `Backup` folder, so open a
/// backup with the project folder as its base.
///
/// Complexity: O(f log f) in the files of the `Backup` folder.
pub fn backups(path: &Path) -> Result<Vec<PathBuf>, IoError> {
    let folder = project_folder(path).join(BACKUP_FOLDER);
    let Some((stem, extension)) = name_parts(path) else { return Ok(Vec::new()) };
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(IoError::io("list", &folder, err)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| IoError::io("list", &folder, err))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stamp) = backup_stamp(&name, &stem, extension.as_deref()) {
            found.push((stamp.to_owned(), entry.path()));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

/// Copies the project at `path`, a file or a folder, into its `Backup` folder, then deletes all but the newest
/// `keep` backups. Does nothing when `keep` is zero or there is nothing at `path`.
///
/// Complexity: O(size of the project) for the copy, plus O(f log f) in the files of the `Backup` folder.
pub(crate) fn back_up(path: &Path, keep: usize) -> Result<(), IoError> {
    if keep == 0 || !path.exists() {
        return Ok(());
    }
    let Some((stem, extension)) = name_parts(path) else { return Ok(()) };
    let folder = project_folder(path).join(BACKUP_FOLDER);
    std::fs::create_dir_all(&folder).map_err(|err| IoError::io("create", &folder, err))?;
    let mut now = SystemTime::now();
    let target = loop {
        let mut name = format!("{stem} {}", timestamp(now));
        if let Some(extension) = &extension {
            name.push('.');
            name.push_str(extension);
        }
        let target = folder.join(name);
        if !target.exists() {
            break target;
        }
        // Two saves in the same millisecond: the later one takes the next free millisecond.
        now += Duration::from_millis(1);
    };
    let temp = temp_path(&target);
    let guard = TempGuard::new(&temp);
    copy_tree(path, &temp)?;
    std::fs::rename(&temp, &target).map_err(|err| IoError::io("create", &target, err))?;
    guard.keep();
    sync_folder(&folder)?;
    for old in backups(path)?.into_iter().skip(keep) {
        let removed = if old.is_dir() { std::fs::remove_dir_all(&old) } else { std::fs::remove_file(&old) };
        removed.map_err(|err| IoError::io("remove", &old, err))?;
    }
    Ok(())
}

/// Copies a file, or a folder and everything in it, skipping temporary files.
///
/// Complexity: O(size of the tree).
pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<(), IoError> {
    if !from.is_dir() {
        std::fs::copy(from, to).map_err(|err| IoError::io("copy", from, err))?;
        return Ok(());
    }
    std::fs::create_dir_all(to).map_err(|err| IoError::io("create", to, err))?;
    let entries = std::fs::read_dir(from).map_err(|err| IoError::io("list", from, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| IoError::io("list", from, err))?;
        let name = entry.file_name();
        if is_temp_name(&name.to_string_lossy()) {
            continue;
        }
        copy_tree(&entry.path(), &to.join(name))?;
    }
    Ok(())
}

/// The stem and extension of a project's file name.
fn name_parts(path: &Path) -> Option<(String, Option<String>)> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned());
    Some((stem, extension))
}

/// The timestamp of a backup named `name`, if it is a backup of the project with this stem and extension.
fn backup_stamp<'a>(name: &'a str, stem: &str, extension: Option<&str>) -> Option<&'a str> {
    let rest = name.strip_prefix(stem)?.strip_prefix(' ')?;
    let stamp = match extension {
        Some(extension) => rest.strip_suffix(extension)?.strip_suffix('.')?,
        None => rest,
    };
    is_stamp(stamp).then_some(stamp)
}

/// Whether `text` has the shape of [`timestamp`]'s output.
fn is_stamp(text: &str) -> bool {
    let pattern = b"dddd-dd-ddTdd-dd-dd.dddZ";
    text.len() == STAMP_LEN && text.bytes().zip(pattern.iter()).all(|(c, &p)| if p == b'd' { c.is_ascii_digit() } else { c == p })
}

/// A UTC time as `YYYY-MM-DDTHH-MM-SS.mmmZ`: safe in file names everywhere, and sorting in time order.
fn timestamp(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}.{:03}Z", of_day / 3600, of_day / 60 % 60, of_day % 60, since.subsec_millis())
}

/// The proleptic Gregorian date of a day counted from 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_and_sortable() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00-00-00.000Z");
        // 2026-10-13 14:05:09.120 UTC.
        let time = UNIX_EPOCH + Duration::from_millis(1_791_900_309_120);
        assert_eq!(timestamp(time), "2026-10-13T14-05-09.120Z");
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert!(is_stamp("2026-10-13T14-05-09.120Z"));
        assert!(!is_stamp("2026-10-13 14-05-09.120Z"));
    }

    #[test]
    fn backup_names_match_only_their_project() {
        let name = "My Song 2026-10-13T14-05-09.120Z.bodo";
        assert_eq!(backup_stamp(name, "My Song", Some("bodo")), Some("2026-10-13T14-05-09.120Z"));
        assert_eq!(backup_stamp(name, "My", Some("bodo")), None);
        assert_eq!(backup_stamp(name, "My Song", Some("bodoproj")), None);
        assert_eq!(backup_stamp("My Song 2026.bodo", "My Song", Some("bodo")), None);
    }
}
