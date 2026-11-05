# Pagination

For list endpoints, prefer **keyset** (cursor) pagination over offset pagination.

## Why keyset

Offset pagination (`LIMIT n OFFSET m`) is unstable under concurrent writes (rows
shift between pages) and degrades on deep pages (the database still scans the
skipped rows). Keyset pagination carries a **cursor** over a stable ordering and
reads the next page with a `WHERE (key) > (cursor)` — stable and O(page), not
O(offset).

## In Seiva

The `list` route convention (a `GET /{version}/{resource}` whose single query DTO
carries the cursor and limit — see
[controllers and routing](transport/controllers-and-routing.md)) is where
pagination lives on the wire. The repository turns it into a bounded, ordered read
through DORM, which supports keyset cursors (the cursor is signed with the
configured key, so it is not forgeable — the gateway requires that key at
`connect`, no insecure default).

## Shape

Return the page plus an opaque `next` cursor; the client passes it back for the
following page. Keep the ordering key stable (a monotonic id or a
`(created_at, id)` tuple) so the cursor is well-defined.

## Related

- [Making queries](data-layer/queries.md) · [Controllers and routing](transport/controllers-and-routing.md)

---

*Part of the [Seiva documentation](../index.md).*
