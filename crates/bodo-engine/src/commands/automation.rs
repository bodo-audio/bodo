//! `automation.*`: automation clips and pattern lanes (project.md section 10.5).
//!
//! An automation clip is a channel of the automation kind, whose lane holds the points, and clips of it on the
//! playlist. Points sit at ticks from the clip's start, with values from 0 to 1: for a volume, 0.8 is 0 dB (see
//! `ParamTarget::value_of`). A pattern can carry its own lanes, which repeat with it.

use std::sync::Arc;

use bodo_kit_core::Tick;
use bodo_model::automation::{AutoPoint, AutomationLane, ChannelParam, CurveShape, InsertParam, ParamTarget};
use bodo_model::channel::ChannelKind;
use bodo_model::mixer::{RouteKey, RouteSource};
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{Channel, ChannelId, Clip, Color, Project, TrackId};
use imbl::Vector;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{field, insert_number, unique_name};
use crate::error::{CommandError, ErrorCode};
use crate::params::{ChannelRef, InsertRef, Length, PatternRef, Position, RouteFromRef, TrackRef, format_position};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

/// The color of new automation channels (project.md section 14.2).
const AUTOMATION_COLOR: Color = Color::rgb(0x22, 0xd3, 0xee);

/// A route by its two ends.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteSpec {
    /// Where the route starts: a channel or an insert.
    from: RouteFromRef,
    /// The insert it goes to.
    to: InsertRef,
}

/// What a lane controls: an insert or channel parameter, or a route's send level.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetSpec {
    /// An insert, with param "volume" (its fader), "pan" or "stereoSep".
    insert: Option<InsertRef>,
    /// A channel, with param "volume", "pan", "pitch", "cc:N" (a MIDI CC, 0 to 127), "pitchBend" or "pressure".
    channel: Option<ChannelRef>,
    /// A route, whose send level the lane controls.
    route: Option<RouteSpec>,
    /// The insert's or channel's parameter; "volume" by default.
    param: Option<String>,
}

/// A target, or "tempo".
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum Target {
    /// "tempo".
    Named(String),
    /// An insert, a channel or a route.
    Spec(TargetSpec),
}

/// How the value moves from a point to the next one.
#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Shape {
    /// Holds the value until the next point.
    Hold,
    /// A straight line.
    #[default]
    Linear,
    /// An exponential bend; the tension, -1 to 1, rises late when positive: `{"exp": 0.4}`.
    Exp(f32),
    /// An S curve; the tension, -1 to 1, makes it steeper: `{"sCurve": 0.5}`.
    SCurve(f32),
    /// A staircase with this many steps: `{"stairs": 4}`.
    Stairs(u8),
    /// Half a cosine.
    Sine,
    /// Jumps halfway between the points.
    Pulse,
}

impl From<Shape> for CurveShape {
    fn from(curve: Shape) -> Self {
        match curve {
            Shape::Hold => Self::Hold,
            Shape::Linear => Self::Linear,
            Shape::Exp(t) => Self::Exp(t.clamp(-1.0, 1.0)),
            Shape::SCurve(t) => Self::SCurve(t.clamp(-1.0, 1.0)),
            Shape::Stairs(n) => Self::Stairs(n.max(1)),
            Shape::Sine => Self::Sine,
            Shape::Pulse => Self::Pulse,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PointSpec {
    /// Where the point is, from the start of the clip (or the pattern).
    at: Position,
    /// The value, 0 to 1; for a volume, 0.8 is 0 dB and 1 about +6 dB.
    value: f32,
    /// How the value moves on to the next point: `"linear"` (the default), `"hold"`, `"sine"`, `"pulse"`,
    /// `{"exp": t}` or `{"sCurve": t}` with t from -1 to 1, or `{"stairs": n}`.
    #[serde(default)]
    curve: Shape,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateFor {
    /// What the clip controls: `"tempo"`, `{"insert": 1, "param": "volume"}` (or pan, stereoSep),
    /// `{"channel": "Kick", "param": "pan"}` (or volume, pitch, cc:74, pitchBend, pressure), or
    /// `{"route": {"from": "insert:1", "to": "insert:5"}}`.
    target: Target,
    /// Where the clip starts; the start of the song by default.
    start: Option<Position>,
    /// How long it lasts; 4 bars by default.
    length: Option<Length>,
    /// The track it goes on; by default the first track with room for it.
    track: Option<TrackRef>,
    /// The automation channel's name; the target's by default, such as "Insert 1 volume".
    name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetPoints {
    /// The automation channel.
    channel: ChannelRef,
    /// Every point, in any order; they are sorted by time.
    points: Vec<PointSpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddPoint {
    /// The automation channel.
    channel: ChannelRef,
    /// Where the point is, from the clip's start.
    at: Position,
    /// The value, 0 to 1.
    value: f32,
    /// How the value moves on to the next point: `"linear"` (the default), `"hold"`, `"sine"`, `"pulse"`,
    /// `{"exp": t}` or `{"sCurve": t}` with t from -1 to 1, or `{"stairs": n}`.
    #[serde(default)]
    curve: Shape,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemovePoint {
    /// The automation channel.
    channel: ChannelRef,
    /// The point's index, from 0 in time order.
    index: Option<usize>,
    /// Or: the time of the point to remove.
    at: Option<Position>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetCurve {
    /// The automation channel.
    channel: ChannelRef,
    /// The point's index, from 0 in time order; the curve runs from it to the next point.
    index: usize,
    /// The curve: `"linear"`, `"hold"`, `"sine"`, `"pulse"`, `{"exp": t}` or `{"sCurve": t}` with t from -1 to 1,
    /// or `{"stairs": n}`.
    curve: Shape,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Clear {
    /// The automation channel.
    channel: ChannelRef,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaceClip {
    /// The automation channel.
    channel: ChannelRef,
    /// Where the clip starts.
    start: Position,
    /// How long it lasts; up to the lane's last point by default, or 4 bars when every point is at its start.
    length: Option<Length>,
    /// The track; the selected track (or the first) by default.
    track: Option<TrackRef>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetLane {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// What the lane controls, as in automation.create_for.
    target: Target,
    /// Every point, from the pattern's start; they are sorted by time.
    points: Vec<PointSpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveLane {
    /// The pattern; the current pattern by default.
    pattern: Option<PatternRef>,
    /// What the lane controls, as in automation.create_for.
    target: Target,
}

/// The parameter a target names.
fn resolve(ctx: &Ctx<'_>, target: &Target) -> Result<ParamTarget, CommandError> {
    let spec = match target {
        Target::Named(name) if name.trim().eq_ignore_ascii_case("tempo") => return Ok(ParamTarget::Tempo),
        Target::Named(name) => {
            let hint = r#"use "tempo", {"insert": 1, "param": "volume"}, {"channel": "Kick", "param": "pan"} or {"route": {...}}"#;
            return Err(CommandError::invalid(format!("`{name}` is not a target")).with_hint(hint));
        }
        Target::Spec(spec) => spec,
    };
    let param = spec.param.as_deref().map(str::trim).unwrap_or("volume");
    let unknown = |what: &str, allowed: &str| {
        CommandError::invalid(format!("`{param}` is not a parameter of {what}")).with_hint(format!("use one of {allowed}"))
    };
    match (&spec.insert, &spec.channel, &spec.route) {
        (Some(insert), None, None) => {
            let id = ctx.insert(Some(insert))?;
            let param = match param.to_ascii_lowercase().as_str() {
                "volume" => InsertParam::Volume,
                "pan" => InsertParam::Pan,
                "stereosep" => InsertParam::StereoSep,
                _ => return Err(unknown("an insert", "volume, pan or stereoSep")),
            };
            Ok(ParamTarget::Insert(id, param))
        }
        (None, Some(channel), None) => {
            let id = ctx.channel(Some(channel))?;
            let lower = param.to_ascii_lowercase();
            let param = match lower.as_str() {
                "volume" => ChannelParam::Volume,
                "pan" => ChannelParam::Pan,
                "pitch" => ChannelParam::Pitch,
                "pitchbend" => ChannelParam::PitchBend,
                "pressure" => ChannelParam::Pressure,
                cc => match cc.strip_prefix("cc:").and_then(|n| n.trim().parse::<u8>().ok()).filter(|n| *n <= 127) {
                    Some(n) => ChannelParam::Cc(n),
                    None => return Err(unknown("a channel", "volume, pan, pitch, cc:N, pitchBend or pressure")),
                },
            };
            Ok(ParamTarget::Channel(id, param))
        }
        (None, None, Some(route)) => {
            let from = ctx.route_source(&route.from)?;
            let to = ctx.insert(Some(&route.to))?;
            let key = RouteKey { from, to };
            if !ctx.project.mixer.routes.contains_key(&key) {
                return Err(CommandError::not_found("route", "between those two").with_hint("add it first with mixer.add_route"));
            }
            Ok(ParamTarget::Route(key))
        }
        _ => Err(CommandError::invalid("give exactly one of insert, channel or route")),
    }
}

/// A name for what `target` controls, such as "Insert 1 volume" or "Kick pan".
fn describe(project: &Project, target: &ParamTarget) -> String {
    let insert = |id| match insert_number(project, id) {
        Some(0) => String::from("Master"),
        Some(n) => format!("Insert {n}"),
        None => String::from("Insert"),
    };
    let channel = |id: &ChannelId| project.channels.get(id).map_or_else(|| String::from("Channel"), |c| c.name.clone());
    match target {
        ParamTarget::Insert(id, InsertParam::Volume) => format!("{} volume", insert(*id)),
        ParamTarget::Insert(id, InsertParam::Pan) => format!("{} pan", insert(*id)),
        ParamTarget::Insert(id, InsertParam::StereoSep) => format!("{} stereo separation", insert(*id)),
        ParamTarget::Channel(id, ChannelParam::Volume) => format!("{} volume", channel(id)),
        ParamTarget::Channel(id, ChannelParam::Pan) => format!("{} pan", channel(id)),
        ParamTarget::Channel(id, ChannelParam::Pitch) => format!("{} pitch", channel(id)),
        ParamTarget::Channel(id, ChannelParam::Cc(n)) => format!("{} CC {n}", channel(id)),
        ParamTarget::Channel(id, ChannelParam::PitchBend) => format!("{} pitch bend", channel(id)),
        ParamTarget::Channel(id, ChannelParam::Pressure) => format!("{} pressure", channel(id)),
        ParamTarget::Route(key) => {
            let from = match key.from {
                RouteSource::Channel(c) => channel(&c),
                RouteSource::Insert(i) => insert(i),
            };
            format!("{from} send to {}", insert(key.to))
        }
        ParamTarget::Slot(id, slot, _) => format!("{} slot {}", insert(*id), slot + 1),
        ParamTarget::Plugin(..) => String::from("Plugin parameter"),
        ParamTarget::Tempo => String::from("Tempo"),
    }
}

/// The target's current value, normalized: where a new clip or lane starts.
fn current(project: &Project, target: &ParamTarget) -> f32 {
    let real = match target {
        ParamTarget::Insert(id, param) => project.mixer.insert(*id).map_or(0.0, |i| match param {
            InsertParam::Volume => i.volume_db,
            InsertParam::Pan => i.pan,
            InsertParam::StereoSep => i.stereo_sep,
        }),
        ParamTarget::Channel(id, param) => project.channels.get(id).map_or(0.0, |c| match param {
            ChannelParam::Volume => c.volume,
            ChannelParam::Pan => c.pan,
            ChannelParam::Pitch => c.pitch_cents,
            ChannelParam::PitchBend => 0.5,
            ChannelParam::Cc(_) | ChannelParam::Pressure => 0.0,
        }),
        ParamTarget::Route(key) => project.mixer.routes.get(key).map_or(1.0, |r| r.send_level),
        ParamTarget::Tempo => project.tempo.bpm_at(Tick::ZERO) as f32,
        ParamTarget::Slot(..) | ParamTarget::Plugin(..) => 0.5,
    };
    target.normalized_of(real)
}

/// The points `specs` describe, checked and sorted by time.
fn points(project: &Project, specs: &[PointSpec]) -> Result<Vector<AutoPoint>, CommandError> {
    let mut points = Vec::with_capacity(specs.len());
    for spec in specs {
        points.push(point(project, &spec.at, spec.value, spec.curve)?);
    }
    points.sort_by_key(|p| p.at);
    Ok(points.into_iter().collect())
}

fn point(project: &Project, at: &Position, value: f32, curve: Shape) -> Result<AutoPoint, CommandError> {
    let at = at.resolve(project)?;
    if at < Tick::ZERO {
        return Err(CommandError::invalid("a point can't come before its clip's start"));
    }
    super::in_range("value", f64::from(value), 0.0..=1.0)?;
    Ok(AutoPoint { at, value, curve: curve.into() })
}

/// The lane of automation channel `id`.
fn lane_of(ctx: &Ctx<'_>, id: ChannelId) -> Result<Arc<AutomationLane>, CommandError> {
    match ctx.project.channels.get(&id).map(|c| &c.kind) {
        Some(ChannelKind::Automation(lane)) => Ok(Arc::clone(lane)),
        Some(_) => Err(CommandError::rejected(format!(
            "{} is not an automation channel",
            ctx.project.channels.get(&id).map(|c| c.name.as_str()).unwrap_or_default()
        ))
        .with_hint("make one with automation.create_for")),
        None => Err(CommandError::not_found("channel", &id.to_string())),
    }
}

/// Replaces the lane of automation channel `id` with what `f` makes of it.
fn edit_lane(
    ctx: &mut Ctx<'_>,
    id: ChannelId,
    f: impl FnOnce(&mut AutomationLane) -> Result<(), CommandError>,
) -> Result<Arc<AutomationLane>, CommandError> {
    let mut lane = (*lane_of(ctx, id)?).clone();
    f(&mut lane)?;
    let lane = Arc::new(lane);
    let kind = ChannelKind::Automation(Arc::clone(&lane));
    ctx.edit_channel(id, |c| c.kind = kind)?;
    Ok(lane)
}

/// `value` to four decimals, without the noise of widening an `f32`.
fn short(value: f32) -> f64 {
    (f64::from(value) * 1e4).round() / 1e4
}

/// A curve as the commands take it.
fn curve_json(curve: CurveShape) -> Value {
    match curve {
        CurveShape::Hold => json!("hold"),
        CurveShape::Linear => json!("linear"),
        CurveShape::Exp(t) => json!({ "exp": short(t) }),
        CurveShape::SCurve(t) => json!({ "sCurve": short(t) }),
        CurveShape::Stairs(n) => json!({ "stairs": n }),
        CurveShape::Sine => json!("sine"),
        CurveShape::Pulse => json!("pulse"),
    }
}

fn lane_json(project: &Project, lane: &AutomationLane) -> Value {
    let points: Vec<Value> = lane
        .points
        .iter()
        .map(|p| json!({ "at": format_position(project, p.at), "ticks": p.at.0, "value": short(p.value), "curve": curve_json(p.curve) }))
        .collect();
    json!({ "target": describe(project, &lane.target), "points": points })
}

/// The first track, in order, with no clip between `start` and `end`.
///
/// Complexity: O(n) in clips on the playlist.
fn free_track(project: &Project, start: Tick, end: Tick) -> Option<TrackId> {
    project
        .track_order
        .iter()
        .copied()
        .find(|id| project.tracks.get(id).is_some_and(|t| !t.locked && t.clips.iter().all(|c| c.end() <= start || c.start >= end)))
}

fn create_for(ctx: &mut Ctx<'_>, p: CreateFor) -> Result<Value, CommandError> {
    let target = resolve(ctx, &p.target)?;
    let start = p.start.as_ref().map(|s| s.resolve(&ctx.project)).transpose()?.unwrap_or(Tick::ZERO);
    if start < Tick::ZERO {
        return Err(CommandError::invalid("a clip can't start before the song"));
    }
    let length = p.length.unwrap_or_else(|| Length::Text(String::from("4 bars"))).resolve(&ctx.project, start)?;
    let end = Tick(start.0 + length.0);
    let track = match &p.track {
        Some(r) => ctx.track(Some(r))?,
        None => match free_track(&ctx.project, start, end) {
            Some(track) => track,
            None => ctx.track(None)?,
        },
    };
    if ctx.project.tracks.get(&track).is_some_and(|t| t.locked) {
        return Err(CommandError::rejected("that track is locked").with_hint("unlock it with playlist.set_track_props"));
    }
    let value = current(&ctx.project, &target);
    let base = p.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| describe(&ctx.project, &target));
    let name = unique_name(base.trim(), ctx.project.channels.values().map(|c| c.name.as_str()));
    let lane = AutomationLane {
        target,
        points: Vector::from(vec![
            AutoPoint { at: Tick::ZERO, value, curve: CurveShape::Linear },
            AutoPoint { at: length, value, curve: CurveShape::Linear },
        ]),
        default: value,
    };
    let id = ctx.ids().channel();
    ctx.project.channels.insert(
        id,
        Arc::new(Channel {
            id,
            name: name.clone(),
            color: AUTOMATION_COLOR,
            kind: ChannelKind::Automation(Arc::new(lane)),
            target: None,
            volume: 1.0,
            pan: 0.0,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
        }),
    );
    ctx.project.rack_order.push_back(id);
    let clip = ctx.ids().clip();
    ctx.edit_track(track, |t| {
        t.insert_clip(Clip {
            id: clip,
            start,
            len: length,
            offset: ClipOffset::default(),
            muted: false,
            color: None,
            source: ClipSource::Automation(id),
            fades: None,
        });
        Ok(())
    })?;
    ctx.editor.clips = std::collections::BTreeSet::from([clip]);
    ctx.editor.track = Some(track);
    ctx.set_label(format!("Automate {}", describe(&ctx.project, &target)));
    Ok(json!({ "channel": id, "name": name, "clip": clip, "track": track, "start": format_position(&ctx.project, start), "value": short(value) }))
}

fn place_clip(ctx: &mut Ctx<'_>, p: PlaceClip) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let lane = lane_of(ctx, id)?;
    let start = p.start.resolve(&ctx.project)?;
    if start < Tick::ZERO {
        return Err(CommandError::invalid("a clip can't start before the song"));
    }
    let length = match (&p.length, lane.points.last().map(|q| q.at).filter(|at| *at > Tick::ZERO)) {
        (Some(length), _) => length.resolve(&ctx.project, start)?,
        (None, Some(last)) => last,
        (None, None) => Length::Text(String::from("4 bars")).resolve(&ctx.project, start)?,
    };
    if length <= Tick::ZERO {
        return Err(CommandError::invalid("a clip needs a length"));
    }
    let track = ctx.track(p.track.as_ref())?;
    if ctx.project.tracks.get(&track).is_some_and(|t| t.locked) {
        return Err(CommandError::rejected("that track is locked").with_hint("unlock it with playlist.set_track_props"));
    }
    let clip = ctx.ids().clip();
    ctx.edit_track(track, |t| {
        t.insert_clip(Clip {
            id: clip,
            start,
            len: length,
            offset: ClipOffset::default(),
            muted: false,
            color: None,
            source: ClipSource::Automation(id),
            fades: None,
        });
        Ok(())
    })?;
    ctx.editor.clips = std::collections::BTreeSet::from([clip]);
    ctx.editor.track = Some(track);
    let name = ctx.project.channels.get(&id).map(|c| c.name.clone()).unwrap_or_default();
    let at = format_position(&ctx.project, start);
    ctx.set_label(format!("Place {name} at {at}"));
    Ok(json!({ "clip": clip, "track": track, "channel": id, "start": at, "ticks": length.0 }))
}

fn set_points(ctx: &mut Ctx<'_>, p: SetPoints) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let points = points(&ctx.project, &p.points)?;
    let lane = edit_lane(ctx, id, |lane| {
        lane.points = points;
        Ok(())
    })?;
    ctx.set_label("Edit automation");
    Ok(lane_json(&ctx.project, &lane))
}

fn add_point(ctx: &mut Ctx<'_>, p: AddPoint) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let new = point(&ctx.project, &p.at, p.value, p.curve)?;
    let mut index = 0;
    let lane = edit_lane(ctx, id, |lane| {
        index = lane.points.iter().take_while(|q| q.at <= new.at).count();
        lane.points.insert(index, new);
        Ok(())
    })?;
    ctx.set_label("Add automation point");
    Ok(json!({ "index": index, "lane": lane_json(&ctx.project, &lane) }))
}

fn remove_point(ctx: &mut Ctx<'_>, p: RemovePoint) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let at = p.at.as_ref().map(|a| a.resolve(&ctx.project)).transpose()?;
    let lane = edit_lane(ctx, id, |lane| {
        let index = match (p.index, at) {
            (Some(index), _) => Some(index).filter(|i| *i < lane.points.len()),
            (None, Some(at)) => lane.points.iter().position(|q| q.at == at),
            (None, None) => return Err(CommandError::invalid("give index or at")),
        };
        let index =
            index.ok_or_else(|| CommandError::new(ErrorCode::NotFound, "no point there").with_hint("list the points with inspect_project"))?;
        lane.points.remove(index);
        Ok(())
    })?;
    ctx.set_label("Remove automation point");
    Ok(lane_json(&ctx.project, &lane))
}

fn set_curve(ctx: &mut Ctx<'_>, p: SetCurve) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let lane = edit_lane(ctx, id, |lane| {
        let point = lane.points.get_mut(p.index).ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("there is no point {}", p.index)))?;
        point.curve = p.curve.into();
        Ok(())
    })?;
    ctx.set_label("Set automation curve");
    Ok(lane_json(&ctx.project, &lane))
}

fn clear(ctx: &mut Ctx<'_>, p: Clear) -> Result<Value, CommandError> {
    let id = ctx.channel(Some(&p.channel))?;
    let lane = edit_lane(ctx, id, |lane| {
        lane.points.clear();
        Ok(())
    })?;
    ctx.set_label("Clear automation");
    Ok(lane_json(&ctx.project, &lane))
}

fn set_lane(ctx: &mut Ctx<'_>, p: SetLane) -> Result<Value, CommandError> {
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let target = resolve(ctx, &p.target)?;
    let points = points(&ctx.project, &p.points)?;
    let lane = AutomationLane { target, points, default: current(&ctx.project, &target) };
    let json = lane_json(&ctx.project, &lane);
    ctx.edit_pattern(pattern, |pattern| {
        let lane = Arc::new(lane);
        match pattern.automation.iter().position(|l| l.target == target) {
            Some(index) => {
                pattern.automation.set(index, lane);
            }
            None => pattern.automation.push_back(lane),
        }
        Ok((true, ()))
    })?;
    ctx.set_label(format!("Automate {} in the pattern", describe(&ctx.project, &target)));
    Ok(json!({ "pattern": pattern, "lane": json }))
}

fn remove_lane(ctx: &mut Ctx<'_>, p: RemoveLane) -> Result<Value, CommandError> {
    let pattern = ctx.pattern(p.pattern.as_ref())?;
    let target = resolve(ctx, &p.target)?;
    let name = describe(&ctx.project, &target);
    ctx.edit_pattern(pattern, |pattern| {
        let index = pattern
            .automation
            .iter()
            .position(|l| l.target == target)
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("{} has no lane for {name}", pattern.name)))?;
        pattern.automation.remove(index);
        Ok((true, ()))
    })?;
    ctx.set_label(format!("Remove the {name} lane"));
    Ok(json!({ "pattern": pattern, "removed": name }))
}

/// Merge key: the automation channel.
fn by_channel(params: &Value) -> String {
    field(params, "channel")
}

/// Merge key: the pattern and the lane's target.
fn by_lane(params: &Value) -> String {
    format!("{}{}", field(params, "pattern"), field(params, "target"))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "automation.create_for",
        "Create automation clip",
        "Makes an automation channel for a parameter and a clip of it on the playlist, starting at the parameter's current value.",
        Effects::UNDOABLE,
        create_for,
    )
    .example(json!({ "target": { "insert": 1, "param": "volume" }, "length": "4 bars" }));
    r.edit(
        "automation.place_clip",
        "Place automation clip",
        "Places another clip of an automation channel on the playlist; every clip of the channel plays its one lane.",
        Effects::UNDOABLE,
        place_clip,
    )
    .example(json!({ "channel": "Insert 1 volume", "start": "5:1:0", "track": 2 }));
    r.edit(
        "automation.set_points",
        "Edit automation",
        "Replaces an automation channel's points; times are from the clip's start, values 0 to 1.",
        Effects::UNDOABLE,
        set_points,
    )
    .example(json!({
        "channel": "Insert 1 volume",
        "points": [{ "at": 0, "value": 0.8 }, { "at": "3:1:0", "value": 0.4, "curve": { "exp": 0.3 } }, { "at": "5:1:0", "value": 0.8 }]
    }))
    .merge(by_channel);
    r.edit("automation.add_point", "Add automation point", "Adds a point to an automation channel.", Effects::UNDOABLE, add_point)
        .example(json!({ "channel": "Insert 1 volume", "at": "2:1:0", "value": 0.6 }))
        .merge(by_channel);
    r.edit(
        "automation.remove_point",
        "Remove automation point",
        "Removes a point from an automation channel, by index or time.",
        Effects::UNDOABLE,
        remove_point,
    )
    .example(json!({ "channel": "Insert 1 volume", "index": 1 }));
    r.edit("automation.set_curve", "Set automation curve", "Sets how the value moves from a point to the next one.", Effects::UNDOABLE, set_curve)
        .example(json!({ "channel": "Insert 1 volume", "index": 0, "curve": "sine" }))
        .merge(by_channel);
    r.edit("automation.clear", "Clear automation", "Removes every point of an automation channel.", Effects::UNDOABLE, clear)
        .example(json!({ "channel": "Insert 1 volume" }));
    r.edit(
        "automation.set_lane",
        "Set pattern lane",
        "Sets a pattern's own lane for a parameter, which repeats with the pattern.",
        Effects::UNDOABLE,
        set_lane,
    )
    .example(json!({
        "pattern": "Verse",
        "target": { "channel": "HiHat", "param": "pan" },
        "points": [{ "at": 0, "value": 0.3 }, { "at": "1:3:0", "value": 0.7 }]
    }))
    .merge(by_lane);
    r.edit("automation.remove_lane", "Remove pattern lane", "Removes a pattern's lane for a parameter.", Effects::UNDOABLE, remove_lane)
        .example(json!({ "pattern": "Verse", "target": { "channel": "HiHat", "param": "pan" } }));
}
