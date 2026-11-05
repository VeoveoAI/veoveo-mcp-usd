---
name: view-stages
description: Find USD scenes in Lore repositories, report what a stage holds, render it to a PNG in Artifact, and open it in the viewer. Use when a task names a USD stage or scene, or asks to see, check or picture one.
---

# View stages

A scene is a folder at the top level of a Lore repository, whose root stage is `{folder}/{folder}.usd`, `.usda`, `.usdc` or
`.usdz`. Every read is yours: a repository you hold no grant on is refused.

1. Call `usd__list_scenes` with nothing to list the repositories you can read, then with a `repository`, and a `branch` if it
   is not `main`, to list its scenes. Each scene names its `path`.
2. Call `usd__inspect_stage` with `repository`, `branch` and `path` to see its layers, time range, cameras, variant sets and
   payloads, and any dependency the repository does not hold.
3. Call `usd__render_stage` with the same three, as a Task, to draw one frame on the GPU:
   - `time`, a time code in the stage's range; by default its start.
   - `camera`, either `{"prim": "/path"}`, one of the stage's cameras, or `{"eye": [x, y, z], "focus": [x, y, z]}`; by
     default the whole stage is framed.
   - `variants`, a list of `{"prim", "set", "selection"}`.

   Its product's `resultUri` is the PNG in Artifact, whose metadata names the repository, revision and path. Renders can be
   found later with `artifact__search_knowledge`.
4. To let a person look, give them `/view/lore/{repository}/{branch}/{path}` on the Hub's origin, which opens the viewer with
   its panels for scenes, prims, variants, payloads, time and cameras. `ui://usd/scenes` browses the same scenes.
