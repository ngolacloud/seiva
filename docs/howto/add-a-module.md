# How to add a module

```console
$ seiva module new billing
```

That is the whole recipe. It:

1. generates `modules/billing/` as the five layer crates (facade, domain,
   application, infrastructure, interface);
2. registers them in the workspace `Cargo.toml`, `seiva.toml` and the project
   schema (a `namespace billing` block with a starter model);
3. **auto-wires** the module into the composition root — adds its
   interface/infrastructure deps to `apps/<name>-api/Cargo.toml` and merges its
   router in `main.rs` (at the managed anchors).

So the module's route appears immediately — confirm with `seiva openapi`.

## Then

- Edit `modules/billing/facade/src/lib.rs` to shape the contract (methods, DTOs,
  `#[requires(...)]`). See [Facades](../topics/transport/facades.md).
- Edit the `namespace billing` block in `schema/<project>.schema` for the model,
  and run `seiva db dev` to migrate it ([Migrations](../topics/data-layer/migrations.md)).

## Related

- [Tutorial part 1](../getting-started/tutorial/part-1-your-first-module.md)

---

*Part of the [Seiva documentation](../index.md).*
