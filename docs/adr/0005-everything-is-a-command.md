# ADR 0005: Everything is a command, and MCP is generated from the registry

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

The UI, shortcuts, the command palette, scripts, external agents and the built-in assistant must all be able to change a project, and they must all behave the same way. They also need the same undo, validation and permission checks. Both pixel3d (`run(id, json)`) and soundcraft (`engine.execute(id, params)` plus MCP) show that one command registry makes an app scriptable and agent-ready almost for free.

## Decision

- **One path for every change:** every project mutation is a registered `CommandSpec`:
  - an id (`namespace.verb`);
  - a JSON Schema for its params (`schemars`);
  - `effects` (undoable, destructive, disk, network);
  - an optional `merge_key`, plus its menu entry, shortcut and summary.

  It runs through `bodo-engine::execute`.
- **Transactions:** a command validates its params, applies to a scratch snapshot, then runs `Project::validate()`. The result is committed or rolled back. A `batch` is one transaction and one undo entry.
- **The UI never owns document data.** Gestures send fast parameter messages while they are in progress, and record one merged command when they end.
- **`bodo-mcp`** generates its tool catalog from the registry. It adds:
  - the generic tools: `list_commands`, `describe_command`, `execute`, `batch`;
  - perception tools;
  - skills, which are themselves commands that expand into batches;
  - live-GUI tools through `bodo-control`.
- **Transports:** in-process for the assistant, stdio (`bodo-cli mcp`), and streamable HTTP. The HTTP transport binds to loopback only and requires a bearer token and a valid `Origin`.
- **Tool profiles:** `full`, `standard` and `small` (about 15 tools, for local models).
- **The permission policy lives in the engine,** keyed on `effects`, not in prompts.

## Consequences

- New features are reachable from MCP and the palette the day they land.
- Agent behavior can be tested with deterministic MCP-only task tests.
- Every command needs a schema, a validation story and snapshot tests. That costs more up front and pays off in reliability.
- The MCP server ships early (milestone M2), so external agents can help build and test Bodo.

## Alternatives considered

- **Hand-written MCP tools separate from the UI actions:** they would drift apart and double the maintenance.
- **Direct model mutation from the UI:** no consistent undo or validation, and the agent could not reach those actions.
