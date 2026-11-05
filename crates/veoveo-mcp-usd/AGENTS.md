# USD MCP Server — Agent Manual

Delta over the repository root `AGENTS.md`. [`DESIGN.md`](DESIGN.md) is the design and
operational contract.

## Purpose

A tenant's named items: `create_item` records one, `usd://item/{item_id}` reads it
and `usd://items` lists them. The domain is the smallest that exercises a tool with
a product, an exact read and a keyset listing. The USD server
replaces it with its own.

## Invariants

- Owns the `usd://` URI scheme and the PostgreSQL schema `usd`, and its SQL
  touches no other schema. Identity: slug `usd`, MCP `/usd/mcp`, port 8806.
- A caller's tenant comes from the forwarded identity. A tool argument never names it,
  and every statement binds it, so another tenant's item reads as missing.
- Ready `Routes` bind typed tool requests and resource readers to their output,
  permission and cache policy. Discovery, registration, annotations and dispatch
  derive from those routes; do not maintain a separate requirements table.
- Addresses come from `UsdResource` through the shared route declaration. Do not
  build or split a URI by hand.
- `create_item` returns `Product<CreatedItem>`, constructed with the item's typed
  address. The SDK generates matching `resultUri`, resource link and structured text.
- Every listing uses the native `Collection`/`Pager` and `ReadCollection` construction
  path. Owner resource addresses contain only path/filter fields; shared limit and
  cursor parsing belongs to the SDK.
- `skills/keep-items/` is the reference skill. The setup's test holds that the
  instructions name every skill, and `tests/mcp.rs` that the manifest names exactly the
  skill's files, each with the digest and size of its text.
- Every statement is a `.sql` file under `src/store/items/` run through sqlx's checked
  macros. Regenerate `.sqlx` after changing a query or a migration.

## Build And Test

- `cargo test -p veoveo-mcp-usd` with the PostgreSQL 18 server programs on
  `PATH`, as the `dev` image provides them.
- `cargo test -p veoveo-mcp-usd --no-default-features --features contract`
  checks the contract alone.
- `tests/mcp.rs` calls the server through the SDK's test Hub against a migrated
  database.
- `tests/database_conformance.rs` runs core's database checks over every statement and
  the `usd` schema.
- `docker build --target check .` runs the gate.
