# Upstream

`crates/usd_bevy`, `crates/usd_macro`, `assets/` and `upstream-examples/` are copied from
[`openusd-rs/bevy_openusd`](https://github.com/openusd-rs/bevy_openusd), MIT, on its `develop` branch at
`5b3ebbc7e4e5da0808dcf3898df96c5d4a63b583`. Its `LICENSE` stays beside each. This is a copy and not a fork or a dependency, so an upstream change reaches
this repository only when someone copies it here and records it below.

## What was copied

| From | To |
|---|---|
| `crates/usd_bevy/` | `crates/usd_bevy/` |
| `crates/usd_macro/` | `crates/usd_macro/` |
| The top-level files of `assets/` | `assets/`, where the crate's tests read them |
| `examples/gif_frames.rs` | `upstream-examples/gif_frames.rs`, not built yet, the start of the render subcommand |
| `examples/web/` | `upstream-examples/web/`, not built yet, the start of the WASM viewer |

Left out: `assets/external`, `media`, `src` (`usdview`) and its dependency `mara`, `benchmarks`, `scripts` and
`vendor/bevy_asset`.

## What we changed

Nothing yet: this commit is the copy.
