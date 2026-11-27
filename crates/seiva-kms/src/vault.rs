//! HashiCorp Vault's (or OpenBao's) transit engine (feature `vault`): Vault
//! makes the data key (`datakey/plaintext`), unwraps it (`decrypt`) and, after
//! the transit key was rotated, re-wraps it with the newest version (`rewrap`).
//! The transit key never leaves Vault.
//!
//! Authentication: a token (given, or read from a file at every call so a
//! sidecar can renew it), **AppRole** (role id + a secret id read from a file)
//! or **Kubernetes** (the pod's service-account token). A login token is kept
//! for half its lease and dropped (one new login, one retry) when Vault answers
//! 403 with it.
//!
//! The minimum policy for the transit key `<k>` on mount `transit`:
//!
//! ```hcl
//! path "transit/datakey/plaintext/<k>" { capabilities = ["update"] }
//! path "transit/decrypt/<k>"           { capabilities = ["update"] }
//! path "transit/rewrap/<k>"            { capabilities = ["update"] }
//! ```
//!
//! Redirects are never followed (a token header must not travel to another
//! host), and the token header is marked sensitive.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::header::HeaderValue;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::keeper::retry_until;
use crate::{DataKey, GeneratedKey, KeyWrapper, KmsError};

/// The default service-account token path in a Kubernetes pod.
pub const K8S_TOKEN_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

/// How this process authenticates to Vault.
#[derive(Clone)]
#[non_exhaustive]
pub enum VaultAuth {
    /// A token given directly (prefer [`VaultAuth::TokenFile`] outside tests).
    Token(String),
    /// A token read from this file at every call (a renewing sidecar, a
    /// mounted Secret).
    TokenFile(PathBuf),
    /// The AppRole auth method.
    AppRole {
        /// The auth mount (`approle`).
        mount: String,
        /// The role id (not secret on its own).
        role_id: String,
        /// The file holding the secret id (read at every login).
        secret_id_file: PathBuf,
    },
    /// The Kubernetes auth method, with the pod's service-account token.
    Kubernetes {
        /// The auth mount (`kubernetes`).
        mount: String,
        /// The Vault role bound to the service account.
        role: String,
        /// The service-account token ([`K8S_TOKEN_FILE`]).
        jwt_file: PathBuf,
    },
}

impl VaultAuth {
    /// AppRole on the `approle` mount.
    pub fn approle(role_id: impl Into<String>, secret_id_file: impl Into<PathBuf>) -> Self {
        VaultAuth::AppRole {
            mount: "approle".into(),
            role_id: role_id.into(),
            secret_id_file: secret_id_file.into(),
        }
    }

    /// Kubernetes on the `kubernetes` mount, with the pod's token.
    pub fn kubernetes(role: impl Into<String>) -> Self {
        VaultAuth::Kubernetes {
            mount: "kubernetes".into(),
            role: role.into(),
            jwt_file: K8S_TOKEN_FILE.into(),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            VaultAuth::Token(_) => "token",
            VaultAuth::TokenFile(_) => "token-file",
            VaultAuth::AppRole { .. } => "approle",
            VaultAuth::Kubernetes { .. } => "kubernetes",
        }
    }

    fn logs_in(&self) -> bool {
        matches!(
            self,
            VaultAuth::AppRole { .. } | VaultAuth::Kubernetes { .. }
        )
    }
}

impl std::fmt::Debug for VaultAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Vault transit settings.
#[derive(Clone, Debug)]
pub struct VaultConfig {
    /// `https://vault.example.com:8200` (no path).
    pub address: String,
    /// The transit key that wraps data keys.
    pub transit_key: String,
    /// The transit engine's mount (`transit`).
    pub mount: String,
    /// How to authenticate.
    pub auth: VaultAuth,
    /// An extra CA (PEM bundle) trusted for Vault's certificate.
    pub ca_file: Option<PathBuf>,
    /// The Vault Enterprise / OpenBao namespace.
    pub namespace: Option<String>,
    /// The timeout of one call to Vault.
    pub request_timeout: Duration,
}

impl VaultConfig {
    /// The transit key `transit_key` at `address`, mount `transit`, 15 s calls.
    pub fn new(
        address: impl Into<String>,
        transit_key: impl Into<String>,
        auth: VaultAuth,
    ) -> Self {
        Self {
            address: address.into(),
            transit_key: transit_key.into(),
            mount: "transit".into(),
            auth,
            ca_file: None,
            namespace: None,
            request_timeout: Duration::from_secs(15),
        }
    }

    /// Another transit mount.
    pub fn with_mount(mut self, mount: impl Into<String>) -> Self {
        self.mount = mount.into();
        self
    }

    /// Trust this CA bundle (PEM) for Vault's certificate.
    pub fn with_ca_file(mut self, ca: impl Into<PathBuf>) -> Self {
        self.ca_file = Some(ca.into());
        self
    }

    /// A Vault namespace.
    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }
}

/// A path segment Vault accepts and that cannot climb (`..`) or inject a query.
fn segment_ok(s: &str, slash: bool) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') || (slash && b == b'/')
        })
        && !s.split('/').any(|p| p.is_empty() || p == "." || p == "..")
}

/// `vault:v3:...` → 3.
pub fn wrap_version(ciphertext: &str) -> Option<u32> {
    ciphertext
        .strip_prefix("vault:v")?
        .split_once(':')?
        .0
        .parse()
        .ok()
}

/// A failed call: the HTTP status (when Vault answered) and the error.
struct CallError {
    status: Option<u16>,
    err: KmsError,
}

impl From<KmsError> for CallError {
    fn from(err: KmsError) -> Self {
        CallError { status: None, err }
    }
}

/// The Vault transit [`KeyWrapper`].
pub struct VaultTransit {
    cfg: VaultConfig,
    base: String,
    http: reqwest::Client,
    /// A login token and when to stop using it.
    login: Mutex<Option<(Zeroizing<String>, Instant)>>,
}

impl std::fmt::Debug for VaultTransit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultTransit")
            .field("address", &self.base)
            .field("mount", &self.cfg.mount)
            .field("transit_key", &self.cfg.transit_key)
            .field("auth", &self.cfg.auth)
            .finish_non_exhaustive()
    }
}

fn read_secret_file(path: &Path, what: &str) -> Result<Zeroizing<String>, KmsError> {
    let t =
        Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            KmsError::Config(format!("reading the {what} {}: {e}", path.display()))
        })?);
    let trimmed = t.trim();
    if trimmed.is_empty() {
        return Err(KmsError::Config(format!(
            "the {what} {} is empty",
            path.display()
        )));
    }
    Ok(Zeroizing::new(trimmed.to_string()))
}

impl VaultTransit {
    /// Checks the settings and builds the HTTPS client (no call to Vault yet:
    /// see [`Self::wait_ready`]).
    pub fn new(cfg: VaultConfig) -> Result<Self, KmsError> {
        let url = reqwest::Url::parse(&cfg.address)
            .map_err(|e| KmsError::Config(format!("Vault address {:?}: {e}", cfg.address)))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.path().trim_end_matches('/').is_empty()
            || url.query().is_some()
            || !url.username().is_empty()
        {
            return Err(KmsError::Config(format!(
                "Vault address {:?} must be http(s)://host[:port] with no path, query or credentials",
                cfg.address
            )));
        }
        if !segment_ok(&cfg.transit_key, false) {
            return Err(KmsError::Config(format!(
                "invalid transit key name {:?}",
                cfg.transit_key
            )));
        }
        if !segment_ok(&cfg.mount, true) {
            return Err(KmsError::Config(format!(
                "invalid transit mount {:?}",
                cfg.mount
            )));
        }
        match &cfg.auth {
            VaultAuth::AppRole { mount, .. } | VaultAuth::Kubernetes { mount, .. }
                if !segment_ok(mount, true) =>
            {
                return Err(KmsError::Config(format!("invalid auth mount {mount:?}")));
            }
            _ => {}
        }
        let mut b = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(cfg.request_timeout)
            .redirect(reqwest::redirect::Policy::none());
        if let Some(p) = &cfg.ca_file {
            let pem = std::fs::read(p).map_err(|e| {
                KmsError::Config(format!("reading the Vault CA {}: {e}", p.display()))
            })?;
            let certs = reqwest::Certificate::from_pem_bundle(&pem)
                .map_err(|e| KmsError::Config(format!("the Vault CA {}: {e}", p.display())))?;
            if certs.is_empty() {
                return Err(KmsError::Config(format!(
                    "the Vault CA {} holds no certificate",
                    p.display()
                )));
            }
            for c in certs {
                b = b.add_root_certificate(c);
            }
        }
        let http = b
            .build()
            .map_err(|e| KmsError::Config(format!("Vault HTTP client: {e}")))?;
        Ok(Self {
            base: cfg.address.trim_end_matches('/').to_string(),
            cfg,
            http,
            login: Mutex::new(None),
        })
    }

    /// Waits until Vault is reachable and this process may use it
    /// ([`KeyWrapper::health`]), retrying until `timeout`. Fails closed: the
    /// caller must not report ready on an error.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), KmsError> {
        retry_until(timeout, || self.health()).await
    }

    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        token: Option<&str>,
    ) -> Result<reqwest::RequestBuilder, KmsError> {
        let mut rb = self
            .http
            .request(method, format!("{}/v1/{path}", self.base));
        if let Some(t) = token {
            let mut v = HeaderValue::from_str(t)
                .map_err(|_| KmsError::Config("the Vault token is not a valid header".into()))?;
            v.set_sensitive(true);
            rb = rb.header("X-Vault-Token", v);
        }
        if let Some(ns) = &self.cfg.namespace {
            rb = rb.header("X-Vault-Namespace", ns);
        }
        Ok(rb)
    }

    async fn send(&self, rb: reqwest::RequestBuilder, what: &str) -> Result<Value, CallError> {
        let r = rb.send().await.map_err(|e| {
            CallError::from(KmsError::Unavailable(format!(
                "Vault unreachable ({what}): {e}"
            )))
        })?;
        let status = r.status();
        let body = r.bytes().await.unwrap_or_default();
        let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(v);
        }
        // Vault lists several errors on separate lines: one line.
        let errs = v["errors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        let errs = errs.split_whitespace().collect::<Vec<_>>().join(" ");
        let msg = format!("Vault {what}: {status} {errs}");
        let err = if status.is_server_error() || status.as_u16() == 429 || errs.contains("sealed") {
            KmsError::Unavailable(msg)
        } else {
            KmsError::Refused(msg)
        };
        Err(CallError {
            status: Some(status.as_u16()),
            err,
        })
    }

    async fn post(&self, path: &str, token: Option<&str>, body: Value) -> Result<Value, CallError> {
        let rb = self
            .request(reqwest::Method::POST, path, token)?
            .json(&body);
        self.send(rb, path).await
    }

    /// A token: the configured one, or a login (kept for half its lease;
    /// `fresh` forces a new login).
    async fn token(&self, fresh: bool) -> Result<Zeroizing<String>, KmsError> {
        let (mount, body) = match &self.cfg.auth {
            VaultAuth::Token(t) => return Ok(Zeroizing::new(t.clone())),
            VaultAuth::TokenFile(f) => return read_secret_file(f, "Vault token file"),
            VaultAuth::AppRole {
                mount,
                role_id,
                secret_id_file,
            } => {
                if let Some(t) = self.cached(fresh) {
                    return Ok(t);
                }
                let secret = read_secret_file(secret_id_file, "AppRole secret id file")?;
                (
                    mount,
                    json!({ "role_id": role_id, "secret_id": secret.as_str() }),
                )
            }
            VaultAuth::Kubernetes {
                mount,
                role,
                jwt_file,
            } => {
                if let Some(t) = self.cached(fresh) {
                    return Ok(t);
                }
                let jwt = read_secret_file(jwt_file, "service-account token")?;
                (mount, json!({ "role": role, "jwt": jwt.as_str() }))
            }
        };
        let v = self
            .post(&format!("auth/{mount}/login"), None, body)
            .await
            .map_err(|e| e.err)?;
        let t = v["auth"]["client_token"]
            .as_str()
            .ok_or_else(|| KmsError::Malformed("Vault login returned no token".into()))?;
        let token = Zeroizing::new(t.to_string());
        // Half the lease (a renewal is never needed); a lease of 0 never expires.
        let keep = match v["auth"]["lease_duration"].as_u64().unwrap_or(0) {
            0 => 3600,
            l => l / 2,
        };
        if keep > 0 {
            *self.login.lock().expect("lock") =
                Some((token.clone(), Instant::now() + Duration::from_secs(keep)));
        }
        Ok(token)
    }

    fn cached(&self, fresh: bool) -> Option<Zeroizing<String>> {
        let mut g = self.login.lock().expect("lock");
        if fresh {
            *g = None;
            return None;
        }
        g.as_ref()
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(t, _)| t.clone())
    }

    /// A transit call with a token; a 403 with a login token logs in again
    /// and retries once (the token was revoked or expired early).
    async fn transit(&self, op: &str, key: &str, body: Value) -> Result<Value, KmsError> {
        if !segment_ok(key, false) {
            return Err(KmsError::Config(format!(
                "invalid transit key name {key:?}"
            )));
        }
        let path = format!("{}/{op}/{key}", self.cfg.mount);
        let token = self.token(false).await?;
        match self.post(&path, Some(&token), body.clone()).await {
            Ok(v) => Ok(v),
            Err(CallError {
                status: Some(403), ..
            }) if self.cfg.auth.logs_in() => {
                let token = self.token(true).await?;
                self.post(&path, Some(&token), body)
                    .await
                    .map_err(|e| e.err)
            }
            Err(e) => Err(e.err),
        }
    }

    fn data_key(v: &Value) -> Result<DataKey, KmsError> {
        let b64 = v["data"]["plaintext"]
            .as_str()
            .ok_or_else(|| KmsError::Malformed("Vault returned no plaintext".into()))?;
        let raw = Zeroizing::new(
            STANDARD
                .decode(b64)
                .map_err(|_| KmsError::Malformed("Vault returned an invalid plaintext".into()))?,
        );
        DataKey::from_slice(&raw, "Vault")
    }

    fn ciphertext(v: &Value) -> Result<String, KmsError> {
        v["data"]["ciphertext"]
            .as_str()
            .filter(|c| wrap_version(c).is_some())
            .map(str::to_string)
            .ok_or_else(|| KmsError::Malformed("Vault returned no ciphertext".into()))
    }
}

#[async_trait::async_trait]
impl KeyWrapper for VaultTransit {
    fn provider(&self) -> &'static str {
        "vault"
    }

    fn kek_id(&self) -> &str {
        &self.cfg.transit_key
    }

    async fn generate(&self) -> Result<GeneratedKey, KmsError> {
        let v = self
            .transit(
                "datakey/plaintext",
                &self.cfg.transit_key,
                json!({ "bits": 256 }),
            )
            .await?;
        Ok(GeneratedKey {
            plain: Self::data_key(&v)?,
            wrapped: Self::ciphertext(&v)?,
        })
    }

    async fn unwrap(&self, kek_id: &str, wrapped: &str) -> Result<DataKey, KmsError> {
        let v = self
            .transit("decrypt", kek_id, json!({ "ciphertext": wrapped }))
            .await?;
        Self::data_key(&v)
    }

    async fn rewrap(
        &self,
        kek_id: &str,
        wrapped: &str,
        _plain: &DataKey,
    ) -> Result<Option<String>, KmsError> {
        let v = self
            .transit("rewrap", kek_id, json!({ "ciphertext": wrapped }))
            .await?;
        let ct = Self::ciphertext(&v)?;
        Ok((wrap_version(&ct) > wrap_version(wrapped)).then_some(ct))
    }

    /// Vault is up and unsealed (`sys/health`, standbys accepted), and this
    /// process's token is valid (`auth/token/lookup-self`, in Vault's default
    /// policy).
    async fn health(&self) -> Result<(), KmsError> {
        let rb = self.request(
            reqwest::Method::GET,
            "sys/health?standbyok=true&perfstandbyok=true",
            None,
        )?;
        match self.send(rb, "sys/health").await {
            Ok(_) => {}
            Err(CallError {
                status: Some(s @ (501 | 503)),
                ..
            }) => {
                return Err(KmsError::Unavailable(format!(
                    "Vault is {}",
                    if s == 501 {
                        "not initialised"
                    } else {
                        "sealed"
                    }
                )))
            }
            Err(e) => return Err(e.err),
        }
        let token = self.token(false).await?;
        let rb = self.request(reqwest::Method::GET, "auth/token/lookup-self", Some(&token))?;
        match self.send(rb, "auth/token/lookup-self").await {
            Ok(_) => Ok(()),
            Err(CallError {
                status: Some(403), ..
            }) if self.cfg.auth.logs_in() => {
                let token = self.token(true).await?;
                let rb =
                    self.request(reqwest::Method::GET, "auth/token/lookup-self", Some(&token))?;
                self.send(rb, "auth/token/lookup-self")
                    .await
                    .map(|_| ())
                    .map_err(|e| e.err)
            }
            Err(e) => Err(e.err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(wrap_version("vault:v1:abc"), Some(1));
        assert_eq!(wrap_version("vault:v12:abc"), Some(12));
        assert_eq!(wrap_version("nope"), None);
        assert!(wrap_version("vault:v2:x") > wrap_version("vault:v1:x"));
    }

    #[test]
    fn settings_are_checked() {
        let ok = |addr: &str, key: &str, mount: &str| {
            VaultTransit::new(
                VaultConfig::new(addr, key, VaultAuth::Token("t".into())).with_mount(mount),
            )
        };
        assert!(ok("http://127.0.0.1:8200", "sentinel", "transit").is_ok());
        assert!(ok("https://vault.example.com/", "k", "kv/transit").is_ok());
        assert!(ok("ftp://vault", "k", "transit").is_err());
        assert!(ok("https://vault/v1", "k", "transit").is_err());
        assert!(ok("https://user:pw@vault", "k", "transit").is_err());
        assert!(ok("not a url", "k", "transit").is_err());
        assert!(ok("https://vault", "a/b", "transit").is_err());
        assert!(ok("https://vault", "..", "transit").is_err());
        assert!(ok("https://vault", "k", "../sys").is_err());
        assert!(ok("https://vault", "k", "").is_err());
        assert!(ok("https://vault", "k?x=1", "transit").is_err());
        let bad_auth = VaultConfig::new(
            "https://vault",
            "k",
            VaultAuth::AppRole {
                mount: "../x".into(),
                role_id: "r".into(),
                secret_id_file: "/nope".into(),
            },
        );
        assert!(VaultTransit::new(bad_auth).is_err());
    }

    #[test]
    fn a_bad_ca_file_fails_at_construction() {
        let dir = std::env::temp_dir().join(format!("seiva-kms-ca-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ca = dir.join("ca.pem");
        std::fs::write(&ca, "not a certificate").unwrap();
        let cfg = |p: &Path| {
            VaultConfig::new("https://vault", "k", VaultAuth::Token("t".into())).with_ca_file(p)
        };
        assert!(matches!(
            VaultTransit::new(cfg(&ca)),
            Err(KmsError::Config(_))
        ));
        assert!(matches!(
            VaultTransit::new(cfg(&dir.join("missing.pem"))),
            Err(KmsError::Config(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn secrets_are_not_printed() {
        let t = VaultTransit::new(VaultConfig::new(
            "http://127.0.0.1:8200",
            "k",
            VaultAuth::Token("hvs.SECRET".into()),
        ))
        .unwrap();
        assert!(!format!("{t:?}").contains("SECRET"));
        assert!(!format!("{:?}", t.cfg).contains("SECRET"));
    }

    #[tokio::test]
    async fn an_unreachable_vault_is_transient_and_boot_fails_closed() {
        // Port 1 on loopback: nothing listens, the connection is refused.
        let t = VaultTransit::new(VaultConfig::new(
            "http://127.0.0.1:1",
            "k",
            VaultAuth::Token("t".into()),
        ))
        .unwrap();
        let e = t.generate().await.unwrap_err();
        assert!(e.is_transient(), "{e}");
        let started = Instant::now();
        let e = t.wait_ready(Duration::from_millis(1200)).await.unwrap_err();
        assert!(e.is_transient(), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn token_sources() {
        let dir = std::env::temp_dir().join(format!("seiva-kms-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("token");
        std::fs::write(&f, "  hvs.abc \n").unwrap();
        let t = VaultTransit::new(VaultConfig::new(
            "http://127.0.0.1:1",
            "k",
            VaultAuth::TokenFile(f.clone()),
        ))
        .unwrap();
        assert_eq!(t.token(false).await.unwrap().as_str(), "hvs.abc");
        std::fs::write(&f, "\n").unwrap();
        assert!(matches!(t.token(false).await, Err(KmsError::Config(_))));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
