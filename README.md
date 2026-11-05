# veoveo-mcp-usd

The Veoveo MCP server for OpenUSD scenes, hosted through
[veoveo-hub-sdk](https://github.com/VeoveoAI/veoveo-hub-sdk), which registers it with the Hub and verifies the identity
the Hub forwards. It lists the USD scenes in the Lore repositories a person can read, reports what a stage holds, renders a
stage to a PNG in Artifact on the GPU, and serves the WASM viewer the Hub opens USD files with. It started from
[veoveo-template-mcp-rust](https://github.com/VeoveoAI/veoveo-template-mcp-rust). The factory builds and serves it.

## Contents

| Path | Purpose |
|---|---|
| `Dockerfile` | The `dev` environment and the `check` gate |
| `openshell/` | The factory's image, with the GPU opt-in, and `run`, which builds the viewer and the server and runs the server |
| `env/profile.d/rust.sh` | Toolchain variables for login shells inside the environment |
| `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` | The pinned toolchain and code rules |
| `deny.toml` | Dependency licenses, advisories, bans and allowed sources |
| `.config/nextest.toml` | Test profiles for local runs and machine-readable results |
| `.cargo/config.toml` | `SQLX_OFFLINE` for core's crates, and the viewer's random source |
| `crates/usd_bevy/`, `crates/usd_macro/`, `assets/`, `UPSTREAM.md` | OpenUSD for Bevy, copied from `bevy_openusd`, and what we changed |
| `crates/veoveo-mcp-usd/` | The server: its contract, MCP surface, Hub calls, render Task and subcommand, App, tests and the documents it serves |
| `crates/usd-viewer/` | The WASM viewer, on WebGPU: it loads a stage by path on its own origin, with panels for scenes, prims, variants, payloads, time and cameras |

## Server

The crate's [`DESIGN.md`](crates/veoveo-mcp-usd/DESIGN.md) describes the domain, and its
[`AGENTS.md`](crates/veoveo-mcp-usd/AGENTS.md) holds its authored operating manual. The server serves both documents under
`usd://docs`.

| Tool | Does |
|---|---|
| `list_scenes` | The repositories you can read, or the scenes of one |
| `inspect_stage` | What a stage holds, read with `openusd` in the server, as you, with no checkout or disk |
| `render_stage` | A Task that draws one frame on the GPU and stores a PNG in Artifact |

`ui://usd/scenes` is a small App that lists the same scenes and opens each in the viewer. `tests/mcp.rs` calls the server
through the SDK's in-process test Hub, with the permissions each case grants.

## Gate

```sh
docker build --target check .
```

The `check` stage runs rustfmt, clippy with warnings denied, on the workspace and on the viewer for wasm32, cargo-deny,
cargo-hack on each feature and the nextest `ci` profile. It runs the same way on macOS and Linux.

## Rendering and the viewer

`veoveo-mcp-usd render STAGE FRAMES.txt [--variant PRIM SET SELECTION]...` draws frames of a stage to PNG files on the GPU, with
no window, and refuses a software adapter. Each line of `FRAMES.txt` is `TIME OUTPUT.png`, optionally followed by a camera's eye
and focus or by `@/PRIM/PATH`, one of the stage's cameras. It prints `ADAPTER`, `BOUNDS`, `READY`, `FRAME` and `DONE` lines.
Under Docker, `--runtime=nvidia` and the factory image's own `ENV` give it the GPU.

`openshell/run` builds the viewer. To try it alone, serve `target/viewer/` and the stage from one origin and open
`index.html?file=/PATH/TO/STAGE.usdz`.

## Development environment

```sh
docker build --target dev -t <repository>:dev .
```

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


The SDK serves readiness at the listener root, `/readyz`, after registration and the database
connection. This probe is not authenticated MCP qualification; discovery, schemas and tools
must still pass through an ordinarily admitted selected Hub session.
