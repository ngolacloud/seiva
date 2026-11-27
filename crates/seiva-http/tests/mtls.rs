//! Mutual TLS end to end (feature `mtls`, ADR 0047): a real TCP listener, real
//! handshakes, certificates minted per test run by `rcgen`.
#![cfg(feature = "mtls")]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use axum::{Extension, Router};
use rcgen::{
    date_time_ymd, BasicConstraints, Certificate, CertificateParams,
    CertificateRevocationListParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyIdMethod, KeyPair,
    KeyUsagePurpose, RevocationReason, RevokedCertParams, SanType, SerialNumber,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use seiva_http::mtls::{
    mtls_make_service, ClientAuth, ClientCertificate, MtlsConfig, MtlsError, MtlsListener,
};
use seiva_http::{
    permission_guard, router_with_modules, AuthConfig, Health, HttpSecurity, RequiredPermission,
};
use seiva_identity::{AuthError, Authenticator, Principal, TokenAuthenticator};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

// --- A throwaway PKI ---------------------------------------------------------

struct Ca {
    cert: Certificate,
    key: KeyPair,
}

impl Ca {
    fn new(name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, name);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let key = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        Self { cert, key }
    }

    fn issue(&self, cn: &str, sans: Vec<SanType>, serial: u8, client: bool) -> Leaf {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, cn);
        params.subject_alt_names = sans;
        params.serial_number = Some(SerialNumber::from(vec![serial]));
        params.extended_key_usages = vec![if client {
            ExtendedKeyUsagePurpose::ClientAuth
        } else {
            ExtendedKeyUsagePurpose::ServerAuth
        }];
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &self.cert, &self.key).unwrap();
        Leaf { cert, key }
    }

    fn crl_revoking(&self, serial: u8) -> String {
        CertificateRevocationListParams {
            this_update: date_time_ymd(2024, 1, 1),
            next_update: date_time_ymd(2099, 1, 1),
            crl_number: SerialNumber::from(1u64),
            issuing_distribution_point: None,
            revoked_certs: vec![RevokedCertParams {
                serial_number: SerialNumber::from(vec![serial]),
                revocation_time: date_time_ymd(2024, 1, 1),
                reason_code: Some(RevocationReason::KeyCompromise),
                invalidity_date: None,
            }],
            key_identifier_method: KeyIdMethod::Sha256,
        }
        .signed_by(&self.cert, &self.key)
        .unwrap()
        .pem()
        .unwrap()
    }
}

struct Leaf {
    cert: Certificate,
    key: KeyPair,
}

fn dns(name: &str) -> SanType {
    SanType::DnsName(name.try_into().unwrap())
}

fn uri(value: &str) -> SanType {
    SanType::URI(value.try_into().unwrap())
}

fn device(ca: &Ca, cn: &str, serial: u8) -> Leaf {
    ca.issue(
        cn,
        vec![uri(&format!("urn:sentinel:device:{cn}"))],
        serial,
        true,
    )
}

// --- The server under test ----------------------------------------------------

/// Maps a device certificate to a principal; `revoked-*` CNs are refused, as an
/// application store would after a revocation.
fn device_authenticator(cert: &ClientCertificate) -> Result<Principal, AuthError> {
    let cn = cert.subject_common_name().ok_or(AuthError::InvalidToken)?;
    if cn.starts_with("revoked-") {
        return Err(AuthError::InvalidToken);
    }
    Ok(Principal::new(format!("device:{cn}"), ["devices.report"]))
}

async fn whoami(
    Extension(principal): Extension<Arc<Principal>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> String {
    format!("{}|{}", principal.subject, peer.ip())
}

async fn report() -> &'static str {
    "reported"
}

fn app() -> Router {
    let module = Router::new().route("/v1/whoami", get(whoami)).merge(
        Router::new()
            .route("/v1/report", get(report))
            .layer(from_fn_with_state(
                RequiredPermission("devices.report"),
                permission_guard,
            )),
    );
    let bearer: Arc<dyn Authenticator> = Arc::new(
        TokenAuthenticator::new()
            .with_token("ui-token", Principal::new("user:ana", ["devices.read"])),
    );
    let auth = AuthConfig::bearer(bearer).with_client_certificates(Arc::new(device_authenticator));
    router_with_modules(Health::starting(), &HttpSecurity::default(), auth, module)
}

struct Server {
    ca: Ca,
    addr: SocketAddr,
}

async fn start(client_auth: ClientAuth, crl: Option<String>) -> Server {
    start_with_ca(Ca::new("Sentinel Test CA"), client_auth, crl).await
}

async fn start_with_ca(ca: Ca, client_auth: ClientAuth, crl: Option<String>) -> Server {
    let server = ca.issue("localhost", vec![dns("localhost")], 1, false);
    let mut builder = MtlsConfig::builder()
        .server_identity_pem(
            server.cert.pem().as_bytes(),
            server.key.serialize_pem().as_bytes(),
        )
        .client_ca_pem(ca.cert.pem().as_bytes())
        .client_auth(client_auth);
    if let Some(crl) = crl {
        builder = builder.crl_pem(crl.as_bytes());
    }
    let config = builder.build().unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener = MtlsListener::from_tcp(tcp, &config).unwrap();
    let addr = axum::serve::Listener::local_addr(&listener).unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mtls_make_service(app()))
            .await
            .unwrap();
    });
    Server { ca, addr }
}

// --- The client ---------------------------------------------------------------

struct Client<'a> {
    trust: &'a Ca,
    identity: Option<&'a Leaf>,
    tls12_only: bool,
}

impl<'a> Client<'a> {
    fn new(trust: &'a Ca) -> Self {
        Self {
            trust,
            identity: None,
            tls12_only: false,
        }
    }

    fn with_identity(mut self, leaf: &'a Leaf) -> Self {
        self.identity = Some(leaf);
        self
    }

    /// A fresh mTLS connection offering `alpn` (empty: no ALPN extension).
    async fn connect(
        &self,
        addr: SocketAddr,
        alpn: &[&[u8]],
    ) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let versions: &[&rustls::SupportedProtocolVersion] = if self.tls12_only {
            &[&rustls::version::TLS12]
        } else {
            &[&rustls::version::TLS13]
        };
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(self.trust.cert.der().to_vec()))
            .unwrap();
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .with_root_certificates(roots);
        let mut config = match self.identity {
            Some(leaf) => builder
                .with_client_auth_cert(
                    vec![CertificateDer::from(leaf.cert.der().to_vec())],
                    PrivateKeyDer::try_from(leaf.key.serialize_der()).unwrap(),
                )
                .unwrap(),
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        let connector = TlsConnector::from(Arc::new(config));
        let tcp = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        connector
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .map_err(|e| e.to_string())
    }

    /// `GET path` over a fresh mTLS connection; `Err` when the handshake or
    /// the exchange is refused. Returns the status code and the body.
    async fn get(
        &self,
        addr: SocketAddr,
        path: &str,
        headers: &str,
    ) -> Result<(u16, String), String> {
        let mut tls = self.connect(addr, &[]).await?;
        let request =
            format!("GET {path} HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n{headers}\r\n");
        tls.write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        tls.read_to_end(&mut raw).await.map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("no HTTP response: {text:?}"))?;
        let body = text.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
        Ok((status, body))
    }

    /// `GET path` over HTTP/2, offering ALPN `h2` then `http/1.1` as reqwest
    /// and other HTTP/2-capable clients do. Returns the negotiated ALPN
    /// protocol, the status code and the body.
    async fn get_h2(&self, addr: SocketAddr, path: &str) -> Result<(Vec<u8>, u16, String), String> {
        let tls = self.connect(addr, &[b"h2", b"http/1.1"]).await?;
        let alpn = tls.get_ref().1.alpn_protocol().unwrap_or_default().to_vec();
        let (mut sender, connection) = hyper::client::conn::http2::handshake(
            hyper_util::rt::TokioExecutor::new(),
            hyper_util::rt::TokioIo::new(tls),
        )
        .await
        .map_err(|e| e.to_string())?;
        tokio::spawn(connection);
        let request = axum::http::Request::get(format!("https://localhost{path}"))
            .body(http_body_util::Empty::<axum::body::Bytes>::new())
            .unwrap();
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map_err(|e| e.to_string())?
            .to_bytes();
        Ok((alpn, status, String::from_utf8_lossy(&body).into_owned()))
    }
}

// --- Required client certificates ---------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_device_certificate_becomes_the_principal() {
    let server = start(ClientAuth::Required, None).await;
    let leaf = device(&server.ca, "pc-0042", 7);
    let client = Client::new(&server.ca).with_identity(&leaf);

    let (status, body) = client.get(server.addr, "/v1/whoami", "").await.unwrap();
    assert_eq!(status, 200);
    // The subject comes from the certificate; ConnectInfo still carries the peer IP.
    assert_eq!(body, "device:pc-0042|127.0.0.1");

    // The permission the authenticator granted passes the facade guard.
    let (status, body) = client.get(server.addr, "/v1/report", "").await.unwrap();
    assert_eq!((status, body.as_str()), (200, "reported"));
}

/// The listener advertises `h2` by ALPN, so it must also speak HTTP/2: a client
/// that negotiates `h2` (reqwest does whenever it can) is served, and each
/// stream still carries the certificate principal and the peer address.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_negotiates_h2_is_served_over_http2() {
    let server = start(ClientAuth::Required, None).await;
    let leaf = device(&server.ca, "pc-0042", 7);
    let client = Client::new(&server.ca).with_identity(&leaf);

    let (alpn, status, body) = client.get_h2(server.addr, "/v1/whoami").await.unwrap();
    assert_eq!(alpn, b"h2");
    assert_eq!((status, body.as_str()), (200, "device:pc-0042|127.0.0.1"));

    let (_, status, body) = client.get_h2(server.addr, "/v1/report").await.unwrap();
    assert_eq!((status, body.as_str()), (200, "reported"));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_certificate_is_refused_in_the_handshake() {
    let server = start(ClientAuth::Required, None).await;
    let outcome = Client::new(&server.ca)
        .get(server.addr, "/v1/whoami", "")
        .await;
    assert!(
        outcome.is_err(),
        "a client without a certificate got {outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_from_another_ca_is_refused() {
    let server = start(ClientAuth::Required, None).await;
    let rogue = Ca::new("Some Other CA");
    let leaf = device(&rogue, "pc-0042", 7);
    let outcome = Client::new(&server.ca)
        .with_identity(&leaf)
        .get(server.addr, "/v1/whoami", "")
        .await;
    assert!(outcome.is_err(), "a foreign certificate got {outcome:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_on_the_crl_is_refused() {
    let ca = Ca::new("Sentinel Test CA");
    let crl = ca.crl_revoking(9);
    let server = start_with_ca(ca, ClientAuth::Required, Some(crl)).await;

    let revoked = device(&server.ca, "pc-0009", 9);
    let outcome = Client::new(&server.ca)
        .with_identity(&revoked)
        .get(server.addr, "/v1/whoami", "")
        .await;
    assert!(outcome.is_err(), "a revoked certificate got {outcome:?}");

    // A sibling the CRL does not list still connects.
    let good = device(&server.ca, "pc-0010", 10);
    let (status, _) = Client::new(&server.ca)
        .with_identity(&good)
        .get(server.addr, "/v1/whoami", "")
        .await
        .unwrap();
    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_application_can_refuse_a_valid_certificate_per_request() {
    // Revocation in the application's store applies on the next request: the
    // handshake passes (the CA signed it), the authenticator says no → 401.
    let server = start(ClientAuth::Required, None).await;
    let leaf = device(&server.ca, "revoked-pc-0003", 3);
    let (status, _) = Client::new(&server.ca)
        .with_identity(&leaf)
        .get(server.addr, "/v1/whoami", "")
        .await
        .unwrap();
    assert_eq!(status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_plus_a_bearer_is_ambiguous_and_refused() {
    let server = start(ClientAuth::Required, None).await;
    let leaf = device(&server.ca, "pc-0042", 7);
    let client = Client::new(&server.ca).with_identity(&leaf);
    let (status, _) = client
        .get(
            server.addr,
            "/v1/whoami",
            "authorization: Bearer ui-token\r\n",
        )
        .await
        .unwrap();
    assert_eq!(status, 401);
    let (status, _) = client
        .get(server.addr, "/v1/whoami", "x-api-key: anything\r\n")
        .await
        .unwrap();
    assert_eq!(status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_1_2_is_refused() {
    let server = start(ClientAuth::Required, None).await;
    let leaf = device(&server.ca, "pc-0042", 7);
    let mut client = Client::new(&server.ca).with_identity(&leaf);
    client.tls12_only = true;
    let outcome = client.get(server.addr, "/v1/whoami", "").await;
    assert!(outcome.is_err(), "a TLS 1.2 client got {outcome:?}");
}

// --- Optional client certificates ---------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn optional_mode_serves_people_by_bearer_and_machines_by_certificate() {
    let server = start(ClientAuth::Optional, None).await;
    let people = Client::new(&server.ca);

    // No certificate, no credential: anonymous — public route serves, guard denies.
    let (status, body) = people.get(server.addr, "/v1/whoami", "").await.unwrap();
    assert_eq!((status, body.as_str()), (200, "|127.0.0.1"));
    let (status, _) = people.get(server.addr, "/v1/report", "").await.unwrap();
    assert_eq!(status, 403);

    // No certificate, bearer: the bearer path, as before.
    let (status, body) = people
        .get(
            server.addr,
            "/v1/whoami",
            "authorization: Bearer ui-token\r\n",
        )
        .await
        .unwrap();
    assert_eq!((status, body.as_str()), (200, "user:ana|127.0.0.1"));

    // A certificate: the device.
    let leaf = device(&server.ca, "pc-0042", 7);
    let (status, body) = Client::new(&server.ca)
        .with_identity(&leaf)
        .get(server.addr, "/v1/whoami", "")
        .await
        .unwrap();
    assert_eq!((status, body.as_str()), (200, "device:pc-0042|127.0.0.1"));

    // Optional still verifies what IS presented: a foreign certificate fails.
    let rogue = Ca::new("Some Other CA");
    let foreign = device(&rogue, "pc-0042", 7);
    let outcome = Client::new(&server.ca)
        .with_identity(&foreign)
        .get(server.addr, "/v1/whoami", "")
        .await;
    assert!(outcome.is_err(), "a foreign certificate got {outcome:?}");
}

// --- Configuration and parsing --------------------------------------------------

#[test]
fn the_config_refuses_missing_or_empty_inputs() {
    let ca = Ca::new("Sentinel Test CA");
    let server = ca.issue("localhost", vec![dns("localhost")], 1, false);
    let (cert, key) = (server.cert.pem(), server.key.serialize_pem());

    let missing_ca = MtlsConfig::builder()
        .server_identity_pem(cert.as_bytes(), key.as_bytes())
        .build();
    assert!(matches!(missing_ca, Err(MtlsError::MissingClientCa)));

    let missing_identity = MtlsConfig::builder()
        .client_ca_pem(ca.cert.pem().as_bytes())
        .build();
    assert!(matches!(
        missing_identity,
        Err(MtlsError::MissingServerIdentity)
    ));

    let empty_ca = MtlsConfig::builder()
        .server_identity_pem(cert.as_bytes(), key.as_bytes())
        .client_ca_pem(b"not a certificate")
        .build();
    assert!(matches!(empty_ca, Err(MtlsError::Empty(_))));

    // A key that does not belong to the certificate.
    let other_key = KeyPair::generate().unwrap().serialize_pem();
    let mismatched = MtlsConfig::builder()
        .server_identity_pem(cert.as_bytes(), other_key.as_bytes())
        .client_ca_pem(ca.cert.pem().as_bytes())
        .build();
    assert!(mismatched.is_err());

    let ok = MtlsConfig::builder()
        .server_identity_pem(cert.as_bytes(), key.as_bytes())
        .client_ca_pem(ca.cert.pem().as_bytes())
        .build()
        .unwrap();
    assert_eq!(ok.client_auth(), ClientAuth::Required);
    // Debug never prints key material.
    assert!(!format!("{ok:?}").contains("PRIVATE"));
}

#[test]
fn a_client_certificate_exposes_its_identity_fields() {
    let ca = Ca::new("Sentinel Test CA");
    let leaf = ca.issue(
        "pc-0042",
        vec![uri("urn:sentinel:device:pc-0042"), dns("pc-0042.lan")],
        0x2a,
        true,
    );
    let der = CertificateDer::from(leaf.cert.der().to_vec());
    let cert = ClientCertificate::from_der(der.clone()).unwrap();
    assert_eq!(cert.subject_common_name(), Some("pc-0042"));
    assert_eq!(cert.uris(), ["urn:sentinel:device:pc-0042"]);
    assert_eq!(cert.dns_names(), ["pc-0042.lan"]);
    assert_eq!(cert.serial_hex(), "2a");
    assert_eq!(cert.fingerprint_sha256().len(), 64);
    assert!(cert.not_after() > 0);
    assert_eq!(cert.der(), &der);

    assert!(ClientCertificate::from_der(CertificateDer::from(vec![1, 2, 3])).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_handshake_neither_blocks_others_nor_lives_forever() {
    let ca = Ca::new("Sentinel Test CA");
    let server_leaf = ca.issue("localhost", vec![dns("localhost")], 1, false);
    let config = MtlsConfig::builder()
        .server_identity_pem(
            server_leaf.cert.pem().as_bytes(),
            server_leaf.key.serialize_pem().as_bytes(),
        )
        .client_ca_pem(ca.cert.pem().as_bytes())
        .handshake_timeout(std::time::Duration::from_millis(300))
        .build()
        .unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener = MtlsListener::from_tcp(tcp, &config).unwrap();
    let addr = axum::serve::Listener::local_addr(&listener).unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mtls_make_service(app()))
            .await
            .unwrap();
    });

    // A client that opens TCP and never speaks TLS.
    let mut stalled = TcpStream::connect(addr).await.unwrap();

    // Others are still served meanwhile.
    let leaf = device(&ca, "pc-0042", 7);
    let (status, _) = Client::new(&ca)
        .with_identity(&leaf)
        .get(addr, "/v1/whoami", "")
        .await
        .unwrap();
    assert_eq!(status, 200);

    // And the stalled one is dropped once the handshake timeout passes.
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(std::time::Duration::from_secs(5), stalled.read(&mut buf))
        .await
        .expect("the stalled connection was never closed");
    assert!(matches!(read, Ok(0) | Err(_)));
}
