//! What audio and automation clips show on the playlist: an audio clip's waveform, read from its sample's peaks and
//! shaped by its fades, with a handle for each fade; an automation clip's curve, with a handle for each point.
//!
//! Both are culled to the visible part of the clip. A waveform costs O(w log n) for w visible points of width, and a
//! curve O(w log p + k) for its p points, k of them visible.

use bodo_kit_core::{PPQ, Tick};
use bodo_model::automation::{AutomationLane, CurveShape};
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{ChannelId, Clip, ClipId, Project};
use egui::epaint::Mesh;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use serde_json::{Value, json};

use super::Scale;
use super::peaks::Waveform;

/// Half the size of a handle, in points.
pub(crate) const HANDLE: f32 = 3.5;

/// What dragging a handle changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    /// An audio clip's fade-in.
    FadeIn,
    /// An audio clip's fade-out.
    FadeOut,
    /// A point of an automation clip, by its index in the lane.
    Point(usize),
}

/// A handle drawn this frame, for hit testing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShownHandle {
    /// The clip it belongs to.
    pub clip: ClipId,
    /// What it changes.
    pub handle: Handle,
    /// Its center.
    pub at: Pos2,
}

/// How a clip of an audio channel reads its sample, as the mixer plays it (see `bodo_mix::audio_clips`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AudioTiming {
    /// Seconds into the file where the clip starts reading.
    offset: f64,
    /// Seconds of the file per second of the song.
    ratio: f64,
    /// When the clip starts, in seconds of the song.
    start: f64,
}

impl AudioTiming {
    /// The timing of `clip`, if it is an audio clip.
    ///
    /// Complexity: O(log m) in tempo changes.
    pub(crate) fn of(project: &Project, clip: &Clip) -> Option<Self> {
        let ClipSource::Audio(channel) = clip.source else { return None };
        let ChannelKind::AudioClip { stretch, tempo, .. } = &project.channels.get(&channel)?.kind else { return None };
        let ratio = bodo_mix::audio_clips::clip_ratio(project, *stretch, *tempo, clip);
        let sample_tempo = tempo.filter(|t| *t > 0.0).unwrap_or_else(|| project.tempo.bpm_at(clip.start));
        let start = project.tempo.seconds_at(clip.start).0;
        let offset = match (clip.offset, stretch) {
            (ClipOffset::Frames(f), _) => f.0 as f64 / f64::from(project.settings.sample_rate.hz()),
            (ClipOffset::Ticks(t), StretchMode::None) => project.tempo.seconds_at(Tick(clip.start.0 + t.0)).0 - start,
            (ClipOffset::Ticks(t), _) => t.0 as f64 / PPQ as f64 * 60.0 / sample_tempo,
        };
        Some(Self { offset, ratio, start })
    }

    /// Seconds into the file at song tick `tick`.
    ///
    /// Complexity: O(log m) in tempo changes.
    fn file_seconds(&self, project: &Project, tick: f64) -> f64 {
        self.offset + (project.tempo.seconds_at_ticks(tick).0 - self.start) * self.ratio
    }

    /// The song tick where the file ends, `seconds` long.
    ///
    /// Complexity: O(log m) in tempo changes.
    pub(crate) fn end_tick(&self, project: &Project, seconds: f64) -> f64 {
        project.tempo.ticks_at(bodo_kit_core::Seconds(self.start + (seconds - self.offset) / self.ratio.max(1e-6)))
    }
}

/// How long a new clip of audio channel `channel` placed at `start` lasts, once its sample's length is known: the
/// whole sample, as `clip.place_audio` places it.
///
/// Complexity: O(log m) in tempo changes.
pub(crate) fn audio_length(project: &Project, channel: ChannelId, start: i64, waveform: &Waveform) -> i64 {
    let clip = Clip {
        id: ClipId::from_u128(0),
        start: Tick(start),
        len: Tick(1),
        offset: ClipOffset::default(),
        muted: false,
        color: None,
        source: ClipSource::Audio(channel),
        fades: None,
    };
    AudioTiming::of(project, &clip).map_or(PPQ * 4, |timing| ((timing.end_tick(project, waveform.seconds()) - start as f64).round() as i64).max(1))
}

/// How long a new clip of automation channel `channel` lasts: up to its last point, or 4 bars, as
/// `automation.place_clip` places it.
pub(crate) fn automation_length(project: &Project, channel: ChannelId) -> i64 {
    let last = match project.channels.get(&channel).map(|c| &c.kind) {
        Some(ChannelKind::Automation(lane)) => lane.points.last().map_or(0, |p| p.at.0),
        _ => 0,
    };
    if last > 0 { last } else { 4 * project.meter.signature_at(Tick::ZERO).bar_ticks() }
}

/// The gain of `clip`'s fades at song tick `tick`: rising over the fade-in and falling over the fade-out, linearly.
fn fade_gain(clip: &Clip, tick: f64) -> f32 {
    let Some(fades) = clip.fades else { return 1.0 };
    let mut gain = 1.0f64;
    if fades.fade_in.0 > 0 {
        gain = gain.min((tick - clip.start.0 as f64) / fades.fade_in.0 as f64);
    }
    if fades.fade_out.0 > 0 {
        gain = gain.min((clip.end().0 as f64 - tick) / fades.fade_out.0 as f64);
    }
    gain.clamp(0.0, 1.0) as f32
}

/// Draws `clip`'s waveform in `rect` (its content area), culled to `visible`.
///
/// Complexity: O(w log n) in visible points of width and frames of the sample.
#[allow(clippy::too_many_arguments, reason = "a waveform needs the clip's place in time and on screen")]
pub(crate) fn waveform(
    painter: &Painter,
    project: &Project,
    clip: &Clip,
    timing: AudioTiming,
    waveform: &Waveform,
    rect: Rect,
    visible: Rect,
    scale: Scale,
    color: Color32,
) {
    let (left, right) = (rect.left().max(visible.left()).floor(), rect.right().min(visible.right()));
    if right <= left || rect.height() < 4.0 {
        return;
    }
    let (middle, half) = (rect.center().y, rect.height() / 2.0 - 1.0);
    let mut mesh = Mesh::default();
    let mut x = left;
    while x < right {
        let (from, to) = (scale.tick(x), scale.tick(x + 1.0));
        let (a, b) = (timing.file_seconds(project, from), timing.file_seconds(project, to));
        if let Some((low, high)) = waveform.range(a.min(b), a.max(b)) {
            let gain = fade_gain(clip, (from + to) / 2.0);
            let top = middle - high.clamp(-1.0, 1.0) * gain * half;
            let bottom = middle - low.clamp(-1.0, 1.0) * gain * half;
            mesh.add_colored_rect(Rect::from_min_max(pos2(x, top.min(middle - 0.5)), pos2(x + 1.0, bottom.max(middle + 0.5))), color);
        }
        x += 1.0;
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws `clip`'s fades over its content area `rect` and returns their handles: at the top of the content, where each
/// fade ends.
pub(crate) fn fades(painter: &Painter, clip: &Clip, rect: Rect, scale: Scale, color: Color32) -> [ShownHandle; 2] {
    let fades = clip.fades.unwrap_or_default();
    let fade_in = scale.x((clip.start.0 + fades.fade_in.0) as f64).clamp(rect.left() + HANDLE, rect.right() - HANDLE);
    let fade_out = scale.x((clip.end().0 - fades.fade_out.0) as f64).clamp(rect.left() + HANDLE, rect.right() - HANDLE);
    let stroke = Stroke::new(1.0, color);
    if fades.fade_in.0 > 0 {
        painter.line_segment([rect.left_bottom(), pos2(fade_in, rect.top())], stroke);
    }
    if fades.fade_out.0 > 0 {
        painter.line_segment([pos2(fade_out, rect.top()), rect.right_bottom()], stroke);
    }
    let handles = [
        ShownHandle { clip: clip.id, handle: Handle::FadeIn, at: pos2(fade_in, rect.top() + HANDLE) },
        ShownHandle { clip: clip.id, handle: Handle::FadeOut, at: pos2(fade_out, rect.top() + HANDLE) },
    ];
    for handle in &handles {
        painter.rect_filled(Rect::from_center_size(handle.at, vec2(2.0 * HANDLE, 2.0 * HANDLE)), 1.0, color);
    }
    handles
}

/// The tick where `clip`'s lane starts: its points count from here.
pub(crate) fn lane_origin(clip: &Clip) -> i64 {
    match clip.offset {
        ClipOffset::Ticks(t) => clip.start.0 - t.0,
        ClipOffset::Frames(_) => clip.start.0,
    }
}

/// The height in `rect` of normalized value `value`.
pub(crate) fn value_y(rect: Rect, value: f32) -> f32 {
    rect.bottom() - value.clamp(0.0, 1.0) * rect.height()
}

/// The normalized value at height `y` in `rect`.
pub(crate) fn y_value(rect: Rect, y: f32) -> f32 {
    ((rect.bottom() - y) / rect.height().max(1.0)).clamp(0.0, 1.0)
}

/// Draws the curve of `lane` for `clip` in its content area `rect`, culled to `visible`, and returns the handles of
/// the visible points inside the clip.
///
/// Complexity: O(w log p + k) in visible points of width, points of the lane, and visible points.
#[allow(clippy::too_many_arguments, reason = "a curve needs the clip's place in time and on screen")]
pub(crate) fn curve(
    painter: &Painter,
    lane: &AutomationLane,
    clip: &Clip,
    rect: Rect,
    visible: Rect,
    scale: Scale,
    color: Color32,
    fill: Color32,
) -> Vec<ShownHandle> {
    let (left, right) = (rect.left().max(visible.left()), rect.right().min(visible.right()));
    if right <= left || rect.height() < 4.0 {
        return Vec::new();
    }
    let origin = lane_origin(clip) as f64;
    let mut line = Vec::new();
    let mut mesh = Mesh::default();
    let mut x = left;
    loop {
        let y = value_y(rect, lane.normalized_at(scale.tick(x) - origin));
        if let Some(last) = line.last().copied() {
            let last: Pos2 = last;
            // The area under the curve, one strip per step.
            let i = mesh.vertices.len() as u32;
            for p in [last, pos2(x, y), pos2(x, rect.bottom()), pos2(last.x, rect.bottom())] {
                mesh.colored_vertex(p, fill);
            }
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        line.push(pos2(x, y));
        if x >= right {
            break;
        }
        x = (x + 2.0).min(right);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(line, Stroke::new(1.5, color)));
    // The points inside the clip and the view, found by binary search.
    let (first_tick, last_tick) = (scale.tick(left).max(clip.start.0 as f64), scale.tick(right).min(clip.end().0 as f64));
    let from = lane
        .points
        .binary_search_by(|p| if (origin + p.at.0 as f64) < first_tick - 1.0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater })
        .unwrap_or_else(|i| i);
    let mut handles = Vec::new();
    for (index, point) in lane.points.iter().enumerate().skip(from) {
        let tick = origin + point.at.0 as f64;
        if tick > last_tick + 1.0 {
            break;
        }
        let at = pos2(scale.x(tick).clamp(rect.left(), rect.right()), value_y(rect, point.value));
        painter.circle(at, HANDLE, color, Stroke::new(1.0, Color32::from_black_alpha(90)));
        handles.push(ShownHandle { clip: clip.id, handle: Handle::Point(index), at });
    }
    handles
}

/// A curve as the automation commands take it.
pub(crate) fn curve_json(curve: CurveShape) -> Value {
    match curve {
        CurveShape::Hold => json!("hold"),
        CurveShape::Linear => json!("linear"),
        CurveShape::Exp(t) => json!({ "exp": t }),
        CurveShape::SCurve(t) => json!({ "sCurve": t }),
        CurveShape::Stairs(n) => json!({ "stairs": n }),
        CurveShape::Sine => json!("sine"),
        CurveShape::Pulse => json!("pulse"),
    }
}

/// The lane of automation channel `channel`.
pub(crate) fn lane(project: &Project, channel: ChannelId) -> Option<&AutomationLane> {
    match &project.channels.get(&channel)?.kind {
        ChannelKind::Automation(lane) => Some(lane),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bodo_model::playlist::Fades;

    use super::*;

    fn clip(start: i64, len: i64, fades: Option<Fades>) -> Clip {
        Clip {
            id: ClipId::from_u128(1),
            start: Tick(start),
            len: Tick(len),
            offset: ClipOffset::Ticks(Tick(120)),
            muted: false,
            color: None,
            source: ClipSource::Automation(ChannelId::from_u128(2)),
            fades,
        }
    }

    #[test]
    fn fades_ramp_linearly_and_lanes_count_from_the_offset() {
        let faded = clip(1000, 1000, Some(Fades { fade_in: Tick(200), fade_out: Tick(500) }));
        assert_eq!(fade_gain(&faded, 1100.0), 0.5);
        assert_eq!(fade_gain(&faded, 1300.0), 1.0);
        assert_eq!(fade_gain(&faded, 1750.0), 0.5);
        assert_eq!(fade_gain(&clip(0, 10, None), 5.0), 1.0);
        assert_eq!(lane_origin(&faded), 880);
        let rect = Rect::from_min_max(pos2(0.0, 10.0), pos2(100.0, 110.0));
        assert_eq!(value_y(rect, 0.25), 85.0);
        assert_eq!(y_value(rect, 85.0), 0.25);
        assert_eq!(y_value(rect, 500.0), 0.0);
    }
}
