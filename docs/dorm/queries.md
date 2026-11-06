# Making queries

Everything starts at `Model::objects()`. It returns a `Manager`, and from there
you build a **`QuerySet`** — lazy and immutable. Every method consumes `self` and
returns a new one, so nothing reaches the database until you `await` a terminal
method, and a base query is safely reusable.

```rust
let base   = Post::objects().filter(post::title.icontains("rust"));
let recent = base.clone().order_by([post::published_at.desc()]).limit(5);
let count  = base.count().await?;   // `base` is untouched by `recent`
```

Two branches built from the same base cannot disturb each other, because neither
mutated it.

> In a Seiva module this builder lives inside the repository adapter, behind the
> application's repository port — the use case asks the port for what it needs
> and never names `QuerySet`. See
> [Making queries (framework view)](../topics/data-layer/queries.md).

## Reading rows

| Terminal | Returns | Notes |
|---|---|---|
| `.all()` | `Vec<T>` | Every matching row. |
| `.first()` | `Option<T>` | The first, respecting the ordering. |
| `.last()` | `Option<T>` | The last; reverses the ordering and takes one. |
| `.get(q)` | `T` | Exactly one. Errors `DoesNotExist` for none, `MultipleObjectsReturned` for more than one. |
| `.count()` | `i64` | `COUNT(*)` of the match. |
| `.exists()` | `bool` | Whether anything matches, without counting. |

```rust
let one   = Post::objects().get(post::id.eq(42)).await?;
let maybe = Post::objects().filter(post::slug.eq("on-engines")).first().await?;
let n     = Post::objects().filter(post::title.icontains("rust")).count().await?;
```

## Filtering: `filter`, `exclude`, and `Q`

`filter` narrows; `exclude` is its complement. Repeated calls are `AND`-ed:

```rust
Post::objects()
    .filter(post::title.icontains("rust"))
    .exclude(post::slug.startswith("draft-"))
```

Each `post::field.<lookup>(value)` builds a `Q`. Combine them with `&` (and),
`|` (or), `!` (not):

```rust
use dorm_core::prelude::*;

let q = (post::title.icontains("rust") | post::title.icontains("ferris"))
      & !post::slug.startswith("draft-");
Post::objects().filter(q).all().await?;
```

### Field lookups

Each typed handle carries only the lookups that make sense for its column type.

| Lookup | Method | SQL |
|---|---|---|
| exact | `field.eq(v)` | `col = $1` |
| comparisons | `field.gt(v)`, `.gte(v)`, `.lt(v)`, `.lte(v)` | `col > $1`, … |
| in a set | `field.is_in([a, b, c])` | `col IN ($1, $2, $3)` |
| in a range | `field.range(lo, hi)` | `col BETWEEN $1 AND $2` |
| is null | `field.isnull(true)` | `col IS NULL` |
| case-insensitive exact | `field.iexact(v)` | `col ILIKE $1` *(text)* |
| contains | `field.contains(v)`, `.icontains(v)` | `col LIKE '%'‖$1‖'%'` *(text)* |
| starts with | `field.startswith(v)` | `col LIKE $1‖'%'` *(text)* |

### The dynamic (string-keyed) form

The same query can be built from strings — the form a REPL or an admin console
sends. It produces the identical plan, and only columns the derive macro
registered resolve; an invented name is an error, not a query:

```rust
Post::objects().filter_str("title__icontains", "rust").all().await?;
```

## Ordering, limit, offset, distinct

```rust
Post::objects()
    .order_by([post::published_at.desc(), post::slug.asc()])
    .offset(20)
    .limit(10)
    .distinct()
    .all()
    .await?;
```

- `order_by` **replaces** rather than appends — the last call wins.
- The string form is `.order_by_str(&["-published_at", "slug"])`.
- With no `order_by`, the model's `#[model(ordering = …)]` applies.

### Keyset pagination for lists

Offset pagination is available, but for user-facing lists prefer **keyset**
(cursor) pagination: a cursor over a stable ordering is stable under concurrent
writes and does not degrade on deep pages. DORM provides keyset cursors with a
**signed** cursor token (the signing key is configured by the host, with no
forgeable default), so a client cannot tamper with the position. In a Seiva
service the `list` route convention carries the cursor parameters and the
repository turns them into a bounded, ordered read; see the framework-level
[pagination note](../topics/data-layer/queries.md#pagination). The signing-key
configuration is part of what the [gateway](using-from-seiva.md) sets up.

## Creating rows

Insert with the generated `New…` struct, which omits database-filled columns.
`create` returns the row as stored, with those columns filled in:

```rust
let ada = Author::objects()
    .create(NewAuthor { name: "Ada Lovelace".into() })
    .await?;
println!("new id = {}", ada.id);   // filled by the database
```

Insert many in a single statement, or fetch-or-insert idempotently under a race
(a lost insert retries as a read):

```rust
Author::objects()
    .bulk_create(vec![NewAuthor { name: "Ada".into() }, NewAuthor { name: "Grace".into() }])
    .await?;

let (author, created) = Author::objects()
    .get_or_create(author::name.eq("Ada"), NewAuthor { name: "Ada".into() })
    .await?;
```

## Updating rows

`update` applies a set of assignments to every matching row and returns how many
it touched. Build assignments with `field.set(expr)` or `Assignment::new`:

```rust
let n = Post::objects()
    .filter(post::slug.eq("on-engines"))
    .update(vec![post::title.set("On Analytical Engines")])
    .await?;
```

An `F` expression computes from the column itself — **atomic in the database**,
so two concurrent callers both take effect where a read-modify-write would lose
one:

```rust
use dorm_core::prelude::*;

// views = views + 1, inside the UPDATE
Post::objects()
    .filter(post::id.eq(42))
    .update(vec![Assignment::new("views", F::col("views") + 1)])
    .await?;
```

## Deleting rows

```rust
let removed = Post::objects()
    .filter(post::slug.startswith("draft-"))
    .delete()
    .await?;   // u64: how many were deleted
```

## Aggregates and projections

`aggregate` computes over the whole query and returns one row of named values;
`annotate` adds a per-group aggregate (counting the reverse side of a relation is
the classic case — see [relations](relations.md)):

```rust
use dorm_core::prelude::*;

let row   = Post::objects()
    .aggregate(vec![Aggregate::new("total", AggKind::Count, "id")]).await?;
let total = row.get("total");   // a Value
```

`AggKind` is `Count`, `Sum`, `Avg`, `Min`, `Max`; `count_of(path)` is shorthand
for a single `COUNT`.

To select named columns instead of whole structs, project with `values` and read
with `rows` (name-to-`Value` maps). `values()` and `all()` are mutually exclusive
— calling `all()` on a projected query is refused rather than decoding the wrong
columns:

```rust
let rows = Post::objects().values(&["id", "title"]).rows().await?;
for r in rows {
    println!("{} {}", r["id"], r["title"]);
}
```

## Seeing the SQL — without a database

Any `QuerySet` can show the SQL and parameters it would run, with values as bound
placeholders. This needs no database and is the foundation of DORM's snapshot
tests:

```rust
let sql = Post::objects().filter(post::title.icontains("rust")).sql()?;
println!("{}", sql.text);          // SELECT … WHERE "posts"."title" ILIKE $1
println!("{}", sql.params.len());  // 1
```

## Related

- [Relations](relations.md) · [Transactions](transactions.md) ·
  [Raw SQL](raw-sql.md)
- [Schema and models](schema-and-models.md) · [Backends](backends.md)

---

*Part of the [Seiva documentation](../index.md).*
