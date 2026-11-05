# How to switch a module to DORM

A scaffolded module uses the in-memory repository. To back it with a real
database:

1. **Set up the data layer** (once per project), choosing the backend:

   ```console
   $ seiva db init                 # PostgreSQL; or --backend mysql | sqlite
   ```

2. **Migrate the module's model** from the project schema:

   ```console
   $ export DELONIX_ORM_DSN="host=localhost user=me dbname=app sslmode=disable"
   $ seiva db dev initial
   ```

   This creates the module's table from its `namespace` in
   `schema/<project>.schema` (see [Migrations](../topics/data-layer/migrations.md)).

3. **Wire the DORM-backed adapter** in the composition root: behind the `dorm`
   feature, swap the module's `InMemoryRecordRepository` for the DORM-backed
   repository built from the `seiva-data-dorm` gateway, and inject it into the
   module's controller (the only place that names a concrete adapter). See
   [the data layer](../topics/data-layer/index.md).

The application and facade layers do not change — they code against the port.

## Related

- [The data layer](../topics/data-layer/index.md) · [Multiple databases](../topics/data-layer/multiple-databases.md)

---

*Part of the [Seiva documentation](../index.md).*
