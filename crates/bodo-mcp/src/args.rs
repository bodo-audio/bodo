//! The arguments of every tool. Their JSON Schemas are the tools' input schemas, so every field is documented and
//! unknown fields are refused.

use bodo_engine::params::PatternRef;
use serde::Deserialize;
use serde_json::Value;

/// `list_commands`.
#[derive(Clone, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListCommandsArgs {
    /// Only commands in this namespace, such as "channel" or "pattern".
    pub namespace: Option<String>,
}

/// `describe_command`.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DescribeCommandArgs {
    /// The command id, such as "pattern.set_steps".
    pub id: String,
}

/// `execute`.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteArgs {
    /// The command id, such as "channel.add". list_commands shows them all.
    pub id: String,
    /// The command's params as an object; describe_command shows them. Empty params act on the current selection.
    #[serde(default)]
    pub params: Value,
}

/// One command of a batch.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchCall {
    /// The command id.
    pub id: String,
    /// The command's params as an object.
    #[serde(default)]
    pub params: Value,
}

/// `batch`.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchArgs {
    /// The commands, run in order as one transaction and one undo step. Each sees the changes of those before it.
    /// If any fails, nothing changes.
    pub commands: Vec<BatchCall>,
    /// The undo step's label; the commands' titles by default.
    pub label: Option<String>,
}

/// Tools that take no arguments.
#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// `inspect_pattern`.
#[derive(Clone, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectPatternArgs {
    /// The pattern; the current pattern by default.
    pub pattern: Option<PatternRef>,
}

/// `ui_inspect`.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiInspectArgs {
    /// Only elements whose label contains this text, ignoring case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Only elements with this role, such as "button" or "slider".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Where to put the pointer: an element by label or id (from ui_inspect), or a point in the window.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiTarget {
    /// The element's label, such as "Kick step 5".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The element's id, as ui_inspect lists it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A point in the window, in points from its top left corner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiButton {
    /// The left button.
    #[default]
    Primary,
    /// The right button.
    Secondary,
    /// The middle button.
    Middle,
}

/// Modifier keys held during an input.
#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiModifiers {
    /// Shift.
    #[serde(default)]
    pub shift: bool,
    /// Control.
    #[serde(default)]
    pub ctrl: bool,
    /// Alt (Option on macOS).
    #[serde(default)]
    pub alt: bool,
    /// Cmd on macOS, Ctrl elsewhere.
    #[serde(default)]
    pub cmd: bool,
}

/// `ui_click`.
#[derive(Clone, Debug, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiClickArgs {
    /// What to click.
    pub target: UiTarget,
    /// The button; the left one by default.
    #[serde(default)]
    pub button: UiButton,
    /// Modifier keys to hold.
    #[serde(default)]
    pub modifiers: UiModifiers,
    /// Click twice.
    #[serde(default)]
    pub double: bool,
}

/// `ui_drag`.
#[derive(Clone, Debug, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiDragArgs {
    /// Where the drag starts.
    pub from: UiTarget,
    /// Where it ends.
    pub to: UiTarget,
    /// Pointer moves between the two, one per frame; 8 by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    /// The button; the left one by default.
    #[serde(default)]
    pub button: UiButton,
    /// Modifier keys to hold.
    #[serde(default)]
    pub modifiers: UiModifiers,
}

/// `ui_key`.
#[derive(Clone, Debug, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiKeyArgs {
    /// The key, such as "Space", "Enter", "Delete", "Z" or "ArrowLeft".
    pub key: String,
    /// Modifier keys to hold.
    #[serde(default)]
    pub modifiers: UiModifiers,
}

/// `ui_text`.
#[derive(Clone, Debug, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiTextArgs {
    /// The text to type into the focused field.
    pub text: String,
}
