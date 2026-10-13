//! Automation hooks: interactive elements record their egui id, label, role and rectangle each frame, so agents and
//! tests can find and drive them through the control channel (`ui.elements`, `ui.click`). The approach follows
//! pixel3d's.

use serde_json::{Value, json};

/// Most elements recorded per frame.
const MAX_ELEMENTS: usize = 8192;

/// An element on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// Its egui id.
    pub id: egui::Id,
    /// What it says or does, such as "Kick step 5".
    pub label: String,
    /// What kind of element it is: button, step, knob, clip and so on.
    pub role: &'static str,
    /// Where it is, in points.
    pub rect: egui::Rect,
    /// Whether it takes input.
    pub enabled: bool,
    /// Whether it is on or selected, for toggles.
    pub selected: Option<bool>,
}

fn key() -> egui::Id {
    egui::Id::new("bodo-elements")
}

/// Forgets last frame's elements. Call at the start of each frame.
pub fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Vec<Element>>(key()));
}

/// Records an element drawn with `response`, if it is visible. Never changes focus or input.
pub fn record(ui: &egui::Ui, response: &egui::Response, label: &str, role: &'static str, selected: Option<bool>) {
    describe(ui, response, label, role, selected);
    record_rect(ui, response.id, response.rect, label, role, selected, response.enabled());
}

/// Tells screen readers what a custom element is when it said nothing itself, such as a channel's name button or a
/// marker on the ruler: its label, in the interface's language where the catalogs have it, and a kind from `role`.
/// Widgets that describe themselves, such as egui's buttons and Kit's knobs, keep what they said.
fn describe(ui: &egui::Ui, response: &egui::Response, label: &str, role: &str, selected: Option<bool>) {
    let silent = ui.ctx().accesskit_node_builder(response.id, |node| node.label().is_none() && node.value().is_none());
    if silent != Some(true) {
        return;
    }
    let kind = match role {
        "knob" | "slider" | "fader" | "point" | "marker" | "splitter" => egui::WidgetType::Slider,
        "checkbox" | "led" | "step" => egui::WidgetType::Checkbox,
        "combo" => egui::WidgetType::ComboBox,
        "number" | "value" | "field" => egui::WidgetType::DragValue,
        "text" => egui::WidgetType::TextEdit,
        "label" | "badge" => egui::WidgetType::Label,
        "canvas" | "graph" | "ruler" | "lane" | "keyboard" | "strip" => egui::WidgetType::Other,
        _ => egui::WidgetType::Button,
    };
    let shown = crate::i18n::lookup(label).unwrap_or(label);
    let enabled = response.enabled();
    response.widget_info(|| match selected {
        Some(on) => egui::WidgetInfo::selected(kind, enabled, on, shown),
        None => egui::WidgetInfo::labeled(kind, enabled, shown),
    });
}

/// Records an element at `rect` that has no response of its own, such as one step of a step row.
pub fn record_rect(ui: &egui::Ui, id: egui::Id, rect: egui::Rect, label: &str, role: &'static str, selected: Option<bool>, enabled: bool) {
    if !rect.is_finite() || !rect.is_positive() || !ui.is_rect_visible(rect) {
        return;
    }
    let element = Element { id, label: label.chars().take(256).collect(), role, rect: rect.intersect(ui.clip_rect()), enabled, selected };
    ui.ctx().data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Vec<Element>>(key());
        if list.len() < MAX_ELEMENTS {
            list.push(element);
        }
    });
}

/// This frame's elements, the last record of each id, in drawing order.
///
/// Complexity: O(n log n) in recorded elements.
pub fn elements(ctx: &egui::Context) -> Vec<Element> {
    let list = ctx.data_mut(|d| d.get_temp::<Vec<Element>>(key()).unwrap_or_default());
    let mut last: std::collections::HashMap<egui::Id, usize> = std::collections::HashMap::with_capacity(list.len());
    for (index, element) in list.iter().enumerate() {
        last.insert(element.id, index);
    }
    let mut keep: Vec<usize> = last.into_values().collect();
    keep.sort_unstable();
    keep.into_iter().filter_map(|i| list.get(i).cloned()).collect()
}

/// An element as JSON.
pub fn element_json(element: &Element) -> Value {
    json!({
        "id": format!("{:016x}", element.id.value()),
        "label": element.label,
        "role": element.role,
        "rect": [element.rect.left(), element.rect.top(), element.rect.width(), element.rect.height()],
        "enabled": element.enabled,
        "selected": element.selected,
    })
}
