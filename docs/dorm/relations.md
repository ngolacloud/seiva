# Relations

DORM models relate to one another through **foreign keys**. A foreign key is a
field typed `ForeignKey<Target>`; the derive macro records the target, the delete
action, and the reverse `related_name`, and the query compiler uses that metadata
to resolve joins for you.

> Within Seiva, relations stay inside a module. DORM's schema
> [forbids a relation that crosses a `namespace`](schema-and-models.md#namespaces-the-module-boundary),
> so one module's models cannot reference another's — the same isolation the
> facade layer enforces on the API side.

## Declaring a foreign key

In the schema, a field typed as another model is a to-one relation; the
`<Model>[]` field on the other side names the back-relation and produces no
column:

```
model Author {
  id    Int    @id @default(autoincrement())
  name  String @unique
  posts Post[]
}

model Post {
  id     Int    @id @default(autoincrement())
  author Author @relation(onDelete: Cascade)
  slug   String @db.VarChar(80)
}
```

In the generated model code this is a `ForeignKey<Author>` field carrying the
`foreign_key`, `on_delete`, and `related_name` metadata:

```rust
#[field(foreign_key = "Author", on_delete = "cascade", related_name = "posts")]
author: ForeignKey<Author>,
```

`on_delete` accepts `cascade`, `restrict`, `set_null`, or `do_nothing`
(the schema spellings are `Cascade`, `Restrict`, `SetNull`, `NoAction`).

## Inserting through a relation

Reference the parent through its key. Every model gets a generated `as_fk()`,
shorthand for `ForeignKey::from_id(parent.pk())`:

```rust
let ada = Author::objects().create(NewAuthor { name: "Ada".into() }).await?;
Post::objects()
    .create(NewPost { author: ada.as_fk(), slug: "on-engines".into() })
    .await?;
```

## Filtering across a relation

Traverse the foreign key by calling the related field on the relation handle; the
compiler resolves the join — you never write SQL or name a join table:

```rust
// posts whose author's name contains "ada"
Post::objects()
    .filter(post::author.author().name().eq("Ada Lovelace"))
    .all()
    .await?;
```

The dynamic form spells the same traversal with `__`:
`.filter_str("author__name", "Ada Lovelace")`. Reverse relations resolve by their
`related_name`, e.g. an author filtered by a field on their posts.

## Fetching the parent in one round-trip

`select_related` joins the parent in and attaches it to each row, so there is no
second query per row — the N+1 problem does not arise:

```rust
for post in Post::objects().select_related("author").all().await? {
    if let Some(author) = post.author.get() {
        println!("{} by {}", post.slug, author.name);
    }
}
```

This is the mechanism behind Seiva's "no N+1" guarantee for the to-one side: one
statement, one round-trip. (For the *to-many* side — loading each author's posts
without a query per author — DORM's production roadmap adds `prefetch_related`,
which issues one extra query per relation rather than one per row; the to-one
`select_related` is the shipped, documented path today.)

## Counting the reverse side

`annotate` with a `Count` over the `related_name` gives a per-row count of the
reverse relation:

```rust
use dorm_core::prelude::*;

// each author, with how many posts they have
Author::objects()
    .annotate(Aggregate::new("posts", AggKind::Count, "posts"))
    .all()
    .await?;
```

## Related

- [Schema and models](schema-and-models.md) · [Making queries](queries.md)
- [Migrations](migrations.md) — how the `REFERENCES … ON DELETE …` constraint is
  generated.

---

*Part of the [Seiva documentation](../index.md).*
