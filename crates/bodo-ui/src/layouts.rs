//! Layouts (project.md section 14.1, ADRs 0012 and 0032): the presets Bodo ships with and the ones the user saves
//! under a name. A layout records which windows are open, where, how large, minimized or maximized, and whether they
//! are detached and where their OS windows were. Layouts live in the user's preferences, not in projects.

use serde_json::{Value, json};

use crate::BodoApp;
use crate::windows::{Layout, Preset, SavedLayout};

/// A layout by name: a preset's or one the user saved.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// A layout Bodo ships with.
    Preset(Preset),
    /// A layout the user saved, by its name.
    Saved(String),
}

/// The layout `name` names: a preset first, then the user's, ignoring case.
pub fn find(app: &BodoApp, name: &str) -> Option<Choice> {
    let name = name.trim();
    Preset::from_id(name)
        .map(Choice::Preset)
        .or_else(|| app.prefs.layouts.iter().find(|l| l.name.eq_ignore_ascii_case(name)).map(|l| Choice::Saved(l.name.clone())))
}

/// Arranges the windows as layout `name` says. The Master preset also selects the master insert, so the mixer
/// shows it.
///
/// Complexity: O(l) in the saved layouts.
pub fn switch(app: &mut BodoApp, name: &str) -> Result<Value, String> {
    let (browser, width) = (app.prefs.layout.browser, app.prefs.layout.browser_width);
    match find(app, name) {
        Some(Choice::Preset(preset)) => {
            // A preset arranges the windows but leaves the Browser as the user has it.
            app.prefs.layout = Layout { browser, browser_width: width, ..Layout::preset(preset) };
            app.prefs.layout_name = Some(preset.label().to_owned());
            if preset == Preset::Master {
                let _ = app.session.execute("edit.select", json!({ "insert": "Master" }));
            }
            Ok(json!({ "layout": preset.label() }))
        }
        Some(Choice::Saved(name)) => {
            let saved = app.prefs.layouts.iter().find(|l| l.name == name).map(|l| l.layout.clone()).unwrap_or_default();
            app.prefs.layout = saved;
            app.prefs.layout_name = Some(name.clone());
            Ok(json!({ "layout": name }))
        }
        None => Err(format!("there is no layout `{}`; the layouts are {}", name.trim(), names(app).join(", "))),
    }
}

/// Every layout's name: the presets, then the user's in the order they were saved.
pub fn names(app: &BodoApp) -> Vec<String> {
    Preset::ALL.iter().map(|p| p.label().to_owned()).chain(app.prefs.layouts.iter().map(|l| l.name.clone())).collect()
}

/// A name a saved layout can take: not empty, and not a preset's.
fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(String::from("a layout needs a name"));
    }
    if Preset::from_id(name).is_some() {
        return Err(format!("{name} is one of Bodo's own layouts; choose another name"));
    }
    Ok(name.to_owned())
}

/// Saves the windows as they are now as layout `name`, replacing a saved layout of that name.
pub fn save(app: &mut BodoApp, name: &str) -> Result<Value, String> {
    let name = check_name(name)?;
    let layout = app.prefs.layout.clone();
    match app.prefs.layouts.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&name)) {
        Some(existing) => {
            existing.name.clone_from(&name);
            existing.layout = layout;
        }
        None => app.prefs.layouts.push(SavedLayout { name: name.clone(), layout }),
    }
    app.prefs.layout_name = Some(name.clone());
    Ok(json!({ "layout": name, "saved": true }))
}

/// Renames saved layout `from` to `to`.
pub fn rename(app: &mut BodoApp, from: &str, to: &str) -> Result<Value, String> {
    let to = check_name(to)?;
    let Some(Choice::Saved(from)) = find(app, from) else { return Err(format!("there is no saved layout `{}`", from.trim())) };
    if !from.eq_ignore_ascii_case(&to) && app.prefs.layouts.iter().any(|l| l.name.eq_ignore_ascii_case(&to)) {
        return Err(format!("there already is a layout called {to}"));
    }
    if let Some(layout) = app.prefs.layouts.iter_mut().find(|l| l.name == from) {
        layout.name.clone_from(&to);
    }
    if app.prefs.layout_name.as_deref() == Some(from.as_str()) {
        app.prefs.layout_name = Some(to.clone());
    }
    Ok(json!({ "layout": to, "was": from }))
}

/// Deletes saved layout `name`. The windows stay as they are.
pub fn delete(app: &mut BodoApp, name: &str) -> Result<Value, String> {
    let Some(Choice::Saved(name)) = find(app, name) else { return Err(format!("there is no saved layout `{}`", name.trim())) };
    app.prefs.layouts.retain(|l| l.name != name);
    if app.prefs.layout_name.as_deref() == Some(name.as_str()) {
        app.prefs.layout_name = None;
    }
    Ok(json!({ "layout": name, "deleted": true }))
}
