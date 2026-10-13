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

fn loop_record(ctx: &mut Ctx<'_>, p: Switch) -> Value {
    let on = p.on.unwrap_or(!ctx.transport.loop_record);
    ctx.transport.loop_record = on;
    ctx.emit(Event::LoopRecord(on));
    json!({ "loopRecord": on })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetLoop {
    /// Where the loop starts.
    start: Option<Position>,
    /// Where it ends; playing wraps back to the start exactly here.
    end: Option<Position>,
    /// True to stop looping.
    #[serde(default)]
    off: bool,
}

fn set_loop(ctx: &mut Ctx<'_>, p: SetLoop) -> Result<Value, CommandError> {
    let region = match (p.off, &p.start, &p.end) {
        (true, None, None) => None,
        (false, Some(start), Some(end)) => {
            let (start, end) = (start.resolve(&ctx.project)?.max(Tick::ZERO), end.resolve(&ctx.project)?);
            if end <= start {
                return Err(CommandError::invalid("the loop has to end after it starts"));
            }
            Some((start, end))
        }
        _ => return Err(CommandError::invalid("give start and end, or off: true")),
    };
    ctx.transport.loop_region = region;
    ctx.emit(Event::Loop(region));
    let shown = region.map(|(start, end)| json!({ "start": format_position(&ctx.project, start), "end": format_position(&ctx.project, end) }));
    Ok(json!({ "loop": shown }))
}

fn state(ctx: &Ctx<'_>) -> Value {
    json!({
        "playing": ctx.transport.playing,
        "recording": ctx.transport.recording,
        "mode": ctx.transport.mode,
        "position": format_position(&ctx.project, ctx.transport.position),
    })
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

/// Starts recording the armed inserts while playing, or stops a recording.
fn record(ctx: &mut Ctx<'_>) -> Value {
    if ctx.transport.recording {
        return stop(ctx);
    }
    ctx.transport.recording = true;
    ctx.transport.playing = true;
    ctx.emit(Event::Record);
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
    r.edit(
        "transport.record",
        "Record",
        "Starts playing and recording the armed inserts, or stops a recording; the takes land on the playlist.",
        Effects::NONE,
        |ctx, _: NoParams| Ok(record(ctx)),
    )
    .example(json!({}));
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
    r.edit(
        "transport.loop_record",
        "Loop record",
        "Turns loop recording on or off: each pass of a loop becomes its own take, and MIDI passes add up.",
        Effects::NONE,
        |ctx, p| Ok(loop_record(ctx, p)),
    )
    .example(json!({ "on": true }));
    r.edit("transport.set_loop", "Set loop region", "Loops Song mode over a region of the playlist, or stops looping.", Effects::NONE, set_loop)
        .example(json!({ "start": "1:1:0", "end": "5:1:0" }));
}
