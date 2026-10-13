//! `transport.*`: play, stop, the song position, the metronome and count-in (project.md section 14.1).
//!
//! The session only records what the transport should do and queues events; whoever plays audio carries them out and
//! reports the playhead back. Stopping returns to the start, as FL users expect.

use bodo_kit_core::Tick;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::params::{Position, format_position};
use crate::registry::{Effects, NoParams, Registry};
use crate::session::Ctx;
use crate::state::{Event, PlayMode};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPosition {
    /// The new song position.
    at: Position,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetMode {
    /// "pattern" loops the current pattern; "song" plays the playlist.
    mode: PlayMode,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Switch {
    /// On or off; toggles when left out.
    on: Option<bool>,
}

fn metronome(ctx: &mut Ctx<'_>, p: Switch) -> Value {
    let on = p.on.unwrap_or(!ctx.transport.metronome);
    ctx.transport.metronome = on;
    ctx.emit(Event::Metronome(on));
    json!({ "metronome": on })
}

fn count_in(ctx: &mut Ctx<'_>, p: Switch) -> Value {
    let on = p.on.unwrap_or(!ctx.transport.count_in);
    ctx.transport.count_in = on;
    ctx.emit(Event::CountIn(on));
    json!({ "countIn": on })
}

fn state(ctx: &Ctx<'_>) -> Value {
    json!({ "playing": ctx.transport.playing, "mode": ctx.transport.mode, "position": format_position(&ctx.project, ctx.transport.position) })
}

fn play(ctx: &mut Ctx<'_>) -> Value {
    ctx.transport.playing = true;
    ctx.emit(Event::Play);
    state(ctx)
}

fn stop(ctx: &mut Ctx<'_>) -> Value {
    ctx.transport.playing = false;
    ctx.transport.recording = false;
    ctx.transport.position = Tick::ZERO;
    ctx.emit(Event::Stop);
    ctx.emit(Event::Seek(Tick::ZERO));
    state(ctx)
}

fn set_position(ctx: &mut Ctx<'_>, p: SetPosition) -> Result<Value, CommandError> {
    let at = p.at.resolve(&ctx.project)?.max(Tick::ZERO);
    ctx.transport.position = at;
    ctx.emit(Event::Seek(at));
    Ok(state(ctx))
}

fn set_mode(ctx: &mut Ctx<'_>, p: SetMode) -> Result<Value, CommandError> {
    if ctx.transport.mode != p.mode {
        ctx.transport.mode = p.mode;
        ctx.transport.position = Tick::ZERO;
        ctx.emit(Event::Seek(Tick::ZERO));
    }
    Ok(state(ctx))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("transport.play", "Play", "Starts playing from the song position.", Effects::NONE, |ctx, _: NoParams| Ok(play(ctx))).example(json!({}));
    r.edit("transport.stop", "Stop", "Stops playing and returns to the start.", Effects::NONE, |ctx, _: NoParams| Ok(stop(ctx))).example(json!({}));
    r.edit("transport.toggle_play", "Play or stop", "Plays when stopped and stops when playing.", Effects::NONE, |ctx, _: NoParams| {
        Ok(if ctx.transport.playing { stop(ctx) } else { play(ctx) })
    })
    .example(json!({}))
    .shortcut("Space");
    r.edit("transport.set_position", "Set song position", "Moves the playhead.", Effects::NONE, set_position).example(json!({ "at": "2:1:0" }));
    r.edit(
        "transport.set_mode",
        "Set play mode",
        "Chooses Pattern mode (loop the current pattern) or Song mode (play the playlist).",
        Effects::NONE,
        set_mode,
    )
    .example(json!({ "mode": "song" }));
    r.edit("transport.metronome", "Metronome", "Turns the metronome on or off; it clicks every beat while playing.", Effects::NONE, |ctx, p| {
        Ok(metronome(ctx, p))
    })
    .example(json!({ "on": true }));
    r.edit("transport.count_in", "Count-in", "Turns count-in on or off: recording starts after a bar of clicks.", Effects::NONE, |ctx, p| {
        Ok(count_in(ctx, p))
    })
    .example(json!({}));
}
