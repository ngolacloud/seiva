//! The optional Delonix Seiva admin console — an **extension** of the Seiva base
//! (ADR 0008), not part of it.
//!
//! Django's signature feature is an auto-generated admin over your models. Seiva
//! has no reflection, but it *does* have the project's full contract at runtime:
//! `GET /openapi.json` already lists every mounted module's operations and — via
//! `WireSchema` (ADR 0007) — the real request/response shapes. This crate turns
//! that into a **self-contained** console: it groups the operations by resource,
//! renders a form for each write operation from the request DTO's schema, and
//! lets you exercise the real (auth-guarded) routes with a bearer token. No
//! per-module code: add a module (`seiva module new`), and it appears.
//!
//! # This is an extension, not the base
//!
//! The framework base (`seiva-http`) carries **no** admin code and answers `404`
//! for `/admin`. A composition root opts the console in explicitly and must
//! recompile for it to appear:
//!
//! ```no_run
//! use axum::Router;
//! # fn modules() -> Router { Router::new() }
//! let app: Router = modules();
//! let app = seiva_console_admin::mount(app); // now GET /admin serves the console
//! ```
//!
//! The dependency is one-way: this crate depends on `axum::Router` (Seiva's
//! public transport type); nothing in the base depends on it, so a backend built
//! without this crate has no admin assets and no `/admin` route — provable with
//! `cargo tree`.
//!
//! # Scope of this first cut
//!
//! The console drives the module **operations** the contract declares
//! (create/get/list/…). Generic *data* management — listing and editing rows of
//! a model — and per-module registration with server-side authorization,
//! typed forms and tenant masking are later increments (ADR 0008, Phase 3). This
//! is an OpenAPI-driven operations console, **not** full Django-admin parity.
//!
//! # Security
//!
//! Unlike an API reference that loads a renderer from a CDN, this page is fully
//! self-contained — inline HTML/CSS/JS, no external fetch beyond the same-origin
//! `/openapi.json` and the API calls it makes. Its [`admin_csp`] forbids every
//! remote origin and, crucially, does **not** use `unsafe-inline`: the inline
//! `<style>` and `<script>` are pinned by their SHA-256 hash, computed from the
//! exact bytes served (so the policy and the payload can never drift apart).
//! The console does no authorization of its own — every operation it invokes is
//! guarded server-side by the module's `#[seiva::controller]`; the bearer token
//! is sent only in the `Authorization` header, never in a URL.

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use sha2::{Digest, Sha256};

/// The path the console is served at when [`mount`]ed. The base framework does
/// not serve this route; this crate does, only once a composition root opts in.
pub const ADMIN_PATH: &str = "/admin";

/// The console's stylesheet — the exact bytes that go between `<style>` and
/// `</style>`, so the CSP's `style-src` hash is computed over this verbatim.
const ADMIN_CSS: &str = r#"
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
"#;

/// The console's script — the exact bytes that go between `<script>` and
/// `</script>`, so the CSP's `script-src` hash is computed over this verbatim.
/// It attaches handlers via `addEventListener` (no inline `on*` attributes) and
/// escapes every value it renders, so no `unsafe-inline`/`unsafe-hashes` is
/// needed.
const ADMIN_JS: &str = r#"
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
"#;

/// The page body between the hashed `<style>`/`<script>` blocks. `{css}`/`{js}`
/// are substituted verbatim so the served inline content equals the hashed
/// bytes.
const ADMIN_SHELL: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Delonix Seiva — Admin</title>
<style>{css}</style>
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
<script>{js}</script>
</body>
</html>
"#;

/// The rendered page, assembled once: the shell with the stylesheet and script
/// inlined verbatim (the exact bytes [`admin_csp`] hashes).
fn admin_html() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| {
        ADMIN_SHELL
            .replacen("{css}", ADMIN_CSS, 1)
            .replacen("{js}", ADMIN_JS, 1)
    })
    .as_str()
}

/// Base64 of the SHA-256 digest of `content` — the value a CSP `'sha256-…'`
/// source expression carries.
fn sha256_b64(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    BASE64.encode(digest)
}

/// The `Content-Security-Policy` for the console: everything same-origin, the
/// inline `<style>`/`<script>` pinned by SHA-256 hash (no `unsafe-inline`), no
/// remote origin at all, and navigation/embedding locked down (`base-uri`,
/// `form-action`, `frame-ancestors` all `'none'`). Computed once from the exact
/// bytes the console serves, so the policy can never drift from the payload.
pub fn admin_csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        format!(
            "default-src 'none'; \
             script-src 'sha256-{js}'; \
             style-src 'sha256-{css}'; \
             connect-src 'self'; \
             img-src 'self' data:; \
             base-uri 'none'; \
             form-action 'none'; \
             frame-ancestors 'none'",
            js = sha256_b64(ADMIN_JS),
            css = sha256_b64(ADMIN_CSS),
        )
    })
    .as_str()
}

/// `GET /admin` — serves the self-contained console under the strict, hash-based
/// [`admin_csp`] (so no `unsafe-inline` is needed and the default `default-src
/// 'none'` does not blank it out).
pub async fn admin_handler() -> Response {
    ([(CONTENT_SECURITY_POLICY, admin_csp())], Html(admin_html())).into_response()
}

/// A `Router` serving only the console at [`ADMIN_PATH`]. Merge it into an
/// application router, or use [`mount`].
pub fn admin_router() -> Router {
    Router::new().route(ADMIN_PATH, get(admin_handler))
}

/// Mounts the console onto `router` (adds `GET /admin`) and returns it. This is
/// the explicit opt-in a composition root calls; without it the backend has no
/// admin route. Mounting twice is rejected by axum (a duplicate route panics at
/// build time), surfacing a double-mount as a startup error rather than silently.
pub fn mount(router: Router) -> Router {
    router.merge(admin_router())
}
