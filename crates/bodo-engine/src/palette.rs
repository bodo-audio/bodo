//! The colors new channels, patterns and clips get (project.md section 14.2): 16 hues from red to pink.

use bodo_model::Color;

/// The clip palette, Tailwind's 400 family from red to pink.
pub const HUES: [Color; 16] = [
    Color::rgb(0xf8, 0x71, 0x71),
    Color::rgb(0xfb, 0x92, 0x3c),
    Color::rgb(0xfb, 0xbf, 0x24),
    Color::rgb(0xfa, 0xcc, 0x15),
    Color::rgb(0xa3, 0xe6, 0x35),
    Color::rgb(0x4a, 0xde, 0x80),
    Color::rgb(0x34, 0xd3, 0x99),
    Color::rgb(0x2d, 0xd4, 0xbf),
    Color::rgb(0x22, 0xd3, 0xee),
    Color::rgb(0x38, 0xbd, 0xf8),
    Color::rgb(0x60, 0xa5, 0xfa),
    Color::rgb(0x81, 0x8c, 0xf8),
    Color::rgb(0xa7, 0x8b, 0xfa),
    Color::rgb(0xc0, 0x84, 0xfc),
    Color::rgb(0xe8, 0x79, 0xf9),
    Color::rgb(0xf4, 0x72, 0xb6),
];

/// The `n`th hue, wrapping around. New patterns take these in order.
pub fn color(n: usize) -> Color {
    HUES[n % HUES.len()]
}

/// The color of the `n`th new channel: hues spread around the palette, so neighbors differ.
pub fn channel_color(n: usize) -> Color {
    HUES[(n * 5) % HUES.len()]
}
