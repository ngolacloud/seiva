//! The runnable AI gateway: a TOML file in, an axum router out.
//!
//! The composition lives here (not in `main`) so the end-to-end test builds the
//! very same app with its own credentials and audit sink.
//!
//! ```toml
//! listen = "127.0.0.1:4000"
//! audit = "stdout"                       # or a file path (JSON lines, appended)
//!
//! [budget]
//! period_secs = 86400
//! per_principal = 2000000
//!
//! [[providers]]
//! name = "anthropic"
//! kind = "anthropic"                     # anthropic | openai | gemini | openai-compatible
//! residency = "abroad"                   # abroad | in_country
//!
//! [[providers]]
//! name = "local"
//! kind = "openai-compatible"
//! endpoint = "http://10.0.0.20:11434/v1"
//! plaintext = true
//! residency = "in_country"
//!
//! [[routes]]
//! name = "support-chat"
//! residency = "abroad"
//! requires = "llm.support"
//! upstreams = ["anthropic/claude-sonnet-5-5", "local/llama3.3"]
//!
//! [[routes]]
//! name = "docs-embed"
//! kind = "embedding"                     # chat (default) | embedding | chat+embedding
//! residency = "in_country"
//! requires = "llm.embed"
//! upstreams = ["local/bge-m3"]           # openai, gemini, openai-compatible providers
//!
//! [gateway]
//! areas_in_country = ["hr"]
//! tools = { customer_lookup = "customers.read" }
//!
//! [[keys]]
//! subject = "support-desk"
//! sha256 = "…"                           # seiva-ai-gateway hash-key < key.txt
//! permissions = ["llm.support"]
//! area = "support"
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use seiva_ai_gateway::server::{router, GatewayKeys, KeyEntry, ServerOptions, ServerState};
use seiva_ai_gateway::{BudgetScope, Gateway, InMemoryBudget, RevealPolicy};
use seiva_identity::Principal;
use seiva_llm::providers::anthropic::Anthropic;
use seiva_llm::providers::openai::OpenAiCompatible;
use seiva_llm::{
    ChatProvider, CredentialSource, Embedder, Endpoint, HttpTransport, Residency, Route, RouteKind,
    Router,
};
use seiva_ports::{AuditEntry, AuditSink, Clock, PortResult};
use serde::Deserialize;

/// The whole configuration file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Where to listen.
    pub listen: SocketAddr,
    /// `stdout` or a file path for the JSON-lines audit record.
    #[serde(default = "default_audit")]
    pub audit: String,
    /// Request limits.
    #[serde(default)]
    pub limits: Limits,
    /// Token budgets.
    pub budget: Budget,
    /// Model providers.
    pub providers: Vec<ProviderConfig>,
    /// Routes clients call by name.
    pub routes: Vec<RouteConfig>,
    /// Gateway policy.
    #[serde(default)]
    pub gateway: Policy,
    /// Gateway keys (SHA-256 only).
    #[serde(default)]
    pub keys: Vec<KeyConfig>,
}

fn default_audit() -> String {
    "stdout".into()
}

/// Request limits.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    /// `max_tokens` when an OpenAI client sends none.
    pub default_max_tokens: u32,
    /// The largest `max_tokens` accepted.
    pub max_tokens_cap: u32,
    /// The largest request body, in MiB.
    pub body_limit_mb: usize,
    /// The time budget of one model attempt, in seconds.
    pub attempt_timeout_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        let o = ServerOptions::default();
        Self {
            default_max_tokens: o.default_max_tokens,
            max_tokens_cap: o.max_tokens_cap,
            body_limit_mb: o.body_limit / (1024 * 1024),
            attempt_timeout_secs: 120,
        }
    }
}

/// Token budgets per period.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// The window, in seconds.
    pub period_secs: u64,
    /// Tokens per principal per window.
    pub per_principal: Option<u64>,
    /// Tokens per tenant per window.
    pub per_tenant: Option<u64>,
    /// Tokens per area per window.
    pub per_area: Option<u64>,
}

/// One provider.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Its name in routes (`anthropic/…`).
    pub name: String,
    /// `anthropic`, `openai`, `gemini` or `openai-compatible`.
    pub kind: String,
    /// `abroad` or `in_country`.
    pub residency: String,
    /// The credential name (an environment variable); optional for
    /// `openai-compatible`.
    pub credential: Option<String>,
    /// Another endpoint; required for `openai-compatible`.
    pub endpoint: Option<String>,
    /// Allow a plain `http` endpoint (a model on the organisation's network).
    #[serde(default)]
    pub plaintext: bool,
}

/// One route.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    /// The name clients send as `model`.
    pub name: String,
    /// `abroad` or `in_country`.
    pub residency: String,
    /// `chat` (the default), `embedding`, or `chat+embedding` — a route serves
    /// one kind unless it says both.
    pub kind: Option<String>,
    /// The permission a caller needs.
    pub requires: Option<String>,
    /// Any authenticated caller may use it.
    #[serde(default)]
    pub public: bool,
    /// `provider/model`, in fallback order.
    pub upstreams: Vec<String>,
}

/// Gateway policy.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Areas whose callers only use `in_country` routes.
    #[serde(default)]
    pub areas_in_country: Vec<String>,
    /// Tool name → the permission a caller needs to offer it.
    #[serde(default)]
    pub tools: BTreeMap<String, String>,
    /// A caller holding this permission may offer any tool (a coding agent).
    pub any_tool: Option<String>,
    /// `caller` (default) or `never`.
    pub reveal: Option<String>,
    /// Only a caller with this permission gets the original values back.
    pub reveal_requires: Option<String>,
    /// Mask on `in_country` routes too (default true).
    pub mask_in_country: Option<bool>,
}

/// One gateway key.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyConfig {
    /// Who the key acts as.
    pub subject: String,
    /// The key's SHA-256, 64 hex characters.
    pub sha256: String,
    /// Its permissions.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Its tenant.
    pub tenant: Option<String>,
    /// Its area.
    pub area: Option<String>,
}

impl Config {
    /// Parse a configuration file's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Read and parse a configuration file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn residency(s: &str, what: &str) -> Result<Residency, String> {
    match s {
        "abroad" => Ok(Residency::Abroad),
        "in_country" => Ok(Residency::InCountry),
        other => Err(format!(
            "{what}: residency must be `abroad` or `in_country`, not `{other}`"
        )),
    }
}

/// A configured provider: its chat side and, when its API has one, its
/// embeddings side (OpenAI-compatible APIs do; Anthropic's does not).
type Built = (Arc<dyn ChatProvider>, Option<Arc<dyn Embedder>>);

fn provider(
    p: &ProviderConfig,
    transport: &Arc<dyn HttpTransport>,
    creds: &Arc<dyn CredentialSource>,
) -> Result<Built, String> {
    let what = format!("provider `{}`", p.name);
    let endpoint = |url: &str| {
        if p.plaintext {
            Endpoint::allow_plaintext(url)
        } else {
            Endpoint::https(url)
        }
        .map_err(|e| format!("{what}: {e}"))
    };
    let t = transport.clone();
    let c = creds.clone();
    Ok(match p.kind.as_str() {
        "anthropic" => {
            let mut a = Anthropic::new(t, c).named(&p.name);
            if let Some(url) = &p.endpoint {
                a = a.with_endpoint(endpoint(url)?);
            }
            if let Some(cred) = &p.credential {
                a = a.with_credential(cred);
            }
            (Arc::new(a), None)
        }
        "openai" | "gemini" => {
            let default_credential = if p.kind == "openai" {
                "OPENAI_API_KEY"
            } else {
                "GEMINI_API_KEY"
            };
            let mut o = match &p.endpoint {
                Some(url) => OpenAiCompatible::new(&p.name, endpoint(url)?, t, c)
                    .with_credential(default_credential),
                None if p.kind == "openai" => OpenAiCompatible::openai(t, c),
                None => OpenAiCompatible::gemini(t, c),
            };
            if let Some(cred) = &p.credential {
                o = o.with_credential(cred);
            }
            // The presets name themselves `openai`/`gemini`: the configured name wins.
            let o = Arc::new(o);
            (
                Arc::new(NamedProvider {
                    name: p.name.clone(),
                    inner: o.clone(),
                }),
                Some(o as Arc<dyn Embedder>),
            )
        }
        "openai-compatible" => {
            let url = p
                .endpoint
                .as_deref()
                .ok_or_else(|| format!("{what}: `endpoint` is required"))?;
            let mut o = OpenAiCompatible::new(&p.name, endpoint(url)?, t, c);
            if let Some(cred) = &p.credential {
                o = o.with_credential(cred);
            }
            let o = Arc::new(o);
            (o.clone(), Some(o as Arc<dyn Embedder>))
        }
        other => return Err(format!("{what}: unknown kind `{other}`")),
    })
}

/// A provider under another name.
struct NamedProvider {
    name: String,
    inner: Arc<dyn ChatProvider>,
}

#[async_trait]
impl ChatProvider for NamedProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn complete(
        &self,
        model: &str,
        request: &seiva_llm::ChatRequest,
    ) -> Result<seiva_llm::ChatResponse, seiva_llm::LlmError> {
        self.inner.complete(model, request).await
    }
    async fn stream(
        &self,
        model: &str,
        request: &seiva_llm::ChatRequest,
    ) -> Result<seiva_llm::EventStream, seiva_llm::LlmError> {
        self.inner.stream(model, request).await
    }
}

/// Build the app: the endpoints plus `/health/live` and `/health/ready`.
pub fn build(
    config: &Config,
    transport: Arc<dyn HttpTransport>,
    credentials: Arc<dyn CredentialSource>,
    audit: Arc<dyn AuditSink>,
    clock: Arc<dyn Clock>,
    health: seiva_http::Health,
) -> Result<axum::Router, String> {
    let mut rb = Router::builder().attempt_timeout(Duration::from_secs(
        config.limits.attempt_timeout_secs.max(1),
    ));
    for p in &config.providers {
        let (chat, embedder) = provider(p, &transport, &credentials)?;
        let where_ = residency(&p.residency, &format!("provider `{}`", p.name))?;
        rb = rb.provider(chat, where_);
        if let Some(e) = embedder {
            rb = rb.embedder(&p.name, e, where_);
        }
    }
    for r in &config.routes {
        let kind = match r.kind.as_deref() {
            None => RouteKind::Chat,
            Some(k) => RouteKind::parse(k).ok_or_else(|| {
                format!(
                    "route `{}`: kind must be `chat`, `embedding` or `chat+embedding`, not `{k}`",
                    r.name
                )
            })?,
        };
        let mut route = Route::new(
            &r.name,
            residency(&r.residency, &format!("route `{}`", r.name))?,
        )
        .with_kind(kind);
        if let Some(p) = &r.requires {
            route = route.requires(p);
        }
        if r.public {
            route = route.public();
        }
        for up in &r.upstreams {
            let (provider, model) = up.split_once('/').ok_or_else(|| {
                format!(
                    "route `{}`: upstream `{up}` must be `provider/model`",
                    r.name
                )
            })?;
            route = route.upstream(provider, model);
        }
        rb = rb.route(route);
    }
    let llm = rb.build().map_err(|e| e.to_string())?;

    let mut budget =
        InMemoryBudget::new(clock, Duration::from_secs(config.budget.period_secs.max(1)));
    if let Some(n) = config.budget.per_principal {
        budget = budget.per(BudgetScope::Principal, n);
    }
    if let Some(n) = config.budget.per_tenant {
        budget = budget.per(BudgetScope::Tenant, n);
    }
    if let Some(n) = config.budget.per_area {
        budget = budget.per(BudgetScope::Area, n);
    }

    let g = &config.gateway;
    let mut gb = Gateway::builder(llm).budget(Arc::new(budget)).audit(audit);
    for area in &g.areas_in_country {
        gb = gb.area_in_country(area);
    }
    for (tool, permission) in &g.tools {
        gb = gb.tool(tool, permission);
    }
    if let Some(p) = &g.any_tool {
        gb = gb.any_tool(p);
    }
    gb = gb.reveal(match (g.reveal.as_deref(), &g.reveal_requires) {
        (_, Some(p)) => RevealPolicy::Requires(p.clone()),
        (None | Some("caller"), None) => RevealPolicy::Caller,
        (Some("never"), None) => RevealPolicy::Never,
        (Some(other), None) => {
            return Err(format!(
                "gateway.reveal must be `caller` or `never`, not `{other}`"
            ))
        }
    });
    if let Some(on) = g.mask_in_country {
        gb = gb.mask_in_country(on);
    }
    let gateway = gb.build().map_err(|e| e.to_string())?;

    let mut keys = GatewayKeys::new();
    for k in &config.keys {
        let principal = match &k.tenant {
            Some(t) => Principal::in_tenant(&k.subject, t, k.permissions.clone()),
            None => Principal::new(&k.subject, k.permissions.clone()),
        };
        keys.add(KeyEntry {
            sha256: k.sha256.clone(),
            principal,
            area: k.area.clone(),
        })
        .map_err(|e| e.to_string())?;
    }
    if keys.is_empty() {
        return Err("no `[[keys]]`: every request would be refused".into());
    }
    let options = ServerOptions {
        default_max_tokens: config.limits.default_max_tokens,
        max_tokens_cap: config.limits.max_tokens_cap,
        body_limit: config.limits.body_limit_mb.max(1) * 1024 * 1024,
    };
    let state = ServerState::new(gateway, keys).with_options(options);
    Ok(router(state).merge(seiva_http::health_router(health)))
}

/// The wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// The audit record as JSON lines, to stdout or appended to a file. A failed
/// write is an error, so the gateway withholds the answer (fail closed).
pub struct JsonLinesAudit {
    target: Mutex<Target>,
}

enum Target {
    Stdout,
    File(std::fs::File),
}

impl JsonLinesAudit {
    /// `stdout`, or a file path opened for appending (created 0600 on Unix).
    pub fn open(spec: &str) -> Result<Self, String> {
        let target = if spec == "stdout" {
            Target::Stdout
        } else {
            let path = PathBuf::from(spec);
            let mut opts = std::fs::OpenOptions::new();
            opts.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            Target::File(
                opts.open(&path)
                    .map_err(|e| format!("audit file {}: {e}", path.display()))?,
            )
        };
        Ok(Self {
            target: Mutex::new(target),
        })
    }
}

/// One audit entry as a JSON object.
pub fn entry_json(entry: &AuditEntry) -> serde_json::Value {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let details: serde_json::Map<String, serde_json::Value> = entry
        .details
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    serde_json::json!({
        "ts": ts,
        "actor": entry.actor,
        "tenant": entry.tenant,
        "operation": entry.operation,
        "object": entry.object,
        "outcome": entry.outcome,
        "request_id": entry.request_id,
        "details": details,
    })
}

#[async_trait]
impl AuditSink for JsonLinesAudit {
    async fn record(&self, entry: &AuditEntry) -> PortResult<()> {
        let line = format!("{}\n", entry_json(entry));
        let mut target = self.target.lock().map_err(|_| "audit sink poisoned")?;
        match &mut *target {
            Target::Stdout => {
                let mut out = std::io::stdout().lock();
                out.write_all(line.as_bytes())?;
                out.flush()?;
            }
            Target::File(f) => {
                f.write_all(line.as_bytes())?;
                f.flush()?;
            }
        }
        Ok(())
    }
}
