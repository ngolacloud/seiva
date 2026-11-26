//! The deny-by-default authentication middleware (ADR 0006).
//!
//! It runs ahead of every route's [`permission_guard`](crate::permission_guard),
//! turning an `Authorization: Bearer <token>` into the [`Principal`] the guard
//! and [`Scoped`](crate::Scoped) read:
//!
//! - **No credential header** → [`Principal::anonymous`] is inserted, so
//!   public routes (`/health`, `/docs`, `/openapi.json`) still serve and a
//!   guarded route's `permission_guard` answers `403`.
//! - **A present-but-invalid credential** (bad signature, wrong `aud`/`iss`,
//!   expired, unknown/revoked key, disallowed algorithm, or an empty/garbled
//!   header) → `401` RFC 9457 [`Problem`] with a `WWW-Authenticate` header. The
//!   request never reaches the route.
//!
//! Two credential sources, each with its own authenticator, run side by side
//! (ADR 0006): an opaque `X-API-Key` (checked first) and `Authorization: Bearer`.
//! With the `mtls` feature a third comes first (ADR 0047): the **client
//! certificate** the TLS handshake verified, mapped by a
//! `CertificateAuthenticator` (`seiva_http::mtls`). A request
//! carrying a verified certificate *and* a header credential is refused (`401`):
//! one request, one identity.
//! The [`Authenticator`] is the only authority on identity; the middleware never
//! trusts a header or body field to set the subject, tenant or permissions.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use seiva_identity::{Authenticator, Principal};

#[cfg(feature = "mtls")]
use crate::mtls::{CertificateAuthenticator, PeerCertificate};
use crate::problem::{Problem, ProblemResponse};
use crate::request_id::RequestIdExt;

/// The `X-API-Key` request header carrying an opaque API key.
static X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

/// The authenticators the [`authn`] middleware consults: an optional
/// `Authorization: Bearer` path (JWT or static token) and an optional opaque
/// `X-API-Key` path running alongside it (ADR 0006). At least one is set — the
/// middleware is never mounted when both are absent (see [`AuthConfig`]).
///
/// Build it with [`Authn::bearer_only`] or [`Authn::with_api_key`], then mount
/// with [`axum::middleware::from_fn_with_state`]`(authn_state, authn)`. A
/// composition root rarely builds it directly: the `router_with_*` builders take
/// an [`AuthConfig`] and derive the right `Authn` from it.
#[derive(Clone)]
pub struct Authn {
    bearer: Option<Arc<dyn Authenticator>>,
    api_key: Option<Arc<dyn Authenticator>>,
    #[cfg(feature = "mtls")]
    client_cert: Option<Arc<dyn CertificateAuthenticator>>,
}

impl Authn {
    /// Authenticate only `Authorization: Bearer` (no `X-API-Key` path). An
    /// `X-API-Key` header, if any, is ignored and the request falls through to the
    /// bearer/anonymous path.
    #[must_use]
    pub fn bearer_only(bearer: Arc<dyn Authenticator>) -> Self {
        Self {
            bearer: Some(bearer),
            api_key: None,
            #[cfg(feature = "mtls")]
            client_cert: None,
        }
    }

    /// Authenticate `X-API-Key` against `api_key` **first**, then
    /// `Authorization: Bearer` against `bearer`. The two are independent
    /// mechanisms; a request presenting neither is the anonymous principal.
    #[must_use]
    pub fn with_api_key(bearer: Arc<dyn Authenticator>, api_key: Arc<dyn Authenticator>) -> Self {
        Self {
            bearer: Some(bearer),
            api_key: Some(api_key),
            #[cfg(feature = "mtls")]
            client_cert: None,
        }
    }
}

/// App-level authentication the `router_with_*` builders mount: an optional
/// `Authorization: Bearer` authenticator and an optional `X-API-Key` one, each
/// admitted only when set (ADR 0006). This is the config an `apps/<name>-api`
/// composition root hands the framework.
///
/// A bare `Option<Arc<dyn Authenticator>>` converts into an `AuthConfig` carrying
/// that bearer and **no** `X-API-Key` path, so every existing call-site keeps
/// working unchanged; reach the `X-API-Key` path with [`AuthConfig::with_api_key`].
/// When neither authenticator is set ([`AuthConfig::none`]) no authn layer is
/// mounted at all — no caller is ever authenticated, so guarded routes answer
/// `403` while public routes still serve.
#[derive(Clone, Default)]
pub struct AuthConfig {
    bearer: Option<Arc<dyn Authenticator>>,
    api_key: Option<Arc<dyn Authenticator>>,
    #[cfg(feature = "mtls")]
    client_cert: Option<Arc<dyn CertificateAuthenticator>>,
}

impl AuthConfig {
    /// No authentication: no caller is ever authenticated (guarded routes `403`,
    /// public routes still serve). The explicit, inference-friendly form of a bare
    /// `None` authenticator.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Authenticate `Authorization: Bearer` against `bearer`; no `X-API-Key` path.
    #[must_use]
    pub fn bearer(bearer: Arc<dyn Authenticator>) -> Self {
        Self {
            bearer: Some(bearer),
            ..Self::default()
        }
    }

    /// Also admit `X-API-Key` callers, authenticated against `api_key` (consulted
    /// **before** the bearer path). Independent of whether a bearer is set, so an
    /// `AuthConfig::none().with_api_key(..)` admits only `X-API-Key` callers.
    #[must_use]
    pub fn with_api_key(mut self, api_key: Arc<dyn Authenticator>) -> Self {
        self.api_key = Some(api_key);
        self
    }

    /// Also admit callers by the **client certificate** the mutual-TLS handshake
    /// verified (feature `mtls`, ADR 0047), mapped to a [`Principal`] by
    /// `certificates`. Consulted before `X-API-Key` and `Authorization`; it only
    /// sees requests served by a [`MtlsListener`](crate::mtls::MtlsListener).
    #[cfg(feature = "mtls")]
    #[must_use]
    pub fn with_client_certificates(
        mut self,
        certificates: Arc<dyn CertificateAuthenticator>,
    ) -> Self {
        self.client_cert = Some(certificates);
        self
    }

    /// The [`Authn`] middleware state for this config, or `None` when no
    /// authenticator is set — in which case no authn layer is mounted
    /// (deny-by-default via the per-route guard still applies).
    pub(crate) fn into_authn(self) -> Option<Authn> {
        #[cfg(feature = "mtls")]
        if self.bearer.is_none() && self.api_key.is_none() && self.client_cert.is_none() {
            return None;
        }
        #[cfg(not(feature = "mtls"))]
        if self.bearer.is_none() && self.api_key.is_none() {
            return None;
        }
        Some(Authn {
            bearer: self.bearer,
            api_key: self.api_key,
            #[cfg(feature = "mtls")]
            client_cert: self.client_cert,
        })
    }
}

impl From<Option<Arc<dyn Authenticator>>> for AuthConfig {
    /// A bearer-only (or no-op, when `None`) config — the historical
    /// `Option<Arc<dyn Authenticator>>` parameter, carried through unchanged.
    fn from(bearer: Option<Arc<dyn Authenticator>>) -> Self {
        Self {
            bearer,
            ..Self::default()
        }
    }
}

/// Middleware that authenticates the caller and inserts a [`Principal`].
///
/// Reads `X-API-Key` first (when an API-key authenticator is configured), then
/// `Authorization: Bearer`; a request with neither becomes
/// [`Principal::anonymous`]. Mount it with
/// [`axum::middleware::from_fn_with_state`]`(authn_state, authn)`, where
/// `authn_state` is an [`Authn`]. [`router_with_auth`](crate::router_with_auth)
/// does this for the whole app.
pub async fn authn(State(authn): State<Authn>, mut request: Request, next: Next) -> Response {
    // 0. The verified client certificate (feature `mtls`), when a certificate
    //    authenticator is configured and the connection presented one.
    #[cfg(feature = "mtls")]
    if let Some(certificates) = authn.client_cert.as_ref() {
        if let Some(peer) = request.extensions().get::<PeerCertificate>().cloned() {
            return certificate_authn(Arc::clone(certificates), peer, request, next).await;
        }
    }

    // 1. `X-API-Key` takes precedence, but only when an API-key authenticator is
    //    configured — otherwise the header is unknown and we fall through.
    if let Some(api_key_auth) = authn.api_key.as_ref() {
        if let Some(key) = request
            .headers()
            .get(&X_API_KEY)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .map(str::to_owned)
        {
            return match api_key_auth.authenticate(&key) {
                Ok(principal) => {
                    request.extensions_mut().insert(Arc::new(principal));
                    next.run(request).await
                }
                Err(_) => unauthorized(request, "ApiKey"),
            };
        }
    }

    // 2. `Authorization: Bearer`.
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let Some(header) = presented else {
        // No credential: the anonymous principal. The per-route guard, not this
        // layer, is what denies a guarded route — so public routes keep serving.
        request
            .extensions_mut()
            .insert(Arc::new(Principal::anonymous()));
        return next.run(request).await;
    };

    // A credential WAS presented: it must authenticate, or the request is refused.
    // With no bearer authenticator configured (an `X-API-Key`-only deployment),
    // there is nothing to validate it against — deny, rather than fall through.
    let Some(bearer_auth) = authn.bearer.as_ref() else {
        return unauthorized(request, "Bearer");
    };

    let bearer = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .unwrap_or("")
        .trim()
        .to_owned();

    // The bearer path is where a JWT verifier lives, and its RSA/ECDSA signature
    // check is CPU-bound and synchronous. Run it on the blocking pool so a burst of
    // verifications never stalls the async executor; a cheap static-token lookup
    // pays only one thread hop. The authenticator is `Send + Sync` behind an `Arc`.
    let bearer_auth = Arc::clone(bearer_auth);
    match tokio::task::spawn_blocking(move || bearer_auth.authenticate(&bearer)).await {
        Ok(Ok(principal)) => {
            request.extensions_mut().insert(Arc::new(principal));
            next.run(request).await
        }
        Ok(Err(_)) => unauthorized(request, "Bearer"),
        // The verification task panicked — an internal fault, never an auth pass.
        Err(_) => auth_unavailable(request),
    }
}

/// The client-certificate path of [`authn`] (ADR 0047).
#[cfg(feature = "mtls")]
async fn certificate_authn(
    certificates: Arc<dyn CertificateAuthenticator>,
    peer: PeerCertificate,
    mut request: Request,
    next: Next,
) -> Response {
    let PeerCertificate::Verified(certificate) = peer else {
        return unauthorized(request, CERTIFICATE_SCHEME);
    };
    // One request, one identity: a header credential next to a verified
    // certificate is ambiguous (which caller is this?), so refuse it rather
    // than silently pick one.
    if request.headers().contains_key(AUTHORIZATION) || request.headers().contains_key(&X_API_KEY) {
        return unauthorized(request, CERTIFICATE_SCHEME);
    }
    // The application's mapping may consult a store (enrolment, revocation):
    // keep it off the async executor, like the bearer path.
    match tokio::task::spawn_blocking(move || certificates.authenticate(&certificate)).await {
        Ok(Ok(principal)) => {
            request.extensions_mut().insert(Arc::new(principal));
            next.run(request).await
        }
        Ok(Err(_)) => unauthorized(request, CERTIFICATE_SCHEME),
        Err(_) => auth_unavailable(request),
    }
}

/// The `WWW-Authenticate` hint on a refused client certificate. HTTP has no
/// registered scheme for TLS client authentication; this names the mechanism
/// the caller must fix (its certificate), not a header to send.
#[cfg(feature = "mtls")]
const CERTIFICATE_SCHEME: &str = "Certificate";

/// Build the `500` RFC 9457 response for an authentication task that failed to
/// run to completion (e.g. a panic inside the verifier). Deny by default: a
/// verifier that cannot produce a verdict is an internal fault, never a pass.
fn auth_unavailable(request: Request) -> Response {
    let request_id = request.extensions().request_id();
    ProblemResponse(
        Problem::internal("the authentication service failed")
            .with_instance(request.uri().path().to_owned())
            .with_request_id_opt(request_id),
    )
    .into_response()
}

/// Build the `401` RFC 9457 response for a present-but-invalid credential, tagged
/// with the `WWW-Authenticate` scheme the caller should use (`Bearer` or `ApiKey`).
fn unauthorized(request: Request, scheme: &'static str) -> Response {
    let request_id = request.extensions().request_id();
    let mut response = ProblemResponse(
        Problem::unauthorized("the presented credential did not authenticate")
            .with_instance(request.uri().path().to_owned())
            .with_request_id_opt(request_id),
    )
    .into_response();
    response
        .headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static(scheme));
    response
}
