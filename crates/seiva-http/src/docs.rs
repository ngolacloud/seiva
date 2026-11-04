//! An interactive API reference page, the FastAPI `/docs` parity piece.
//!
//! FastAPI serves Swagger UI at `/docs` out of the box; Seiva served only the
//! raw `/openapi.json`. [`docs_handler`] closes that gap by serving a small HTML
//! page that renders the service's own `/openapi.json` with
//! [Scalar](https://github.com/scalar/scalar) — a single modern API reference UI.
//!
//! The page points at the **relative** `/openapi.json`, so it works behind any
//! host or path prefix. The Scalar bundle itself loads from a CDN (as FastAPI's
//! Swagger assets do); this is a developer-facing documentation page, not a
//! data-plane response, so the CDN dependency is acceptable here.
//!
//! Because the framework's default response CSP is `default-src 'none'` (see
//! [`security_headers`](crate::security_headers)), this page would otherwise load
//! nothing — the CDN script and the inline bootstrap would both be blocked and
//! the page would render blank. So [`docs_handler`] sets its **own**, narrower
//! CSP ([`DOCS_CSP`]) that permits exactly Scalar's CDN and the same-origin spec
//! fetch; `security_headers` only fills in a header that is *absent*, so this
//! scoped policy wins for `/docs` while every API/JSON response keeps the strict
//! default.

use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::response::{Html, IntoResponse, Response};

/// The self-contained HTML for the API reference page. It loads the service's
/// `/openapi.json` and renders it with Scalar.
pub const DOCS_HTML: &str = r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Delonix Seiva — API reference</title>
  </head>
  <body>
    <script id="api-reference" data-url="/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>
"#;

/// The Content-Security-Policy for the `/docs` page specifically — narrower than
/// a wildcard, wider than the API's `default-src 'none'`. It permits only what
/// Scalar needs: its script from jsDelivr (plus the inline bootstrap tag), the
/// styles it injects, and a same-origin fetch of `/openapi.json`. Everything
/// else stays denied, so the documentation page is no softer than it must be.
pub const DOCS_CSP: &str = "default-src 'none'; \
    script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
    style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
    img-src 'self' data: https://cdn.jsdelivr.net; \
    font-src 'self' data: https://cdn.jsdelivr.net; \
    connect-src 'self' https://cdn.jsdelivr.net; \
    frame-ancestors 'none'";

/// `GET /docs` — serves the interactive [`DOCS_HTML`] API reference page under
/// the scoped [`DOCS_CSP`] (so the strict default CSP does not blank it out).
pub async fn docs_handler() -> Response {
    ([(CONTENT_SECURITY_POLICY, DOCS_CSP)], Html(DOCS_HTML)).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{router, Health};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    #[tokio::test]
    async fn docs_page_is_served_and_points_at_the_spec() {
        let app = router(Health::starting());
        let res = app
            .oneshot(Request::builder().uri("/docs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        // It renders the service's OWN spec with Scalar.
        assert!(body.contains("/openapi.json"));
        assert!(body.contains("@scalar/api-reference"));
    }

    #[tokio::test]
    async fn docs_csp_permits_scalar_instead_of_the_strict_default() {
        // Served through the full router, so `security_headers` has run. The page
        // must carry its OWN CSP (allowing Scalar's CDN), not the API's
        // `default-src 'none'` that would block the script and leave it blank.
        let app = router(Health::starting());
        let res = app
            .oneshot(Request::builder().uri("/docs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("https://cdn.jsdelivr.net"),
            "the /docs CSP must allow Scalar's CDN, got: {csp}"
        );
        assert_ne!(
            csp, "default-src 'none'; frame-ancestors 'none'",
            "the /docs page must not inherit the API's blank-page CSP"
        );
    }
}
