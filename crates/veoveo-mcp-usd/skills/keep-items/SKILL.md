---
name: keep-items
description: Keep a list of named items for the person you work for, create one and read it back. Use when a task asks to record or look up an item. This is the template's reference skill, the shape every server's skills follow.
---

# Keep items

1. Call `usd__create_item` with a `name`. Its product's `resultUri` is the item's
   canonical address, `usd://item/{item_id}`.
2. Read the item at its `resultUri`.
3. `usd://items` lists the person's items in their tenant.

A skill like this one says how to use the server's tools for a task, names each tool as
callers see it through the Hub, `{slug}__{tool}`, and each resource by the address callers
read. The server's `instructions` say only what it is and name each skill's address.
