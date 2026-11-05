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
| `examples/gif_frames.rs` | Ported to `crates/veoveo-mcp-usd/src/render.rs`, the `render` subcommand |
| `examples/web/` | `upstream-examples/web/`, not built yet, the start of the WASM viewer |

Left out: `assets/external`, `media`, `src` (`usdview`) and its dependency `mara`, `benchmarks`, `scripts` and
`vendor/bevy_asset`.

## What we changed

- **Bevy 0.20.1.** The `crates/` part of upstream's draft PR #4 (`dave-gray101/bevy_openusd`, `feat-bevy-0.20` at
  `5cbdee3f4c49`, 39 files, including shaders from WGSL to WESL), which targets Bevy 0.20.0 on `main`. It applied to `develop`
  with one conflict: `develop` had removed the test `package_entry_budget_preserves_existing_destination`, which the PR
  edits, so it stays removed. Its `usdview` and `mara` changes are left out. Then Bevy 0.20.1.
- **`zip` 9.3.0,** from 8, because 8.6.0 has open advisories (RUSTSEC-2026-0367, 0368, 0369 and 0429). `name()` now returns a
  `Result`, in `reload.rs` and in `persistence.rs`'s tests.
- **Deprecated calls.** `Mat4::perspective_rh` becomes `bevy::math::proj::perspective` in four tests, as Bevy's `glam` asks,
  because rustc warnings fail our gate. `crates/usd_bevy/src/read/xform.rs` keeps it, on upstream's own `glam` 0.30.
- **Manifests.** Both crates are `publish = false`, as `cargo deny` requires of path dependencies, and allow `clippy::all`,
  since upstream does not run clippy and fixing 110 findings would only move this copy further from it. rustc warnings,
  rustfmt, `cargo deny` and the tests still apply to them.
- **Tests that need a left-out asset are removed:**
  - `source::tests::usdc_empty_string_arrays_read_as_empty`, on `assets/external/chameleon.usdz`
  - `live::tests::loads_kitchen_usdz`, on `assets/external/Kitchen_set.usdz`

  Both scenes go into the `usd-demo` Lore repository.
- **No patched `bevy_asset`.** Upstream's `vendor/bevy_asset` serializes loads of one source, so that an older snapshot
  never lands after a newer one during hot reload. Nothing here reloads a source while it loads, so it is left out. Its two
  own tests, the only two that fail without it, are removed:
  - `asset::tests::bevy_reload_serializes_source_reads_without_blocking_other_sources`
  - `asset::tests::bevy_reload_serializes_older_successful_source_reads`

  Hot reload is therefore unordered here, if anything ever uses it.
- **Tests no longer depend on machine load.** `UsdProjectionBudget` defaults to a 10 ms wall-clock budget per frame, and many
  tests expect a scene to finish projecting in one update. Under CPU load, about a third of 300 runs of one such test failed,
  and 14 tests failed across six runs of the suite. Its default is unbounded under `cfg(test)`, and all 644 tests then passed
  six runs under the same load. Outside tests it stays 10 ms.
- **WESL modules imported only by name are loaded.** `import usd_bevy::route::gpu_instancing_types::...` needs that module
  registered, and `embedded_asset!` only stores its bytes, while Bevy's loader follows asset-path imports and not package
  ones. So `gpu_instancing_types`, `sheen_functions`, `strand_functions` and `flat_functions` now go through
  `load_shader_library!`. Without it, the GPU instancer's pipeline waited for ever on an unresolved import.
- **A material's shader bytes come before its plugin.** `flat_material`, `sheen_material`, `strand_material` and
  `medium_material` added `MaterialPlugin` and then `embedded_asset!`, while the plugin's load runs on another thread. It
  lost that race on some runs (`Path not found: usd_bevy/route/flat_material.wesl`), and the bytes now come first.
- **`mesh::bounds::StageBounds`, new.** The bounds of everything visible, including the GPU instancer's draws, which have no
  mesh to measure. The render and the viewer frame a stage with it. `examples/gif_frames.rs`, which measures meshes alone,
  would wait for ever on a stage whose instancers are all on the GPU.

The ports differ from the examples in these ways:

- **The render has no upload cap.** `gif_frames` caps `RenderAssetBytesPerFrame` at 256 MiB, and its own comment says this can
  starve materials. On `hummingbird.usdz` it left every frame blank until the first time change. A batch render has no frame
  rate to protect, so the render leaves Bevy's default, no cap.
- **The render** drops the overlays, the streaming mode and `usdview`'s environment, and refuses a software adapter.
