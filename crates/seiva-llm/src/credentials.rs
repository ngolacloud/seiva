//! Provider credentials: where they come from, and a type that will not print
//! them.
//!
//! A provider key is the organisation's, never the caller's (ADR 0048 §2,
//! Identify). It is read by the codec at the moment it builds a request, put on
//! one header marked sensitive, and never stored in an error, a log line or a
//! response.

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;

use crate::LlmError;

/// A secret value. `Debug` and `Display` print `***`; the value is read only
/// through [`Secret::expose`], so a stray `{:?}` cannot leak it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that must send it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Where provider credentials come from. Async, so a Vault-backed source (ADR
/// 0048 §1) fits the same port as the environment.
#[async_trait]
pub trait CredentialSource: Send + Sync {
    /// The credential called `name` (`ANTHROPIC_API_KEY`, say). A missing one is
    /// a [`LlmError::Config`] naming it — never a request sent without it.
    async fn get(&self, name: &str) -> Result<Secret, LlmError>;
}

/// Credentials from the process environment (12-factor, the default).
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvCredentials;

#[async_trait]
impl CredentialSource for EnvCredentials {
    async fn get(&self, name: &str) -> Result<Secret, LlmError> {
        match std::env::var(name) {
            Ok(v) if !v.is_empty() => Ok(Secret::new(v)),
            _ => Err(LlmError::Config(format!("credential `{name}` is not set"))),
        }
    }
}

/// Credentials from a fixed map — composition roots that read them elsewhere, and
/// tests.
#[derive(Clone, Default)]
pub struct StaticCredentials(HashMap<String, Secret>);

impl StaticCredentials {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a credential.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(name.into(), Secret::new(value));
        self
    }
}

impl fmt::Debug for StaticCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

#[async_trait]
impl CredentialSource for StaticCredentials {
    async fn get(&self, name: &str) -> Result<Secret, LlmError> {
        self.0
            .get(name)
            .cloned()
            .ok_or_else(|| LlmError::Config(format!("credential `{name}` is not set")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints_its_value() {
        let s = Secret::new("sk-ant-very-secret");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(format!("{s}"), "***");
        let creds = StaticCredentials::new().with("KEY", "sk-ant-very-secret");
        assert!(!format!("{creds:?}").contains("very-secret"));
    }

    #[tokio::test]
    async fn a_missing_credential_is_a_config_error_naming_it() {
        let err = StaticCredentials::new()
            .get("ANTHROPIC_API_KEY")
            .await
            .unwrap_err();
        assert_eq!(
            err,
            LlmError::Config("credential `ANTHROPIC_API_KEY` is not set".into())
        );
    }
}
