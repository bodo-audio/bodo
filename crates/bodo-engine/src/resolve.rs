//! Turning references into ids (project.md section 8.2).
//!
//! A reference is tried, in order, as an id (`ch_01J...`), as `kind:N` with a 1-based number, as an exact name
//! (ignoring case), and as a bare number. A name that matches several entities is an error that lists them, and an
//! unknown one lists the names that contain it.

use std::fmt::Display;
use std::str::FromStr;

use bodo_model::mixer::RouteSource;
use bodo_model::{ChannelId, ClipId, InsertId, PatternId, Project, TrackId};

use crate::error::{CommandError, ErrorCode};
use crate::params::{ChannelRef, ClipRef, InsertRef, PatternRef, RefText, RouteFromRef, TrackRef};

/// Most candidates an error lists.
const MAX_CANDIDATES: usize = 10;

/// Resolves `text` among `entries` (in display order, numbered from 1). `zero` is the entity numbered 0, if any; it
/// answers to its id and name too.
fn find<Id: Copy + PartialEq + FromStr + Display>(
    kind: &str,
    prefix: &str,
    text: &RefText,
    entries: &[(Id, &str)],
    zero: Option<(Id, &str)>,
) -> Result<Id, CommandError> {
    let ordinal = |n: u64| -> Result<Id, CommandError> {
        if n == 0
            && let Some((id, _)) = zero
        {
            return Ok(id);
        }
        let index = usize::try_from(n).ok().and_then(|n| n.checked_sub(1));
        index
            .and_then(|i| entries.get(i))
            .map(|(id, _)| *id)
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("there is no {kind} {n}; there are {} {kind}s", entries.len())))
    };
    let text = match text {
        RefText::Number(n) => return ordinal(*n),
        RefText::Text(text) => text.trim(),
    };
    if let Ok(id) = text.parse::<Id>() {
        let known = entries.iter().chain(zero.as_ref()).any(|(e, _)| *e == id);
        return if known { Ok(id) } else { Err(CommandError::not_found(kind, text)) };
    }
    if let Some(n) = text.strip_prefix(prefix).and_then(|rest| rest.strip_prefix(':')).and_then(|n| n.trim().parse::<u64>().ok()) {
        return ordinal(n);
    }
    let named: Vec<&(Id, &str)> = entries.iter().chain(zero.as_ref()).filter(|(_, name)| name.trim().eq_ignore_ascii_case(text)).collect();
    match named.as_slice() {
        [(id, _)] => return Ok(*id),
        [] => {}
        many => {
            let candidates = many.iter().take(MAX_CANDIDATES).map(|(id, name)| format!("{id} ({name})")).collect();
            return Err(CommandError::new(ErrorCode::Ambiguous, format!("`{text}` names {} {kind}s", many.len()))
                .with_candidates(candidates)
                .with_hint(format!("use the id or {prefix}:N")));
        }
    }
    if let Ok(n) = text.parse::<u64>() {
        return ordinal(n);
    }
    let lower = text.to_lowercase();
    let mut candidates: Vec<String> =
        entries.iter().filter(|(_, name)| name.to_lowercase().contains(&lower)).take(MAX_CANDIDATES).map(|(_, name)| (*name).to_owned()).collect();
    if candidates.is_empty() {
        candidates = entries.iter().take(MAX_CANDIDATES).map(|(_, name)| (*name).to_owned()).collect();
    }
    Err(CommandError::not_found(kind, text).with_candidates(candidates))
}

/// The channel `r` names.
///
/// Complexity: O(c) in channels.
pub fn channel(project: &Project, r: &ChannelRef) -> Result<ChannelId, CommandError> {
    let entries: Vec<(ChannelId, &str)> =
        project.rack_order.iter().filter_map(|id| project.channels.get(id)).map(|c| (c.id, c.name.as_str())).collect();
    find("channel", "channel", &r.0, &entries, None)
}

/// The pattern `r` names. Patterns are numbered in the order they were made.
///
/// Complexity: O(p) in patterns.
pub fn pattern(project: &Project, r: &PatternRef) -> Result<PatternId, CommandError> {
    let entries: Vec<(PatternId, &str)> = project.patterns.values().map(|p| (p.id, p.name.as_str())).collect();
    find("pattern", "pattern", &r.0, &entries, None)
}

/// The playlist track `r` names.
///
/// Complexity: O(t) in tracks.
pub fn track(project: &Project, r: &TrackRef) -> Result<TrackId, CommandError> {
    let entries: Vec<(TrackId, &str)> = project.track_order.iter().filter_map(|id| project.tracks.get(id)).map(|t| (t.id, t.name.as_str())).collect();
    find("track", "track", &r.0, &entries, None)
}

/// The mixer insert `r` names: inserts are numbered from 1, and number 0 (or the name `Master`) is the master.
///
/// Complexity: O(i) in inserts.
pub fn insert(project: &Project, r: &InsertRef) -> Result<InsertId, CommandError> {
    let master = &project.mixer.master;
    let entries: Vec<(InsertId, &str)> =
        project.mixer.order.iter().filter_map(|id| project.mixer.inserts.get(id)).map(|i| (i.id, i.name.as_str())).collect();
    find("insert", "insert", &r.0, &entries, Some((master.id, master.name.as_str())))
}

/// The track and clip `r` names.
///
/// Complexity: O(n) in clips on the playlist.
pub fn clip(project: &Project, r: &ClipRef) -> Result<(TrackId, ClipId), CommandError> {
    let text = r.0.as_text();
    let id: ClipId =
        text.parse().map_err(|_| CommandError::invalid(format!("`{text}` is not a clip id")).with_hint("clip ids look like clip_01J..."))?;
    project
        .tracks
        .values()
        .find_map(|track| track.clips.iter().any(|c| c.id == id).then_some((track.id, id)))
        .ok_or_else(|| CommandError::not_found("clip", &text))
}

/// The channel or insert a route starts from.
///
/// Complexity: O(c + i) in channels and inserts.
pub fn route_source(project: &Project, r: &RouteFromRef) -> Result<RouteSource, CommandError> {
    let text = r.0.as_text();
    let as_channel = channel(project, &ChannelRef(r.0.clone()));
    let as_insert = insert(project, &InsertRef(r.0.clone()));
    let explicit_channel = text.starts_with("ch_") || text.starts_with("channel:");
    let explicit_insert = text.starts_with("ins_") || text.starts_with("insert:") || text.eq_ignore_ascii_case("master");
    match (as_channel, as_insert) {
        (Ok(c), _) if explicit_channel => Ok(RouteSource::Channel(c)),
        (_, Ok(i)) if explicit_insert => Ok(RouteSource::Insert(i)),
        (Ok(c), Err(_)) => Ok(RouteSource::Channel(c)),
        (Err(_), Ok(i)) => Ok(RouteSource::Insert(i)),
        (Ok(_), Ok(_)) => Err(CommandError::new(ErrorCode::Ambiguous, format!("`{text}` names both a channel and an insert"))
            .with_hint("write channel:N or insert:N, or use an id")),
        (Err(e), Err(_)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bodo_model::channel::{ChannelKind, SamplerParams};
    use bodo_model::{Channel, Color};

    use super::*;

    fn project() -> Project {
        let mut project = Project::new(120.0).unwrap();
        for (n, name) in ["Kick", "Snare", "Kick"].into_iter().enumerate() {
            let id = ChannelId::from_u128(n as u128 + 1);
            let channel = Channel {
                id,
                name: name.into(),
                color: Color::GRAY,
                kind: ChannelKind::Sampler(Arc::new(SamplerParams::default())),
                target: None,
                volume: 1.0,
                pan: 0.0,
                pitch_cents: 0.0,
                swing: 0.0,
                muted: false,
                midi_in: None,
            };
            project.channels.insert(id, Arc::new(channel));
            project.rack_order.push_back(id);
        }
        project
    }

    #[test]
    fn references_resolve_by_id_number_and_name() {
        let project = project();
        let snare = ChannelId::from_u128(2);
        assert_eq!(channel(&project, &ChannelRef::new("snare")), Ok(snare));
        assert_eq!(channel(&project, &ChannelRef::new("channel:2")), Ok(snare));
        assert_eq!(channel(&project, &ChannelRef(RefText::Number(2))), Ok(snare));
        assert_eq!(channel(&project, &ChannelRef::new("2")), Ok(snare));
        assert_eq!(channel(&project, &ChannelRef::new(snare.to_string())), Ok(snare));
        let ambiguous = channel(&project, &ChannelRef::new("Kick")).unwrap_err();
        assert_eq!((ambiguous.code, ambiguous.candidates.len()), (ErrorCode::Ambiguous, 2));
        let unknown = channel(&project, &ChannelRef::new("Kik")).unwrap_err();
        assert_eq!(unknown.code, ErrorCode::NotFound);
        assert_eq!(channel(&project, &ChannelRef::new("channel:9")).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn inserts_count_from_one_and_zero_is_the_master() {
        let project = project();
        let master = project.mixer.master.id;
        assert_eq!(insert(&project, &InsertRef::new("master")), Ok(master));
        assert_eq!(insert(&project, &InsertRef::new("insert:0")), Ok(master));
        assert_eq!(insert(&project, &InsertRef(RefText::Number(3))), Ok(project.mixer.order[2]));
        assert_eq!(insert(&project, &InsertRef::new("Insert 16")), Ok(project.mixer.order[15]));
        assert!(insert(&project, &InsertRef(RefText::Number(17))).is_err(), "17 is past the 16 inserts, not the master");
        assert_eq!(route_source(&project, &RouteFromRef::new("insert:2")), Ok(RouteSource::Insert(project.mixer.order[1])));
        assert_eq!(route_source(&project, &RouteFromRef::new("Snare")), Ok(RouteSource::Channel(ChannelId::from_u128(2))));
    }
}
