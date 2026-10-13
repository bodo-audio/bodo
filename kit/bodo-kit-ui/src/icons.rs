//! The icon catalog: SVG path data on a 24 x 24 grid, drawn by [`Icon::paint`].
//!
//! Sources:
//!
//! - The first group is path data from [Tabler Icons](https://tabler.io/icons) 3.34.1, Copyright (c) 2020-2024
//!   Paweł Kuna, MIT License, taken from pixel3d's icon table.
//! - [`SPARKLES`] was drawn for pixel3d (MIT OR Apache-2.0) in the same style.
//! - The icons after the `Drawn for Bodo in the same style` comment were drawn for Bodo Kit on the same grid with
//!   the same 2-unit strokes and round joins. [`FILE_MUSIC`] reuses Tabler's page outline.
//!
//! Transport icons ([`PLAYER_PLAY`], [`PLAYER_STOP`], [`PLAYER_PAUSE`], [`PLAYER_RECORD`]) and [`FILLED_CIRCLE`] are
//! filled; every other icon is stroked.

use crate::icon::{Icon, Source};

macro_rules! icons {
    ($( $(#[$doc:meta])* $name:ident = $label:literal, $fill:literal, [$($path:literal),* $(,)?]; )*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, reason = "variants mirror the constant names")]
        #[derive(Clone, Copy)]
        #[repr(u16)]
        enum Index {
            $($name,)*
        }

        $( $(#[$doc])* pub const $name: Icon = Icon(Index::$name as u16); )*

        /// Where each icon comes from, indexed by the icon.
        pub(crate) static SOURCES: &[Source] = &[$(Source { name: $label, paths: &[$($path),*], filled: $fill },)*];

        /// Every icon of the catalog, in declaration order.
        pub const ALL: &[Icon] = &[$($name,)*];
    };
}

icons! {
    // Tabler Icons 3.34.1 (MIT).

    /// A page with a plus: new project.
    FILE_PLUS = "file-plus", false, [
        "M14 3v4a1 1 0 0 0 1 1h4",
        "M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z",
        "M12 11l0 6",
        "M9 14l6 0",
    ];
    /// An open folder: open a file.
    FOLDER_OPEN = "folder-open", false, [
        "M5 19l2.757 -7.351a1 1 0 0 1 .936 -.649h12.307a1 1 0 0 1 .986 1.164l-.996 5.211a2 2 0 0 1 -1.964 1.625h-14.026a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2h4l3 3h7a2 2 0 0 1 2 2v2",
    ];
    /// A closed folder.
    FOLDER = "folder", false, ["M5 4h4l3 3h7a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2"];
    /// A floppy disk: save.
    DEVICE_FLOPPY = "device-floppy", false, [
        "M6 4h10l4 4v10a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2",
        "M12 14m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M14 4l0 4l-6 0l0 -4",
    ];
    /// An arrow turning back: undo.
    ARROW_BACK_UP = "arrow-back-up", false, ["M9 14l-4 -4l4 -4", "M5 10h11a4 4 0 1 1 0 8h-1"];
    /// An arrow turning forward: redo.
    ARROW_FORWARD_UP = "arrow-forward-up", false, ["M15 14l4 -4l-4 -4", "M19 10h-11a4 4 0 1 0 0 8h1"];
    /// A mouse pointer: the select tool.
    POINTER = "pointer", false, [
        "M7.904 17.563a1.2 1.2 0 0 0 2.228 .308l2.09 -3.093l4.907 4.907a1.067 1.067 0 0 0 1.509 0l1.047 -1.047a1.067 1.067 0 0 0 0 -1.509l-4.907 -4.907l3.113 -2.09a1.2 1.2 0 0 0 -.309 -2.228l-13.582 -3.904l3.904 13.563z",
    ];
    /// A magnifying glass: search.
    SEARCH = "search", false, ["M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0", "M21 21l-6 -6"];
    /// A gear: settings.
    SETTINGS = "settings", false, [
        "M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543 -.94 3.31 .826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.31 -2.37 2.37a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.31 -.826 -2.37 -2.37a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.31 2.37 -2.37c1 .608 2.296 .07 2.572 -1.065z",
        "M9 12a3 3 0 1 0 6 0a3 3 0 0 0 -6 0",
    ];
    /// A question mark in a circle: help.
    HELP = "help", false, ["M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0", "M12 17l0 .01", "M12 13.5a1.5 1.5 0 0 1 1 -1.5a2.6 2.6 0 1 0 -3 -4"];
    /// A plus: add.
    PLUS = "plus", false, ["M12 5l0 14", "M5 12l14 0"];
    /// A cross: close.
    X = "x", false, ["M18 6l-12 12", "M6 6l12 12"];
    /// A chevron pointing down.
    CHEVRON_DOWN = "chevron-down", false, ["M6 9l6 6l6 -6"];
    /// A chevron pointing right.
    CHEVRON_RIGHT = "chevron-right", false, ["M9 6l6 6l-6 6"];
    /// A chevron pointing up.
    CHEVRON_UP = "chevron-up", false, ["M6 15l6 -6l6 6"];
    /// Three lines: a menu.
    MENU_2 = "menu-2", false, ["M4 6l16 0", "M4 12l16 0", "M4 18l16 0"];
    /// A padlock: locked.
    LOCK = "lock", false, [
        "M5 13a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-6z",
        "M11 16a1 1 0 1 0 2 0a1 1 0 0 0 -2 0",
        "M8 11v-4a4 4 0 1 1 8 0v4",
    ];
    /// Three sliders: the mixer.
    ADJUSTMENTS_HORIZONTAL = "adjustments-horizontal", false, [
        "M14 6m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M4 6l8 0",
        "M16 6l4 0",
        "M8 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M4 12l2 0",
        "M10 12l10 0",
        "M17 18m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M4 18l11 0",
        "M19 18l1 0",
    ];
    /// Two pages: copy.
    COPY = "copy", false, [
        "M7 7m0 2.667a2.667 2.667 0 0 1 2.667 -2.667h8.666a2.667 2.667 0 0 1 2.667 2.667v8.666a2.667 2.667 0 0 1 -2.667 2.667h-8.666a2.667 2.667 0 0 1 -2.667 -2.667z",
        "M4.012 16.737a2.005 2.005 0 0 1 -1.012 -1.737v-10c0 -1.1 .9 -2 2 -2h10c.75 0 1.158 .385 1.5 1",
    ];
    /// A bin: delete.
    TRASH = "trash", false, [
        "M4 7l16 0",
        "M10 11l0 6",
        "M14 11l0 6",
        "M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2l1 -12",
        "M9 7v-3a1 1 0 0 1 1 -1h4a1 1 0 0 1 1 1v3",
    ];
    /// Four arrows pointing out: full screen.
    ARROWS_MAXIMIZE = "arrows-maximize", false, [
        "M16 4l4 0l0 4",
        "M14 10l6 -6",
        "M8 20l-4 0l0 -4",
        "M4 20l6 -6",
        "M16 20l4 0l0 -4",
        "M14 14l6 6",
        "M8 4l-4 0l0 4",
        "M4 4l6 6",
    ];
    /// A computer keyboard: the typing keyboard.
    KEYBOARD = "keyboard", false, [
        "M2 6m0 2a2 2 0 0 1 2 -2h16a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2z",
        "M6 10l0 .01",
        "M10 10l0 .01",
        "M14 10l0 .01",
        "M18 10l0 .01",
        "M6 14l0 .01",
        "M18 14l0 .01",
        "M10 14l4 .01",
    ];
    /// A sine wave: the oscilloscope and synthesizers.
    WAVE_SINE = "wave-sine", false, [
        "M21 12h-2c-.894 0 -1.662 -.857 -1.761 -2c-.296 -3.45 -.749 -6 -2.749 -6s-2.5 3.582 -2.5 8s-.5 8 -2.5 8s-2.452 -2.547 -2.749 -6c-.1 -1.147 -.867 -2 -1.763 -2h-2",
    ];
    /// Arrows left and right: the slip tool.
    ARROWS_HORIZONTAL = "arrows-horizontal", false, ["M7 8l-4 4l4 4", "M17 8l4 4l-4 4", "M3 12l18 0"];
    /// Scissors: the slice tool.
    SCISSORS = "scissors", false, [
        "M6 7m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
        "M6 17m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
        "M8.6 8.6l10.4 10.4",
        "M8.6 15.4l10.4 -10.4",
    ];
    /// An eraser: the delete tool.
    ERASER = "eraser", false, [
        "M19 20h-10.5l-4.21 -4.3a1 1 0 0 1 0 -1.41l10 -10a1 1 0 0 1 1.41 0l5 5a1 1 0 0 1 0 1.41l-9.2 9.3",
        "M18 13.3l-6.3 -6.3",
    ];
    /// A magnet: snap.
    MAGNET = "magnet", false, [
        "M4 13v-8a2 2 0 0 1 2 -2h1a2 2 0 0 1 2 2v8a2 2 0 0 0 6 0v-8a2 2 0 0 1 2 -2h1a2 2 0 0 1 2 2v8a8 8 0 0 1 -16 0",
        "M4 8l5 0",
        "M15 8l4 0",
    ];
    /// A check mark.
    CHECK = "check", false, ["M5 12l5 5l10 -10"];
    /// A 3 x 3 grid.
    GRID_3X3 = "grid-3x3", false, ["M3 8h18", "M3 16h18", "M8 3v18", "M16 3v18"];
    /// A window split in two columns: layouts.
    LAYOUT_COLUMNS = "layout-columns", false, ["M4 4m0 2a2 2 0 0 1 2 -2h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2z", "M12 4l0 16"];
    /// A clock with a turning arrow: history.
    HISTORY = "history", false, ["M12 8l0 4l2 2", "M3.05 11a9 9 0 1 1 .5 4m-.5 5v-5h5"];
    /// A filled disc.
    FILLED_CIRCLE = "filled-circle", true, ["M7 3.34a10 10 0 1 1 -4.995 8.984l-.005 -.324l.005 -.324a10 10 0 0 1 4.995 -8.336z"];
    /// A dashed rectangle: marquee selection.
    MARQUEE_2 = "marquee-2", false, [
        "M4 6v-1a1 1 0 0 1 1 -1h1m5 0h2m5 0h1a1 1 0 0 1 1 1v1m0 5v2m0 5v1a1 1 0 0 1 -1 1h-1m-5 0h-2m-5 0h-1a1 1 0 0 1 -1 -1v-1m0 -5v-2",
    ];
    /// An i in a circle: information.
    INFO_CIRCLE = "info-circle", false, ["M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0", "M12 9h.01", "M11 12h1v4h1"];
    /// An exclamation mark in a triangle: a warning.
    ALERT_TRIANGLE = "alert-triangle", false, [
        "M12 9v4",
        "M10.363 3.591l-8.106 13.534a1.914 1.914 0 0 0 1.636 2.871h16.214a1.914 1.914 0 0 0 1.636 -2.87l-8.106 -13.536a1.914 1.914 0 0 0 -3.274 0z",
        "M12 16h.01",
    ];

    // Drawn for pixel3d in the same style (MIT OR Apache-2.0).

    /// A large and a small sparkle: the assistant.
    SPARKLES = "sparkles", false, [
        "M10 3l1.6 4.4l4.4 1.6l-4.4 1.6l-1.6 4.4l-1.6 -4.4l-4.4 -1.6l4.4 -1.6z",
        "M17 13l.9 2.1l2.1 .9l-2.1 .9l-.9 2.1l-.9 -2.1l-2.1 -.9l2.1 -.9z",
    ];

    // Drawn for Bodo in the same style.

    /// A triangle pointing right: play.
    PLAYER_PLAY = "player-play", true, ["M7.5 5.8v12.4a1.2 1.2 0 0 0 1.8 1l10.2 -6.2a1.2 1.2 0 0 0 0 -2l-10.2 -6.2a1.2 1.2 0 0 0 -1.8 1z"];
    /// A rounded square: stop.
    PLAYER_STOP = "player-stop", true, ["M5.5 7.5a2 2 0 0 1 2 -2h9a2 2 0 0 1 2 2v9a2 2 0 0 1 -2 2h-9a2 2 0 0 1 -2 -2z"];
    /// A disc: record.
    PLAYER_RECORD = "player-record", true, ["M12 12m-6.5 0a6.5 6.5 0 1 0 13 0a6.5 6.5 0 1 0 -13 0"];
    /// Two bars: pause.
    PLAYER_PAUSE = "player-pause", true, [
        "M6 6.5a1.5 1.5 0 0 1 1.5 -1.5h2a1.5 1.5 0 0 1 1.5 1.5v11a1.5 1.5 0 0 1 -1.5 1.5h-2a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M13 6.5a1.5 1.5 0 0 1 1.5 -1.5h2a1.5 1.5 0 0 1 1.5 1.5v11a1.5 1.5 0 0 1 -1.5 1.5h-2a1.5 1.5 0 0 1 -1.5 -1.5z",
    ];
    /// A pencil: the draw tool.
    PENCIL = "pencil", false, ["M5 19l1 -4l9.5 -9.5a2.12 2.12 0 0 1 3 3l-9.5 9.5z", "M13.5 7.5l3 3", "M6 15l3 3"];
    /// A paint brush: the paint tool.
    BRUSH = "brush", false, [
        "M4 20c0 -2.5 1 -4.5 3.5 -4.5a2.5 2.5 0 0 1 2.5 2.5c0 1.5 -1.5 2 -3 2z",
        "M9.5 13.5l8.5 -8.5a1.5 1.5 0 0 1 2 2l-8.5 8.5",
        "M9.5 13.5l2 2",
    ];
    /// A speaker with sound waves: volume.
    VOLUME = "volume", false, ["M4 9h3l5 -4v14l-5 -4h-3z", "M15.5 9a4 4 0 0 1 0 6", "M18 6.5a8 8 0 0 1 0 11"];
    /// A speaker with a cross: muted.
    VOLUME_OFF = "volume-off", false, ["M4 9h3l5 -4v14l-5 -4h-3z", "M15.5 9.5l5 5", "M20.5 9.5l-5 5"];
    /// A magnifying glass with a plus: zoom in.
    ZOOM_IN = "zoom-in", false, ["M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0", "M7 10h6", "M10 7v6", "M21 21l-6 -6"];
    /// A magnifying glass with a minus: zoom out.
    ZOOM_OUT = "zoom-out", false, ["M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0", "M7 10h6", "M21 21l-6 -6"];
    /// A horizontal line: minimize, or subtract.
    MINUS = "minus", false, ["M5 12h14"];
    /// A rounded square outline: maximize.
    SQUARE = "square", false, ["M5 7a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2z"];
    /// Two overlapping squares: restore a maximized window.
    WINDOW_RESTORE = "window-restore", false, [
        "M4 10a2 2 0 0 1 2 -2h8a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-8a2 2 0 0 1 -2 -2z",
        "M8 8v-2a2 2 0 0 1 2 -2h8a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-2",
    ];
    /// A box with an arrow leaving it: detach a window.
    EXTERNAL_LINK = "external-link", false, ["M12 6h-6a2 2 0 0 0 -2 2v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2 -2v-6", "M11 13l9 -9", "M15 4h5v5"];
    /// A chevron pointing left.
    CHEVRON_LEFT = "chevron-left", false, ["M15 6l-6 6l6 6"];
    /// An arrow pointing up: the parent folder.
    ARROW_UP = "arrow-up", false, ["M12 19v-14", "M6 11l6 -6l6 6"];
    /// An arrow pointing left: back.
    ARROW_LEFT = "arrow-left", false, ["M19 12h-14", "M11 6l-6 6l6 6"];
    /// An arrow pointing right: forward, or follow the playhead.
    ARROW_RIGHT = "arrow-right", false, ["M5 12h14", "M13 6l6 6l-6 6"];
    /// Two beamed notes: music.
    MUSIC = "music", false, [
        "M6 17m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
        "M16 15m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
        "M9 17v-12l10 -2v12",
        "M9 9l10 -2",
    ];
    /// A page with a note: an audio file.
    FILE_MUSIC = "file-music", false, [
        "M14 3v4a1 1 0 0 0 1 1h4",
        "M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z",
        "M10.5 16m-1.5 0a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0",
        "M12 16v-5l2.5 1.5",
    ];
    /// A metronome.
    METRONOME = "metronome", false, ["M8.5 4h7l3.5 16h-14z", "M12 16l5 -9", "M7 16h10"];
    /// A microphone: record audio.
    MICROPHONE = "microphone", false, ["M9 5a3 3 0 0 1 6 0v5a3 3 0 0 1 -6 0z", "M5 10a7 7 0 0 0 14 0", "M8 21h8", "M12 17v4"];
    /// A shopping bag: the marketplace.
    SHOPPING_BAG = "shopping-bag", false, [
        "M6.3 8h11.4a1 1 0 0 1 1 .9l.9 10a1 1 0 0 1 -1 1.1h-13.2a1 1 0 0 1 -1 -1.1l.9 -10a1 1 0 0 1 1 -.9z",
        "M9 11v-4a3 3 0 0 1 6 0v4",
    ];
    /// Lines of a list with a note: the playlist.
    PLAYLIST = "playlist", false, ["M4 6h12", "M4 11h12", "M4 16h7", "M17 18m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0", "M19 18v-11l2 1"];
    /// Piano keys: the piano roll.
    PIANO = "piano", false, [
        "M3 6a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z",
        "M7.5 4v9h3v-9",
        "M13.5 4v9h3v-9",
        "M9 13v7",
        "M15 13v7",
    ];
    /// Three dots: more options.
    DOTS = "dots", false, [
        "M5 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
        "M12 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
        "M19 12m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0",
    ];
    /// A window with a top bar and a side bar: window layouts.
    LAYOUT = "layout", false, ["M4 6a2 2 0 0 1 2 -2h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2z", "M4 9h16", "M10 9v11"];
    /// Rows of small squares: the channel rack's step sequencer.
    CHANNEL_RACK = "channel-rack", false, [
        "M3 5.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M10 5.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M17 5.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M3 11.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M10 11.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M17 11.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M3 17.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M10 17.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
        "M17 17.5a1.5 1.5 0 0 1 1.5 -1.5h1a1.5 1.5 0 0 1 1.5 1.5v1a1.5 1.5 0 0 1 -1.5 1.5h-1a1.5 1.5 0 0 1 -1.5 -1.5z",
    ];
    /// A drum seen from the side, with two sticks: a drum insert.
    DRUM = "drum", false, [
        "M4 10a8 3 0 1 0 16 0a8 3 0 1 0 -16 0",
        "M4 10v6c0 1.7 3.6 3 8 3s8 -1.3 8 -3v-6",
        "M14 3l-3 5",
        "M10 3l3 5",
    ];
    /// An acoustic guitar: a guitar insert.
    GUITAR = "guitar", false, [
        "M10.5 21c-2.8 0 -5 -2.2 -5 -5c0 -1.3 .5 -2.3 1.3 -3c-.2 -1.9 1.3 -3.5 3.2 -3.5c.9 0 1.7 .3 2.3 .9l4.7 -4.7",
        "M10.5 21c2.8 0 5 -2.2 5 -5c0 -1.3 -.5 -2.3 -1.3 -3",
        "M15.5 4.5l2 -2l2 2l-2 2z",
        "M9.5 15.5m-1.5 0a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0 -3 0",
    ];
    /// Headphones: a monitoring insert.
    HEADPHONES = "headphones", false, [
        "M4 15v-3a8 8 0 0 1 16 0v3",
        "M4 15a2 2 0 0 1 2 -2h1v7h-1a2 2 0 0 1 -2 -2z",
        "M20 15a2 2 0 0 0 -2 -2h-1v7h1a2 2 0 0 0 2 -2z",
    ];
    /// Three nodes joined by lines: the mixer's routing graph.
    ROUTING = "routing", false, [
        "M6 6m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M6 18m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M18 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M7.7 7.1l8.6 3.8",
        "M7.7 16.9l8.6 -3.8",
    ];
    /// Two arrows chasing each other round a dot: loop recording.
    LOOP_RECORD = "loop-record", false, [
        "M4 12a8 8 0 0 1 13.7 -5.6",
        "M18 3v4h-4",
        "M20 12a8 8 0 0 1 -13.7 5.6",
        "M6 21v-4h4",
        "M12 12m-2 0a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
    ];
}
