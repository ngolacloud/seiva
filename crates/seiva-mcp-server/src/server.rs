//! The MCP server: Streamable HTTP (`POST /mcp`, JSON responses), the person's
//! own token, permission checks, human approval for writes, audit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use seiva::contract::ToolEffect;
use seiva_http::Ctx;
use seiva_identity::{Authenticator, Principal};
use seiva_ports::{AuditEntry, AuditSink};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::tools::{Tool, ToolSet};

/// The MCP protocol revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The argument that carries a confirmed approval when a write tool is called again.
pub const APPROVAL_ARG: &str = "_approval";

const MAX_PENDING: usize = 10_000;

struct Approval {
    subject: String,
    tool: String,
    arguments: Value,
    digest: String,
    created: Instant,
    confirmed: bool,
}

struct Inner {
    tools: ToolSet,
    auth: Arc<dyn Authenticator>,
    audit: Arc<dyn AuditSink>,
    resource: Option<String>,
    authorization_servers: Vec<String>,
    allowed_origins: HashSet<String>,
    auto_approve: HashMap<String, String>,
    approval_ttl: Duration,
    call_timeout: Duration,
    body_limit: usize,
    approvals: Mutex<HashMap<String, Approval>>,
}

/// Serves a [`ToolSet`] over MCP. Cheap to clone.
#[derive(Clone)]
pub struct McpServer {
    inner: Arc<Inner>,
}

/// Builds an [`McpServer`].
pub struct McpServerBuilder {
    tools: ToolSet,
    auth: Arc<dyn Authenticator>,
    audit: Arc<dyn AuditSink>,
    resource: Option<String>,
    authorization_servers: Vec<String>,
    allowed_origins: HashSet<String>,
    auto_approve: HashMap<String, String>,
    approval_ttl: Duration,
    call_timeout: Duration,
    body_limit: usize,
}

impl McpServerBuilder {
    /// The public URL of this server (`https://api.example.com/mcp`), announced in
    /// the OAuth protected-resource metadata (RFC 9728) and in the
    /// `WWW-Authenticate` header of a 401.
    #[must_use]
    pub fn resource(mut self, url: impl Into<String>) -> Self {
        self.resource = Some(url.into());
        self
    }

    /// An OAuth authorization server clients get tokens from (ADR 0038/0040).
    #[must_use]
    pub fn authorization_server(mut self, url: impl Into<String>) -> Self {
        self.authorization_servers.push(url.into());
        self
    }

    /// A browser origin allowed to call the server. A request with an `Origin`
    /// header that is not listed is refused (DNS-rebinding protection, as the MCP
    /// transport requires); clients that send none are unaffected.
    #[must_use]
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.insert(origin.into());
        self
    }

    /// A caller holding `permission` runs the write tool `tool` without a
    /// separate approval. An explicit, audited choice.
    #[must_use]
    pub fn auto_approve(mut self, tool: impl Into<String>, permission: impl Into<String>) -> Self {
        self.auto_approve.insert(tool.into(), permission.into());
        self
    }

    /// How long an approval stays valid (5 minutes by default).
    #[must_use]
    pub fn approval_ttl(mut self, ttl: Duration) -> Self {
        self.approval_ttl = ttl;
        self
    }

    /// The time budget of one tool call (60 s by default).
    #[must_use]
    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Build the server.
    pub fn build(self) -> McpServer {
        McpServer {
            inner: Arc::new(Inner {
                tools: self.tools,
                auth: self.auth,
                audit: self.audit,
                resource: self.resource,
                authorization_servers: self.authorization_servers,
                allowed_origins: self.allowed_origins,
                auto_approve: self.auto_approve,
                approval_ttl: self.approval_ttl,
                call_timeout: self.call_timeout,
                body_limit: self.body_limit,
                approvals: Mutex::new(HashMap::new()),
            }),
        }
    }
}

impl McpServer {
    /// A server for `tools`, authenticating callers with `auth` — the same
    /// authenticator as the HTTP API, so the token an agent presents is the
    /// person's own — and recording every call in `audit`.
    pub fn builder(
        tools: ToolSet,
        auth: Arc<dyn Authenticator>,
        audit: Arc<dyn AuditSink>,
    ) -> McpServerBuilder {
        McpServerBuilder {
            tools,
            auth,
            audit,
            resource: None,
            authorization_servers: Vec::new(),
            allowed_origins: HashSet::new(),
            auto_approve: HashMap::new(),
            approval_ttl: Duration::from_secs(300),
            call_timeout: Duration::from_secs(60),
            body_limit: 1024 * 1024,
        }
    }

    /// The routes: `POST /mcp`, `GET /mcp` (405: no server-initiated stream),
    /// `GET`/`POST /mcp/approvals/{id}` (what the person sees, and confirms) and
    /// `GET /.well-known/oauth-protected-resource`.
    pub fn router(&self) -> axum::Router {
        axum::Router::new()
            .route("/mcp", post(rpc).get(no_stream))
            .route(
                "/mcp/approvals/{id}",
                get(show_approval).post(confirm_approval),
            )
            .route(
                "/.well-known/oauth-protected-resource",
                get(resource_metadata),
            )
            .layer(axum::extract::DefaultBodyLimit::max(self.inner.body_limit))
            .with_state(self.clone())
    }

    fn origin_ok(&self, headers: &HeaderMap) -> bool {
        match headers.get("origin").and_then(|v| v.to_str().ok()) {
            None => true,
            Some(o) => self.inner.allowed_origins.contains(o),
        }
    }

    fn authenticate(&self, headers: &HeaderMap) -> Option<Principal> {
        let token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })?
            .trim();
        self.inner.auth.authenticate(token).ok()
    }

    fn unauthorized(&self) -> Response {
        let mut res = (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "a bearer token is required"})),
        )
            .into_response();
        let challenge = match &self.inner.resource {
            Some(r) => format!(
                "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
                origin_of(r)
            ),
            None => "Bearer".to_owned(),
        };
        if let Ok(v) = HeaderValue::from_str(&challenge) {
            res.headers_mut().insert("www-authenticate", v);
        }
        res
    }

    fn may_use(principal: &Principal, tool: &Tool) -> bool {
        tool.requires.is_empty() || principal.has(tool.requires)
    }

    async fn record(
        &self,
        principal: &Principal,
        tool: &str,
        outcome: &str,
        details: &[(&str, String)],
    ) -> Result<(), String> {
        let mut entry = AuditEntry::new(principal.subject.clone(), "mcp.tools/call", outcome)
            .in_tenant(principal.tenant.clone())
            .on_object(Some(tool.to_owned()));
        for (k, v) in details {
            entry = entry.with_detail(*k, v.clone());
        }
        self.inner
            .audit
            .record(&entry.redacted())
            .await
            .map_err(|e| e.to_string())
    }

    fn tools_list(&self, principal: &Principal) -> Value {
        let tools: Vec<Value> = self
            .inner
            .tools
            .tools()
            .filter(|t| Self::may_use(principal, t))
            .map(|t| {
                let read = t.effect == ToolEffect::Read;
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": {"readOnlyHint": read, "destructiveHint": !read, "openWorldHint": false},
                })
            })
            .collect();
        json!({"tools": tools})
    }

    /// One `tools/call`: the result, or a JSON-RPC error.
    async fn tools_call(
        &self,
        principal: &Principal,
        params: &Value,
    ) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let unknown = || (-32602, format!("Unknown tool: {name}"));
        // A tool the caller may not use is reported as unknown: its existence is
        // not disclosed.
        let tool = match self.inner.tools.get(name) {
            Some(t) if Self::may_use(principal, t) => t.clone(),
            _ => return Err(unknown()),
        };
        let mut args = match params.get("arguments") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(Value::Object(m)) => Value::Object(m.clone()),
            Some(_) => return Err((-32602, "`arguments` must be an object".into())),
        };
        let approval_id = args
            .as_object_mut()
            .and_then(|m| m.remove(APPROVAL_ARG))
            .and_then(|v| v.as_str().map(str::to_owned));
        let digest = digest(&args);

        if tool.effect == ToolEffect::Write {
            let auto = self
                .inner
                .auto_approve
                .get(&tool.name)
                .is_some_and(|p| principal.has(p));
            let approved = match (&approval_id, auto) {
                (_, true) => Some("auto".to_owned()),
                (Some(id), false) => match self.take_approval(id, principal, &tool.name, &digest) {
                    Ok(()) => Some(id.clone()),
                    Err(reason) => {
                        let _ = self
                            .record(
                                principal,
                                &tool.name,
                                "approval_refused",
                                &[("effect", "write".into()), ("reason", reason.clone())],
                            )
                            .await;
                        return Ok(tool_error(&format!("Not run: {reason}.")));
                    }
                },
                (None, false) => None,
            };
            let Some(approval) = approved else {
                let id = self.open_approval(principal, &tool.name, &args, &digest);
                let _ = self
                    .record(
                        principal,
                        &tool.name,
                        "approval_required",
                        &[("effect", "write".into()), ("approval", id.clone())],
                    )
                    .await;
                let ttl = self.inner.approval_ttl.as_secs();
                let text = format!(
                    "Not run yet: `{}` changes data and needs the approval of the person you act for. Ask them \
                     to approve request {id} in the app; then call the tool again with the same arguments \
                     plus \"{APPROVAL_ARG}\": \"{id}\". The approval expires in {ttl} seconds.",
                    tool.name
                );
                return Ok(json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": {"approval_required": {"id": id, "tool": tool.name, "expires_in": ttl}},
                    "isError": true,
                }));
            };
            // The critical audit: written before the change, or the change does not run.
            if let Err(e) = self
                .record(
                    principal,
                    &tool.name,
                    "started",
                    &[
                        ("effect", "write".into()),
                        ("approval", approval.clone()),
                        ("arguments_digest", digest.clone()),
                    ],
                )
                .await
            {
                return Ok(tool_error(&format!(
                    "Not run: the audit record could not be written ({e})."
                )));
            }
        }

        let ctx = Ctx::new(principal.clone());
        let future = self
            .inner
            .tools
            .call(&tool.name, ctx, args)
            .ok_or_else(unknown)?;
        let outcome = match tokio::time::timeout(self.inner.call_timeout, future).await {
            Err(_) => Err(format!(
                "the call took longer than {} seconds",
                self.inner.call_timeout.as_secs()
            )),
            Ok(r) => r.map_err(|e| e.to_string()),
        };
        let effect = tool.effect.as_str().to_owned();
        match outcome {
            Ok(value) => {
                let audited = self
                    .record(
                        principal,
                        &tool.name,
                        "ok",
                        &[("effect", effect), ("arguments_digest", digest)],
                    )
                    .await;
                if audited.is_err() && tool.effect == ToolEffect::Read {
                    return Ok(tool_error(
                        "The result is withheld: the audit record could not be written.",
                    ));
                }
                let structured = match &value {
                    Value::Object(_) => value.clone(),
                    other => json!({"result": other}),
                };
                Ok(json!({
                    "content": [{"type": "text", "text": value.to_string()}],
                    "structuredContent": structured,
                    "isError": false,
                }))
            }
            Err(message) => {
                let _ = self
                    .record(
                        principal,
                        &tool.name,
                        "failed",
                        &[("effect", effect), ("reason", message.clone())],
                    )
                    .await;
                Ok(tool_error(&message))
            }
        }
    }

    fn open_approval(
        &self,
        principal: &Principal,
        tool: &str,
        args: &Value,
        digest: &str,
    ) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seed = format!(
            "{nanos}:{}:{}:{tool}:{digest}",
            COUNTER.fetch_add(1, Ordering::Relaxed),
            principal.subject
        );
        let id = format!("apr_{}", &hex(&Sha256::digest(seed.as_bytes()))[..24]);
        let mut map = self.inner.approvals.lock().expect("approvals poisoned");
        let ttl = self.inner.approval_ttl;
        map.retain(|_, a| a.created.elapsed() < ttl);
        if map.len() >= MAX_PENDING {
            // Bounded: the oldest pending approval goes first.
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, a)| a.created)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(
            id.clone(),
            Approval {
                subject: principal.subject.clone(),
                tool: tool.to_owned(),
                arguments: args.clone(),
                digest: digest.to_owned(),
                created: Instant::now(),
                confirmed: false,
            },
        );
        id
    }

    /// Consume a confirmed approval for exactly this caller, tool and arguments.
    fn take_approval(
        &self,
        id: &str,
        principal: &Principal,
        tool: &str,
        digest: &str,
    ) -> Result<(), String> {
        let mut map = self.inner.approvals.lock().expect("approvals poisoned");
        let ttl = self.inner.approval_ttl;
        let ok = match map.get(id) {
            None => return Err("the approval is unknown or was already used".into()),
            Some(a) if a.created.elapsed() >= ttl => Err("the approval has expired".to_owned()),
            Some(a) if a.subject != principal.subject => {
                Err("the approval belongs to someone else".to_owned())
            }
            Some(a) if a.tool != tool => Err("the approval is for another tool".to_owned()),
            Some(a) if a.digest != digest => {
                Err("the arguments differ from the approved ones".to_owned())
            }
            Some(a) if !a.confirmed => return Err("the person has not approved it yet".into()),
            Some(_) => Ok(()),
        };
        // Single use, whatever the outcome (an expired or mismatched approval is spent too).
        map.remove(id);
        ok
    }
}

async fn no_stream() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "this server does not open server-initiated streams",
    )
        .into_response()
}

async fn resource_metadata(State(server): State<McpServer>) -> Response {
    Json(json!({
        "resource": server.inner.resource,
        "authorization_servers": server.inner.authorization_servers,
        "bearer_methods_supported": ["header"],
    }))
    .into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

async fn rpc(State(server): State<McpServer>, headers: HeaderMap, body: Bytes) -> Response {
    if !server.origin_ok(&headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let Some(principal) = server.authenticate(&headers) else {
        return server.unauthorized();
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Json(rpc_error(Value::Null, -32700, "Parse error")).into_response(),
    };
    let Value::Object(msg) = &message else {
        return Json(rpc_error(
            Value::Null,
            -32600,
            "Invalid Request (batches are not supported)",
        ))
        .into_response();
    };
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(id) = msg.get("id").cloned() else {
        // A notification (or a response to us): accepted, nothing to answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "seiva-mcp-server", "version": seiva::VERSION},
            "instructions": "Tools act on behalf of the signed-in person, with their permissions. A tool that \
                             changes data asks for that person's approval first.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(server.tools_list(&principal)),
        "tools/call" => server.tools_call(&principal, &params).await,
        other => Err((-32601, format!("Method not found: {other}"))),
    };
    Json(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, m)) => rpc_error(id, code, &m),
    })
    .into_response()
}

/// What the person is asked to approve: the tool, its description and the
/// arguments. Only for the person the approval belongs to.
async fn show_approval(
    State(server): State<McpServer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !server.origin_ok(&headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let Some(principal) = server.authenticate(&headers) else {
        return server.unauthorized();
    };
    let map = server.inner.approvals.lock().expect("approvals poisoned");
    match map.get(&id) {
        Some(a)
            if a.subject == principal.subject
                && a.created.elapsed() < server.inner.approval_ttl =>
        {
            let description = server
                .inner
                .tools
                .get(&a.tool)
                .map(|t| t.description)
                .unwrap_or_default();
            Json(json!({"id": id, "tool": a.tool, "description": description, "arguments": a.arguments, "confirmed": a.confirmed}))
                .into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The person approves. The model cannot reach this endpoint: it is the app's
/// (the MCP host's) call, made with the person's token after showing them the
/// request.
async fn confirm_approval(
    State(server): State<McpServer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !server.origin_ok(&headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let Some(principal) = server.authenticate(&headers) else {
        return server.unauthorized();
    };
    let tool = {
        let mut map = server.inner.approvals.lock().expect("approvals poisoned");
        match map.get_mut(&id) {
            Some(a)
                if a.subject == principal.subject
                    && a.created.elapsed() < server.inner.approval_ttl =>
            {
                a.confirmed = true;
                a.tool.clone()
            }
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    if let Err(e) = server
        .record(&principal, &tool, "approved", &[("approval", id.clone())])
        .await
    {
        // An approval that cannot be audited does not count.
        server
            .inner
            .approvals
            .lock()
            .expect("approvals poisoned")
            .remove(&id);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": format!("the approval could not be audited: {e}")})),
        )
            .into_response();
    }
    Json(json!({"id": id, "tool": tool, "approved": true})).into_response()
}

/// The scheme and authority of a URL (`https://api.example.com`).
fn origin_of(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let authority = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{authority}")
}

/// SHA-256 of the arguments in canonical JSON (keys sorted at every level), so
/// the same arguments in another key order are the same request.
pub(crate) fn digest(value: &Value) -> String {
    fn canonical(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let sorted: BTreeMap<&String, Value> =
                    m.iter().map(|(k, v)| (k, canonical(v))).collect();
                Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }
    let text = serde_json::to_string(&canonical(value)).unwrap_or_default();
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_ignores_key_order() {
        assert_eq!(
            digest(&json!({"a": 1, "b": {"y": 2, "x": 1}})),
            digest(&json!({"b": {"x": 1, "y": 2}, "a": 1}))
        );
        assert_ne!(digest(&json!({"a": 1})), digest(&json!({"a": 2})));
    }

    #[test]
    fn the_metadata_url_is_on_the_resource_origin() {
        assert_eq!(
            origin_of("https://api.example.com/mcp"),
            "https://api.example.com"
        );
    }
}
