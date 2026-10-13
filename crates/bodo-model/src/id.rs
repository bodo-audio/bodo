//! Typed identifiers (project.md section 8.2).
//!
//! Every entity has a ULID wrapped in its own type, so a channel id can't be used where a pattern id is expected.
//! In text, an id is its type's prefix and the ULID, such as `ch_01JABCDEF...`, which is how project files, MCP and
//! agents name things. Ids are stable across save and load, undo and branch merges. ULIDs sort by creation time, so
//! maps keyed by id keep things in the order they were made.

use core::fmt;
use core::str::FromStr;

use ulid::Ulid;

/// Why a string isn't an id of the expected type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` is not a valid {kind} id")]
pub struct ParseIdError {
    /// The text that failed.
    pub text: String,
    /// The kind of id expected.
    pub kind: &'static str,
}

macro_rules! typed_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal, $kind:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Ulid);

        impl $name {
            /// The text prefix of this kind of id.
            pub const PREFIX: &'static str = $prefix;

            /// A new, unique id.
            pub fn new() -> Self {
                Self(Ulid::generate())
            }

            /// An id with a fixed value, for tests and fixtures.
            pub const fn from_u128(value: u128) -> Self {
                Self(Ulid(value))
            }

            /// The underlying ULID.
            pub fn ulid(self) -> Ulid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}_{}", $prefix, self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                text.strip_prefix(concat!($prefix, "_"))
                    .and_then(|rest| Ulid::from_string(rest).ok())
                    .map(Self)
                    .ok_or_else(|| ParseIdError { text: text.to_owned(), kind: $kind })
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

typed_id!(
    /// Identifies a project.
    ProjectId,
    "proj",
    "project"
);
typed_id!(
    /// Identifies a channel of the channel rack.
    ChannelId,
    "ch",
    "channel"
);
typed_id!(
    /// Identifies a pattern.
    PatternId,
    "pat",
    "pattern"
);
typed_id!(
    /// Identifies a playlist track.
    TrackId,
    "trk",
    "track"
);
typed_id!(
    /// Identifies a clip on a playlist track.
    ClipId,
    "clip",
    "clip"
);
typed_id!(
    /// Identifies a mixer insert, including the master.
    InsertId,
    "ins",
    "insert"
);
typed_id!(
    /// Identifies a sample file the project uses.
    SampleId,
    "smp",
    "sample"
);
typed_id!(
    /// Identifies a plugin instance.
    PluginInstanceId,
    "plug",
    "plugin"
);

/// Identifies a note within its pattern. It maps directly to a CLAP note id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct NoteId(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_print_and_parse_with_their_prefix() {
        let id = ChannelId::from_u128(42);
        let text = id.to_string();
        assert!(text.starts_with("ch_"), "{text}");
        assert_eq!(text.len(), 3 + 26);
        assert_eq!(text.parse::<ChannelId>(), Ok(id));
        assert!(text.parse::<PatternId>().is_err(), "the prefix must match the type");
        assert!("ch_not-a-ulid".parse::<ChannelId>().is_err());
    }

    #[test]
    fn new_ids_are_unique() {
        let a = TrackId::new();
        let b = TrackId::new();
        assert_ne!(a, b);
    }
}
