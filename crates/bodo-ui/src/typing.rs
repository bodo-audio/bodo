//! The typing keyboard (project.md section 14.1): while it is on, the computer keyboard plays the selected channel,
//! two rows of keys as two octaves the way FL lays them out. Z to M is the octave from C4, with S, D, G, H and J as
//! its black keys; Q to U is the octave from C5, with 2, 3, 5, 6 and 7 as its black keys. The notes go through the
//! engine's live input, so a recording takes them like notes from a MIDI keyboard. While it is on, those keys play
//! notes instead of choosing tools.

use bodo_model::ChannelId;
use egui::{Event, Key, Modifiers};

use crate::BodoApp;

/// The key of the lowest note, C4.
const BASE: u8 = 60;

/// Each key and its distance in semitones from [`BASE`].
const LAYOUT: [(Key, u8); 34] = [
    (Key::Z, 0),
    (Key::S, 1),
    (Key::X, 2),
    (Key::D, 3),
    (Key::C, 4),
    (Key::V, 5),
    (Key::G, 6),
    (Key::B, 7),
    (Key::H, 8),
    (Key::N, 9),
    (Key::J, 10),
    (Key::M, 11),
    (Key::Comma, 12),
    (Key::L, 13),
    (Key::Period, 14),
    (Key::Semicolon, 15),
    (Key::Slash, 16),
    (Key::Q, 12),
    (Key::Num2, 13),
    (Key::W, 14),
    (Key::Num3, 15),
    (Key::E, 16),
    (Key::R, 17),
    (Key::Num5, 18),
    (Key::T, 19),
    (Key::Num6, 20),
    (Key::Y, 21),
    (Key::Num7, 22),
    (Key::U, 23),
    (Key::I, 24),
    (Key::Num9, 25),
    (Key::O, 26),
    (Key::Num0, 27),
    (Key::P, 28),
];

/// Whether the typing keyboard is on, and the notes its keys hold.
#[derive(Debug, Default)]
pub struct Typing {
    /// Whether the computer keyboard plays notes.
    pub on: bool,
    /// The keys held down, with the channel and key each one plays.
    pub held: Vec<(Key, ChannelId, u8)>,
}

/// Plays and releases notes for this frame's key presses, and takes the presses so they don't also run shortcuts.
/// Turning the keyboard off, or typing into a text field, releases every held note.
///
/// Complexity: O(e) in this frame's events.
pub(crate) fn handle(app: &mut BodoApp, ctx: &egui::Context) {
    if !app.view.typing.on || ctx.egui_wants_keyboard_input() {
        release_all(app);
        return;
    }
    let project = app.session.project();
    let Some(channel) = app.session.editor().channel.filter(|c| project.channels.contains_key(c)).or_else(|| project.rack_order.front().copied())
    else {
        return;
    };
    let events: Vec<(Key, bool, bool)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed, repeat, modifiers, .. } if !(modifiers.command || modifiers.ctrl || modifiers.alt) => {
                    Some((*key, *pressed, *repeat))
                }
                _ => None,
            })
            .collect()
    });
    for (key, pressed, repeat) in events {
        let Some(offset) = LAYOUT.iter().find(|(k, _)| *k == key).map(|(_, o)| *o) else { continue };
        if pressed && !repeat && !app.view.typing.held.iter().any(|(k, ..)| *k == key) {
            let note = BASE + offset;
            app.audio.play_note(channel, note, 0.8, true);
            app.view.typing.held.push((key, channel, note));
        } else if !pressed && let Some(index) = app.view.typing.held.iter().position(|(k, ..)| *k == key) {
            let (_, channel, note) = app.view.typing.held.remove(index);
            app.audio.play_note(channel, note, 0.0, false);
        }
        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, key));
    }
}

/// Releases every note the keyboard holds.
fn release_all(app: &mut BodoApp) {
    for (_, channel, note) in std::mem::take(&mut app.view.typing.held) {
        app.audio.play_note(channel, note, 0.0, false);
    }
}
