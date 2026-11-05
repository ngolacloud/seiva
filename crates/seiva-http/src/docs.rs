//! The interactive API-reference pages — Seiva's FastAPI `/docs` parity.
//!
//! FastAPI serves **Swagger UI** at `/docs` and **ReDoc** at `/redoc` out of the
//! box, both rendering the service's own OpenAPI document. Seiva matches that and
//! adds a third, modern renderer:
//!
//! | route      | renderer   | FastAPI analogue |
//! |------------|------------|------------------|
//! | `/docs`    | Swagger UI | `/docs`          |
//! | `/redoc`   | ReDoc      | `/redoc`         |
//! | `/scalar`  | Scalar     | — (bonus)        |
//!
//! Every page points at the **relative** `/openapi.json`, so it works behind any
//! host or path prefix, and the document itself is generated from the facades
//! linked into the binary (no hand-written spec, no per-handler annotations — the
//! `async fn` contract *is* the source). The viewer bundles load from a CDN, as
//! FastAPI's Swagger/ReDoc assets do; these are developer-facing documentation
//! pages, not data-plane responses, so the CDN dependency is acceptable here.
//!
//! Because the framework's default response CSP is `default-src 'none'` (see
//! [`security_headers`](crate::security_headers)), these pages would otherwise
//! load nothing — the CDN script and the inline bootstrap would both be blocked
//! and the page would render blank. So each handler sets its **own**, narrower
//! CSP ([`DOCS_CSP`]) that permits exactly the CDN and the same-origin spec fetch;
//! `security_headers` only fills in a header that is *absent*, so this scoped
//! policy wins for the docs pages while every API/JSON response keeps the strict
//! default.

use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::response::{Html, IntoResponse, Response};

/// `GET /docs` — **Swagger UI** over the service's `/openapi.json`. This is the
/// FastAPI-identical route and renderer: the same page a FastAPI developer
/// reaches at `/docs`, so the muscle memory transfers.
pub const SWAGGER_HTML: &str = r##"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Delonix Seiva — API reference (Swagger UI)</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist/swagger-ui.css" />
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist/swagger-ui-bundle.js"></script>
    <script>
      window.onload = function () {
        window.ui = SwaggerUIBundle({
          url: "/openapi.json",
          dom_id: "#swagger-ui",
          deepLinking: true,
          tryItOutEnabled: true,
        });
      };
    </script>
  </body>
</html>
"##;

/// Back-compat alias: the `/docs` page HTML. Historically this served Scalar;
/// it now serves Swagger UI for FastAPI parity. Kept so downstream `pub use`
/// sites keep compiling.
pub const DOCS_HTML: &str = SWAGGER_HTML;

/// `GET /redoc` — **ReDoc** over the service's `/openapi.json`. FastAPI's second
/// built-in renderer; a three-pane reference that reads well for large APIs.
pub const REDOC_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Delonix Seiva — API reference (ReDoc)</title>
  </head>
  <body>
    <redoc spec-url="/openapi.json"></redoc>
    <script src="https://cdn.jsdelivr.net/npm/redoc/bundles/redoc.standalone.js"></script>
  </body>
</html>
"#;

/// `GET /scalar` — **Scalar**, a single-page modern API reference. Kept as a
/// bonus renderer alongside the FastAPI-parity Swagger UI and ReDoc pages.
pub const SCALAR_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Delonix Seiva — API reference (Scalar)</title>
  </head>
  <body>
    <script id="api-reference" data-url="/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>
"#;

/// The Content-Security-Policy for the documentation pages — narrower than a
/// wildcard, wider than the API's `default-src 'none'`. It permits only what the
/// viewers need: their scripts and styles from jsDelivr (plus the inline
/// bootstrap), data/CDN images and fonts, a same-origin fetch of `/openapi.json`,
/// and `blob:` web workers (ReDoc renders in a worker). Everything else stays
/// denied, so the documentation pages are no softer than they must be.
pub const DOCS_CSP: &str = "default-src 'none'; \
    script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
    style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
    img-src 'self' data: https://cdn.jsdelivr.net; \
    font-src 'self' data: https://cdn.jsdelivr.net; \
    connect-src 'self' https://cdn.jsdelivr.net; \
    worker-src 'self' blob:; \
    child-src 'self' blob:; \
    frame-ancestors 'none'";

/// `GET /docs` — serves [`SWAGGER_HTML`] under the scoped [`DOCS_CSP`].
pub async fn docs_handler() -> Response {
    ([(CONTENT_SECURITY_POLICY, DOCS_CSP)], Html(SWAGGER_HTML)).into_response()
}

/// `GET /redoc` — serves [`REDOC_HTML`] under the scoped [`DOCS_CSP`].
pub async fn redoc_handler() -> Response {
    ([(CONTENT_SECURITY_POLICY, DOCS_CSP)], Html(REDOC_HTML)).into_response()
}

/// `GET /scalar` — serves [`SCALAR_HTML`] under the scoped [`DOCS_CSP`].
pub async fn scalar_handler() -> Response {
    ([(CONTENT_SECURITY_POLICY, DOCS_CSP)], Html(SCALAR_HTML)).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{router, Health};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    async fn fetch(uri: &str) -> (StatusCode, String, String) {
        let app = router(Health::starting());
        let res = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let ctype = res
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let csp = res
            .headers()
            .get("content-security-policy")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(status, StatusCode::OK, "{uri} should be 200");
        assert!(ctype.starts_with("text/html"), "{uri} should be html");
        // Each page renders the service's OWN spec.
        assert!(body.contains("/openapi.json"), "{uri} must point at the spec");
        (status, body, csp)
    }

    #[tokio::test]
    async fn docs_serves_swagger_ui() {
        let (_s, body, _csp) = fetch("/docs").await;
        assert!(
            body.contains("swagger-ui") && body.contains("SwaggerUIBundle"),
            "/docs must render Swagger UI (FastAPI parity)"
        );
    }

    #[tokio::test]
    async fn redoc_serves_redoc() {
        let (_s, body, _csp) = fetch("/redoc").await;
        assert!(body.contains("redoc"), "/redoc must render ReDoc");
    }

    #[tokio::test]
    async fn scalar_serves_scalar() {
        let (_s, body, _csp) = fetch("/scalar").await;
        assert!(
            body.contains("@scalar/api-reference"),
            "/scalar must render Scalar"
        );
    }

    #[tokio::test]
    async fn docs_pages_carry_their_own_csp_not_the_blank_default() {
        for uri in ["/docs", "/redoc", "/scalar"] {
            let (_s, _body, csp) = fetch(uri).await;
            assert!(
                csp.contains("https://cdn.jsdelivr.net"),
                "the {uri} CSP must allow the viewer CDN, got: {csp}"
            );
            assert_ne!(
                csp, "default-src 'none'; frame-ancestors 'none'",
                "the {uri} page must not inherit the API's blank-page CSP"
            );
        }
    }
}
