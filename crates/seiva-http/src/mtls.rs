//! Mutual TLS for the HTTP transport (ADR 0047) — feature `mtls`.
//!
//! A service whose callers are machines (agents, gateways, other services) can
//! authenticate them by **client certificate** instead of a bearer secret. The
//! certificate is verified during the TLS handshake against the deployment's
//! own CA, and the verified leaf then becomes a [`Principal`] through the same
//! deny-by-default [`authn`](fn@crate::authn) middleware every other credential
//! goes through — one identity model, one permission guard.
//!
//! The pieces, in the order a request meets them:
//!
//! 1. [`MtlsConfig`] — the server identity, the client CA roots, optional CRLs
//!    and the [`ClientAuth`] mode. **TLS 1.3 only**, `ring` provider, ALPN
//!    `h2` + `http/1.1`.
//! 2. [`MtlsListener`] — an [`axum::serve::Listener`] that accepts TCP and runs
//!    each handshake on its own task, bounded by a timeout and a cap on pending
//!    handshakes, so a slow or hostile client never stalls the accept loop.
//! 3. [`mtls_make_service`] — the per-connection service handed to
//!    [`axum::serve`](fn@axum::serve): it puts the peer [`ConnectInfo`]`<SocketAddr>` (so the
//!    rate limiter keeps keying on the client IP) and the [`PeerCertificate`]
//!    into every request on that connection.
//! 4. [`CertificateAuthenticator`] — the application's mapping from a verified
//!    [`ClientCertificate`] to a [`Principal`] (look the device up, check it is
//!    not revoked, grant its permissions). Plug it into
//!    [`AuthConfig::with_client_certificates`](crate::AuthConfig::with_client_certificates).
//!
//! The handshake proves the caller holds a key the CA certified; the
//! authenticator decides, **per request**, what that identity may do — so a
//! revocation recorded in the application's store takes effect on the next
//! request, not only on the next connection.
//!
//! ```no_run
//! # async fn run(router: axum::Router, cert: &[u8], key: &[u8], ca: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! use seiva_http::mtls::{mtls_make_service, ClientAuth, MtlsConfig, MtlsListener};
//!
//! let config = MtlsConfig::builder()
//!     .server_identity_pem(cert, key)
//!     .client_ca_pem(ca)
//!     .client_auth(ClientAuth::Required)
//!     .build()?;
//! let listener = MtlsListener::bind("0.0.0.0:8443".parse()?, &config).await?;
//! axum::serve(listener, mtls_make_service(router)).await?;
//! # Ok(()) }
//! ```

use std::convert::Infallible;
use std::fmt;
use std::future::{ready, Ready};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::{ConnectInfo, Request};
use axum::response::Response;
use axum::serve::{IncomingStream, Listener};
use axum::Router;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use seiva_identity::{AuthError, Principal};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

/// How long a client has to complete the TLS handshake before the connection
/// is dropped. Long enough for a slow link, short enough that a stalled client
/// does not hold a handshake slot.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many handshakes may be in flight (and completed connections waiting to
/// be served) at once. Past it the listener stops accepting until a slot frees,
/// so a flood of half-open TLS connections cannot exhaust memory.
pub const DEFAULT_MAX_PENDING_HANDSHAKES: usize = 1024;

/// Whether the handshake demands a client certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientAuth {
    /// Every connection must present a certificate the client CA verifies;
    /// otherwise the handshake fails and no HTTP request is ever read. The
    /// default: a listener dedicated to machine callers.
    #[default]
    Required,
    /// A certificate is verified **if** presented; a client without one still
    /// connects and falls through to the bearer/API-key/anonymous paths of
    /// [`authn`](fn@crate::authn). For one listener serving both machines
    /// (certificate) and people (OIDC bearer).
    Optional,
}

/// Why an [`MtlsConfig`] could not be built, or a certificate not be read.
#[derive(Debug)]
pub enum MtlsError {
    /// No server certificate chain and key were given.
    MissingServerIdentity,
    /// No client CA was given: there would be nothing to verify clients against.
    MissingClientCa,
    /// A PEM input held no item of the expected kind (`what` names it).
    Empty(&'static str),
    /// A PEM input could not be parsed (`what` names it).
    Pem(&'static str, String),
    /// rustls refused the configuration (e.g. a key that does not match the
    /// certificate, or a CA certificate it cannot use as a trust anchor).
    Tls(rustls::Error),
    /// The client-certificate verifier could not be built (e.g. an unusable CRL).
    Verifier(String),
    /// A verified certificate could not be parsed as X.509.
    Certificate(String),
}

impl fmt::Display for MtlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingServerIdentity => f.write_str("mTLS: no server certificate and key"),
            Self::MissingClientCa => f.write_str("mTLS: no client CA to verify clients against"),
            Self::Empty(what) => write!(f, "mTLS: no {what} found in the PEM input"),
            Self::Pem(what, e) => write!(f, "mTLS: invalid {what} PEM: {e}"),
            Self::Tls(e) => write!(f, "mTLS: TLS configuration refused: {e}"),
            Self::Verifier(e) => write!(f, "mTLS: client verifier: {e}"),
            Self::Certificate(e) => write!(f, "mTLS: unreadable client certificate: {e}"),
        }
    }
}

impl std::error::Error for MtlsError {}

impl From<rustls::Error> for MtlsError {
    fn from(e: rustls::Error) -> Self {
        Self::Tls(e)
    }
}

/// A validated server-side mTLS configuration. Build it with
/// [`MtlsConfig::builder`]; cheap to clone (the rustls config is shared).
#[derive(Clone)]
pub struct MtlsConfig {
    tls: Arc<ServerConfig>,
    client_auth: ClientAuth,
    handshake_timeout: Duration,
    max_pending_handshakes: usize,
}

impl fmt::Debug for MtlsConfig {
    // Never prints key material: only the policy knobs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MtlsConfig")
            .field("client_auth", &self.client_auth)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("max_pending_handshakes", &self.max_pending_handshakes)
            .finish_non_exhaustive()
    }
}

impl MtlsConfig {
    /// Start a configuration. Server identity and client CA are mandatory.
    #[must_use]
    pub fn builder() -> MtlsConfigBuilder {
        MtlsConfigBuilder::default()
    }

    /// The client-certificate policy this configuration enforces.
    #[must_use]
    pub fn client_auth(&self) -> ClientAuth {
        self.client_auth
    }

    /// The rustls server configuration (TLS 1.3 only, client verifier set).
    #[must_use]
    pub fn rustls_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.tls)
    }
}

/// Builder for [`MtlsConfig`]; every input is PEM bytes, so the caller decides
/// where they come from (a file, a secret store, an environment variable).
#[derive(Default)]
pub struct MtlsConfigBuilder {
    chain_pem: Option<Vec<u8>>,
    key_pem: Option<Vec<u8>>,
    client_ca_pem: Option<Vec<u8>>,
    crl_pems: Vec<Vec<u8>>,
    client_auth: ClientAuth,
    end_entity_revocation_only: bool,
    handshake_timeout: Option<Duration>,
    max_pending_handshakes: Option<usize>,
}

impl fmt::Debug for MtlsConfigBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MtlsConfigBuilder")
            .field("has_server_identity", &self.chain_pem.is_some())
            .field("has_client_ca", &self.client_ca_pem.is_some())
            .field("crls", &self.crl_pems.len())
            .field("client_auth", &self.client_auth)
            .finish_non_exhaustive()
    }
}

impl MtlsConfigBuilder {
    /// The server's certificate chain (leaf first) and its private key, PEM.
    #[must_use]
    pub fn server_identity_pem(mut self, chain: &[u8], key: &[u8]) -> Self {
        self.chain_pem = Some(chain.to_vec());
        self.key_pem = Some(key.to_vec());
        self
    }

    /// The CA certificate(s) client certificates must chain to, PEM. Only these
    /// are trusted — never the system roots, so a public CA (or a TLS
    /// interception proxy's CA) cannot mint an accepted client.
    #[must_use]
    pub fn client_ca_pem(mut self, ca: &[u8]) -> Self {
        self.client_ca_pem = Some(ca.to_vec());
        self
    }

    /// A certificate revocation list, PEM (repeatable). With CRLs set, every
    /// certificate in the client's chain must have a known, unrevoked status
    /// unless [`end_entity_revocation_only`](Self::end_entity_revocation_only)
    /// narrows the check to the leaf.
    #[must_use]
    pub fn crl_pem(mut self, crl: &[u8]) -> Self {
        self.crl_pems.push(crl.to_vec());
        self
    }

    /// Check revocation only for the client's own certificate, not its issuing
    /// CAs — for a PKI whose offline root publishes no CRL alongside the
    /// intermediate's.
    #[must_use]
    pub fn end_entity_revocation_only(mut self) -> Self {
        self.end_entity_revocation_only = true;
        self
    }

    /// Whether a client certificate is [`Required`](ClientAuth::Required)
    /// (default) or [`Optional`](ClientAuth::Optional).
    #[must_use]
    pub fn client_auth(mut self, mode: ClientAuth) -> Self {
        self.client_auth = mode;
        self
    }

    /// How long a client has to finish the handshake
    /// (default [`DEFAULT_HANDSHAKE_TIMEOUT`]).
    #[must_use]
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = Some(timeout);
        self
    }

    /// The cap on in-flight handshakes (default
    /// [`DEFAULT_MAX_PENDING_HANDSHAKES`]; `0` is treated as `1`).
    #[must_use]
    pub fn max_pending_handshakes(mut self, max: usize) -> Self {
        self.max_pending_handshakes = Some(max);
        self
    }

    /// Validate the inputs and build the configuration.
    ///
    /// # Errors
    ///
    /// [`MtlsError`] when an input is missing, unparseable or refused by rustls.
    pub fn build(self) -> Result<MtlsConfig, MtlsError> {
        let (Some(chain_pem), Some(key_pem)) = (self.chain_pem, self.key_pem) else {
            return Err(MtlsError::MissingServerIdentity);
        };
        let ca_pem = self.client_ca_pem.ok_or(MtlsError::MissingClientCa)?;

        let chain = certificates(&chain_pem, "server certificate")?;
        let key = PrivateKeyDer::from_pem_slice(&key_pem)
            .map_err(|e| MtlsError::Pem("server private key", e.to_string()))?;

        let mut roots = RootCertStore::empty();
        for ca in certificates(&ca_pem, "client CA certificate")? {
            roots.add(ca)?;
        }

        let mut crls = Vec::new();
        for pem in &self.crl_pems {
            let mut found = false;
            for crl in CertificateRevocationListDer::pem_slice_iter(pem) {
                crls.push(crl.map_err(|e| MtlsError::Pem("CRL", e.to_string()))?);
                found = true;
            }
            if !found {
                return Err(MtlsError::Empty("CRL"));
            }
        }

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
                .with_crls(crls);
        if self.end_entity_revocation_only {
            verifier = verifier.only_check_end_entity_revocation();
        }
        if self.client_auth == ClientAuth::Optional {
            verifier = verifier.allow_unauthenticated();
        }
        let verifier = verifier
            .build()
            .map_err(|e| MtlsError::Verifier(e.to_string()))?;

        let mut tls = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(verifier)
            .with_single_cert(chain, key)?;
        tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(MtlsConfig {
            tls: Arc::new(tls),
            client_auth: self.client_auth,
            handshake_timeout: self.handshake_timeout.unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT),
            max_pending_handshakes: self
                .max_pending_handshakes
                .unwrap_or(DEFAULT_MAX_PENDING_HANDSHAKES)
                .max(1),
        })
    }
}

/// Every certificate in a PEM input; an input with none is an error.
fn certificates(pem: &[u8], what: &'static str) -> Result<Vec<CertificateDer<'static>>, MtlsError> {
    let certs = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MtlsError::Pem(what, e.to_string()))?;
    if certs.is_empty() {
        return Err(MtlsError::Empty(what));
    }
    Ok(certs)
}

/// The verified client certificate of a connection, as the fields an
/// authenticator maps to an identity. Built only from a chain rustls already
/// verified against the configured client CA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCertificate {
    der: CertificateDer<'static>,
    subject_common_name: Option<String>,
    dns_names: Vec<String>,
    uris: Vec<String>,
    serial_hex: String,
    not_after: i64,
    fingerprint_sha256: String,
}

impl ClientCertificate {
    /// Parse a DER certificate. Use on a **verified** leaf only: parsing proves
    /// nothing about trust.
    ///
    /// # Errors
    ///
    /// [`MtlsError::Certificate`] when the bytes are not a readable X.509
    /// certificate.
    pub fn from_der(der: CertificateDer<'static>) -> Result<Self, MtlsError> {
        let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref())
            .map_err(|e| MtlsError::Certificate(e.to_string()))?;

        let subject_common_name = cert
            .subject()
            .iter_common_name()
            .next()
            .and_then(|cn| cn.as_str().ok())
            .map(str::to_owned);

        let mut dns_names = Vec::new();
        let mut uris = Vec::new();
        if let Some(san) = cert
            .subject_alternative_name()
            .map_err(|e| MtlsError::Certificate(e.to_string()))?
        {
            for name in &san.value.general_names {
                match name {
                    x509_parser::extensions::GeneralName::DNSName(n) => {
                        dns_names.push((*n).to_owned());
                    }
                    x509_parser::extensions::GeneralName::URI(u) => uris.push((*u).to_owned()),
                    _ => {}
                }
            }
        }

        let serial_hex = hex(cert.tbs_certificate.raw_serial());
        let not_after = cert.validity().not_after.timestamp();
        let fingerprint_sha256 = hex(&Sha256::digest(der.as_ref()));

        Ok(Self {
            subject_common_name,
            dns_names,
            uris,
            serial_hex,
            not_after,
            fingerprint_sha256,
            der,
        })
    }

    /// The subject's Common Name, if any.
    #[must_use]
    pub fn subject_common_name(&self) -> Option<&str> {
        self.subject_common_name.as_deref()
    }

    /// The DNS names of the Subject Alternative Name extension.
    #[must_use]
    pub fn dns_names(&self) -> &[String] {
        &self.dns_names
    }

    /// The URIs of the Subject Alternative Name extension (e.g. a SPIFFE-style
    /// `urn:` or `spiffe://` device identity).
    #[must_use]
    pub fn uris(&self) -> &[String] {
        &self.uris
    }

    /// The serial number, lowercase hex without separators.
    #[must_use]
    pub fn serial_hex(&self) -> &str {
        &self.serial_hex
    }

    /// The end of validity, Unix seconds.
    #[must_use]
    pub fn not_after(&self) -> i64 {
        self.not_after
    }

    /// SHA-256 of the DER certificate, lowercase hex — a stable handle for
    /// pinning or a revocation list kept by the application.
    #[must_use]
    pub fn fingerprint_sha256(&self) -> &str {
        &self.fingerprint_sha256
    }

    /// The DER-encoded certificate.
    #[must_use]
    pub fn der(&self) -> &CertificateDer<'static> {
        &self.der
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// What a connection presented, as a request extension. Absent when the client
/// sent no certificate (only possible with [`ClientAuth::Optional`]).
#[derive(Debug, Clone)]
pub enum PeerCertificate {
    /// A certificate the client CA verified, parsed.
    Verified(Arc<ClientCertificate>),
    /// rustls verified a chain whose leaf could not be parsed as X.509. The
    /// [`authn`](fn@crate::authn) middleware answers `401` rather than guess.
    Unreadable,
}

/// Maps a verified client certificate to the caller's [`Principal`] — the
/// application's decision (which device or service this is, whether it is
/// still enrolled, which permissions it holds). Runs on **every request**, so a
/// revocation in the application's store applies immediately.
///
/// Any `Fn(&ClientCertificate) -> Result<Principal, AuthError>` closure is one.
pub trait CertificateAuthenticator: Send + Sync {
    /// Authenticate the caller behind `certificate`.
    ///
    /// # Errors
    ///
    /// An [`AuthError`] when the certificate maps to no active identity; the
    /// request is answered `401`.
    fn authenticate(&self, certificate: &ClientCertificate) -> Result<Principal, AuthError>;
}

impl<F> CertificateAuthenticator for F
where
    F: Fn(&ClientCertificate) -> Result<Principal, AuthError> + Send + Sync,
{
    fn authenticate(&self, certificate: &ClientCertificate) -> Result<Principal, AuthError> {
        self(certificate)
    }
}

/// An [`axum::serve::Listener`] that terminates mutual TLS.
///
/// TCP accepts run on a background task; each handshake runs on its own task
/// under the configured timeout, and at most `max_pending_handshakes` are in
/// flight. Only completed handshakes reach `axum::serve`. Dropping the listener
/// stops the background task.
#[derive(Debug)]
pub struct MtlsListener {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local_addr: SocketAddr,
    acceptor: JoinHandle<()>,
}

impl MtlsListener {
    /// Bind `addr` and serve mutual TLS on it.
    ///
    /// # Errors
    ///
    /// The bind error.
    pub async fn bind(addr: SocketAddr, config: &MtlsConfig) -> io::Result<Self> {
        Self::from_tcp(TcpListener::bind(addr).await?, config)
    }

    /// Serve mutual TLS on an already-bound `listener` (e.g. one bound to port
    /// `0` to learn the ephemeral port first).
    ///
    /// # Errors
    ///
    /// When the listener's local address cannot be read.
    pub fn from_tcp(listener: TcpListener, config: &MtlsConfig) -> io::Result<Self> {
        let local_addr = listener.local_addr()?;
        let (tx, ready) = mpsc::channel(config.max_pending_handshakes);
        let acceptor = tokio::spawn(accept_loop(
            listener,
            TlsAcceptor::from(config.rustls_config()),
            tx,
            Arc::new(Semaphore::new(config.max_pending_handshakes)),
            config.handshake_timeout,
        ));
        Ok(Self {
            ready,
            local_addr,
            acceptor,
        })
    }
}

impl Drop for MtlsListener {
    fn drop(&mut self) {
        self.acceptor.abort();
    }
}

impl Listener for MtlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(connection) => connection,
            // The accept task is gone (it only ends when the listener is
            // dropped, or on a runtime shutdown): no more connections will come.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    ready: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
    slots: Arc<Semaphore>,
    timeout: Duration,
) {
    loop {
        // Wait for a free slot BEFORE accepting, so back-pressure reaches the
        // kernel's accept queue instead of piling up tasks here.
        let Ok(slot) = Arc::clone(&slots).acquire_owned().await else {
            return;
        };
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                // Per-connection errors (reset before accept) are routine; a
                // resource error (EMFILE/ENFILE) needs a pause, as axum does.
                if !is_connection_error(&e) {
                    tracing::warn!(error = %e, "mTLS listener: accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let ready = ready.clone();
        tokio::spawn(async move {
            let _slot = slot;
            match tokio::time::timeout(timeout, acceptor.accept(tcp)).await {
                Ok(Ok(stream)) => {
                    // A closed receiver means the listener is gone: drop it.
                    let _ = ready.send((stream, peer)).await;
                }
                Ok(Err(e)) => tracing::debug!(%peer, error = %e, "mTLS handshake refused"),
                Err(_) => tracing::debug!(%peer, "mTLS handshake timed out"),
            }
        });
    }
}

fn is_connection_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

/// The client certificate rustls verified on `stream`, if the client sent one.
fn peer_certificate(stream: &TlsStream<TcpStream>) -> Option<PeerCertificate> {
    let leaf = stream.get_ref().1.peer_certificates()?.first()?.clone();
    Some(match ClientCertificate::from_der(leaf) {
        Ok(certificate) => PeerCertificate::Verified(Arc::new(certificate)),
        Err(e) => {
            tracing::warn!(error = %e, "mTLS: verified client certificate is unreadable");
            PeerCertificate::Unreadable
        }
    })
}

/// The make-service for [`axum::serve`](fn@axum::serve) over an [`MtlsListener`]: each
/// connection's requests carry [`ConnectInfo`]`<SocketAddr>` and, when the
/// client presented one, its [`PeerCertificate`].
#[must_use]
pub fn mtls_make_service(router: Router) -> MtlsMakeService {
    MtlsMakeService { router }
}

/// See [`mtls_make_service`].
#[derive(Clone, Debug)]
pub struct MtlsMakeService {
    router: Router,
}

impl tower_service::Service<IncomingStream<'_, MtlsListener>> for MtlsMakeService {
    type Response = MtlsConnection;
    type Error = Infallible;
    type Future = Ready<Result<MtlsConnection, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, stream: IncomingStream<'_, MtlsListener>) -> Self::Future {
        ready(Ok(MtlsConnection {
            router: self.router.clone(),
            remote_addr: *stream.remote_addr(),
            peer: peer_certificate(stream.io()),
        }))
    }
}

/// The service serving one mutual-TLS connection (see [`mtls_make_service`]).
#[derive(Clone, Debug)]
pub struct MtlsConnection {
    router: Router,
    remote_addr: SocketAddr,
    peer: Option<PeerCertificate>,
}

impl tower_service::Service<Request> for MtlsConnection {
    type Response = Response;
    type Error = Infallible;
    type Future = <Router as tower_service::Service<Request>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        tower_service::Service::<Request>::poll_ready(&mut self.router, cx)
    }

    fn call(&mut self, mut request: Request) -> Self::Future {
        let extensions = request.extensions_mut();
        extensions.insert(ConnectInfo(self.remote_addr));
        // Never trust an inbound extension of this type: overwrite or remove.
        match &self.peer {
            Some(peer) => {
                extensions.insert(peer.clone());
            }
            None => {
                extensions.remove::<PeerCertificate>();
            }
        }
        tower_service::Service::<Request>::call(&mut self.router, request)
    }
}
