//! The real [`HttpSender`] adapter for the ADR 0033 delivery engine: a thin
//! reqwest/TLS binding of the one seam the DORM-free `seiva-webhook-delivery`
//! leaves open.
//!
//! The engine hands the adapter an [`HttpRequest`] with the destination URL and
//! the three Standard Webhooks headers already computed (`webhook-id`,
//! `webhook-timestamp`, `webhook-signature`); the adapter only has to put them on
//! the wire. A transport failure (no response at all — a connection refused, a TLS
//! error, a DNS failure) becomes a [`SendError`] so the engine treats it as
//! transient; any HTTP status that *did* come back is returned as an
//! [`HttpResponse`] for the engine to classify.

use async_trait::async_trait;
use seiva_webhooks::delivery::{HttpRequest, HttpResponse, HttpSender, SendError};
use seiva_webhooks::{ID_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER};

/// A [`HttpSender`] backed by a [`reqwest::Client`].
///
/// The client is injected rather than built here, so a deployment owns the TLS
/// roots, connection pool, proxy and timeouts (and a test can point a hostname at
/// its local server). The adapter is otherwise stateless.
#[derive(Clone)]
pub struct ReqwestSender {
    client: reqwest::Client,
}

impl ReqwestSender {
    /// Build an adapter over a configured [`reqwest::Client`].
    #[must_use]
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpSender for ReqwestSender {
    async fn post(&self, request: HttpRequest<'_>) -> Result<HttpResponse, SendError> {
        let response = self
            .client
            .post(request.url)
            .header(ID_HEADER, request.id)
            .header(TIMESTAMP_HEADER, request.timestamp.to_string())
            .header(SIGNATURE_HEADER, request.signature)
            .header(reqwest::header::CONTENT_TYPE, request.content_type)
            .body(request.body.to_vec())
            .send()
            .await
            // No HTTP response at all: a transport-level failure the engine retries
            // and counts against the endpoint's breaker. The message carries no
            // payload or secret.
            .map_err(|error| SendError::new(error.to_string()))?;

        let status = response.status().as_u16();
        // A body the receiver returned is only for the ledger excerpt; the engine
        // truncates it. A body-read error is not itself a delivery failure — the
        // status already decided the outcome.
        let body_excerpt = response.text().await.unwrap_or_default();
        Ok(HttpResponse::new(status, body_excerpt))
    }
}
