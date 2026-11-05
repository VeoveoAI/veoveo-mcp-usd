# USD MCP Server — Agent Manual

Delta over the repository root `AGENTS.md`. [`DESIGN.md`](DESIGN.md) is the design and
operational contract.

## Purpose

Shows USD stages that live in Lore: `list_scenes`, `inspect_stage`, `render_stage` as a Task, the App `ui://usd/scenes` and the
WASM viewer the Hub opens USD files with.

## Invariants

- Owns the `usd://` URI scheme. It has no tables. Identity: slug `usd`, MCP `/usd/mcp`, viewer `/usd/viewer/`, port 8806.
- Every read of a scene goes through the Hub's routes with `hub_bearer`, as the caller or the Task's owner, so Lore's rules decide
  it. Never read Lore another way, and never keep what was read past the request or the step.
- A tool argument never names a tenant or a person: those come from the forwarded identity.
- Ready `Routes` bind typed tool requests and readers to their output, permission and cache policy. Discovery, registration,
  annotations and dispatch derive from those routes; do not maintain a separate requirements table.
- Addresses come from `UsdResource` through the shared route declaration. Do not build or split a URI by hand.
- `render_stage` stores its PNG through `ArtifactClient` bound to the Task, with an idempotency key from the Task's identity.
- The viewer asks only its own origin, and the public route serves only the viewer's published files.
- `skills/view-stages/` is the server's skill. The setup's test holds that the instructions name every skill, and `tests/mcp.rs`
  that the manifest names exactly the skill's files, each with the digest and size of its text.
- Copied crates change only as `UPSTREAM.md` records.

## Build And Test

- `cargo test -p veoveo-mcp-usd`, as the `dev` image provides it.
- `cargo test -p veoveo-mcp-usd --no-default-features --features contract`
  checks the contract alone.
- `tests/mcp.rs` calls the server through the SDK's test Hub.
- `docker build --target check .` runs the gate.
