# Quickstart

This walks from an empty checkout to a running Seiva service. Everything here
builds **without** the private DORM repository — the default data layer is
in-memory until you enable the `dorm` feature.

Prerequisites: a recent stable Rust toolchain (`rustup`), and this workspace
checked out.

## 1. Build and meet the CLI

Install the `seiva` binary from the workspace:

```console
$ cargo install --path crates/seiva-cli
$ seiva doctor
Delonix Seiva
  framework version : 0.1.0
  data layer        : DORM (feature `dorm` on)
```

The CLI ships with the `dorm` feature on by default; build it with
`--no-default-features` and `doctor` reports `external (feature `dorm` off)`.
(Or run without installing: `cargo run -q -p seiva-cli --bin seiva -- doctor`.)

`seiva --help` lists every command; the ones exercised below are `project`,
`module`, `openapi`, `proto` and `user`.

## 2. See the generated contracts

The CLI prints the gRPC and OpenAPI documents generated from the built-in
demonstration facade — the same generators a real module uses:

```console
$ seiva proto
syntax = "proto3";
package seiva.notes.v1;
service Notes {
  rpc Create(CreateRequest) returns (CreateResponse);
}
...

$ seiva openapi | head
{
  "openapi": "3.1.0",
  ...
}
```

## 3. Create a user through the accounts module

`seiva user create` runs the real `accounts` module use case: it reads the
password from **stdin** (never the command line), hashes it with Argon2id,
enforces the domain rules and emits the `accounts.user_created` event. The output
carries the id and username — never the password or its hash.

```console
$ printf 'correct horse battery staple\n' | seiva user create --username alice
seiva: event emitted — accounts.user_created
{
  "id": "1793783057320429770",
  "username": "alice"
}
```

Persistence is in-memory for the command's lifetime; it becomes durable once a
DORM repository is wired behind the `dorm` feature.

## 4. Scaffold a project and a module

```console
$ seiva project new shop
$ cd shop
$ seiva module new catalog
```

Names are lowercase snake_case (`shop`, `order_items`). `project new` lays out
`modules/`, `apps/shop-api`, `schema/`, `shared/kernel` and `seiva.toml`.
`seiva module new` then emits the five layers wired to the framework:
`#[seiva::facade]` on the contract, `#[seiva::controller]` on a concrete
interface adapter (the `axum` router), and interface helpers that generate the
module's OpenAPI and `.proto` from its descriptor. The default repository is
in-memory, so the generated project compiles with no database.

## 5. Run the server

The `seiva-server` runtime boots the HTTP app (controllers + `/openapi.json` +
`/health`) and the worker host, reading its configuration 12-factor:

```console
$ SEIVA_HTTP_ADDR=127.0.0.1:8080 cargo run -q -p seiva-server --bin seiva-server
```

Configuration precedence for the bind address: `SEIVA_HTTP_ADDR` (full
`host:port`) › `PORT` (bare port, bound on `0.0.0.0`) › `127.0.0.1:8080`.
`APP_ENV=production` turns on the production-safety guard (dev shortcuts such as
`SEIVA_EXPOSE_ERROR_DETAILS` are then rejected at startup).

In another shell, the unguarded endpoints respond immediately:

```console
$ curl -s localhost:8080/health/ready        # 200 once Ready, 503 while Starting
$ curl -s localhost:8080/openapi.json | head  # the generated OpenAPI 3.1 document
```

A facade route such as `POST /v1/notes` is guarded by its declared permission
(`notes.create`): a caller without it gets an RFC 9457 `403`. Send `SIGTERM`
(or Ctrl-C) and the server flips health to `Draining` and drains in-flight
requests before exiting.

## 6. Keep the architecture honest

Run the fitness functions locally — the same checks CI enforces:

```console
$ seiva lint
```

They verify the layer dependency rule and the module zero-trust contract, so a
violation fails fast instead of shipping.

## Next

- [Architecture guide](architecture.md) — the layering, the contract, and every
  crate's role.
- Enabling DORM end to end requires the private-repo CI token; see the
  architecture guide's "DORM: default, optional, and isolated".
