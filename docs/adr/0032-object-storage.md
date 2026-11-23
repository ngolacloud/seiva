# ADR 0032 — `seiva-storage`: a pluggable `ObjectStore` port with S3/MinIO and local-fs adapters, streaming and tenant-aware

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0002 (core ports), ADR 0019 (fail-closed authorization), ADR 0026 (per-route load control), ADR 0030 (pluggable data layer)

## Context

Seiva has a data layer for **records** and nothing for **blobs**. A platform
needs to store large, opaque bytes — recordings, exports, uploaded documents,
avatars, generated reports — and the relational store is the wrong place for
them. delonix-meet proves the point: its recorder writes IVF/OGG/MP4 artefacts,
its analytics emits CSV/PDF exports, its profiles carry avatars, and today every
one of those paths opens an S3 client by hand, re-deriving credentials, bucket
layout, tenant prefixing and presigning in the product. That is a horizontal
*mechanism* — an object store is true for any platform — welded into domain code.

A naive helper that reads a `Vec<u8>` and `PUT`s it is a trap: a 2 GB recording
would be buffered whole in process memory before it ever reached the store. The
primitive has to be **streaming** from the first line, and it has to be
**tenant-safe** from the first line, because a blob keyed only by a product-chosen
name is one missing prefix away from a cross-tenant read.

The forces:

- **A port, not a vendor.** The framework must expose an `ObjectStore` *capability*
  and let the composition root bind S3, MinIO or a local directory — the same
  mechanism-vs-policy line as the DORM gateway (ADR 0030). Core crates must not
  link an AWS SDK by default.
- **Streaming, never whole-file buffering.** `put`/`get` must move bytes as an
  `AsyncRead`/`AsyncWrite` so a large object costs a bounded buffer, not its full
  size in RAM. Large uploads need **multipart**; the app process must never be the
  memory bottleneck for a blob.
- **Tenant-aware and fail-closed.** Every key is resolved under a **per-tenant
  prefix** derived from the ambient `Scope`; a call with no scope **fails closed**
  (ADR 0019), it does not fall back to a shared root. The local-fs adapter must
  reject any key that could traverse out of its tenant prefix (`..`, absolute,
  symlink).
- **The transfer should not pass through the app.** For client up/download the
  store must mint **presigned URLs** so bytes flow client↔storage directly; the
  app only authorises and signs, it does not proxy the payload.
- **Zero-cost when unused and DORM-free.** A service that stores nothing links no
  object-store client and pays nothing; the crate never touches the data layer.

## Decision

Add a new workspace member **`seiva-storage`** — data-layer-free, feature-gated,
streaming and tenant-aware.

- **`ObjectStore`** — the port (an `async_trait`), byte-oriented and
  vendor-neutral:
  - `put(key, body: impl AsyncRead, meta: ObjectMeta)` and
    `get(key) -> impl AsyncRead` — **streaming** both ways, never materialising
    the whole object.
  - `delete(key)`, `list(prefix) -> stream of ObjectRef`, `head(key) -> ObjectMeta`.
  - `presign_get`/`presign_put(key, ttl) -> Url` — a short-TTL signed URL that
    delegates the transfer to the store.
  - `create_multipart`/`upload_part`/`complete_multipart` — for large objects,
    streamed part by part with bounded memory.
- **`ObjectMeta`** — content-type, length, and an optional product-set metadata map;
  content-type and a size bound are **validated** before a `put` commits.
- **`Key` resolution is tenant-scoped.** The store wraps every caller key under
  `tenant/{tenant_id}/…` taken from the ambient `Scope`; a `None` scope is a
  fail-closed error, not a shared-root write. Tenants' blobs are isolated by
  construction, mirroring the data layer's `Scope` (ADR 0030).
- **Adapters, bound at the root:**
  - **`S3ObjectStore`** (feature `s3`, `aws-sdk-s3`) — speaks S3; works against AWS
    S3 **and** MinIO/Ceph-RGW via an endpoint override.
  - **`LocalFsObjectStore`** (default, dev) — a directory tree; every key is
    canonicalised and **confined** to its tenant prefix, rejecting `..`, absolute
    paths and symlink escape.
- **Resilience is reused, not reinvented** — outbound S3 calls are wrapped with
  `seiva-resilience` (`RetryPolicy` backoff + `timeout()` + optional
  `CircuitBreaker`); an upload carries a client token so a retried `put` is
  idempotent rather than a duplicate.

`tokio` enters with `default-features = false` (`io-util`, `fs`); the AWS SDK and
its runtime are pulled only by the `s3` feature, so the default build stays light.

## Alternatives considered

- **A single hard-coded S3 client in a `seiva-util` helper.** Rejected: it welds the
  framework to one vendor, cannot run offline in dev or test, and offers no seam for
  MinIO/Ceph or a local directory. A port with adapters is the same choice ADR 0030
  made for the data layer.
- **A `bytes`/`Vec<u8>` in-memory API ("just `put(key, data)`").** Rejected: it
  makes the app process the memory bottleneck for every blob and makes large
  recordings impossible. Streaming `AsyncRead`/`AsyncWrite` + multipart is the only
  shape that bounds memory.
- **Proxy every download through the app.** Rejected: it burns app bandwidth and a
  worker slot per transfer. Presigned URLs let the store serve bytes directly while
  the app keeps authorisation; it is both faster and cheaper.
- **Let each product prefix its own keys.** Rejected: a single forgotten prefix is a
  cross-tenant blob read. Tenant prefixing belongs in the mechanism, fail-closed,
  so no product can opt out of isolation.

## Consequences

- **Performance.** Streaming `put`/`get` bound process memory to a buffer regardless
  of object size; multipart carries large objects part by part; presigned URLs move
  the actual bytes client↔storage so the transfer never consumes an app worker. The
  crate links no S3 runtime unless the `s3` feature is on — zero-cost when unused.
- **Security.** Per-tenant prefix derived from the ambient `Scope`, **fail-closed**
  on a missing scope (ADR 0019); presigned URLs carry a **short TTL** and a tight
  verb/key scope; the local-fs adapter blocks path traversal and symlink escape;
  content-type and size are validated before a write commits. Credentials are read
  from config, never logged.
- **Resilience.** S3 calls get `seiva-resilience` retry-with-backoff, per-call
  timeouts and an optional circuit breaker so a degraded store does not stall the
  service; a client upload token makes a retried `put` idempotent instead of a
  duplicate object.
- delonix-meet deletes its hand-rolled S3 plumbing and binds `S3ObjectStore` (MinIO
  in dev, Ceph-RGW in prod); the recorder and the export pipeline stream artefacts
  through the port while the **policy** — what is stored, bucket names, retention —
  stays in the product.
- It does **not** become a CDN or a media-serving edge: for public hot assets a
  deployment still fronts the store with a cache. This is the in-process object
  capability, not a delivery network.
