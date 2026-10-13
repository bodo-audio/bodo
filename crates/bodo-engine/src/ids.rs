//! Where new ids come from: fresh ULIDs normally, or a counter so tests and snapshots are reproducible.

use bodo_model::{ChannelId, ClipId, InsertId, PatternId, PluginInstanceId, ProjectId, SampleId, TrackId};

/// Makes new entity ids.
#[derive(Clone, Debug, Default)]
pub struct IdSource {
    next: Option<u128>,
}

macro_rules! make {
    ($($(#[$doc:meta])* $fn:ident -> $ty:ident;)*) => {
        $(
            $(#[$doc])*
            pub fn $fn(&mut self) -> $ty {
                match self.next.as_mut() {
                    Some(next) => {
                        *next += 1;
                        $ty::from_u128(*next)
                    }
                    None => $ty::new(),
                }
            }
        )*
    };
}

impl IdSource {
    /// Fresh, globally unique ULIDs.
    pub fn random() -> Self {
        Self { next: None }
    }

    /// Ids counting up from `start + 1`, the same on every run.
    pub fn counting(start: u128) -> Self {
        Self { next: Some(start) }
    }

    /// Whether ids are counted rather than random.
    pub fn is_counting(&self) -> bool {
        self.next.is_some()
    }

    make! {
        /// A new project id.
        project -> ProjectId;
        /// A new channel id.
        channel -> ChannelId;
        /// A new pattern id.
        pattern -> PatternId;
        /// A new playlist track id.
        track -> TrackId;
        /// A new clip id.
        clip -> ClipId;
        /// A new mixer insert id.
        insert -> InsertId;
        /// A new sample id.
        sample -> SampleId;
        /// A new plugin instance id.
        plugin -> PluginInstanceId;
    }
}
