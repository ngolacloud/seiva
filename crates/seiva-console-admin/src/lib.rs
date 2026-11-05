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

use axum::extract::FromRequestParts;
use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use delonix_seiva::admin::{admin_registry_conflicts, registered_admin_resources, AdminResource};
use seiva_identity::Principal;
use sha2::{Digest, Sha256};

/// The path the console is served at when [`mount`]ed. The base framework does
/// not serve this route; this crate does, only once a composition root opts in.
pub const ADMIN_PATH: &str = "/admin";

/// The path serving the authorization-filtered registry the console UI reads to
/// decide which resources to show. Mounted alongside [`ADMIN_PATH`] by
/// [`admin_router`].
pub const REGISTRY_PATH: &str = "/admin/registry.json";

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
// Operations in the contract whose resource segment (`/{version}/{resource}`)
// matches a registered resource — the console shows ONLY these.
function opsForResource(spec, resource){
  const out = [];
  const paths = spec.paths || {};
  for (const [path, item] of Object.entries(paths)) {
    const seg = path.split('/').filter(Boolean);
    const res = seg[1] || seg[0];
    if (res !== resource) continue;
    for (const method of ['get','post','put','patch','delete']) {
      const op = item[method]; if (!op) continue;
      out.push({ method, path, op });
    }
  }
  return out;
}
function formFor(method, path, op){
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
  return form;
}
function renderResource(r, spec){
  const el = document.createElement('div'); el.className = 'res';
  el.innerHTML = `<h2>${esc(r.label)} <span class="muted">${esc(r.namespace)}/${esc(r.resource)}</span></h2>`;
  if (Array.isArray(r.list_fields) && r.list_fields.length) {
    el.innerHTML += `<div class="op muted">fields: ${r.list_fields.map(esc).join(', ')}</div>`;
  }
  const ops = opsForResource(spec, r.resource);
  let shown = 0;
  for (const {method, path, op} of ops) {
    const isWrite = method !== 'get';
    // The caller sees a write form only if the registry says they may create or
    // change; the server still enforces the per-operation guard regardless.
    if (isWrite && !(r.can && (r.can.create || r.can.change))) continue;
    el.appendChild(formFor(method, path, op));
    shown++;
  }
  if (!shown) el.innerHTML += `<div class="op muted">No operations available to you for this resource.</div>`;
  return el;
}
async function load(){
  let reg, spec;
  try {
    [reg, spec] = await Promise.all([
      fetch('/admin/registry.json').then(r => { if (!r.ok) throw new Error('registry ' + r.status); return r.json(); }),
      fetch('/openapi.json').then(r => r.json()),
    ]);
  } catch (e) {
    document.getElementById('intro').textContent = 'could not load the console: ' + e;
    return;
  }
  const resources = (reg && reg.resources) || [];
  const root = document.getElementById('resources'); root.innerHTML = '';
  document.getElementById('intro').textContent = resources.length
    ? `${resources.length} resource(s) you may administer.`
    : 'No resources are registered for you. A module appears here when it registers an admin resource (seiva module new --admin) and you hold its view permission.';
  for (const r of resources) root.appendChild(renderResource(r, spec));
}
document.getElementById('token').addEventListener('change', load);
load();
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

/// The authenticated caller, read from the request extensions the `authn`
/// middleware populates. Infallible: a request with no principal (no authn layer,
/// or an anonymous one) resolves to [`Principal::anonymous`], which holds no
/// permissions — so the registry endpoint fails closed, returning nothing.
struct Caller(Principal);

impl<S: Sync> FromRequestParts<S> for Caller {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Caller(
            parts
                .extensions
                .get::<Principal>()
                .cloned()
                .unwrap_or_else(Principal::anonymous),
        ))
    }
}

/// Whether `caller` may at least *view* `resource`: its `view` permission is
/// declared (non-empty) and the caller holds it. A resource with an empty `view`
/// permission is hidden from everyone (effectively disabled in the console).
fn may_view(caller: &Principal, resource: &AdminResource) -> bool {
    !resource.permissions.view.is_empty() && caller.has(resource.permissions.view)
}

/// Builds the JSON view of one resource the caller may see, resolving each
/// operation's permission to a `can` flag so the UI hides what the caller cannot
/// do (the server still enforces every guard).
fn resource_json(caller: &Principal, r: &AdminResource) -> serde_json::Value {
    let can = |perm: &str| !perm.is_empty() && caller.has(perm);
    serde_json::json!({
        "namespace": r.namespace,
        "resource": r.resource,
        "label": r.label,
        "facade": r.facade,
        "version": r.version,
        "path": r.path(),
        "list_fields": r.list_fields,
        "search_fields": r.search_fields,
        "filters": r.filters,
        "ordering": r.ordering,
        "page_size": r.page_size,
        "can": {
            "view": can(r.permissions.view),
            "create": can(r.permissions.create),
            "change": can(r.permissions.change),
            "delete": can(r.permissions.delete),
        },
    })
}

/// `GET /admin/registry.json` — the resources the caller is **registered and
/// authorized** to administer. A resource appears only when it is registered
/// (via `register_admin!`) *and* the caller holds its `view` permission;
/// `/openapi.json` membership alone never surfaces a resource here (OWASP
/// API9/API1). A duplicate registration is a `500` so a mis-wire fails loudly
/// rather than silently shadowing one resource with another.
async fn registry_handler(Caller(caller): Caller) -> Response {
    let (status, body) = build_registry(
        &caller,
        &registered_admin_resources(),
        &admin_registry_conflicts(),
    );
    (status, Json(body)).into_response()
}

/// The registry endpoint's pure core: given the caller, the registered resources
/// and any conflicts, produce the status and JSON body. Split out from the
/// handler so it is testable with crafted inputs without touching the
/// process-global link-time inventory.
fn build_registry(
    caller: &Principal,
    resources: &[AdminResource],
    conflicts: &[String],
) -> (StatusCode, serde_json::Value) {
    if !conflicts.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "error": "admin registry has conflicting (namespace, resource) registrations",
                "conflicts": conflicts,
            }),
        );
    }
    let visible: Vec<serde_json::Value> = resources
        .iter()
        .filter(|r| may_view(caller, r))
        .map(|r| resource_json(caller, r))
        .collect();
    (StatusCode::OK, serde_json::json!({ "resources": visible }))
}

/// Validates the admin registry: `Err` listing the conflicting ids when two
/// registrations share a `(namespace, resource)`. A composition root should call
/// this before serving so a double-registration is a startup error:
///
/// ```no_run
/// seiva_console_admin::check().expect("admin registry is consistent");
/// ```
///
/// # Errors
///
/// Returns the conflicting `{namespace}/{resource}` ids when the registry is
/// inconsistent.
pub fn check() -> Result<(), Vec<String>> {
    let conflicts = admin_registry_conflicts();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(conflicts)
    }
}

/// A `Router` serving the console at [`ADMIN_PATH`] and its authorization-filtered
/// registry at [`REGISTRY_PATH`]. Merge it into an application router, or use
/// [`mount`].
pub fn admin_router() -> Router {
    Router::new()
        .route(ADMIN_PATH, get(admin_handler))
        .route(REGISTRY_PATH, get(registry_handler))
}

/// Mounts the console onto `router` (adds `GET /admin` and `GET
/// /admin/registry.json`) and returns it. This is
/// the explicit opt-in a composition root calls; without it the backend has no
/// admin route. Mounting twice is rejected by axum (a duplicate route panics at
/// build time), surfacing a double-mount as a startup error rather than silently.
pub fn mount(router: Router) -> Router {
    router.merge(admin_router())
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use delonix_seiva::admin::AdminPermissions;

    fn resource(ns: &'static str, res: &'static str, view: &'static str) -> AdminResource {
        AdminResource {
            namespace: ns,
            resource: res,
            label: "L",
            facade: "F",
            version: "v1",
            list_fields: &["id"],
            search_fields: &[],
            filters: &[],
            ordering: &[],
            page_size: 25,
            permissions: AdminPermissions {
                view,
                create: "shop.create",
                change: "",
                delete: "",
            },
        }
    }

    #[test]
    fn a_conflict_is_a_500_and_lists_no_resources() {
        let (status, body) = build_registry(
            &Principal::new("admin", ["shop.read"]),
            &[resource("shop", "products", "shop.read")],
            &["shop/products".to_owned()],
        );
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.get("resources").is_none());
        assert_eq!(body["conflicts"][0], "shop/products");
    }

    #[test]
    fn only_resources_the_caller_may_view_are_listed() {
        let resources = [
            resource("shop", "products", "shop.read"),
            resource("secret", "vault", "secret.read"),
        ];
        let caller = Principal::new("clerk", ["shop.read"]);
        let (status, body) = build_registry(&caller, &resources, &[]);
        assert_eq!(status, StatusCode::OK);
        let list = body["resources"].as_array().unwrap();
        assert_eq!(list.len(), 1, "only the authorized resource: {body}");
        assert_eq!(list[0]["resource"], "products");
        assert_eq!(list[0]["path"], "/admin/shop/products");
        // The caller holds view but not create → can.view true, can.create false.
        assert_eq!(list[0]["can"]["view"], true);
        assert_eq!(list[0]["can"]["create"], false);
    }

    #[test]
    fn a_caller_who_may_create_sees_the_create_affordance() {
        let resources = [resource("shop", "products", "shop.read")];
        let caller = Principal::new("mgr", ["shop.read", "shop.create"]);
        let (_, body) = build_registry(&caller, &resources, &[]);
        assert_eq!(body["resources"][0]["can"]["create"], true);
    }

    #[test]
    fn an_anonymous_caller_sees_nothing() {
        let resources = [resource("shop", "products", "shop.read")];
        let (status, body) = build_registry(&Principal::anonymous(), &resources, &[]);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["resources"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_resource_with_empty_view_permission_is_hidden_from_everyone() {
        // view="" means "disabled in the console": even a caller holding every
        // other permission cannot see it.
        let resources = [resource("shop", "products", "")];
        let caller = Principal::new("root", ["shop.read", "shop.create"]);
        let (_, body) = build_registry(&caller, &resources, &[]);
        assert_eq!(body["resources"].as_array().unwrap().len(), 0);
    }
}
