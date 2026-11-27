//! `seiva-llm-reqwest` — the reqwest/rustls [`HttpTransport`] for `seiva-llm`
//! (ADR 0048, G1).
//!
//! ```no_run
//! use std::sync::Arc;
//! use seiva_llm::providers::anthropic::Anthropic;
//! use seiva_llm::EnvCredentials;
//! use seiva_llm_reqwest::ReqwestTransport;
//!
//! # fn demo() -> Result<(), seiva_llm::LlmError> {
//! let transport = Arc::new(ReqwestTransport::with_defaults()?);
//! let anthropic = Anthropic::new(transport, Arc::new(EnvCredentials));
//! # let _ = anthropic; Ok(()) }
//! ```
//!
//! The port's contract, kept here:
//!
//! - **No redirects.** A `3xx` comes back as a status: following it would carry
//!   the credential header to another host. [`ReqwestTransport::client_builder`]
//!   starts from `redirect::Policy::none()`; a client injected through
//!   [`ReqwestTransport::new`] must keep that policy.
//! - **Credential headers are marked sensitive**, so reqwest and hyper keep them
//!   out of their own debug output.
//! - **No response at all** (DNS, TLS, refused, reset) is
//!   [`LlmError::Transport`], with the URL stripped from the message; any status
//!   that did come back is an [`HttpResponse`] for the codec to read.
//!
//! The guarded outbound client of ADR 0045 (resolved-IP checks, pinned dial) plugs
//! in here when it lands: it is a `reqwest::Client` configuration (a DNS resolver
//! and connector), passed to [`ReqwestTransport::new`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use seiva_llm::{HttpRequest, HttpResponse, HttpTransport, LlmError, Method};

/// An [`HttpTransport`] over a [`reqwest::Client`]. Cheap to clone (the client
/// is a shared pool).
#[derive(Clone, Debug)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// A transport over a configured client. The client must not follow
    /// redirects — start from [`client_builder`](Self::client_builder).
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// The recommended starting point: no redirects, a 10 s connect timeout, a
    /// `seiva-llm` user agent. Total time is bounded per attempt by the router,
    /// not here, so a long streamed answer is not cut.
    pub fn client_builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("seiva-llm/", env!("CARGO_PKG_VERSION")))
    }

    /// A transport with [`client_builder`](Self::client_builder)'s defaults.
    pub fn with_defaults() -> Result<Self, LlmError> {
        Self::client_builder()
            .build()
            .map(Self::new)
            .map_err(|e| LlmError::Config(format!("cannot build the HTTP client: {e}")))
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, LlmError> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            _ => reqwest::Method::POST,
        };
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            let mut v = reqwest::header::HeaderValue::from_str(value.as_str())
                .map_err(|_| LlmError::Config(format!("header `{name}` has an invalid value")))?;
            v.set_sensitive(value.is_sensitive());
            builder = builder.header(name.as_str(), v);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| LlmError::Transport(e.without_url().to_string()))?;
        let status = response.status().as_u16();
        // The headers a client reads (retry-after, mcp-session-id, content-type).
        // Values that are not visible ASCII are dropped rather than guessed.
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str().to_owned(), v.to_owned())))
            .collect();
        let body = response.bytes_stream().map(|chunk| {
            chunk
                .map(|b| b.to_vec())
                .map_err(|e| LlmError::Transport(e.without_url().to_string()))
        });
        let mut out = HttpResponse::new(status, Box::pin(body));
        for (name, value) in headers {
            out = out.with_header(name, value);
        }
        Ok(out)
    }
}
