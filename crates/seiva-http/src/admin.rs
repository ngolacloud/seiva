//! A generated admin console, the Django-admin parity piece (roadmap P1-5).
//!
//! Django's signature feature is an auto-generated admin over your models. Seiva
//! has no reflection, but it *does* have the project's full contract at runtime:
//! `GET /openapi.json` already lists every mounted module's operations and — via
//! `WireSchema` (ADR 0007) — the real request/response shapes. [`admin_handler`]
//! turns that into a **self-contained** console: it groups the operations by
//! resource, renders a form for each write operation from the request DTO's
//! schema, and lets you exercise the real (auth-guarded) routes with a bearer
//! token. No per-module code: add a module (`seiva module new`), and it appears.
//!
//! Scope of this first cut: the console drives the module **operations** the
//! contract declares (create/get/list/…). Generic *data* management — listing
//! and editing rows of a model — arrives with the data-layer admin (it needs the
//! DORM model registry) and is a separate increment.
//!
//! Unlike [`docs`](crate::docs) (the Scalar API reference, which loads from a
//! CDN), this page is fully self-contained — inline HTML/CSS/JS, no external
//! fetch beyond the same-origin `/openapi.json` and the API calls it makes — so
//! its CSP forbids every remote origin.

use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::response::{Html, IntoResponse, Response};

/// The Content-Security-Policy for `/admin`: everything same-origin, inline
/// script/style allowed for the self-contained console, no remote origin at all
/// (narrower than [`DOCS_CSP`](crate::docs::DOCS_CSP), which must allow a CDN).
pub const ADMIN_CSP: &str = "default-src 'none'; \
    script-src 'unsafe-inline'; \
    style-src 'unsafe-inline'; \
    connect-src 'self'; \
    img-src 'self' data:; \
    frame-ancestors 'none'";

/// The self-contained admin console. Pure HTML/CSS/JS: it fetches the service's
/// own `/openapi.json`, groups operations by resource, and renders a try-form per
/// write operation built from the request schema, with a shared bearer token.
pub const ADMIN_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Delonix Seiva — Admin</title>
<style>
  :root { color-scheme: light dark; --b: #8883; --fg: #222; --mut: #666; }
  @media (prefers-color-scheme: dark) { :root { --fg: #e8e8e8; --mut: #9aa; } }
  * { box-sizing: border-box; }
  body { font: 15px/1.5 system-ui, sans-serif; margin: 0; color: var(--fg); }
  header { padding: 16px 24px; border-bottom: 1px solid var(--b); display: flex; gap: 16px; align-items: center; flex-wrap: wrap; }
  header h1 { font-size: 18px; margin: 0; }
  header .tok { flex: 1; min-width: 240px; }
  input, textarea, button { font: inherit; padding: 6px 8px; border: 1px solid var(--b); border-radius: 6px; background: transparent; color: inherit; }
  input, textarea { width: 100%; }
  main { padding: 24px; max-width: 920px; margin: 0 auto; }
  .res { border: 1px solid var(--b); border-radius: 10px; margin: 0 0 20px; overflow: hidden; }
  .res > h2 { margin: 0; padding: 12px 16px; font-size: 16px; background: var(--b); }
  .op { padding: 14px 16px; border-top: 1px solid var(--b); }
  .op .sig { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .m { font-weight: 700; font-size: 12px; padding: 2px 8px; border-radius: 5px; border: 1px solid var(--b); }
  .path { font-family: ui-monospace, monospace; }
  .req { font-size: 12px; color: var(--mut); }
  .fields { margin: 10px 0; display: grid; gap: 8px; }
  .fields label { font-size: 13px; color: var(--mut); display: block; }
  .out { font-family: ui-monospace, monospace; white-space: pre-wrap; font-size: 13px; margin-top: 8px; padding: 8px; border: 1px dashed var(--b); border-radius: 6px; display: none; }
  .muted { color: var(--mut); }
</style>
</head>
<body>
<header>
  <h1>Seiva Admin</h1>
  <div class="tok"><input id="token" placeholder="Bearer token (for guarded operations)" autocomplete="off" /></div>
</header>
<main>
  <p class="muted" id="intro">Loading the service contract…</p>
  <div id="resources"></div>
</main>
<script>
const PLURAL = {};
function esc(s){ return String(s).replace(/[&<>]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;'}[c])); }
function inputFor(name, schema){
  const t = (schema && schema.type) || 'string';
  const ph = t === 'object' ? 'JSON object' : t;
  if (t === 'object' || t === 'array') return `<textarea data-k="${esc(name)}" data-json="1" rows="3" placeholder="${esc(ph)}"></textarea>`;
  const it = t === 'integer' || t === 'number' ? 'number' : (t === 'boolean' ? 'checkbox' : 'text');
  return `<input data-k="${esc(name)}" data-type="${esc(t)}" type="${it}" placeholder="${esc(ph)}" />`;
}
function collect(form){
  const body = {};
  form.querySelectorAll('[data-k]').forEach(el => {
    const k = el.dataset.k;
    if (el.dataset.json) { if (el.value.trim()) body[k] = JSON.parse(el.value); return; }
    if (el.type === 'checkbox') { body[k] = el.checked; return; }
    if (el.value === '') return;
    body[k] = (el.dataset.type === 'integer' || el.dataset.type === 'number') ? Number(el.value) : el.value;
  });
  return body;
}
async function run(ev){
  ev.preventDefault();
  const f = ev.target, out = f.querySelector('.out');
  out.style.display = 'block'; out.textContent = '…';
  const headers = { 'content-type': 'application/json' };
  const tok = document.getElementById('token').value.trim();
  if (tok) headers['authorization'] = 'Bearer ' + tok;
  const opts = { method: f.dataset.method, headers };
  if (f.dataset.method !== 'GET') { try { opts.body = JSON.stringify(collect(f)); } catch (e) { out.textContent = 'invalid JSON: ' + e; return; } }
  try {
    const res = await fetch(f.dataset.path, opts);
    const text = await res.text();
    let pretty = text; try { pretty = JSON.stringify(JSON.parse(text), null, 2); } catch (_) {}
    out.textContent = res.status + ' ' + res.statusText + '\n' + pretty;
  } catch (e) { out.textContent = 'request failed: ' + e; }
}
function render(spec){
  const paths = spec.paths || {};
  const byRes = {};
  for (const [path, item] of Object.entries(paths)) {
    const seg = path.split('/').filter(Boolean);
    const resource = seg[1] || seg[0] || path; // /{version}/{resource}/...
    (byRes[resource] = byRes[resource] || []).push([path, item]);
  }
  document.getElementById('intro').textContent =
    spec.info ? `${spec.info.title} — ${Object.keys(byRes).length} resource(s), generated from the live contract.` : '';
  const root = document.getElementById('resources');
  root.innerHTML = '';
  for (const [resource, entries] of Object.entries(byRes).sort()) {
    const el = document.createElement('div'); el.className = 'res';
    el.innerHTML = `<h2>${esc(resource)}</h2>`;
    for (const [path, item] of entries) {
      for (const method of ['get','post','put','patch','delete']) {
        const op = item[method]; if (!op) continue;
        const req = op.requires || op['x-seiva-requires'];
        const schema = (((op.requestBody||{}).content||{})['application/json']||{}).schema || {};
        const props = schema.properties || (schema.type === 'object' ? {} : null);
        let fields = '';
        if (method !== 'get') {
          if (props && Object.keys(props).length) {
            fields = '<div class="fields">' + Object.entries(props).map(([n,s]) =>
              `<label>${esc(n)} <span class="muted">(${esc((s&&s.type)||'string')})</span>${inputFor(n,s)}</label>`).join('') + '</div>';
          } else if (schema.type) {
            fields = `<div class="fields"><label>body <span class="muted">(${esc(schema.type)})</span><textarea data-k="" data-json="1" rows="3"></textarea></label></div>`;
          }
        }
        const form = document.createElement('form'); form.className = 'op';
        form.dataset.method = method.toUpperCase(); form.dataset.path = path;
        form.innerHTML =
          `<div class="sig"><span class="m">${method.toUpperCase()}</span><span class="path">${esc(path)}</span>`
          + (req ? `<span class="req">requires <code>${esc(req)}</code></span>` : `<span class="req">public</span>`) + `</div>`
          + fields + `<button type="submit">Send</button><div class="out"></div>`;
        form.addEventListener('submit', run);
        el.appendChild(form);
      }
    }
    root.appendChild(el);
  }
  if (!Object.keys(byRes).length) document.getElementById('intro').textContent =
    'No modules are mounted yet. Add one with `seiva module new <name>` and it will appear here.';
}
fetch('/openapi.json').then(r => r.json()).then(render).catch(e => {
  document.getElementById('intro').textContent = 'could not load /openapi.json: ' + e;
});
</script>
</body>
</html>
"#;

/// `GET /admin` — serves the self-contained [`ADMIN_HTML`] console under the
/// scoped [`ADMIN_CSP`] (so the strict default CSP does not blank it out).
pub async fn admin_handler() -> Response {
    ([(CONTENT_SECURITY_POLICY, ADMIN_CSP)], Html(ADMIN_HTML)).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{router, Health};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn admin_page_is_served_self_contained_from_the_contract() {
        let app = router(Health::starting());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
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
        // It builds itself from the service's OWN live contract…
        assert!(body.contains("/openapi.json"));
        assert!(body.contains("Seiva Admin"));
        // …and is self-contained: no external origin (unlike the Scalar /docs).
        assert!(!body.contains("http://") && !body.contains("https://"));
    }

    #[tokio::test]
    async fn admin_csp_is_self_contained_and_served() {
        let app = router(Health::starting());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        // Same-origin only: no remote host is permitted anywhere in the policy.
        assert!(csp.contains("default-src 'none'"));
        assert!(csp.contains("connect-src 'self'"));
        assert!(!csp.contains("https://"), "admin must not allow any CDN");
    }
}
