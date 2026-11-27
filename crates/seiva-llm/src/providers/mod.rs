//! The provider codecs. Each is pure: it builds an [`HttpRequest`](crate::HttpRequest), sends it
//! through the injected [`HttpTransport`](crate::HttpTransport), and reads the JSON answer or the event
//! stream. None of them builds an HTTP client.

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "openai-compatible")]
pub mod openai;
#[cfg(any(feature = "anthropic", feature = "openai-compatible"))]
pub(crate) mod sse;
#[cfg(any(feature = "anthropic", feature = "openai-compatible"))]
pub(crate) use common::*;

#[cfg(any(feature = "anthropic", feature = "openai-compatible"))]
mod common {
    use serde_json::Value;

    use crate::transport::{HttpRequest, HttpResponse, HttpTransport};
    use crate::LlmError;

    /// How much of an error body is read (64 KiB): enough for any provider's error
    /// JSON, never a provider-sized allocation.
    const MAX_ERROR_BODY: usize = 64 * 1024;

    /// Send `request`; a non-2xx status becomes [`LlmError::Status`] with the
    /// provider's message, and a `3xx` is never followed. Whatever the provider or
    /// the transport says, the values of the sensitive headers are wiped from the
    /// error before it leaves: a provider that echoes the request cannot leak the
    /// key into a log.
    pub(crate) async fn send(
        transport: &dyn HttpTransport,
        request: HttpRequest,
    ) -> Result<HttpResponse, LlmError> {
        let secrets = sensitive_values(&request);
        let response = match transport.send(request).await {
            Ok(r) => r,
            Err(e) => return Err(scrub(e, &secrets)),
        };
        if (200..300).contains(&response.status) {
            return Ok(response);
        }
        let status = response.status;
        let body = response.collect(MAX_ERROR_BODY).await.unwrap_or_default();
        Err(scrub(error_from(status, &body), &secrets))
    }

    /// The values of the sensitive headers, whole and token by token (so
    /// `Bearer <key>` also wipes a bare `<key>`), longest first.
    fn sensitive_values(request: &HttpRequest) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, v) in request.headers.iter().filter(|(_, v)| v.is_sensitive()) {
            out.push(v.as_str().to_owned());
            out.extend(
                v.as_str()
                    .split_whitespace()
                    .filter(|t| t.len() >= 8)
                    .map(str::to_owned),
            );
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.len()));
        out.dedup();
        out
    }

    fn scrub(error: LlmError, secrets: &[String]) -> LlmError {
        let wipe = |text: String| {
            secrets
                .iter()
                .filter(|s| !s.is_empty())
                .fold(text, |t, s| t.replace(s.as_str(), "***"))
        };
        match error {
            LlmError::Status { status, message } => LlmError::Status {
                status,
                message: wipe(message),
            },
            LlmError::Transport(m) => LlmError::Transport(wipe(m)),
            LlmError::Decode(m) => LlmError::Decode(wipe(m)),
            other => other,
        }
    }

    /// The provider's error message: `error.message` in the JSON both APIs use, or
    /// the raw text.
    pub(crate) fn error_from(status: u16, body: &[u8]) -> LlmError {
        let message = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_owned());
        LlmError::status(status, message)
    }

    /// Read a JSON body.
    pub(crate) fn json(body: &[u8]) -> Result<Value, LlmError> {
        serde_json::from_slice(body).map_err(|e| LlmError::Decode(e.to_string()))
    }

    pub(crate) fn u64_at(v: &Value, pointer: &str) -> u64 {
        v.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
    }
}

#[cfg(all(test, any(feature = "anthropic", feature = "openai-compatible")))]
mod tests {
    use super::*;
    use crate::LlmError;

    #[test]
    fn the_error_message_comes_from_the_json_or_the_text() {
        let e = error_from(401, br#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        assert_eq!(e, LlmError::status(401, "invalid x-api-key"));
        assert_eq!(
            error_from(502, b" Bad Gateway \n"),
            LlmError::status(502, "Bad Gateway")
        );
    }
}
