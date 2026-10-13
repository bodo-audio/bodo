# ADR 0019: Agents reach the running app through the control channel

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M2 ships the basic MCP server (project.md section 12.3, ADR 0005). An external agent must be able to work headless on a file, and also on the running app, where the user watches and the agent's edits share the user's undo history. Section 12.4 plans a token-protected loopback control channel for UI automation, and ADR 0005 also plans a streamable HTTP transport inside the app.

`bodo-mcp`, `bodo-control` and `bodo-ui` all sit in layer L8, and a crate may depend only on lower layers (ADR 0011).

## Decision

- **Two stdio entry points** in `bodo-cli`: `bodo-cli mcp [file]` runs the MCP server on its own session; `bodo-cli mcp --connect [port]` runs the same server and forwards every tool call to the running app over the control channel.
- **The control channel is the one door into the app.** JSON lines over 127.0.0.1, a random 32-byte token sent in a `hello` first, at most 16 clients, and a 60 second answer timeout (the protocol follows pixel3d's). The app listens by default, on port 7870 when it is free and a random port otherwise, and publishes the port and token in files only the user can read (`<data>/control/`).
- **Methods:** `mcp.call` runs an MCP tool on the app's own session; `ui.inspect`, `ui.click`, `ui.drag`, `ui.key`, `ui.text` and `ui.screenshot` read and drive the interface with real input events; `ui.command` and `ui.commands` run and list commands.
- **Peers in L8 meet in the app.** None of the three L8 crates depends on another:
  - `bodo-mcp`'s `Remote` sends requests through any function, and `bodo-cli` supplies one backed by `bodo-control`'s client;
  - `bodo-ui` takes requests through its `ControlSource` trait and answers unknown methods through an extension hook; `apps/bodo` plugs in `bodo-control`'s server and answers `mcp.call` with `bodo-mcp`.
- **The HTTP transport waits for M7,** when the assistant needs a catalog in the app anyway. Clients that only speak HTTP can't connect to the app until then; stdio clients can.

## Consequences

- One protocol serves agents, UI tests and the user's own scripts, and the layer rules hold with no exceptions.
- Agent edits in the app land in the user's history and can be undone like the user's own.
- Any local process that can read the user's files can drive Bodo; the token stops other users, web pages and processes without that access. Restricting agents to the project's folders is part of the assistant's safety work in M7.
- The app needs a few lines of glue for each new peer crate.

## Alternatives considered

- **HTTP MCP inside the app now:** a second network protocol and an async runtime in the app before anything needs them.
- **A layer exception** letting `bodo-ui` and `bodo-mcp` use `bodo-control`: it works, but it weakens the rule for a few lines of glue.
- **Off by default:** agents are a core part of Bodo, and the token keeps the default safe.
