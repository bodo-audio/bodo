//! New projects (project.md section 14.1).
//!
//! Every template starts at 120 BPM in 4/4 with the master, 16 inserts, 16 playlist tracks and one empty pattern. The
//! basic template also loads a starter drum kit from the factory content, one channel per insert, so a beat can start
//! right away.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_kit_core::TempoMap;
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::project::Meta;
use bodo_model::{Channel, Color, FACTORY_PREFIX, MeterMap, Mixer, Pattern, PlaylistTrack, Project, SampleRef, Settings};
use imbl::{OrdMap, Vector};

use crate::ids::IdSource;
use crate::palette;
use crate::session::Env;
use crate::state::EditorState;

/// What a new project starts with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Template {
    /// Tracks, inserts and one pattern; no channels.
    Empty,
    /// The empty template plus a starter drum kit: Kick, Clap, HiHat and Snare on inserts 1 to 4.
    #[default]
    Basic,
}

/// The starter kit of the basic template, in channel rack order.
pub const STARTER_KIT: [&str; 4] = ["Kick", "Clap", "HiHat", "Snare"];

/// Where the factory drum kit lives inside the factory content.
pub const DRUM_KIT: &str = "Packs/Bodo Drums";

/// Builds a new project from `template` and the editor state to start with. `created` is the creation time to record.
pub fn new_project(template: Template, ids: &mut IdSource, env: &Env, created: Option<String>) -> (Project, EditorState) {
    let master = ids.insert();
    let inserts: Vec<_> = (0..16).map(|_| ids.insert()).collect();
    let mixer = Mixer::with_inserts(master, inserts.iter().copied());
    let mut project = Project {
        id: ids.project(),
        meta: Arc::new(Meta { created, ..Meta::default() }),
        settings: Arc::new(Settings::default()),
        tempo: Arc::new(TempoMap::default()),
        meter: Arc::new(MeterMap::common_time()),
        channels: OrdMap::new(),
        rack_order: Vector::new(),
        patterns: OrdMap::new(),
        tracks: OrdMap::new(),
        track_order: Vector::new(),
        mixer: Arc::new(mixer),
        samples: OrdMap::new(),
        markers: Vector::new(),
    };
    for n in 1..=16 {
        let id = ids.track();
        project.tracks.insert(id, Arc::new(PlaylistTrack::new(id, format!("Track {n}"), Color::GRAY)));
        project.track_order.push_back(id);
    }
    let pattern = ids.pattern();
    project.patterns.insert(pattern, Arc::new(Pattern::new(pattern, "Pattern 1", palette::color(0))));

    if template == Template::Basic
        && let Some(factory) = &env.factory
    {
        for (n, name) in STARTER_KIT.iter().enumerate() {
            let relative = format!("{DRUM_KIT}/{name}.wav");
            let file = factory.join(&relative);
            if !file.is_file() {
                continue;
            }
            let sample = ids.sample();
            let hash = crate::samples::hash(&file);
            project.samples.insert(sample, Arc::new(SampleRef { id: sample, path: format!("{FACTORY_PREFIX}{relative}"), hash, market: None }));
            let channel = ids.channel();
            let params = SamplerParams { sample: Some(sample), ..SamplerParams::default() };
            project.channels.insert(
                channel,
                Arc::new(Channel {
                    id: channel,
                    name: (*name).to_owned(),
                    color: palette::channel_color(n),
                    kind: ChannelKind::Sampler(Arc::new(params)),
                    target: inserts.get(n).copied(),
                    volume: 1.0,
                    pan: 0.0,
                    pitch_cents: 0.0,
                    swing: 0.0,
                    muted: false,
                    midi_in: None,
                }),
            );
            project.rack_order.push_back(channel);
        }
    }
    let editor = EditorState {
        pattern: Some(pattern),
        channel: project.rack_order.front().copied(),
        track: project.track_order.front().copied(),
        insert: inserts.first().copied(),
        ..EditorState::default()
    };
    (project, editor)
}

/// Whether `dir` looks like Bodo's factory content.
pub fn is_factory_dir(dir: &Path) -> bool {
    dir.join(DRUM_KIT).is_dir()
}

/// The environment variable that points Bodo at its factory content, overriding the usual places.
pub const FACTORY_ENV: &str = "BODO_FACTORY";

/// Finds Bodo's factory content: the folder named by `BODO_FACTORY`, a `factory` folder next to the executable (or in
/// `../Resources` inside a macOS app bundle, or `../share/bodo` on Linux), or the repository's `assets/factory` in a
/// development build.
///
/// Does I/O: checks a few folders.
pub fn locate_factory() -> Option<PathBuf> {
    let from_env = std::env::var_os(FACTORY_ENV).map(PathBuf::from);
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    let beside_exe = exe_dir.iter().flat_map(|dir| [dir.join("factory"), dir.join("../Resources/factory"), dir.join("../share/bodo/factory")]);
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/factory");
    from_env.into_iter().chain(beside_exe).chain(std::iter::once(repository)).find(|dir| is_factory_dir(dir)).and_then(|dir| dir.canonicalize().ok())
}

/// The current time as an RFC 3339 UTC timestamp, such as `2026-10-13T08:30:00Z`.
pub fn now_rfc3339() -> String {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The calendar date of a day counted from 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_convert_from_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_739), (2026, 10, 13));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn templates_are_valid_and_deterministic() {
        let env = Env::default();
        let (a, editor) = new_project(Template::Basic, &mut IdSource::counting(0), &env, None);
        let (b, _) = new_project(Template::Basic, &mut IdSource::counting(0), &env, None);
        assert_eq!(a, b);
        assert_eq!(a.validate(), Ok(()));
        assert_eq!((a.tracks.len(), a.patterns.len(), a.mixer.order.len()), (16, 1, 16));
        assert!(a.channels.is_empty(), "no factory content, no starter kit");
        assert!(editor.pattern.is_some());
    }
}
