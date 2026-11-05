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

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::{controller, facade, register_admin};
use dorm_core::value::Value;
use seiva_data_dorm::DormUnitOfWork;
use seiva_http::{Problem, Validate, ValidationError};
use seiva_ports::{BoxError, PortResult, UnitOfWork};
use serde::{Deserialize, Serialize};

/// DDL for the demo table. `IF NOT EXISTS`, so a second run is a no-op.
#[must_use]
pub fn catalog_table_ddl() -> &'static str {
    "CREATE TABLE IF NOT EXISTS catalog_items (\
       id bigserial PRIMARY KEY, \
       name TEXT NOT NULL, \
       price BIGINT NOT NULL)"
}

/// Create body.
#[derive(Debug, Clone, Deserialize, Serialize, delonix_seiva::WireSchema)]
pub struct CreateItem {
    /// The item's name (non-empty).
    pub name: String,
    /// The item's price, in minor units.
    pub price: i64,
}

impl Validate for CreateItem {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::new("name must not be empty"));
        }
        Ok(())
    }
}

/// Update body: the id plus the new field values.
#[derive(Debug, Clone, Deserialize, Serialize, delonix_seiva::WireSchema)]
pub struct UpdateItem {
    /// The id of the item to change.
    pub id: u64,
    /// The new name (non-empty).
    pub name: String,
    /// The new price.
    pub price: i64,
}

impl Validate for UpdateItem {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::new("name must not be empty"));
        }
        Ok(())
    }
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

/// Response DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, delonix_seiva::WireSchema)]
pub struct ItemDto {
    /// The item's id.
    pub id: u64,
    /// The item's name.
    pub name: String,
    /// The item's price.
    pub price: i64,
}

/// The facade's error type. Mapped to an RFC 9457 problem by the controller.
#[derive(Debug, Clone)]
pub struct CatalogError(pub String);

impl From<CatalogError> for Problem {
    fn from(error: CatalogError) -> Self {
        Problem::unprocessable_entity(error.0)
    }
}

/// The `catalog` module's public contract: full CRUD over catalogue items.
#[facade(version = "v1")]
pub trait Catalog {
    /// Create an item (`POST /v1/catalog`).
    #[requires("catalog.create")]
    async fn create(&self, cmd: CreateItem) -> Result<ItemDto, CatalogError>;

    /// Fetch one item by id (`GET /v1/catalog/{id}`).
    #[requires("catalog.read")]
    async fn get(&self, id: u64) -> Result<ItemDto, CatalogError>;

    /// List items (`GET /v1/catalog`).
    #[requires("catalog.read")]
    async fn list(&self) -> Result<Vec<ItemDto>, CatalogError>;

    /// Change an item (`POST /v1/catalog/update`).
    #[requires("catalog.change")]
    async fn update(&self, cmd: UpdateItem) -> Result<ItemDto, CatalogError>;

    /// Delete an item (`POST /v1/catalog/delete`).
    #[requires("catalog.delete")]
    async fn delete(&self, cmd: DeleteItem) -> Result<(), CatalogError>;
}

// Surface `catalog` in the admin console (ADR 0009): couples only to the
// framework contract, not the admin UI. Permissions mirror the facade guards.
register_admin!(AdminResource {
    namespace: "catalog",
    resource: "catalog",
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
}

impl CatalogController {
    /// Builds the controller over a DORM unit of work.
    #[must_use]
    pub fn new(uow: Arc<DormUnitOfWork>) -> Self {
        Self { uow }
    }
}

/// Maps a boundary error to the facade error type.
fn err(e: impl std::fmt::Display) -> CatalogError {
    CatalogError(e.to_string())
}

/// A boxed boundary error from a static message (e.g. "item not found"), so a
/// missing row surfaces as an `Err` the handler maps to a `Problem`.
fn boxed(msg: &'static str) -> BoxError {
    msg.into()
}

/// Reads an [`ItemDto`] out of a row of `(id, name, price)`.
fn row_to_item(row: &dorm_core::row::Row) -> PortResult<ItemDto> {
    Ok(ItemDto {
        id: u64::try_from(row.try_get::<i64>(0).map_err(|e| Box::new(e) as BoxError)?)
            .map_err(|e| Box::new(e) as BoxError)?,
        name: row
            .try_get::<String>(1)
            .map_err(|e| Box::new(e) as BoxError)?,
        price: row.try_get::<i64>(2).map_err(|e| Box::new(e) as BoxError)?,
    })
}

#[controller]
impl Catalog for CatalogController {
    async fn create(&self, cmd: CreateItem) -> Result<ItemDto, CatalogError> {
        self.uow
            .transaction(|| async move {
                let rows = dorm_core::db::sql(
                    "INSERT INTO catalog_items (name, price) VALUES ($1, $2) \
                     RETURNING id, name, price",
                    &[Value::Text(cmd.name), Value::I64(cmd.price)],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                row_to_item(
                    rows.first()
                        .ok_or_else(|| boxed("insert returned no row"))?,
                )
            })
            .await
            .map_err(err)
    }

    async fn get(&self, id: u64) -> Result<ItemDto, CatalogError> {
        let id = i64::try_from(id).map_err(err)?;
        self.uow
            .transaction(move || async move {
                let rows = dorm_core::db::sql(
                    "SELECT id, name, price FROM catalog_items WHERE id = $1",
                    &[Value::I64(id)],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                row_to_item(rows.first().ok_or_else(|| boxed("item not found"))?)
            })
            .await
            .map_err(err)
    }

    async fn list(&self) -> Result<Vec<ItemDto>, CatalogError> {
        self.uow
            .transaction(|| async {
                let rows = dorm_core::db::sql(
                    "SELECT id, name, price FROM catalog_items ORDER BY id LIMIT 100",
                    &[],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                rows.iter().map(row_to_item).collect::<PortResult<Vec<_>>>()
            })
            .await
            .map_err(err)
    }

    async fn update(&self, cmd: UpdateItem) -> Result<ItemDto, CatalogError> {
        let id = i64::try_from(cmd.id).map_err(err)?;
        self.uow
            .transaction(move || async move {
                let rows = dorm_core::db::sql(
                    "UPDATE catalog_items SET name = $2, price = $3 WHERE id = $1 \
                     RETURNING id, name, price",
                    &[Value::I64(id), Value::Text(cmd.name), Value::I64(cmd.price)],
                )
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
                row_to_item(rows.first().ok_or_else(|| boxed("item not found"))?)
            })
            .await
            .map_err(err)
    }

    async fn delete(&self, cmd: DeleteItem) -> Result<(), CatalogError> {
        let id = i64::try_from(cmd.id).map_err(err)?;
        self.uow
            .transaction(move || async move {
                dorm_core::db::run("DELETE FROM catalog_items WHERE id = $1", &[Value::I64(id)])
                    .await
                    .map(|_| ())
                    .map_err(|e| Box::new(e) as BoxError)
            })
            .await
            .map_err(err)
    }
}
