//! The command reference, generated from the registry: `docs/agents/commands.md` and MCP's `describe_command`.

use serde_json::Value;

use crate::registry::{CommandSpec, Registry};

/// The type of a JSON Schema as a short phrase, such as `string`, `ChannelRef` or `array of NoteSpec`.
fn type_of(schema: &Value) -> String {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return reference.rsplit('/').next().unwrap_or(reference).to_owned();
    }
    if let Some(any) = schema.get("anyOf").or_else(|| schema.get("oneOf")).and_then(Value::as_array) {
        let types: Vec<String> = any.iter().map(type_of).filter(|t| t != "null").collect();
        return types.join(" or ");
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.iter().filter_map(Value::as_str).map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join(" or ");
    }
    match schema.get("type") {
        Some(Value::String(t)) if t == "array" => format!("array of {}", schema.get("items").map_or_else(|| String::from("values"), type_of)),
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(types)) => types.iter().filter_map(Value::as_str).filter(|t| *t != "null").collect::<Vec<_>>().join(" or "),
        _ => String::from("value"),
    }
}

/// One line per param: name, type, whether it's required, and its description.
pub fn params_table(spec: &CommandSpec) -> Vec<(String, String, bool, String)> {
    let required: Vec<&str> =
        spec.params.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let Some(properties) = spec.params.get("properties").and_then(Value::as_object) else { return Vec::new() };
    properties
        .iter()
        .map(|(name, schema)| {
            let description = schema.get("description").and_then(Value::as_str).unwrap_or("").replace('\n', " ");
            (name.clone(), type_of(schema), required.contains(&name.as_str()), description)
        })
        .collect()
}

/// The reference entry of one command, in Markdown.
pub fn command_markdown(spec: &CommandSpec) -> String {
    let mut out = format!("### `{}`\n\n{}\n\n", spec.id, spec.summary);
    out.push_str(&format!("- Effects: {}\n", spec.effects.describe()));
    if spec.merge_key.is_some() {
        out.push_str("- Calls in one gesture fold into one undo step.\n");
    }
    if spec.is_session_command() {
        out.push_str("- Acts on the session, so it can't run inside a batch.\n");
    }
    if let Some(menu) = spec.menu {
        out.push_str(&format!("- Menu: {}\n", menu.replace('/', " > ")));
    }
    if let Some(shortcut) = spec.shortcut {
        out.push_str(&format!("- Shortcut: {shortcut}\n"));
    }
    let params = params_table(spec);
    if params.is_empty() {
        out.push_str("\nNo params.\n");
    } else {
        out.push_str("\n| Param | Type | Description |\n|---|---|---|\n");
        for (name, kind, required, description) in params {
            let kind = if required { kind } else { format!("{kind}, optional") };
            out.push_str(&format!("| `{name}` | {} | {} |\n", kind.replace('|', "\\|"), description.replace('|', "\\|")));
        }
    }
    if let Some(example) = &spec.example {
        let example = serde_json::to_string_pretty(example).unwrap_or_default();
        out.push_str(&format!("\nExample:\n\n```json\n{example}\n```\n"));
    }
    out
}

/// The whole command reference, in Markdown, grouped by namespace.
pub fn commands_markdown(registry: &Registry) -> String {
    let mut out = String::from(
        "# Commands\n\n\
         Every command registered in `bodo-engine`, generated from the registry by `cargo test -p bodo-engine --test docs` \
         (set `BODO_BLESS=1` to rewrite it). MCP serves the same text and generates its tools from the same registry \
         ([ADR 0005](../adr/0005-everything-is-a-command.md)); AGENTS.md section 7 explains how to add a command.\n\n\
         Conventions:\n\n\
         - Empty params act on the current selection: the current pattern, the piano roll's channel, the selected \
         channels, clips, notes, insert or track.\n\
         - Positions are ticks (960 per quarter note) or text: `\"bar:beat:tick\"` counted from 1, or seconds such as \
         `\"1.5s\"`. Lengths are ticks or text such as `\"2 bars\"`, `\"1 beat\"` or `\"4 steps\"`.\n\
         - Channels, patterns, tracks and inserts are named by id, by number (`\"channel:3\"`, `\"insert:0\"` for the \
         master, or just `3`) or by name. An ambiguous name is an error that lists the candidates.\n\
         - Effects: `undoable` commands change the project and make one undo step; `destructive` ones can lose unsaved \
         work; `disk` ones write files.\n",
    );
    let mut namespace = "";
    for spec in registry.iter() {
        if spec.namespace() != namespace {
            namespace = spec.namespace();
            out.push_str(&format!("\n## {namespace}\n"));
        }
        out.push('\n');
        out.push_str(&command_markdown(spec));
    }
    out
}
