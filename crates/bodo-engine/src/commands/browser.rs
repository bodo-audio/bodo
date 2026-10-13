//! `browser.*`: finding sounds to load (project.md section 14.1).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::library;
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    /// Words the sound's folder and name must contain, ignoring case, such as "drums kick". Empty lists everything.
    #[serde(default)]
    query: String,
    /// The most results, from 1 to 500; 50 by default.
    limit: Option<usize>,
}

fn search(ctx: &mut Ctx<'_>, p: Search) -> Result<Value, CommandError> {
    let limit = p.limit.unwrap_or(50);
    if !(1..=500).contains(&limit) {
        return Err(CommandError::invalid("limit must be from 1 to 500"));
    }
    let sounds = library::search(ctx.env(), &p.query, limit);
    Ok(json!({ "sounds": sounds, "count": sounds.len() }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "browser.search",
        "Search sounds",
        "Finds audio files in the factory content and the user's sample folders; pass a result's path to channel.add.",
        Effects::NONE,
        search,
    )
    .example(json!({ "query": "kick" }));
}
