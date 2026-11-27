//! Serving mutual TLS from the service itself (ADR 0047) — feature `mtls`.
//!
//! [`serve_router_mtls`] is [`serve_router`](crate::serve_router) over a
//! [`MtlsListener`]: the same Starting → Ready → Draining lifecycle, the same
//! route-collision check, but TLS 1.3 terminated here and each client
//! certificate verified against the deployment's CA before a request is read.
//! [`MtlsFiles`] reads the 12-factor configuration:
//!
//! | Variable | Meaning |
//! |---|---|
//! | `SEIVA_TLS_CERT_FILE` | Server certificate chain, PEM (leaf first) |
//! | `SEIVA_TLS_KEY_FILE` | Server private key, PEM |
//! | `SEIVA_TLS_CLIENT_CA_FILE` | CA(s) client certificates must chain to, PEM |
//! | `SEIVA_TLS_CLIENT_AUTH` | `required` (default) or `optional` |
//! | `SEIVA_TLS_CRL_FILE` | Optional CRL(s), PEM |
//!
//! The first three come together or not at all: a partial set fails the boot,
//! so a typo cannot silently start a listener without client verification.

use std::future::Future;
use std::net::SocketAddr;

use axum::Router;
use seiva_config::{ConfigError, EnvReader, FromEnv};
use seiva_http::mtls::{mtls_make_service, ClientAuth, MtlsConfig, MtlsListener};
use seiva_http::Health;

use crate::ServeError;

/// Server certificate chain file (PEM, leaf first).
pub const TLS_CERT_FILE_VAR: &str = "SEIVA_TLS_CERT_FILE";
/// Server private key file (PEM).
pub const TLS_KEY_FILE_VAR: &str = "SEIVA_TLS_KEY_FILE";
/// Client CA file (PEM): the only roots a client certificate may chain to.
pub const TLS_CLIENT_CA_FILE_VAR: &str = "SEIVA_TLS_CLIENT_CA_FILE";
/// `required` (default) or `optional`; see [`ClientAuth`].
pub const TLS_CLIENT_AUTH_VAR: &str = "SEIVA_TLS_CLIENT_AUTH";
/// Optional CRL file (PEM, one or more CRLs).
pub const TLS_CRL_FILE_VAR: &str = "SEIVA_TLS_CRL_FILE";

/// Where the mutual-TLS material lives, as read from `SEIVA_TLS_*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtlsFiles {
    /// [`TLS_CERT_FILE_VAR`].
    pub cert_file: String,
    /// [`TLS_KEY_FILE_VAR`].
    pub key_file: String,
    /// [`TLS_CLIENT_CA_FILE_VAR`].
    pub client_ca_file: String,
    /// [`TLS_CLIENT_AUTH_VAR`].
    pub client_auth: ClientAuth,
    /// [`TLS_CRL_FILE_VAR`].
    pub crl_file: Option<String>,
}

/// `Option<MtlsFiles>` as a [`FromEnv`] target: `None` when no `SEIVA_TLS_*`
/// variable is set.
struct MaybeMtlsFiles(Option<MtlsFiles>);

impl FromEnv for MaybeMtlsFiles {
    fn from_env(reader: &mut EnvReader) -> Option<Self> {
        let present = [
            TLS_CERT_FILE_VAR,
            TLS_KEY_FILE_VAR,
            TLS_CLIENT_CA_FILE_VAR,
            TLS_CLIENT_AUTH_VAR,
            TLS_CRL_FILE_VAR,
        ]
        .iter()
        .any(|name| reader.optional_var(name).is_some());
        if !present {
            return Some(Self(None));
        }
        // require_var records a "missing" problem for each of the three left unset.
        let cert_file = reader.require_var(TLS_CERT_FILE_VAR);
        let key_file = reader.require_var(TLS_KEY_FILE_VAR);
        let client_ca_file = reader.require_var(TLS_CLIENT_CA_FILE_VAR);
        let client_auth = match reader.optional_var(TLS_CLIENT_AUTH_VAR).as_deref() {
            None | Some("required") => Some(ClientAuth::Required),
            Some("optional") => Some(ClientAuth::Optional),
            Some(other) => {
                reader.guard(Err(ConfigError::single(format!(
                    "{TLS_CLIENT_AUTH_VAR} must be `required` or `optional`, not `{other}`"
                ))));
                None
            }
        };
        let crl_file = reader.optional_var(TLS_CRL_FILE_VAR);
        Some(Self(Some(MtlsFiles {
            cert_file: cert_file?,
            key_file: key_file?,
            client_ca_file: client_ca_file?,
            client_auth: client_auth?,
            crl_file,
        })))
    }
}

impl MtlsFiles {
    /// Read `SEIVA_TLS_*` from the process environment: `Ok(None)` when none is
    /// set (serve plain HTTP, e.g. behind a TLS-terminating proxy).
    ///
    /// # Errors
    ///
    /// A [`ConfigError`] listing every problem when the set is partial or
    /// `SEIVA_TLS_CLIENT_AUTH` is not `required`/`optional`.
    pub fn from_process_env() -> Result<Option<Self>, ConfigError> {
        seiva_config::load::<MaybeMtlsFiles>().map(|m| m.0)
    }

    /// Like [`from_process_env`](Self::from_process_env), over an explicit map
    /// (tests, or a configuration assembled elsewhere).
    ///
    /// # Errors
    ///
    /// As [`from_process_env`](Self::from_process_env).
    pub fn from_map(
        vars: std::collections::HashMap<String, String>,
    ) -> Result<Option<Self>, ConfigError> {
        seiva_config::load_from::<MaybeMtlsFiles>(vars).map(|m| m.0)
    }

    /// Read the files and build the validated [`MtlsConfig`].
    ///
    /// # Errors
    ///
    /// [`ServeError::Tls`] naming the file that could not be read, or what
    /// rustls refused.
    pub fn load(&self) -> Result<MtlsConfig, ServeError> {
        let read = |path: &str| {
            std::fs::read(path).map_err(|e| ServeError::Tls(format!("cannot read {path}: {e}")))
        };
        let mut builder = MtlsConfig::builder()
            .server_identity_pem(&read(&self.cert_file)?, &read(&self.key_file)?)
            .client_ca_pem(&read(&self.client_ca_file)?)
            .client_auth(self.client_auth);
        if let Some(crl) = &self.crl_file {
            builder = builder.crl_pem(&read(crl)?);
        }
        builder.build().map_err(|e| ServeError::Tls(e.to_string()))
    }
}

/// [`serve_router`](crate::serve_router) over mutual TLS: bind `addr` with
/// `mtls`, flip `health` to Ready once listening, Draining on `shutdown`.
///
/// Each request carries [`ConnectInfo`](axum::extract::ConnectInfo)`<SocketAddr>`
/// (so the rate limiter keys on the client IP) and, when the client presented
/// one, its verified [`PeerCertificate`](seiva_http::mtls::PeerCertificate) —
/// which the `authn` layer maps to a `Principal` when the router was built with
/// [`AuthConfig::with_client_certificates`](seiva_http::AuthConfig::with_client_certificates).
///
/// # Errors
///
/// [`ServeError::RouteCollision`], [`ServeError::Bind`] or [`ServeError::Serve`],
/// as [`serve_router`](crate::serve_router).
pub async fn serve_router_mtls(
    addr: SocketAddr,
    router: Router,
    health: Health,
    mtls: &MtlsConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let collisions = seiva_http::route_collisions();
    if !collisions.is_empty() {
        return Err(ServeError::RouteCollision(collisions));
    }

    let listener = MtlsListener::bind(addr, mtls)
        .await
        .map_err(ServeError::Bind)?;

    health.set_ready();

    let draining = {
        let health = health.clone();
        async move {
            shutdown.await;
            health.set_draining();
        }
    };

    axum::serve(listener, mtls_make_service(router))
        .with_graceful_shutdown(draining)
        .await
        .map_err(ServeError::Serve)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn no_tls_variable_means_plain_http() {
        assert_eq!(MtlsFiles::from_map(vars(&[])).unwrap(), None);
    }

    #[test]
    fn the_full_set_is_read_with_required_by_default() {
        let files = MtlsFiles::from_map(vars(&[
            (TLS_CERT_FILE_VAR, "/tls/server.pem"),
            (TLS_KEY_FILE_VAR, "/tls/server.key"),
            (TLS_CLIENT_CA_FILE_VAR, "/tls/ca.pem"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(files.client_auth, ClientAuth::Required);
        assert_eq!(files.crl_file, None);

        let optional = MtlsFiles::from_map(vars(&[
            (TLS_CERT_FILE_VAR, "/tls/server.pem"),
            (TLS_KEY_FILE_VAR, "/tls/server.key"),
            (TLS_CLIENT_CA_FILE_VAR, "/tls/ca.pem"),
            (TLS_CLIENT_AUTH_VAR, "optional"),
            (TLS_CRL_FILE_VAR, "/tls/ca.crl"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(optional.client_auth, ClientAuth::Optional);
        assert_eq!(optional.crl_file.as_deref(), Some("/tls/ca.crl"));
    }

    #[test]
    fn a_partial_set_fails_the_boot_naming_what_is_missing() {
        let err = MtlsFiles::from_map(vars(&[(TLS_CERT_FILE_VAR, "/tls/server.pem")])).unwrap_err();
        let problems = err.problems().join("\n");
        assert!(problems.contains(TLS_KEY_FILE_VAR), "{problems}");
        assert!(problems.contains(TLS_CLIENT_CA_FILE_VAR), "{problems}");
    }

    #[test]
    fn an_unknown_client_auth_mode_fails_the_boot() {
        let err = MtlsFiles::from_map(vars(&[
            (TLS_CERT_FILE_VAR, "/tls/server.pem"),
            (TLS_KEY_FILE_VAR, "/tls/server.key"),
            (TLS_CLIENT_CA_FILE_VAR, "/tls/ca.pem"),
            (TLS_CLIENT_AUTH_VAR, "sometimes"),
        ]))
        .unwrap_err();
        assert!(err.problems().join("\n").contains(TLS_CLIENT_AUTH_VAR));
    }

    #[test]
    fn an_unreadable_file_is_a_tls_startup_error() {
        let files = MtlsFiles {
            cert_file: "/nonexistent/server.pem".into(),
            key_file: "/nonexistent/server.key".into(),
            client_ca_file: "/nonexistent/ca.pem".into(),
            client_auth: ClientAuth::Required,
            crl_file: None,
        };
        let err = files.load().unwrap_err();
        assert!(matches!(&err, ServeError::Tls(m) if m.contains("/nonexistent/server.pem")));
    }
}
