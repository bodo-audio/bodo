//! Presets dragged out of the Browser and dropped on a window (project.md section 14.1, ADR 0029).
//!
//! Where a preset lands decides what it does:
//!
//! - a **channel preset** on a channel rack row replaces that channel; anywhere else on the channel rack it becomes a
//!   new channel;
//! - a **mixer preset** on a mixer strip loads on that insert; anywhere else on the mixer, on the selected insert;
//! - a **score** on a channel rack row adds its notes to that channel; on the piano roll, to the piano roll's channel.
//!
//! The targets are found from the elements every window records for agents and tests, so the windows need no code
//! of their own for it.

use bodo_engine::library::EntryKind;
use serde_json::json;

use super::FilePayload;
use crate::windows::{WindowKind, area_id};
use crate::{BodoApp, control};

/// Applies a preset dropped this frame. Call after every window has been drawn.
pub(crate) fn handle_drops(app: &mut BodoApp, ctx: &egui::Context) {
    let Some(payload) = egui::DragAndDrop::payload::<FilePayload>(ctx) else { return };
    let released = ctx.input(|i| i.pointer.any_released());
    let Some(at) = ctx.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos())) else { return };
    if !released {
        return;
    }
    egui::DragAndDrop::clear_payload(ctx);
    let window = ctx.layer_id_at(at).and_then(|layer| WindowKind::ALL.into_iter().find(|kind| layer.id == area_id(*kind)));
    let elements = control::elements(ctx);
    let under = |role: &str| elements.iter().rev().find(|e| e.role == role && e.rect.contains(at)).map(|e| e.label.clone());
    let path = payload.path.display().to_string();
    let call = match (payload.kind, window) {
        (EntryKind::ChannelPreset, Some(WindowKind::ChannelRack)) => match under("channel") {
            Some(channel) => Some(("preset.load_channel", json!({ "path": path, "channel": channel }))),
            None => Some(("preset.load_channel", json!({ "path": path }))),
        },
        (EntryKind::MixerPreset, Some(WindowKind::Mixer)) => match under("strip").and_then(|s| s.strip_suffix(" strip").map(str::to_owned)) {
            Some(insert) => Some(("preset.load_mixer", json!({ "path": path, "insert": insert }))),
            None => Some(("preset.load_mixer", json!({ "path": path }))),
        },
        (EntryKind::Score, Some(WindowKind::ChannelRack)) => {
            under("channel").map(|channel| ("preset.load_score", json!({ "path": path, "channel": channel })))
        }
        (EntryKind::Score, Some(WindowKind::PianoRoll)) => Some(("preset.load_score", json!({ "path": path }))),
        _ => None,
    };
    if let Some((id, params)) = call {
        let _ = app.run(id, params);
    }
}
