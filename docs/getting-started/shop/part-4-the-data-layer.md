# Part 4: The data layer

So far every module keeps its data in memory. In this part we give the shop real
tables with the DORM data layer — the same `seiva db` loop from the first
tutorial, now across several modules. For the full data-layer story see the
[DORM section](../../dorm/index.md).

## Set up and migrate

```console
$ seiva db init                 # PostgreSQL by default; or --backend mysql | sqlite
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=shop sslmode=disable"
$ seiva db dev initial
```

`db init` writes `data/dorm.schema` from the project's `schema/shop.schema`, where
each module contributes its own `namespace`. By the end of this part the schema
holds a table per module — products, stock, orders (with their line items),
payments, notifications — and the migration ledger.

```schema
namespace catalog {
  model Product {
    id          Uuid @id @default(uuid())
    name        String
    price_cents Int
  }
}

namespace inventory {
  model StockItem {
    product_id Uuid @id
    available  Int  @default(0)
  }
}

namespace orders {
  model Order {
    id       Uuid   @id @default(uuid())
    customer String
    status   String @default("placed")
  }
  // order lines reference a product by id — a value owned by `catalog`,
  // referenced across the module boundary by id, not by a foreign-key join.
}
```

Each module's `namespace` is its own slice of the database. A module reads and
writes only its own tables; it refers to another module's data **by id**
(`product_id`), never by reaching into the other module's tables. That is the same
boundary the code enforces — carried down to the data layer.

## One transaction, one outbox

Because each use case runs in one `UnitOfWork::transaction` and publishes its
event inside it (Part 3), the DORM gateway lands that event in a transactional
**outbox** in the same transaction as the write. The relay worker forwards it
afterwards. So `orders.placed` can never be emitted for an order that did not
commit — the event and the data are atomic.

## Swap the adapter

The modules still use their in-memory repositories in the composition root;
swapping each for its DORM-backed adapter (behind the `dorm` feature) is the
production wiring. The use cases do not change — they already code against their
repository **port**. See [switch to DORM](../../howto/switch-to-dorm.md).

## Next

[Part 5: Authorization across modules](part-5-authorization.md).

---

*Part of the [Seiva documentation](../../index.md).*
