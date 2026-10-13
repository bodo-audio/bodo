//! MIDI 1.0 channel voice messages and the small integer types they carry.

macro_rules! bounded_uint {
    ($(#[$doc:meta])* $name:ident, $inner:ty, $max:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($inner);

        impl $name {
            /// The smallest value, zero.
            pub const MIN: Self = Self(0);
            /// The largest value.
            pub const MAX: Self = Self($max);

            /// Returns `value`, or `None` when it is out of range.
            pub const fn new(value: $inner) -> Option<Self> {
                if value <= $max { Some(Self(value)) } else { None }
            }

            /// Returns `value` with the bits above the range cleared.
            pub const fn from_masked(value: $inner) -> Self {
                Self(value & $max)
            }

            /// The value as a plain integer.
            pub const fn get(self) -> $inner {
                self.0
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> $inner {
                value.0
            }
        }
    };
}

bounded_uint!(
    /// A 4-bit value from 0 to 15, such as a MIDI channel.
    U4,
    u8,
    0x0F
);
bounded_uint!(
    /// A 7-bit MIDI data value from 0 to 127.
    U7,
    u8,
    0x7F
);
bounded_uint!(
    /// A 14-bit MIDI value from 0 to 16383, such as a pitch bend.
    U14,
    u16,
    0x3FFF
);

impl U14 {
    /// The middle of the range, 8192: no pitch bend.
    pub const CENTER: Self = Self(0x2000);
}

/// A MIDI 1.0 channel voice message, without its channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MidiMessage {
    /// Releases a note.
    NoteOff {
        /// The key.
        key: U7,
        /// The release velocity.
        velocity: U7,
    },
    /// Starts a note. By convention a velocity of zero means note off.
    NoteOn {
        /// The key.
        key: U7,
        /// The velocity.
        velocity: U7,
    },
    /// Changes the pressure on one held key (polyphonic aftertouch).
    PolyPressure {
        /// The key.
        key: U7,
        /// The new pressure.
        pressure: U7,
    },
    /// Sets a controller (CC).
    ControlChange {
        /// The controller number.
        controller: U7,
        /// The new value.
        value: U7,
    },
    /// Selects a program (instrument).
    ProgramChange {
        /// The program number.
        program: U7,
    },
    /// Changes the pressure for the whole channel (channel aftertouch).
    ChannelPressure {
        /// The new pressure.
        pressure: U7,
    },
    /// Sets the channel's pitch bend. [`U14::CENTER`] means no bend.
    PitchBend {
        /// The bend value.
        value: U14,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_values_check_their_range() {
        assert_eq!(U7::new(127).map(U7::get), Some(127));
        assert_eq!(U7::new(128), None);
        assert_eq!(U4::new(16), None);
        assert_eq!(U14::new(0x4000), None);
        assert_eq!(U7::from_masked(0xFF), U7::MAX);
        assert_eq!(u16::from(U14::CENTER), 8192);
    }
}
