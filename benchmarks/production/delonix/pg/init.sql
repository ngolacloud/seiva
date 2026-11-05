-- Create the Catalog table once, at database init, so the two replicas never
-- race on the bigserial sequence creation (two concurrent CREATE TABLE IF NOT
-- EXISTS hit a 23505 on pg_class). Mirrors admin_data_e2e::catalog_table_ddl();
-- keep in sync (the app still runs that DDL, which becomes a no-op here).
CREATE TABLE IF NOT EXISTS catalog_items (
    id bigserial PRIMARY KEY,
    tenant TEXT NOT NULL,
    name TEXT NOT NULL,
    price BIGINT NOT NULL,
    cost BIGINT NOT NULL DEFAULT 0
);
