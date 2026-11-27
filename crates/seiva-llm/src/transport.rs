//! The HTTP port the codecs talk through, and the endpoint rule.
//!
//! The codecs never build an HTTP client: they hand an [`HttpRequest`] to an
//! injected [`HttpTransport`]. That keeps this crate http-client-free (the real
//! reqwest/TLS transport is `seiva-llm-reqwest`) and gives the deployment one
//! place to put the guarded outbound client of ADR 0045 — redirect policy,
//! resolved-IP checks, proxy, TLS roots — for every model call.

use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;

use crate::LlmError;

/// The largest non-streamed response body the codecs read (8 MiB). A larger one
/// is an error, not an allocation the provider gets to size.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// A response body, as it arrives.
pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, LlmError>> + Send>>;

/// An HTTP method the codecs use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
}

/// A header value, marked sensitive when it carries a credential: a sensitive
/// value prints as `***`, so a logged request never shows the key.
#[derive(Clone, PartialEq, Eq)]
pub struct HeaderValue {
    value: String,
    sensitive: bool,
}

impl HeaderValue {
    /// A plain header value.
    pub fn plain(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            sensitive: false,
        }
    }

    /// A header value that carries a credential.
    pub fn sensitive(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            sensitive: true,
        }
    }

    /// The value to put on the wire.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Whether it carries a credential (a transport should mark it so too, e.g.
    /// `http::HeaderValue::set_sensitive`).
    pub fn is_sensitive(&self) -> bool {
        self.sensitive
    }
}

impl fmt::Debug for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.sensitive {
            f.write_str("***")
        } else {
            fmt::Debug::fmt(&self.value, f)
        }
    }
}

/// One request a codec wants sent.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HttpRequest {
    /// The method.
    pub method: Method,
    /// The absolute URL, always under a validated [`Endpoint`].
    pub url: String,
    /// The headers, credential ones marked sensitive.
    pub headers: Vec<(String, HeaderValue)>,
    /// The JSON body (empty for a `GET`).
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// A JSON `POST`.
    pub fn post_json(url: String, body: Vec<u8>) -> Self {
        Self {
            method: Method::Post,
            url,
            headers: vec![(
                "content-type".into(),
                HeaderValue::plain("application/json"),
            )],
            body,
        }
    }

    /// Add a header.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: HeaderValue) -> Self {
        self.headers.push((name.into(), value));
        self
    }
}

/// The response the transport got back: any status, its headers, and its body
/// as a stream.
#[non_exhaustive]
pub struct HttpResponse {
    /// The HTTP status.
    pub status: u16,
    /// The response headers (names in lower case), for the few a client reads:
    /// `retry-after`, `mcp-session-id`, `content-type`.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: BodyStream,
}

impl HttpResponse {
    /// A response from a status and a body stream, with no headers.
    pub fn new(status: u16, body: BodyStream) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
        }
    }

    /// Add a response header (the name is stored in lower case).
    #[must_use]
    pub fn with_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.headers
            .push((name.as_ref().to_ascii_lowercase(), value.into()));
        self
    }

    /// The first header called `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// A response with a body known up front (test transports).
    pub fn from_bytes(status: u16, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Self::new(
            status,
            Box::pin(futures_util::stream::once(async move { Ok(body) })),
        )
    }

    /// Read the whole body, refusing more than `limit` bytes.
    pub async fn collect(mut self, limit: usize) -> Result<Vec<u8>, LlmError> {
        let mut out = Vec::new();
        while let Some(chunk) = self.body.next().await {
            let chunk = chunk?;
            if out.len() + chunk.len() > limit {
                return Err(LlmError::Decode(format!(
                    "response body larger than {limit} bytes"
                )));
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// The HTTP port. A transport returns any status it got as an [`HttpResponse`]
/// and keeps [`LlmError::Transport`] for "no response at all". It must not follow
/// redirects (a `3xx` is returned as is): a redirect would carry the credential
/// header to another host.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// Send one request.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, LlmError>;
}

/// The base URL of a provider. `https` only, unless the composition root opts a
/// plaintext endpoint in **explicitly** — a model on the same host or the
/// organisation's own network (Ollama, vLLM). No user-info, query or fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint(String);

impl Endpoint {
    /// An `https` endpoint.
    pub fn https(url: &str) -> Result<Self, LlmError> {
        Self::parse(url, false)
    }

    /// An endpoint that may be plain `http`. Only for a model on a network the
    /// organisation controls; the egress guard (ADR 0045) still decides whether
    /// the host may be reached.
    pub fn allow_plaintext(url: &str) -> Result<Self, LlmError> {
        Self::parse(url, true)
    }

    fn parse(url: &str, plaintext_ok: bool) -> Result<Self, LlmError> {
        let bad = |why: &str| LlmError::Config(format!("endpoint `{url}`: {why}"));
        let rest = if let Some(rest) = url.strip_prefix("https://") {
            rest
        } else if let Some(rest) = url.strip_prefix("http://") {
            if !plaintext_ok {
                return Err(bad(
                    "must be https (use Endpoint::allow_plaintext for a local model)",
                ));
            }
            rest
        } else {
            return Err(bad("must start with https://"));
        };
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.is_empty() {
            return Err(bad("has no host"));
        }
        if authority.contains('@') {
            return Err(bad("must not carry credentials in the URL"));
        }
        if url.contains('?') || url.contains('#') {
            return Err(bad("must not have a query or a fragment"));
        }
        if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad("contains whitespace or control characters"));
        }
        Ok(Self(url.trim_end_matches('/').to_owned()))
    }

    /// The base URL, without a trailing slash.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The URL of `path` under this endpoint.
    pub fn join(&self, path: &str) -> String {
        format!("{}/{}", self.0, path.trim_start_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_is_https_unless_plaintext_is_opted_in() {
        assert_eq!(
            Endpoint::https("https://api.example.com/v1/")
                .unwrap()
                .join("/chat"),
            "https://api.example.com/v1/chat"
        );
        assert!(Endpoint::https("http://localhost:11434/v1").is_err());
        assert!(Endpoint::allow_plaintext("http://localhost:11434/v1").is_ok());
        assert!(Endpoint::https("ftp://x").is_err());
    }

    #[test]
    fn an_endpoint_refuses_credentials_queries_and_odd_characters() {
        assert!(Endpoint::https("https://user:pw@api.example.com").is_err());
        assert!(Endpoint::https("https://api.example.com/v1?key=x").is_err());
        assert!(Endpoint::https("https:///v1").is_err());
        assert!(Endpoint::https("https://api.example.com/v 1").is_err());
    }

    #[test]
    fn a_sensitive_header_prints_as_stars() {
        let req = HttpRequest::post_json("https://x/y".into(), b"{}".to_vec())
            .header("x-api-key", HeaderValue::sensitive("sk-live"));
        let printed = format!("{req:?}");
        assert!(!printed.contains("sk-live"), "{printed}");
        assert!(printed.contains("***"));
    }

    #[tokio::test]
    async fn collect_refuses_a_body_over_the_limit() {
        let r = HttpResponse::from_bytes(200, vec![0u8; 10]);
        assert!(matches!(r.collect(5).await, Err(LlmError::Decode(_))));
        let r = HttpResponse::from_bytes(200, vec![1u8; 3]);
        assert_eq!(r.collect(5).await.unwrap(), vec![1, 1, 1]);
    }
}
