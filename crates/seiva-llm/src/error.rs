//! The one error type of the model port, and how it is classified for retry and
//! fallback.

/// How long a provider's error message may be when it is kept. A provider body is
/// untrusted text: it is cut, never echoed whole into logs or to the caller.
pub(crate) const MAX_PROVIDER_MESSAGE: usize = 512;

/// Something went wrong calling a model.
///
/// No variant ever carries a credential, a prompt or an answer: messages are the
/// provider's own error text (cut to a few hundred characters) or the codec's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LlmError {
    /// No HTTP response at all: connection refused, TLS, DNS, a reset stream.
    #[error("transport error: {0}")]
    Transport(String),
    /// The attempt outlived its time budget.
    #[error("the model call timed out")]
    Timeout,
    /// The provider answered with an error status.
    #[error("provider returned HTTP {status}: {message}")]
    Status {
        /// The HTTP status.
        status: u16,
        /// The provider's error message, cut.
        message: String,
    },
    /// The provider answered something the codec cannot read.
    #[error("cannot decode the provider response: {0}")]
    Decode(String),
    /// The configuration is wrong: an unknown provider, a residency violation, a
    /// missing credential, an endpoint that is not `https`.
    #[error("configuration error: {0}")]
    Config(String),
    /// No route with this name.
    #[error("unknown route `{0}`")]
    UnknownRoute(String),
    /// Every upstream of the route failed; `last` is the final error.
    #[error("every upstream of route `{route}` failed; last: {last}")]
    AllUpstreamsFailed {
        /// The route.
        route: String,
        /// The error of the last upstream tried.
        last: Box<LlmError>,
    },
}

impl LlmError {
    /// A provider error with its message cut to a safe length.
    pub fn status(status: u16, message: impl AsRef<str>) -> Self {
        Self::Status {
            status,
            message: cut(message.as_ref(), MAX_PROVIDER_MESSAGE),
        }
    }

    /// Whether trying the **same** upstream again may succeed: a transport
    /// failure, a timeout, a rate limit, an overloaded or failing provider.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Timeout => true,
            Self::Status { status, .. } => {
                matches!(status, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
            }
            _ => false,
        }
    }

    /// Whether trying the **next** upstream of the route may succeed: everything
    /// retryable, plus the errors that belong to one provider (its key rejected,
    /// the model unknown there). A malformed request (`400`, `413`, `422`) fails
    /// the same everywhere, so it stops the route.
    pub fn allows_fallback(&self) -> bool {
        self.is_retryable()
            || matches!(
                self,
                Self::Status {
                    status: 401 | 403 | 404,
                    ..
                }
            )
            || matches!(self, Self::Decode(_))
    }
}

/// Cut `s` to at most `max` bytes on a character boundary, marking the cut.
pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_and_overload_are_retried_a_bad_request_is_not() {
        assert!(LlmError::status(429, "slow down").is_retryable());
        assert!(LlmError::status(529, "overloaded").is_retryable());
        assert!(LlmError::Timeout.is_retryable());
        assert!(!LlmError::status(400, "bad").is_retryable());
        assert!(!LlmError::status(400, "bad").allows_fallback());
    }

    #[test]
    fn a_rejected_key_moves_to_the_next_upstream_without_retrying() {
        let e = LlmError::status(401, "invalid x-api-key");
        assert!(!e.is_retryable());
        assert!(e.allows_fallback());
    }

    #[test]
    fn a_long_provider_message_is_cut_on_a_char_boundary() {
        let long = "é".repeat(600);
        let LlmError::Status { message, .. } = LlmError::status(500, &long) else {
            unreachable!()
        };
        assert!(message.len() <= MAX_PROVIDER_MESSAGE + '…'.len_utf8());
        assert!(message.ends_with('…'));
    }
}
