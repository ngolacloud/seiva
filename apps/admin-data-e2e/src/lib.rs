//! The data-layer admin proof: a compact DORM-backed CRUD module (`Catalog`)
//! the admin console drives through its CONTRACT OPERATIONS (ADR 0009 Phase-3).
//!
//! `Catalog` exposes the five CRUD operations over a real PostgreSQL table
//! (`catalog_items`) via the DORM gateway's [`DormUnitOfWork`] — each handler runs
//! inside a transaction and reads/writes with `dorm_core::db` raw SQL. The console
//! never touches the table: it calls `create`/`get`/`list`/`update`/`delete`, so
//! the module keeps its domain invariants (§4/§5). It registers an `AdminResource`
//! (ADR 0009) so the console surfaces it, gated by `catalog.*` permissions.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, facade, register_admin};
use dorm_core::value::Value;
use seiva_data_dorm::DormUnitOfWork;
use seiva_http::{fingerprint, scoped_key, Ctx, Problem, Validate, ValidationError};
use seiva_ports::{BoxError, IdempotencyOutcome, IdempotencyStore, PortResult, UnitOfWork};
use serde::{Deserialize, Serialize};

/// The durable idempotency window: a key is honoured for replay for this long
/// after its first use (ADR 0011). A day is a generous client-retry horizon.
const IDEMPOTENCY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// DDL for the demo table. `IF NOT EXISTS`, so a second run is a no-op. Every row
/// carries a `tenant`; the operations scope by the caller's tenant (`&Ctx`), so a
/// caller never sees or touches another tenant's rows (server-side, ADR 0006).
#[must_use]
pub fn catalog_table_ddl() -> &'static str {
    "CREATE TABLE IF NOT EXISTS catalog_items (\
       id bigserial PRIMARY KEY, \
       tenant TEXT NOT NULL, \
       name TEXT NOT NULL, \
       price BIGINT NOT NULL, \
       cost BIGINT NOT NULL DEFAULT 0, \
       version BIGINT NOT NULL DEFAULT 1)"
}

/// Create body.
#[derive(Debug, Clone, Deserialize, Serialize, delonix_seiva::WireSchema)]
pub struct CreateItem {
    /// The item's name (non-empty).
    pub name: String,
    /// The item's price, in minor units.
    pub price: i64,
    /// The internal cost (a masked field): only returned to a caller holding
    /// `catalog.cost.view`. Optional on create (defaults to 0).
    #[serde(default)]
    pub cost: Option<i64>,
}

impl Validate for CreateItem {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::new("name must not be empty"));
        }
        Ok(())
    }
}

/// Update body: the id, the expected version (optimistic concurrency) plus the
/// new field values.
#[derive(Debug, Clone, Deserialize, Serialize, delonix_seiva::WireSchema)]
pub struct UpdateItem {
    /// The id of the item to change.
    pub id: u64,
    /// The version the caller last saw (from a prior read). The update applies
    /// only if the stored version still matches — otherwise the row changed under
    /// the caller and the update is a `409` (optimistic concurrency, ETag-style).
    pub version: u64,
    /// The new name (non-empty).
    pub name: String,
    /// The new price.
    pub price: i64,
    /// The new internal cost (masked field); defaults to 0 if omitted.
    #[serde(default)]
    pub cost: Option<i64>,
}

impl Validate for UpdateItem {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::new("name must not be empty"));
        }
        Ok(())
    }
}

/// List query (read from the query string on `GET /v1/catalogs`): optional
/// substring search, ordering and pagination. Every field is optional, so an
/// empty query lists the first page in the default order. The server validates
/// `order` against a fixed allow-list (mirroring the registered `ordering`) and
/// clamps `page_size`, so a client can never inject a column name or ask for an
/// unbounded page.
#[derive(Debug, Clone, Default, Deserialize, Serialize, delonix_seiva::WireSchema)]
pub struct ListQuery {
    /// Case-insensitive substring to match against the name (parameterised ILIKE).
    #[serde(default)]
    pub search: Option<String>,
    /// Column to order by: `id` or `name` (anything else falls back to `id`).
    #[serde(default)]
    pub order: Option<String>,
    /// Order descending instead of ascending.
    #[serde(default)]
    pub desc: Option<bool>,
    /// Zero-based page index (default 0).
    #[serde(default)]
    pub page: Option<u32>,
    /// Rows per page (default 25, clamped to 1..=100).
    #[serde(default)]
    pub page_size: Option<u32>,
}

/// Delete body: just the id.
#[derive(Debug, Clone, Deserialize, Serialize, delonix_seiva::WireSchema)]
pub struct DeleteItem {
    /// The id of the item to remove.
    pub id: u64,
}

impl Validate for DeleteItem {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// Response DTO. Also `Deserialize` so a stored idempotent response can be read
/// back for a replay (ADR 0011).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, delonix_seiva::WireSchema)]
pub struct ItemDto {
    /// The item's id.
    pub id: u64,
    /// The item's name.
    pub name: String,
    /// The item's price.
    pub price: i64,
    /// The internal cost — a **masked** field: `None` (omitted) unless the caller
    /// holds `catalog.cost.view`. The mask is applied server-side from the `&Ctx`,
    /// so an unauthorized caller never receives the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<i64>,
    /// The row's current version — the value a client echoes back in `UpdateItem`
    /// so a stale edit is rejected (optimistic concurrency).
    pub version: u64,
}

/// The facade's error type: a message plus the HTTP status it maps to. Mapped to
/// an RFC 9457 problem by the controller. Most boundary failures are `422`
/// (unprocessable); an idempotency conflict is `409` (ADR 0011).
#[derive(Debug, Clone)]
pub struct CatalogError {
    /// The HTTP status this error maps to.
    pub status: StatusCode,
    /// The human-readable detail.
    pub message: String,
}

impl CatalogError {
    /// A `422 Unprocessable Entity` boundary error.
    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
        }
    }

    /// A `409 Conflict` — an idempotency key reused with different content, or a
    /// first request with this key still in flight.
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }
}

impl From<CatalogError> for Problem {
    fn from(error: CatalogError) -> Self {
        Problem::new(error.status, "Catalog error").with_detail(error.message)
    }
}

/// The `catalog` module's public contract: full CRUD over catalogue items.
#[facade(version = "v1")]
pub trait Catalog {
    /// Create an item (`POST /v1/catalogs`), owned by the caller's tenant.
    #[requires("catalog.create")]
    async fn create(&self, ctx: &Ctx, cmd: CreateItem) -> Result<ItemDto, CatalogError>;

    /// Fetch one of the caller's tenant's items by id (`GET /v1/catalogs/{id}`).
    #[requires("catalog.read")]
    async fn get(&self, ctx: &Ctx, id: u64) -> Result<ItemDto, CatalogError>;

    /// List the caller's tenant's items (`GET /v1/catalogs`), with optional
    /// search, ordering and pagination from the query string.
    #[requires("catalog.read")]
    async fn list(&self, ctx: &Ctx, query: ListQuery) -> Result<Vec<ItemDto>, CatalogError>;

    /// Change one of the caller's tenant's items (`POST /v1/catalogs/update`).
    #[requires("catalog.change")]
    async fn update(&self, ctx: &Ctx, cmd: UpdateItem) -> Result<ItemDto, CatalogError>;

    /// Delete one of the caller's tenant's items (`POST /v1/catalogs/delete`).
    #[requires("catalog.delete")]
    async fn delete(&self, ctx: &Ctx, cmd: DeleteItem) -> Result<(), CatalogError>;
}

// Surface `catalog` in the admin console (ADR 0009): couples only to the
// framework contract, not the admin UI. Permissions mirror the facade guards.
register_admin!(AdminResource {
    namespace: "catalog",
    // Must equal the OpenAPI resource segment, which is the facade name lowercased
    // AND pluralised (`Catalog` -> `catalogs`), so the console pairs the resource
    // with its `/v1/catalogs/...` operations.
    resource: "catalogs",
    label: "Catalogue",
    facade: "Catalog",
    version: "v1",
    list_fields: &["id", "name", "price"],
    search_fields: &["name"],
    filters: &[],
    ordering: &["id", "name"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "catalog.read",
        create: "catalog.create",
        change: "catalog.change",
        delete: "catalog.delete",
    },
});

/// A DORM-backed `Catalog` service: each operation runs inside a
/// [`DormUnitOfWork`] transaction and reads/writes with raw SQL. `Clone` (holds an
/// `Arc`) so the generated router can hold it as axum state.
#[derive(Clone)]
pub struct CatalogController {
    uow: Arc<DormUnitOfWork>,
    idem: Arc<dyn IdempotencyStore>,
}

impl CatalogController {
    /// Builds the controller over a DORM unit of work and an idempotency store
    /// (both durable, cross-replica — the store makes a retried write dedupe even
    /// across replicas, ADR 0011).
    #[must_use]
    pub fn new(uow: Arc<DormUnitOfWork>, idem: Arc<dyn IdempotencyStore>) -> Self {
        Self { uow, idem }
    }
}

/// Maps a boundary error to the facade error type (`422`).
fn err(e: impl std::fmt::Display) -> CatalogError {
    CatalogError::unprocessable(e.to_string())
}

/// What a `create` attempt resolved to inside the transaction, so the `409`
/// idempotency conflicts map to the right status *after* the transaction commits
/// (returning them as an `Err` from the closure would roll back and remap to 422).
enum CreateOutcome {
    /// A fresh insert or a replayed prior response — both an idempotent success.
    Done(ItemDto),
    /// A `409`: the key is reused with different content, or its first request is
    /// still in flight.
    Conflict(&'static str),
}

/// What an `update` attempt resolved to, so a stale-version conflict maps to `409`
/// and a missing row to `422` — distinguished *inside* the transaction (an
/// unaffected update is ambiguous between the two without a follow-up read).
enum UpdateOutcome {
    /// Applied; the new row (version bumped).
    Done(ItemDto),
    /// No such row for this tenant — `422`.
    NotFound,
    /// The row exists but its version moved on — `409` (optimistic concurrency).
    VersionConflict,
}

/// A boxed boundary error from a static message (e.g. "item not found"), so a
/// missing row surfaces as an `Err` the handler maps to a `Problem`.
fn boxed(msg: &'static str) -> BoxError {
    msg.into()
}

/// Reads an [`ItemDto`] out of a row of `(id, name, price, cost, version)`, masking
/// the `cost` to `None` unless `show_cost` (the caller holds `catalog.cost.view`).
fn row_to_item(row: &dorm_core::row::Row, show_cost: bool) -> PortResult<ItemDto> {
    Ok(ItemDto {
        id: u64::try_from(row.try_get::<i64>(0).map_err(|e| Box::new(e) as BoxError)?)
            .map_err(|e| Box::new(e) as BoxError)?,
        name: row
            .try_get::<String>(1)
            .map_err(|e| Box::new(e) as BoxError)?,
        price: row.try_get::<i64>(2).map_err(|e| Box::new(e) as BoxError)?,
        cost: if show_cost {
            Some(row.try_get::<i64>(3).map_err(|e| Box::new(e) as BoxError)?)
        } else {
            None
        },
        version: u64::try_from(row.try_get::<i64>(4).map_err(|e| Box::new(e) as BoxError)?)
            .map_err(|e| Box::new(e) as BoxError)?,
    })
}

/// The caller's tenant, or an error — an untenanted/anonymous caller administers
/// no tenant's rows.
fn tenant_of(ctx: &Ctx) -> Result<String, CatalogError> {
    ctx.tenant()
        .map(ToOwned::to_owned)
        .ok_or_else(|| CatalogError::unprocessable("caller has no tenant"))
}

const COST_PERMISSION: &str = "catalog.cost.view";

#[controller]
impl Catalog for CatalogController {
    async fn create(&self, ctx: &Ctx, cmd: CreateItem) -> Result<ItemDto, CatalogError> {
        let tenant = tenant_of(ctx)?;
        let show_cost = ctx.has(COST_PERMISSION);
        // Durable idempotency is opt-in: when the caller sent an `Idempotency-Key`,
        // fold it (scoped to this identity + operation) and a canonical fingerprint
        // of the body into the store. The claim runs INSIDE the transaction, so the
        // reservation commits atomically with the insert — two replicas racing one
        // key resolve to exactly one insert (ADR 0011).
        let idem = ctx.idempotency_key().map(|key| {
            let scoped = scoped_key(ctx.principal(), ctx.method(), ctx.path(), key);
            let body = serde_json::to_value(&cmd).unwrap_or(serde_json::Value::Null);
            let print = fingerprint(ctx.method(), ctx.path(), None, &body);
            (scoped, print)
        });
        let store = Arc::clone(&self.idem);
        let outcome = self
            .uow
            .transaction(move || async move {
                if let Some((scoped, print)) = &idem {
                    match store.begin(scoped, print, IDEMPOTENCY_TTL).await? {
                        IdempotencyOutcome::Fresh => {} // fall through to the insert
                        IdempotencyOutcome::Replay(bytes) => {
                            let dto = serde_json::from_slice::<ItemDto>(&bytes)
                                .map_err(|e| Box::new(e) as BoxError)?;
                            return Ok(CreateOutcome::Done(dto));
                        }
                        IdempotencyOutcome::InProgress => {
                            return Ok(CreateOutcome::Conflict(
                                "a request with this Idempotency-Key is still being processed; retry shortly",
                            ));
                        }
                        IdempotencyOutcome::Mismatch => {
                            return Ok(CreateOutcome::Conflict(
                                "this Idempotency-Key was already used for a request with different content",
                            ));
                        }
                    }
                }
                let rows = dorm_core::db::sql(
                    "INSERT INTO catalog_items (tenant, name, price, cost) \
                     VALUES ($1, $2, $3, $4) RETURNING id, name, price, cost, version",
                    &[
                        Value::Text(tenant),
                        Value::Text(cmd.name),
                        Value::I64(cmd.price),
                        Value::I64(cmd.cost.unwrap_or(0)),
                    ],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                let dto = row_to_item(
                    rows.first()
                        .ok_or_else(|| boxed("insert returned no row"))?,
                    show_cost,
                )?;
                // Persist the response for replay — the exact bytes a retry returns.
                if let Some((scoped, _)) = &idem {
                    let bytes = serde_json::to_vec(&dto).map_err(|e| Box::new(e) as BoxError)?;
                    store.finish(scoped, &bytes).await?;
                }
                Ok(CreateOutcome::Done(dto))
            })
            .await
            .map_err(err)?;
        match outcome {
            CreateOutcome::Done(dto) => Ok(dto),
            CreateOutcome::Conflict(msg) => Err(CatalogError::conflict(msg)),
        }
    }

    async fn get(&self, ctx: &Ctx, id: u64) -> Result<ItemDto, CatalogError> {
        let id = i64::try_from(id).map_err(err)?;
        let tenant = tenant_of(ctx)?;
        let show_cost = ctx.has(COST_PERMISSION);
        self.uow
            .transaction(move || async move {
                let rows = dorm_core::db::sql(
                    "SELECT id, name, price, cost, version FROM catalog_items \
                     WHERE id = $1 AND tenant = $2",
                    &[Value::I64(id), Value::Text(tenant)],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                row_to_item(
                    rows.first().ok_or_else(|| boxed("item not found"))?,
                    show_cost,
                )
            })
            .await
            .map_err(err)
    }

    async fn list(&self, ctx: &Ctx, query: ListQuery) -> Result<Vec<ItemDto>, CatalogError> {
        let tenant = tenant_of(ctx)?;
        let show_cost = ctx.has(COST_PERMISSION);
        // Validate the order column against a FIXED allow-list (mirrors the
        // registered `ordering`) — a client-supplied column name is never
        // interpolated unchecked. Direction, page size and offset are integers or
        // literals, so they are safe to format; the search term is parameterised.
        let order_col = match query.order.as_deref() {
            Some("name") => "name",
            _ => "id",
        };
        let dir = if query.desc.unwrap_or(false) {
            "DESC"
        } else {
            "ASC"
        };
        let page_size = i64::from(query.page_size.unwrap_or(25).clamp(1, 100));
        let offset = i64::from(query.page.unwrap_or(0)).saturating_mul(page_size);
        let search = query.search.filter(|s| !s.trim().is_empty());
        self.uow
            .transaction(move || async move {
                let mut sql = String::from(
                    "SELECT id, name, price, cost, version FROM catalog_items WHERE tenant = $1",
                );
                let mut params = vec![Value::Text(tenant)];
                if let Some(term) = search {
                    params.push(Value::Text(format!("%{term}%")));
                    sql.push_str(" AND name ILIKE $2");
                }
                sql.push_str(&format!(
                    " ORDER BY {order_col} {dir} LIMIT {page_size} OFFSET {offset}"
                ));
                let rows = dorm_core::db::sql(&sql, &params)
                    .await
                    .map_err(|e| Box::new(e) as BoxError)?;
                rows.iter()
                    .map(|row| row_to_item(row, show_cost))
                    .collect::<PortResult<Vec<_>>>()
            })
            .await
            .map_err(err)
    }

    async fn update(&self, ctx: &Ctx, cmd: UpdateItem) -> Result<ItemDto, CatalogError> {
        let id = i64::try_from(cmd.id).map_err(err)?;
        let version = i64::try_from(cmd.version).map_err(err)?;
        let tenant = tenant_of(ctx)?;
        let show_cost = ctx.has(COST_PERMISSION);
        let outcome = self
            .uow
            .transaction(move || async move {
                // Apply only if the version still matches, and bump it. A stale
                // edit updates no row.
                let rows = dorm_core::db::sql(
                    "UPDATE catalog_items SET name = $3, price = $4, cost = $5, \
                     version = version + 1 \
                     WHERE id = $1 AND tenant = $2 AND version = $6 \
                     RETURNING id, name, price, cost, version",
                    &[
                        Value::I64(id),
                        Value::Text(tenant.clone()),
                        Value::Text(cmd.name),
                        Value::I64(cmd.price),
                        Value::I64(cmd.cost.unwrap_or(0)),
                        Value::I64(version),
                    ],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                if let Some(row) = rows.first() {
                    return Ok(UpdateOutcome::Done(row_to_item(row, show_cost)?));
                }
                // Nothing updated: the row is absent (NotFound) or its version
                // moved on under the caller (VersionConflict). A scoped read tells
                // them apart — still inside the same transaction.
                let exists = dorm_core::db::sql(
                    "SELECT 1 FROM catalog_items WHERE id = $1 AND tenant = $2",
                    &[Value::I64(id), Value::Text(tenant)],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                Ok(if exists.is_empty() {
                    UpdateOutcome::NotFound
                } else {
                    UpdateOutcome::VersionConflict
                })
            })
            .await
            .map_err(err)?;
        match outcome {
            UpdateOutcome::Done(dto) => Ok(dto),
            UpdateOutcome::NotFound => Err(CatalogError::unprocessable("item not found")),
            UpdateOutcome::VersionConflict => Err(CatalogError::conflict(
                "the item was modified since you loaded it (version conflict); reload and retry",
            )),
        }
    }

    async fn delete(&self, ctx: &Ctx, cmd: DeleteItem) -> Result<(), CatalogError> {
        let id = i64::try_from(cmd.id).map_err(err)?;
        let tenant = tenant_of(ctx)?;
        self.uow
            .transaction(move || async move {
                dorm_core::db::run(
                    "DELETE FROM catalog_items WHERE id = $1 AND tenant = $2",
                    &[Value::I64(id), Value::Text(tenant)],
                )
                .await
                .map(|_| ())
                .map_err(|e| Box::new(e) as BoxError)
            })
            .await
            .map_err(err)
    }
}
