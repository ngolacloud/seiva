# ADR 0047 — Mutual TLS in `seiva-http`: a verified client certificate is one more credential of the same identity model

- Status: Accepted
- Date: 2026-11-26
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0012 (cloud-native health probes and limits), ADR 0026 (per-route load control)

## Context

Some Seiva services are called mostly by **machines**: endpoint agents, site
gateways, other services. A bearer secret is a poor credential for them. It sits
on disk, a TLS-intercepting proxy can read it, and it is valid until someone
remembers to rotate it. The usual answer is **mutual TLS**: each machine holds a
private key it generated itself (in a TPM, a keychain or a `0600` file) and a
short-lived certificate from the deployment's own CA, and it proves possession
during the handshake.

Until now Seiva served plain HTTP only (`seiva_server::serve_router` over a
`TcpListener`); TLS ended at a reverse proxy. A project that wanted mTLS had two
bad options:

- **Terminate mTLS at the proxy and forward the certificate in a header**
  (`X-Client-Cert`). The application then trusts a header. One proxy
  misconfiguration, or one direct connection that bypasses the proxy, and any
  caller can claim any certificate.
- **Run a second listener of its own** beside the Seiva router, with its own
  authentication. That is a second identity model outside ADR 0006: the
  permission guard, `Scoped` and the audit trail would no longer see one
  `Principal`.

The forces:

- **The certificate check must happen where the key is proven**, in the
  handshake, against the deployment's CA only. The system roots are never used,
  so neither a public CA nor an interception proxy's CA can mint an accepted
  client.
- **One identity model.** A verified certificate must end up as the same
  `Principal` a JWT or an API key produces, so `#[requires]`, `Scoped` and audit
  work unchanged.
- **Revocation must be prompt.** A CRL checked at handshake time is necessary but
  coarse. The application also keeps its own record of which devices are
  enrolled, and checking it on each request means a revocation applies to the
  next request, not only to the next connection.
- **People and machines may share one port.** A UI user with an OIDC bearer and an
  agent with a certificate may hit the same listener.
- **The handshake is an attack surface.** A client that opens TCP and never
  finishes the handshake must not stall the accept loop or hold memory forever.
- **Opt-in and DORM-free.** A service behind a TLS-terminating proxy must link
  none of this.

## Decision

Add mutual TLS to `seiva-http` behind an **optional `mtls` feature**, with a
thin runtime counterpart in `seiva-server` (feature `mtls`).

**In `seiva-http` (`seiva_http::mtls`):**

- **`MtlsConfig`**, built from PEM bytes: the server chain and key, the
  **client CA** (the only trust anchors), optional **CRLs** (with an option to
  check the leaf only, for a PKI whose offline root publishes no CRL), and
  `ClientAuth::Required` (default) or `ClientAuth::Optional`. It is
  **TLS 1.3 only**, with the rustls `ring` provider and ALPN `h2` + `http/1.1`.
  Both protocols are served: the `mtls` feature turns on axum's `http2`, so a
  client that negotiates `h2` (reqwest does) gets HTTP/2, not a closed
  connection (fixed in 0.2.2). `Debug` never prints key material.
- **`MtlsListener`**, an `axum::serve::Listener`. TCP accepts run on a background
  task. Each handshake runs on its own task under a **timeout** (10 s by default),
  and at most **N handshakes** (1024 by default) are in flight. A slot is taken
  *before* `accept`, so back-pressure reaches the kernel's accept queue. Only
  completed handshakes reach `axum::serve`.
- **`mtls_make_service(router)`**, the per-connection service. It inserts
  `ConnectInfo<SocketAddr>` into every request, so the rate limiter keeps keying
  on the client IP, and inserts the connection's **`PeerCertificate`**:
  `Verified(ClientCertificate)` or `Unreadable`. Any inbound value of that type
  is overwritten or removed; a request can never forge one.
- **`ClientCertificate`**: the verified leaf, parsed. It exposes the subject CN,
  the DNS and URI SANs, the serial, `not_after`, the SHA-256 fingerprint and the
  DER. It is only ever built from a chain rustls has already verified.
- **`CertificateAuthenticator`**: the application's map from a
  `ClientCertificate` to a `Principal`. Any
  `Fn(&ClientCertificate) -> Result<Principal, AuthError>` is one.

**In the `authn` middleware (ADR 0006):** `AuthConfig::with_client_certificates`
adds a third credential source, consulted **first**:

- a `Verified` certificate is passed to the authenticator, which returns the
  `Principal` or `401`. It runs on the blocking pool, like the JWT check, because
  it may consult a store;
- an `Unreadable` certificate is refused with `401`; the middleware does not
  guess;
- a verified certificate **and** an `Authorization` or `X-API-Key` header in the
  same request is ambiguous, so it is refused with `401`: **one request, one
  identity**;
- no certificate (only possible with `ClientAuth::Optional`) falls through to
  the existing `X-API-Key` → bearer → anonymous paths, unchanged.

**In `seiva-server`:** `serve_router_mtls(addr, router, health, &mtls, shutdown)`
is `serve_router` over an `MtlsListener`, with the same route-collision check
and the same Starting → Ready → Draining lifecycle. `MtlsFiles` reads the
12-factor configuration (`SEIVA_TLS_CERT_FILE`, `SEIVA_TLS_KEY_FILE`,
`SEIVA_TLS_CLIENT_CA_FILE`, `SEIVA_TLS_CLIENT_AUTH`, `SEIVA_TLS_CRL_FILE`).
Certificate, key and CA come together or not at all; a partial set fails the
boot. A file that cannot be read is the startup error `ServeError::Tls`.

## Alternatives considered

- **Certificate in a header from the proxy.** Rejected: the identity would rest
  on the network path, not on cryptography. It stays possible for a deployment
  that wants it, as a custom `Authenticator`, but it is not the framework's mTLS.
- **A separate `seiva-mtls` crate with its own listener and middleware.**
  Rejected: a second authentication path outside `authn` is exactly the drift
  ADR 0006 exists to prevent. The listener lives next to the transport it
  serves, behind a feature.
- **Map the certificate to a `Principal` once per connection.** Rejected: a
  long-lived HTTP/2 connection would keep a revoked device authenticated until
  it reconnects. Per-request mapping costs one lookup and makes revocation
  immediate. The parse runs once per connection.
- **`axum-server` with its rustls acceptor.** Rejected: it brings a second serve
  loop next to `axum::serve` and still needs custom code to carry the peer
  certificate into the request. `axum::serve::Listener` plus a make-service is
  smaller and keeps Seiva's existing graceful shutdown.
- **Accept TLS 1.2.** Rejected for this listener: the callers are agents and
  services the deployment controls, so there is no legacy client to keep.
  TLS 1.3 removes the renegotiation and cipher-suite surface.

## Consequences

- A project enables `seiva-server/mtls`, sets `SEIVA_TLS_*`, adds
  `AuthConfig::with_client_certificates(...)` and calls `serve_router_mtls`. Its
  facades, `#[requires]`, `Scoped` and audit are unchanged.
- The default build is unchanged and links no TLS stack. With the feature, the
  extra dependencies are `rustls` (ring), `tokio-rustls`, `x509-parser` and
  `tower-service`. All are MIT/Apache-licensed and pass `cargo deny`.
- The CI `check` job runs the `mtls` tests explicitly
  (`cargo test -p seiva-http -p seiva-server --features …/mtls`), because
  `cargo test --workspace` compiles them out.
- The PKI itself is **not** part of Seiva: issuing, rotating and revoking
  certificates, and the CSR enrolment flow, belong to the application. Seiva
  verifies and maps.
- Hot reload of the certificate and CRL is not supported yet: changing them
  takes a restart. A reloadable `MtlsConfig` can come later without changing the
  `CertificateAuthenticator` contract.

## Related

- ADR 0006: identity and authentication (the `Principal`, the `authn` order).
- ADR 0012: health probes (Ready/Draining around the listener).
- ADR 0026: per-route load control (the handshake cap is the connection-level
  counterpart).
