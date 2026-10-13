//! Colors of channels, patterns, tracks, clips and inserts.

use core::fmt;
use core::str::FromStr;

/// An sRGB color. In text it is `#rrggbb`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
}

impl Color {
    /// A color from its components.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Neutral gray, for things that have no color of their own.
    pub const GRAY: Self = Self::rgb(0x73, 0x73, 0x73);
}

/// Why a string isn't a `#rrggbb` color.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a #rrggbb color")]
pub struct ParseColorError(pub String);

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseColorError(text.to_owned());
        let hex = text.strip_prefix('#').filter(|h| h.len() == 6 && h.is_ascii()).ok_or_else(error)?;
        let channel = |range: core::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).map_err(|_| error());
        Ok(Self { r: channel(0..2)?, g: channel(2..4)?, b: channel(4..6)? })
    }
}

impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_round_trip_as_hex() {
        let color = Color::rgb(0xfa, 0xcc, 0x15);
        assert_eq!(color.to_string(), "#facc15");
        assert_eq!("#facc15".parse(), Ok(color));
        assert_eq!("#FACC15".parse(), Ok(color));
        assert!("facc15".parse::<Color>().is_err());
        assert!("#facc1".parse::<Color>().is_err());
        assert!("#gggggg".parse::<Color>().is_err());
    }
}
