//! `serve_router_mtls` end to end (feature `mtls`, ADR 0047): configuration
//! from `SEIVA_TLS_*` files, the Ready/Draining lifecycle, a real handshake.
#![cfg(feature = "mtls")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use seiva_http::{router, Health};
use seiva_server::mtls::{TLS_CERT_FILE_VAR, TLS_CLIENT_CA_FILE_VAR, TLS_KEY_FILE_VAR};
use seiva_server::{serve_router_mtls, MtlsFiles};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio_rustls::TlsConnector;

#[tokio::test(flavor = "multi_thread")]
async fn serves_health_over_mutual_tls_and_drains_on_shutdown() {
    // A CA, a server certificate for localhost and a client certificate.
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Test CA");
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let leaf = |cn: &str, sans: Vec<SanType>, purpose| {
        let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
        p.distinguished_name.push(DnType::CommonName, cn);
        p.subject_alt_names = sans;
        p.extended_key_usages = vec![purpose];
        let key = KeyPair::generate().unwrap();
        let cert = p.signed_by(&key, &ca, &ca_key).unwrap();
        (cert, key)
    };
    let (server_cert, server_key) = leaf(
        "localhost",
        vec![SanType::DnsName("localhost".try_into().unwrap())],
        ExtendedKeyUsagePurpose::ServerAuth,
    );
    let (client_cert, client_key) = leaf("agent-1", vec![], ExtendedKeyUsagePurpose::ClientAuth);

    // The files and the SEIVA_TLS_* configuration pointing at them.
    let dir = std::env::temp_dir().join(format!("seiva-mtls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: String| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path.to_string_lossy().into_owned()
    };
    let vars: HashMap<String, String> = [
        (TLS_CERT_FILE_VAR, write("server.pem", server_cert.pem())),
        (
            TLS_KEY_FILE_VAR,
            write("server.key", server_key.serialize_pem()),
        ),
        (TLS_CLIENT_CA_FILE_VAR, write("ca.pem", ca.pem())),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let mtls = MtlsFiles::from_map(vars).unwrap().unwrap().load().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    // Learn a free port, then serve on it.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let health = Health::starting();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn({
        let health = health.clone();
        async move {
            serve_router_mtls(addr, router(health.clone()), health, &mtls, async {
                let _ = stopped.await;
            })
            .await
        }
    });

    // A client presenting its certificate.
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(ca.der().to_vec())).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_root_certificates(roots)
    .with_client_auth_cert(
        vec![CertificateDer::from(client_cert.der().to_vec())],
        PrivateKeyDer::try_from(client_key.serialize_der()).unwrap(),
    )
    .unwrap();
    let connector = TlsConnector::from(Arc::new(config));

    let mut response = String::new();
    for _ in 0..50 {
        if let Ok(tcp) = tokio::net::TcpStream::connect(addr).await {
            let mut tls = connector
                .connect(ServerName::try_from("localhost").unwrap(), tcp)
                .await
                .unwrap();
            tls.write_all(
                b"GET /health/ready HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            tls.read_to_string(&mut response).await.unwrap();
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(response.starts_with("HTTP/1.1 200"), "got {response:?}");

    // Shutdown: health goes Draining and the server returns cleanly.
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the server did not stop")
        .unwrap()
        .unwrap();
    assert!(!health.is_ready());
}
