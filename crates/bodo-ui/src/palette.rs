//! The command palette, Cmd/Ctrl+K (project.md section 14.1): fuzzy search over every command. A command whose params
//! are all optional runs on the selection at once; one with required params opens a small form generated from its
//! JSON Schema.

use egui::{Align2, Key, RichText, vec2};
use serde_json::{Map, Value};

use crate::BodoApp;
use crate::theme::{Tokens, size};

/// The palette's search, highlighted row and open form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaletteState {
    /// What the user typed.
    pub query: String,
    /// The highlighted result.
    pub selected: usize,
    /// The command whose params are being filled in, and its fields.
    pub form: Option<(String, Vec<Field>)>,
}

/// A param in the palette's form: its name, the text typed, and whether it is required.
pub type Field = (String, String, bool);

/// One searchable command.
#[derive(Clone, Debug)]
struct Entry {
    id: String,
    title: String,
    summary: String,
    shortcut: Option<String>,
    required: Vec<String>,
    fields: Vec<(String, bool)>,
}

fn entries(app: &BodoApp) -> Vec<Entry> {
    let mut out: Vec<Entry> = app
        .session
        .registry()
        .iter()
        .map(|spec| {
            let required: Vec<String> = spec
                .params
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            let fields = bodo_engine::docs::params_table(spec).into_iter().map(|(name, _, required, _)| (name, required)).collect();
            Entry {
                id: spec.id.to_owned(),
                title: spec.title.to_owned(),
                summary: spec.summary.to_owned(),
                shortcut: spec.shortcut.map(crate::shortcuts::display),
                required,
                fields,
            }
        })
        .collect();
    out.extend(BodoApp::ui_commands().iter().map(|(id, title, summary)| Entry {
        id: (*id).to_owned(),
        title: (*title).to_owned(),
        summary: (*summary).to_owned(),
        shortcut: None,
        required: Vec::new(),
        fields: Vec::new(),
    }));
    out
}

/// How well `query` matches `text`: every query character in order, earlier and tighter matches scoring higher.
/// `None` when it doesn't match.
///
/// Complexity: O(n) in the text.
pub(crate) fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut at = 0;
    let mut last = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = text[at..].iter().position(|c| *c == q)? + at;
        score += match last {
            Some(prev) if found == prev + 1 => 8,
            _ => 1,
        };
        if found == 0 || text.get(found.wrapping_sub(1)).is_some_and(|c| !c.is_alphanumeric()) {
            score += 4;
        }
        last = Some(found);
        at = found + 1;
    }
    Some(score * 100 - text.len() as i32)
}

/// Draws the palette when it is open.
pub(crate) fn show(app: &mut BodoApp, ctx: &egui::Context) {
    let Some(mut state) = app.view.palette.take() else { return };
    let tk = Tokens::get();
    let mut run: Option<(String, Value)> = None;
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let mut back = false;
    egui::Area::new(egui::Id::new("bodo-palette")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, vec2(0.0, 90.0)).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(tk.popup).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(520.0);
            if let Some((id, mut fields)) = state.form.clone() {
                ui.label(RichText::new(id.as_str()).monospace().color(tk.accent));
                for (name, text, required) in fields.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.add_sized(vec2(110.0, 20.0), egui::Label::new(if *required { format!("{name} *") } else { name.clone() }));
                        ui.add(egui::TextEdit::singleline(text).desired_width(360.0));
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Run").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        let mut params = Map::new();
                        for (name, text, _) in fields.iter().filter(|(_, t, _)| !t.trim().is_empty()) {
                            let value = serde_json::from_str::<Value>(text.trim()).unwrap_or_else(|_| Value::String(text.trim().to_owned()));
                            params.insert(name.clone(), value);
                        }
                        run = Some((id.clone(), Value::Object(params)));
                    }
                    if ui.button("Back").clicked() {
                        back = true;
                    }
                });
                state.form = if back { None } else { Some((id, fields)) };
                return;
            }
            let field = ui.add(egui::TextEdit::singleline(&mut state.query).hint_text("Type a command").desired_width(f32::INFINITY));
            field.request_focus();
            let mut found: Vec<(i32, Entry)> =
                entries(app).into_iter().filter_map(|e| score(&state.query, &format!("{} {}", e.title, e.id)).map(|s| (s, e))).collect();
            found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
            found.truncate(12);
            if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                state.selected = (state.selected + 1).min(found.len().saturating_sub(1));
            }
            if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
                state.selected = state.selected.saturating_sub(1);
            }
            state.selected = state.selected.min(found.len().saturating_sub(1));
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(6.0);
            let mut chosen = None;
            for (index, (_, entry)) in found.iter().enumerate() {
                let selected = index == state.selected;
                let response = ui
                    .horizontal(|ui| {
                        let row = ui.selectable_label(selected, RichText::new(&entry.title).color(tk.text));
                        ui.label(RichText::new(&entry.id).monospace().size(size::SMALL).color(tk.text_faint));
                        if let Some(shortcut) = &entry.shortcut {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(shortcut).size(size::SMALL).color(tk.text_dim))
                            });
                        }
                        row
                    })
                    .inner
                    .on_hover_text(&entry.summary);
                if response.clicked() || (selected && enter) {
                    chosen = Some(entry.clone());
                }
            }
            if found.is_empty() {
                ui.label(RichText::new("No command matches").color(tk.text_faint));
            }
            if let Some(entry) = chosen {
                if entry.required.is_empty() {
                    run = Some((entry.id, Value::Object(Map::new())));
                } else {
                    state.form = Some((entry.id, entry.fields.into_iter().map(|(name, required)| (name, String::new(), required)).collect()));
                }
            }
        });
    });
    if let Some((id, params)) = run {
        let _ = app.run(&id, params);
        close = true;
    }
    if !close {
        app.view.palette = Some(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scores_prefer_tight_early_matches() {
        assert!(score("nwpat", "New pattern pattern.new").is_some());
        assert!(score("xyz", "New pattern").is_none());
        let tight = score("undo", "Undo edit.undo").unwrap_or(0);
        let loose = score("undo", "Unmute dock").unwrap_or(i32::MIN);
        assert!(tight > loose);
    }
}
