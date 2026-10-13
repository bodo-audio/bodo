//! `project.json`: the project as JSON, with its format header and forward migrations (project.md section 18.1).
//!
//! The top-level object starts with `"format": "bodo"` and `"format_version"`, followed by the fields of
//! [`Project`]. A file of an older version goes through one migration step per version, as JSON, before it is read.

use bodo_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::IoError;

/// The value of the `format` key of every project file.
pub const FORMAT: &str = "bodo";

/// The format version this build writes, and the newest it reads.
pub const FORMAT_VERSION: u32 = 1;

/// One migration step: turns a file of version `n` into version `n + 1`, in place.
type Migration = fn(&mut Value) -> Result<(), String>;

/// The migration chain. Entry `i` migrates version `i + 1` to version `i + 2`, so the chain has
/// `FORMAT_VERSION - 1` entries. Version 1 is the first, so there are none yet.
const MIGRATIONS: &[Migration] = &[];

/// What is written: the format header, then the project's own fields.
#[derive(Serialize)]
struct Envelope<'a> {
    format: &'static str,
    format_version: u32,
    #[serde(flatten)]
    project: &'a Project,
}

/// The format header, read on its own before anything else.
#[derive(Deserialize)]
struct Header {
    format: Option<Value>,
    format_version: Option<Value>,
}

/// The project as `project.json`: pretty-printed, with the format header first and a final newline. The same
/// project always gives the same text.
///
/// Complexity: O(N) in the size of the project.
pub fn to_json(project: &Project) -> Result<String, IoError> {
    let envelope = Envelope { format: FORMAT, format_version: FORMAT_VERSION, project };
    let mut text = serde_json::to_string_pretty(&envelope).map_err(|err| IoError::Encode { message: err.to_string() })?;
    text.push('\n');
    Ok(text)
}

/// Reads `project.json`: checks the format header, migrates older versions, and refuses a project that breaks
/// the model's invariants ([`Project::validate`]).
///
/// Complexity: O(N log N) in the size of the project, for the validation.
pub fn from_json(text: &str) -> Result<Project, IoError> {
    let header: Header = serde_json::from_str(text).map_err(|err| json_error(&err))?;
    match &header.format {
        Some(Value::String(format)) if format == FORMAT => {}
        Some(other) => return Err(IoError::NotBodo { path: None, found: other.to_string() }),
        None => return Err(IoError::NotBodo { path: None, found: String::from("missing") }),
    }
    let version = match &header.format_version {
        Some(Value::Number(number)) => match number.as_u64() {
            Some(version) if version >= 1 => version,
            _ => return Err(IoError::BadVersion { path: None, found: number.to_string() }),
        },
        Some(other) => return Err(IoError::BadVersion { path: None, found: other.to_string() }),
        None => return Err(IoError::BadVersion { path: None, found: String::from("nothing") }),
    };
    let project: Project = match u32::try_from(version) {
        Ok(FORMAT_VERSION) => serde_json::from_str(text).map_err(|err| json_error(&err))?,
        Ok(older) if older < FORMAT_VERSION => {
            let mut value: Value = serde_json::from_str(text).map_err(|err| json_error(&err))?;
            migrate(&mut value, older, FORMAT_VERSION, MIGRATIONS)?;
            serde_json::from_value(value).map_err(|err| IoError::Json {
                path: None,
                line: 0,
                column: 0,
                message: format!("{err} (after migrating from format version {older})"),
            })?
        }
        _ => return Err(IoError::TooNew { path: None, found: version }),
    };
    project.validate().map_err(|error| IoError::Invalid { path: None, error })?;
    Ok(project)
}

/// Runs the steps of `chain` on `value` from version `from` up to version `to`, then stamps `to` on it. Entry `i`
/// of `chain` migrates version `i + 1` to `i + 2`.
fn migrate(value: &mut Value, from: u32, to: u32, chain: &[Migration]) -> Result<(), IoError> {
    for version in from..to {
        let step = version.checked_sub(1).and_then(|index| chain.get(usize::try_from(index).ok()?)).ok_or_else(|| IoError::Migration {
            path: None,
            from: version,
            message: String::from("this build has no migration for it"),
        })?;
        step(value).map_err(|message| IoError::Migration { path: None, from: version, message })?;
    }
    if let Value::Object(map) = value {
        map.insert(String::from("format_version"), Value::from(to));
    }
    Ok(())
}

fn json_error(err: &serde_json::Error) -> IoError {
    let (line, column) = (err.line(), err.column());
    // serde_json ends its message with the position, which the error already carries.
    let text = err.to_string();
    let message = text.strip_suffix(&format!(" at line {line} column {column}")).unwrap_or(&text).to_owned();
    IoError::Json { path: None, line, column, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename_x(value: &mut Value) -> Result<(), String> {
        let x = value.as_object_mut().and_then(|map| map.remove("x")).ok_or("no x")?;
        value["y"] = x;
        Ok(())
    }

    fn double_y(value: &mut Value) -> Result<(), String> {
        let y = value["y"].as_i64().ok_or("no y")?;
        value["y"] = Value::from(y * 2);
        Ok(())
    }

    #[test]
    fn migration_steps_run_in_order_and_stamp_the_version() {
        // Version 1 is the only real version, so a pretend chain of two steps checks the mechanism.
        let chain: &[Migration] = &[rename_x, double_y];
        let mut value = serde_json::json!({ "format": "bodo", "format_version": 1, "x": 21 });
        migrate(&mut value, 1, 3, chain).unwrap();
        assert_eq!(value, serde_json::json!({ "format": "bodo", "format_version": 3, "y": 42 }));
        // Starting midway runs only the later steps.
        let mut value = serde_json::json!({ "format_version": 2, "y": 5 });
        migrate(&mut value, 2, 3, chain).unwrap();
        assert_eq!(value["y"], 10);
        // The real chain covers every version below the current one.
        assert_eq!(MIGRATIONS.len(), FORMAT_VERSION as usize - 1);
    }

    #[test]
    fn failing_and_missing_steps_are_reported() {
        let chain: &[Migration] = &[rename_x];
        let err = migrate(&mut serde_json::json!({}), 1, 2, chain).unwrap_err();
        assert!(matches!(&err, IoError::Migration { from: 1, message, .. } if message == "no x"), "{err}");
        let err = migrate(&mut serde_json::json!({ "x": 1 }), 1, 3, chain).unwrap_err();
        assert!(matches!(err, IoError::Migration { from: 2, .. }), "{err}");
    }
}
