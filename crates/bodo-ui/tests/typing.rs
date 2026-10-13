//! The typing keyboard (project.md section 14.1): the top bar's toggle, notes held while their keys are, and the
//! letters playing notes instead of choosing tools.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_ui::Tool;
use egui::{Key, Theme};

#[test]
fn the_computer_keyboard_plays_the_selected_channel_while_it_is_on() {
    let mut h = common::harness(common::app(Theme::Dark), Theme::Dark, false);
    h.run_steps(3);
    common::click(&mut h, "Typing keyboard");
    assert!(h.state().view.typing.on);
    let kick = h.state().session.project().rack_order[0];

    h.key_down(Key::Z);
    h.step();
    h.key_down(Key::B);
    h.step();
    let held: Vec<(Key, u8)> = h
        .state()
        .view
        .typing
        .held
        .iter()
        .map(|(key, channel, note)| {
            assert_eq!(*channel, kick, "notes play the selected channel");
            (*key, *note)
        })
        .collect();
    assert_eq!(held, [(Key::Z, 60), (Key::B, 67)], "C4 and G4");
    assert_eq!(h.state().view.playlist.tool, Tool::Draw, "B played a note instead of choosing the paint tool");

    h.key_up(Key::Z);
    h.step();
    assert_eq!(h.state().view.typing.held.iter().map(|(_, _, n)| *n).collect::<Vec<_>>(), [67]);
    h.key_down(Key::Q);
    h.step();
    assert!(h.state().view.typing.held.iter().any(|(_, _, n)| *n == 72), "Q plays C5");

    // Turning it off lets go of everything, and the letters choose tools again.
    common::click(&mut h, "Typing keyboard");
    assert!(!h.state().view.typing.on && h.state().view.typing.held.is_empty());
    h.key_press(Key::B);
    h.step();
    assert_eq!(h.state().view.playlist.tool, Tool::Paint);
}
