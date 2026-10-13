# ADR 0012: Window model: two side panels and free windows

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

project.md section 14.1 planned a docked layout: every panel lived in a `craft-ui` split, tab or stack, and could be floated out. The owner's layout sketches (2026-10-13) ask for something else:

- The Browser stays on the left and the AI Assistant on the right, attached to the main window.
- The Channel Rack floats inside the main window and can be maximized.
- Every other window (the Playlist, the Mixer, the Marketplace and the rest) is its own window: it can be moved anywhere, maximized, made full screen and closed.

This is how people coming from FL Studio expect to work: they arrange windows freely, often across two monitors.

## Decision

- **The main window** has a two-row top bar, the Browser docked on the left, the AI Assistant docked on the right, and a workspace between them.
- **Side panels:** the Browser and the Assistant can be resized, hidden and shown again from the top bar. The Assistant can also collapse to a thin strip. Neither can leave the main window.
- **The Channel Rack** is a floating window inside the workspace. It can be moved, resized, maximized to fill the workspace, minimized and closed. It can't leave the main window.
- **Free windows:** the Playlist, Piano Roll, Mixer, Marketplace and channel windows (the Sampler and other instruments) are free windows. Each one:
  - opens inside the workspace, at the position the current layout gives it;
  - has a title bar with its options menu, title, minimize, maximize and close;
  - can be moved, resized, maximized to fill the workspace, minimized and closed;
  - can be detached into its own OS window, by dragging it out of the main window or with a title-bar button. Detached, it can go to another monitor and full screen, and it can be docked back.
- **Plugin editors** are always separate OS windows, owned by the main window (project.md section 7.4).
- **Layouts:** a layout records which windows are open, where they are, their sizes, and whether they are detached and on which monitor. The owner's sketch is the default layout. Layouts are saved in the user's settings, and the Layouts button in the top bar switches between them.
- **Implementation:** windows inside the workspace are egui windows clipped to the workspace area. Detached windows are egui viewports, which eframe renders as native OS windows on the wgpu backend. The side panels use `craft-ui` panels. Every window draws through the same widgets and tokens, whether or not it is detached.

## Consequences

- Users can arrange their work freely, including across monitors, which the sketches ask for.
- Every visible OS window renders its own frames. The repaint rules in project.md section 14.6 apply to each one, so idle windows cost nothing.
- Window state belongs to the user, not to the project document. Reopening a project never moves windows around unexpectedly.
- Keyboard focus has to follow the active window, including detached ones, so that shortcuts such as Space for play and stop work everywhere.
- UI snapshot tests cover each window on its own and the default layout as a whole.

## Alternatives considered

- **The fully docked layout from the first version of section 14.1:** tidy, but it can't give the free movement and full-screen windows the owner asked for.
- **Every window a native OS window from the start:** windows would lose their place inside the app, and the default layout would scatter over the desktop on small screens.
- **Letting the Browser and Assistant float too:** the owner wants them fixed at the sides, where they are always within reach.
