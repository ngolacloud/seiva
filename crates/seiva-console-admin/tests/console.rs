//! The console serves itself, self-contained, under a strict hash-based CSP —
//! and nothing else. Migrated from the tests that lived in `seiva-http`'s
//! `admin` module before the extraction (ADR 0008), plus the hardening checks.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use seiva_console_admin::{admin_csp, admin_router, mount, ADMIN_PATH};

async fn get(path: &str) -> axum::http::Response<Body> {
    admin_router()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn admin_page_is_served_self_contained_from_the_contract() {
    let res = get(ADMIN_PATH).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    // It builds itself from the service's OWN live contract…
    assert!(body.contains("/openapi.json"));
    assert!(body.contains("Seiva Admin"));
    // …and is self-contained: no external origin, no CDN.
    assert!(!body.contains("http://") && !body.contains("https://"));
}

#[tokio::test]
async fn admin_csp_is_strict_hash_based_no_unsafe_inline() {
    let res = get(ADMIN_PATH).await;
    let csp = res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    // Same-origin only, no remote host anywhere.
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("connect-src 'self'"));
    assert!(!csp.contains("https://"), "admin must not allow any CDN");
    // The hardening: inline code is pinned by hash, never allowed wholesale.
    assert!(
        !csp.contains("unsafe-inline"),
        "the console must not fall back to 'unsafe-inline': {csp}"
    );
    assert!(
        csp.contains("script-src 'sha256-") && csp.contains("style-src 'sha256-"),
        "script/style must be pinned by SHA-256 hash: {csp}"
    );
    // Navigation/embedding are locked down too.
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("base-uri 'none'"));
    assert!(csp.contains("form-action 'none'"));
}

#[tokio::test]
async fn csp_hash_matches_the_served_bytes() {
    // The browser recomputes the hash over the exact inline block content and
    // refuses to run it on a mismatch. Prove the policy pins what we serve: the
    // page must contain the <style>/<script> whose hashes the CSP lists.
    let res = get(ADMIN_PATH).await;
    let csp = admin_csp().to_owned();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();

    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use sha2::{Digest, Sha256};
    // Pull the inline block bodies back out of the served HTML…
    let css = between(&body, "<style>", "</style>");
    let js = between(&body, "<script>", "</script>");
    let css_hash = B64.encode(Sha256::digest(css.as_bytes()));
    let js_hash = B64.encode(Sha256::digest(js.as_bytes()));
    // …and show each hash is the one the CSP advertises.
    assert!(
        csp.contains(&format!("'sha256-{css_hash}'")),
        "style hash in CSP must match the served <style>"
    );
    assert!(
        csp.contains(&format!("'sha256-{js_hash}'")),
        "script hash in CSP must match the served <script>"
    );
}

#[tokio::test]
async fn mount_adds_only_the_admin_route() {
    // `mount` opts a composition root in; it adds GET /admin and nothing else.
    let app = mount(axum::Router::new());
    let ok = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(ADMIN_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let other = app
        .oneshot(
            Request::builder()
                .uri("/not-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::NOT_FOUND);
}

/// The substring strictly between the first `open` and the following `close`.
fn between<'a>(haystack: &'a str, open: &str, close: &str) -> &'a str {
    let start = haystack.find(open).expect("open tag present") + open.len();
    let end = start + haystack[start..].find(close).expect("close tag present");
    &haystack[start..end]
}
