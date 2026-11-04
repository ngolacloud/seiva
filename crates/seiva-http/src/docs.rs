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
//! data-plane response, so the CDN dependency is acceptable here while the
//! strict `default-src 'none'` CSP still protects the API's JSON responses.

use axum::response::Html;

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

/// `GET /docs` — serves the interactive [`DOCS_HTML`] API reference page.
pub async fn docs_handler() -> Html<&'static str> {
    Html(DOCS_HTML)
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
}
