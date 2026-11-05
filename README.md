# veoveo-mcp-usd

The Veoveo MCP server for OpenUSD scenes, hosted through
[veoveo-hub-sdk](https://github.com/VeoveoAI/veoveo-hub-sdk), which registers it with the Hub and verifies the identity
the Hub forwards, and storing its state through [veoveo-core](https://github.com/VeoveoAI/veoveo-core). It started from
[veoveo-template-mcp-rust](https://github.com/VeoveoAI/veoveo-template-mcp-rust) and still carries that template's item
domain, which the USD server replaces.

## Contents

| Path | Purpose |
|---|---|
| `Dockerfile` | The `dev` environment, the `check` gate, the release `build` and the `runtime` image |
| `env/profile.d/rust.sh` | Toolchain variables for login shells inside the environment |
| `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` | The pinned toolchain and code rules |
| `deny.toml` | Dependency licenses, advisories, bans and allowed sources |
| `.config/nextest.toml` | Test profiles for local runs and machine-readable results |
| `.cargo/config.toml` | `SQLX_OFFLINE`, so every build reads the committed query metadata |
| `.sqlx/` | sqlx's checked metadata for every query the workspace compiles |
| `crates/usd_bevy/`, `crates/usd_macro/`, `assets/`, `upstream-examples/`, `UPSTREAM.md` | OpenUSD for Bevy, copied from `bevy_openusd`, and what we changed |
| `crates/veoveo-mcp-usd/` | The server: its contract, MCP surface, store, binary, migrations, tests and the documents it serves |
| `crates/*/migrations/` | The server's schema, applied in its own PostgreSQL schema at startup |

## Server

The crate is a working MCP server hosted through veoveo-hub-sdk. It keeps a list of
named items for each tenant: the `create_item` tool records one and returns its address
as `resultUri`, `usd://item/{item_id}` reads it, and `usd://items` pages
through the tenant's items. The crate's [`DESIGN.md`](crates/veoveo-mcp-usd/DESIGN.md)
describes the domain, and its [`AGENTS.md`](crates/veoveo-mcp-usd/AGENTS.md)
holds its authored operating manual. The server serves both documents under
`usd://docs`.

| Module | Holds |
|---|---|
| `src/contract/` | The `contract` feature: `ItemId` through the shared `id` declaration, `UsdResource` through `resource_address`, the `Vocabulary` scope and permissions, and the tool's request and result |
| `src/mcp/` | One typed `Routes` table for tools, resource readers and access policies; setup with `server_docs!`; registration and permission requirements derived from those routes |
| `src/store/` | The `usd` schema's statements and the keyset listing |
| `src/server/` | Configuration, registration with the Hub, the database login it issues, the process instance and serving |

At startup the server registers with the Hub, connects with the database login the
Hub issues, starts a process instance, applies its migrations and serves MCP at
`{public_base_url}/usd/mcp`. Process stop retains the native saved registration for restart; explicit retirement deletes it.

`tests/mcp.rs` calls the server through the SDK's in-process test Hub, with the
permissions each case grants, against a migrated database.

## Database

The server owns one tenant-scoped table, `usd.item`, and one listing over it. The
listing pages by key with veoveo-core's `PageSize` and a `KeysetCursor` bound to the
tenant, through a first-page and a next-page statement. Every statement is a `.sql`
file that sqlx checks at compile time against the migrated schema.

The statements and tables follow the server schema rules of veoveo-core's
[database design](https://github.com/VeoveoAI/veoveo-core/blob/9a6b3ff95469ff19f9ccb513421a57ba6751aa9e/docs/DATABASE.md):
tenant predicates, row bounds, index use under generic plans, and a primary key, an
index behind every foreign key and a `Retention:` comment on every table.
The crate's `tests/database_conformance.rs` checks them against a fresh database. Tests need the
PostgreSQL 18 server programs on `PATH`; each test clones its own database from a
migrated database template.

After changing a query or a migration, regenerate `.sqlx` against a database with the
kernel migrations of the pinned veoveo-core and the crate's migrations applied, from a
target directory with no earlier build:

```sh
CARGO_TARGET_DIR=<empty directory> cargo sqlx prepare --workspace -- --all-targets --all-features
```

The metadata also covers the queries of the core crates, and prepare records only the
crates it compiles. The `check` stage of the `Dockerfile` builds that database and fails
when the metadata is stale.

## Tasks, providers, usage and audit

The crate runs no Tasks yet. A server that does runs them through the SDK's
`TaskWorker`, where `ctx.step` keeps a restarted Task from repeating finished work and
`ctx.operation` with `ctx.wait` parks a Task on an outside provider until the
provider's event reaches the server's callback route; the SDK's
[Task work](https://github.com/VeoveoAI/veoveo-hub-sdk/blob/main/crates/veoveo-hub-sdk/DESIGN.md#task-work) describes both. Metered or billed work records an estimate
and the actual usage through `ctx.usage()` against usage kinds the server declares, in
core's [usage ledger](https://github.com/VeoveoAI/veoveo-core/blob/9a6b3ff95469ff19f9ccb513421a57ba6751aa9e/platform/usage/DESIGN.md). Audit classes, reason codes and
target URIs are the server's own, under its slug, as core's
[audit record contract](https://github.com/VeoveoAI/veoveo-core/blob/9a6b3ff95469ff19f9ccb513421a57ba6751aa9e/platform/audit/contract/DESIGN.md) describes.

## Runtime image

Build the runtime image with its version and source revision:

```sh
docker build --target runtime -t <repository>:<version> \
  --build-arg VERSION=<version> --build-arg REVISION="$(git rev-parse HEAD)" .
```

## Gate

```sh
docker build --target check .
```

The `check` stage checks the `.sqlx` metadata against a migrated PostgreSQL 18.6
database, then runs rustfmt, clippy with warnings denied, cargo-deny, cargo-hack on each
feature and the nextest `ci` profile. It runs the same way on macOS and Linux.

## Development environment

```sh
docker build --target dev -t <repository>:dev .
```

An installation from
[veoveo-template-installation](https://github.com/VeoveoAI/veoveo-template-installation)
runs this image, under its own layer, as a Compose service that works in the
installation's Lore repository at `/sandbox/src` and reaches PostgreSQL at
`vv-postgres.internal:5432`. Add the component to the installation like `vv-mcp-time`:
a service with its `.internal` alias and Lore token volume, a `vv-lore-tokens` target,
a `--lore-environment` for the Hub's Lore clients, and a published port on the edge.

Compose services and containers use a short form of the repository name: `vv-` plus a
short kind and the component name. The kinds are `srv` for services, `mcp` for MCP
servers, `rt` for runtimes, `app`, `sdk`, `tpl` for templates, `demo` for showcases and
`qual` for qualification suites, and repositories without a kind use `vv-<name>`. For
example, `veoveo-hub` runs as `vv-srv-hub` and `veoveo-mcp-time` as
`vv-mcp-time`. Repositories, crates, binaries and images keep their full names.

Inside the environment, the `sandbox` user owns the Rust toolchain and Cargo home, so
rustup and Cargo install components, targets and crates as needed through the
installation's proxy. A coding agent works in the environment's Lore working copy,
pushes its own branch to Lore, and builds and tests inside the environment.

## License

Apache-2.0. See [LICENSE](LICENSE).

## Registration And Offline Description

`VEOVEO_DEPLOYMENT_ID` (`--deployment-id`) is a UUIDv7 shared by registration and
token verification; Hub supplies it with the registration token. Keep it and the
private key directory on ordinary process restart.

The server registers with `VEOVEO_REGISTRATION_SECRET` on its first start and resumes
its saved registration afterwards, including the original private database grant;
process stop does not retire it. When its declarations change, or the Hub no longer
knows the saved registration, it registers again with the same token and replaces
the saved state.

To print the public registration metadata offline, reusing the key in
`--key-directory`, without registering or connecting to Hub or the database:

```sh
veoveo-mcp-usd --describe-registration \
  --deployment-id <allocated-uuidv7> \
  --public-base-url https://candidate.example \
  --key-directory /sandbox/builds/<build-id>/keys
```


The SDK serves readiness at the listener root, `/readyz`. The image installs this
readiness check after registration, database connection and migration, and
reports unavailable if its kernel instance is lost. Server authors should bind
`HostedServer::readiness` to their actual application prerequisites. This probe is
not authenticated MCP qualification; discovery, schemas and tools must still pass
through an ordinarily admitted selected Hub session.
