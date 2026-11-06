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

// --- The registry endpoint, end to end over the real link-time inventory ------

use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::register_admin;
use seiva_identity::Principal;

// A real registration in THIS test binary's inventory (non-conflicting).
register_admin!(AdminResource {
    namespace: "shop",
    resource: "widgets",
    label: "Widgets",
    facade: "Shop",
    version: "v1",
    list_fields: &["id", "name"],
    search_fields: &["name"],
    filters: &[],
    ordering: &["name"],
    page_size: 25,
    permissions: AdminPermissions {
        view: "shop.read",
        create: "shop.create",
        change: "",
        delete: "",
    },
});

async fn registry_as(principal: Option<Principal>) -> serde_json::Value {
    let mut builder = Request::builder().uri("/admin/registry.json");
    if let Some(p) = principal {
        builder = builder.extension(p);
    }
    let res = admin_router()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn registry_lists_a_registered_resource_the_caller_may_view() {
    let body = registry_as(Some(Principal::new("clerk", ["shop.read"]))).await;
    let ids: Vec<&str> = body["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["resource"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"widgets"),
        "registered+authorized resource shows: {body}"
    );
}

#[tokio::test]
async fn registry_hides_the_resource_from_an_unauthorized_caller() {
    // A caller without `shop.read` must not see the resource, nor its existence.
    let body = registry_as(Some(Principal::new("stranger", ["other.read"]))).await;
    assert_eq!(body["resources"].as_array().unwrap().len(), 0, "{body}");
}

#[tokio::test]
async fn registry_is_empty_for_an_anonymous_caller() {
    let body = registry_as(None).await;
    assert_eq!(body["resources"].as_array().unwrap().len(), 0, "{body}");
}

// --- The shipped extension manifest matches the first-party contract ----------

#[test]
fn shipped_extension_manifest_matches_the_builtin_contract() {
    // The console ships a declarative `seiva-extension.toml` (ADR 0010) that the
    // independent DX reads; `seiva`'s built-in manifest must agree with it, or a
    // DX-driven install and a `seiva ext add` would wire the crate differently.
    let shipped = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/seiva-extension.toml"));
    let builtin = delonix_seiva::scaffold::CONSOLE_ADMIN_EXTENSION;
    assert!(shipped.contains(&format!("crate = \"{}\"", builtin.crate_name)));
    assert!(shipped.contains(&format!("version = \"{}\"", builtin.version_req)));
    assert!(shipped.contains(&format!("mount = \"{}\"", builtin.mount_stmt)));
}

// --- End-to-end: the console sees the Principal THROUGH the framework layers ---
//
// The security fix (ADR 0010): the console must mount into the pre-layer
// `modules` router so `router_with_modules` wraps /admin with authn — otherwise
// the Principal is never populated and the registry is always empty. This proves
// the composed, layered path, with a real authenticator.

use std::sync::Arc;

#[tokio::test]
async fn registry_sees_the_authenticated_principal_through_the_layers() {
    use seiva_http::{router_with_modules, Health, HttpSecurity, Principal, TokenAuthenticator};

    // The composition a real root uses: console mounted into `modules`, then layered.
    let modules = mount(axum::Router::new());
    let auth = TokenAuthenticator::new().with_token("good", Principal::new("clerk", ["shop.read"]));
    let app = router_with_modules(
        Health::starting(),
        &HttpSecurity::default(),
        Some(Arc::new(auth)),
        modules,
    );

    // With a valid bearer, authn populates the Principal → the registry lists the
    // resource the caller may view (proving the Principal reached the handler).
    let authed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/registry.json")
                .header("authorization", "Bearer good")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authed.status(), StatusCode::OK);
    let bytes = authed.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let ids: Vec<&str> = body["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["resource"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"widgets"),
        "the authenticated caller's authorized resource must appear: {body}"
    );

    // Without a token the caller is anonymous → the registry is empty (fail-closed).
    let anon = app
        .oneshot(
            Request::builder()
                .uri("/admin/registry.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = anon.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["resources"].as_array().unwrap().len(), 0);
}

// --- ADR 0016: the opt-in dxjs console -----------------------------------

async fn get_dx(path: &str) -> axum::http::Response<Body> {
    seiva_console_admin::admin_dx_router()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn dx_page_loads_an_external_same_origin_module() {
    let res = get_dx(seiva_console_admin::ADMIN_DX_PATH).await;
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    // External same-origin module + stylesheet, an #app mount point, no inline
    // script/style and no CDN.
    assert!(body.contains(r#"<script type="module" src="/admin/dx.js">"#));
    assert!(body.contains(r#"<link rel="stylesheet" href="/admin/dx.css" />"#));
    assert!(body.contains(r#"id="app""#));
    assert!(!body.contains("http://") && !body.contains("https://"));
}

#[tokio::test]
async fn dx_stylesheet_is_served_as_css_same_origin() {
    let res = get_dx(seiva_console_admin::ADMIN_DX_CSS_PATH).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let css = String::from_utf8(bytes.to_vec()).unwrap();
    // It is the vendored @dxjs/ui kit sheet (served same-origin, style-src 'self').
    assert!(
        css.contains(".dx-shell"),
        "the @dxjs/ui stylesheet is embedded"
    );
}

#[tokio::test]
async fn dx_csp_is_strict_self_no_unsafe() {
    let res = get_dx(seiva_console_admin::ADMIN_DX_PATH).await;
    let csp = res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("script-src 'self'"));
    assert!(csp.contains("connect-src 'self'"));
    assert!(!csp.contains("unsafe-inline"));
    assert!(!csp.contains("unsafe-eval"));
}

#[tokio::test]
async fn dx_bundle_is_served_as_javascript_and_is_csp_clean() {
    let res = get_dx(seiva_console_admin::ADMIN_DX_JS_PATH).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("javascript"));
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let js = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!js.is_empty(), "the vendored dxjs bundle is embedded");
    // dxjs is eval-free by construction (the whole point vs. the StillJS audit).
    assert!(!js.contains("eval("), "the dxjs bundle contains no eval(");
}

#[tokio::test]
async fn mount_dx_keeps_the_inline_console_and_adds_the_dx_route() {
    let app = mount(axum::Router::new());
    let app_dx = seiva_console_admin::mount_dx(axum::Router::new());
    // The inline page stays at /admin on both; the dx page exists only with mount_dx.
    let inline = app
        .oneshot(
            Request::builder()
                .uri(ADMIN_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(inline.status(), StatusCode::OK);
    let dx = app_dx
        .oneshot(
            Request::builder()
                .uri(seiva_console_admin::ADMIN_DX_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dx.status(), StatusCode::OK);
}
