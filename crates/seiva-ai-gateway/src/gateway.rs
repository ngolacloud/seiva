//! The gateway: the stages around every model call, in a fixed order.
//!
//! ```text
//! Identify → Authorize → Budget.reserve → Mask → Guard.input
//!         → model (the route)
//!         → Guard.output → Unmask → Budget.settle → Audit
//! ```
//!
//! The order is a correctness invariant (masking before anything leaves, the
//! budget held before the spend, the audit last so it records the outcome), so it
//! is fixed here and asserted by a test; what each stage does is pluggable. Every
//! stage fails closed, and every outcome — refusals included — is audited.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures_util::StreamExt;
use seiva_identity::Principal;
use seiva_llm::{
    ChatRequest, ChatResponse, Content, EventStream, LlmError, Residency, Router, StreamEvent,
    Usage,
};
use seiva_ports::{AuditEntry, AuditSink};
use tracing::Instrument;

use crate::budget::{BudgetError, BudgetKey, BudgetScope, BudgetStore, Reservation};
use crate::detect::{self, Detector};
use crate::error::GatewayError;
use crate::guard::{
    label_untrusted, request_chars, CredentialLeak, GuardContext, Guardrail, Verdict,
};
use crate::mask::{unmask_response, Masker, Vault};

/// Who is calling: the authenticated principal (ADR 0006) and, if the product
/// knows it, the caller's area (department) and the request's correlation id.
#[derive(Debug, Clone)]
pub struct Caller {
    /// The principal, as the transport's authentication produced it.
    pub principal: Principal,
    /// The caller's area, for residency and budget.
    pub area: Option<String>,
    /// The request id, for the audit record.
    pub request_id: Option<String>,
}

impl Caller {
    /// A caller with no area and no request id.
    pub fn new(principal: Principal) -> Self {
        Self {
            principal,
            area: None,
            request_id: None,
        }
    }

    /// The caller works in `area`.
    #[must_use]
    pub fn in_area(mut self, area: impl Into<String>) -> Self {
        self.area = Some(area.into());
        self
    }

    /// Correlate the audit record with `id`.
    #[must_use]
    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }
}

/// What to do with a credential found in a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CredentialPolicy {
    /// Refuse the request (OWASP LLM07). The default.
    #[default]
    Refuse,
    /// Mask it like personal data.
    Mask,
}

/// Who gets the original values back in the answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RevealPolicy {
    /// The caller: every masked value came from the caller's own request. The
    /// default.
    #[default]
    Caller,
    /// Only a caller holding this permission; others see the placeholders.
    Requires(String),
    /// Nobody: the answer keeps the placeholders.
    Never,
}

/// Whether a failed audit write withholds the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditMode {
    /// The answer is withheld when the record cannot be written. The default.
    #[default]
    Required,
    /// The answer goes out; the failure is logged.
    BestEffort,
}

/// A gateway stage, as recorded in [`CallReport::stages`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// The caller is authenticated.
    Identify,
    /// The caller may use the route, its tools and its residency.
    Authorize,
    /// Units held in every budget.
    BudgetReserve,
    /// Sensitive values replaced, untrusted content labelled.
    Mask,
    /// Input guardrails passed.
    GuardInput,
    /// The model answered.
    Model,
    /// Output guardrails passed.
    GuardOutput,
    /// Placeholders restored (or kept, by the reveal policy).
    Unmask,
    /// The real usage counted.
    BudgetSettle,
    /// The record written.
    Audit,
}

/// What happened in one call, for the caller and for tests.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CallReport {
    /// The stages that ran, in order.
    pub stages: Vec<Stage>,
    /// How many distinct values were masked, per label.
    pub masked: BTreeMap<String, usize>,
    /// Whether tool output was labelled as untrusted.
    pub untrusted_labelled: bool,
    /// `provider/model` that answered.
    pub upstream: Option<String>,
    /// The model's usage.
    pub usage: Usage,
    /// The units held before the call.
    pub reserved: u64,
}

/// A gated answer.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Gated {
    /// The answer, unmasked by the reveal policy.
    pub response: ChatResponse,
    /// What happened.
    pub report: CallReport,
}

/// The gateway could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot build the gateway: {0}")]
pub struct BuildError(String);

struct Inner {
    router: Router,
    masker: Masker,
    guards: Vec<Arc<dyn Guardrail>>,
    budget: Arc<dyn BudgetStore>,
    audit: Arc<dyn AuditSink>,
    tools: HashMap<String, String>,
    in_country_areas: HashSet<String>,
    mask_in_country: bool,
    credentials: CredentialPolicy,
    reveal: RevealPolicy,
    audit_mode: AuditMode,
    store_masked_text: bool,
}

/// The gateway. Cheap to clone.
#[derive(Clone)]
pub struct Gateway {
    inner: Arc<Inner>,
}

/// Builds a [`Gateway`]. A budget store and an audit sink are required: a
/// gateway without them could not meter or record, so it is not built.
pub struct GatewayBuilder {
    router: Router,
    detectors: Vec<Arc<dyn Detector>>,
    guards: Vec<Arc<dyn Guardrail>>,
    budget: Option<Arc<dyn BudgetStore>>,
    audit: Option<Arc<dyn AuditSink>>,
    tools: HashMap<String, String>,
    in_country_areas: HashSet<String>,
    mask_in_country: bool,
    credentials: CredentialPolicy,
    reveal: RevealPolicy,
    audit_mode: AuditMode,
    store_masked_text: bool,
}

impl GatewayBuilder {
    /// Add a detector after the built-in ones.
    #[must_use]
    pub fn detector(mut self, detector: Arc<dyn Detector>) -> Self {
        self.detectors.push(detector);
        self
    }

    /// Replace every detector, built-ins included (the credential detector too:
    /// keep [`detect::builtin`]'s first entry if the credential rules should hold).
    #[must_use]
    pub fn detectors(mut self, detectors: Vec<Arc<dyn Detector>>) -> Self {
        self.detectors = detectors;
        self
    }

    /// Add a guardrail; they run in the order added.
    #[must_use]
    pub fn guard(mut self, guard: Arc<dyn Guardrail>) -> Self {
        self.guards.push(guard);
        self
    }

    /// The budget store.
    #[must_use]
    pub fn budget(mut self, store: Arc<dyn BudgetStore>) -> Self {
        self.budget = Some(store);
        self
    }

    /// The audit sink.
    #[must_use]
    pub fn audit(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(sink);
        self
    }

    /// A tool the model may be offered, and the permission a caller must hold for
    /// it. A tool not registered here is refused.
    #[must_use]
    pub fn tool(mut self, name: impl Into<String>, permission: impl Into<String>) -> Self {
        self.tools.insert(name.into(), permission.into());
        self
    }

    /// Callers in `area` may only use `in_country` routes.
    #[must_use]
    pub fn area_in_country(mut self, area: impl Into<String>) -> Self {
        self.in_country_areas.insert(area.into());
        self
    }

    /// Whether to mask on `in_country` routes too (default `true`). On `abroad`
    /// routes masking always runs.
    #[must_use]
    pub fn mask_in_country(mut self, on: bool) -> Self {
        self.mask_in_country = on;
        self
    }

    /// What to do with a credential in a request.
    #[must_use]
    pub fn credentials(mut self, policy: CredentialPolicy) -> Self {
        self.credentials = policy;
        self
    }

    /// Who gets the original values back.
    #[must_use]
    pub fn reveal(mut self, policy: RevealPolicy) -> Self {
        self.reveal = policy;
        self
    }

    /// Whether a failed audit write withholds the answer.
    #[must_use]
    pub fn audit_mode(mut self, mode: AuditMode) -> Self {
        self.audit_mode = mode;
        self
    }

    /// Also record the **masked** request and answer text in the audit entry
    /// (off by default).
    #[must_use]
    pub fn store_masked_text(mut self, on: bool) -> Self {
        self.store_masked_text = on;
        self
    }

    /// Build the gateway.
    pub fn build(self) -> Result<Gateway, BuildError> {
        let budget = self
            .budget
            .ok_or_else(|| BuildError("a budget store is required".into()))?;
        let audit = self
            .audit
            .ok_or_else(|| BuildError("an audit sink is required".into()))?;
        if let Some((tool, _)) = self.tools.iter().find(|(_, p)| p.trim().is_empty()) {
            return Err(BuildError(format!("tool `{tool}` has an empty permission")));
        }
        let masker = Masker::new(self.detectors);
        let mut guards: Vec<Arc<dyn Guardrail>> = vec![Arc::new(CredentialLeak {
            masker: masker.clone(),
        })];
        guards.extend(self.guards);
        Ok(Gateway {
            inner: Arc::new(Inner {
                router: self.router,
                masker,
                guards,
                budget,
                audit,
                tools: self.tools,
                in_country_areas: self.in_country_areas,
                mask_in_country: self.mask_in_country,
                credentials: self.credentials,
                reveal: self.reveal,
                audit_mode: self.audit_mode,
                store_masked_text: self.store_masked_text,
            }),
        })
    }
}

/// What the input stages hand to the model.
struct Prepared {
    masked: ChatRequest,
    vault: Vault,
    reservation: Reservation,
}

impl Gateway {
    /// Start a gateway over `router`, with the built-in detectors.
    pub fn builder(router: Router) -> GatewayBuilder {
        GatewayBuilder {
            router,
            detectors: detect::builtin(),
            guards: Vec::new(),
            budget: None,
            audit: None,
            tools: HashMap::new(),
            in_country_areas: HashSet::new(),
            mask_in_country: true,
            credentials: CredentialPolicy::Refuse,
            reveal: RevealPolicy::Caller,
            audit_mode: AuditMode::Required,
            store_masked_text: false,
        }
    }

    /// A whole answer from `route`, through every stage.
    pub async fn complete(
        &self,
        caller: &Caller,
        route: &str,
        request: ChatRequest,
    ) -> Result<Gated, GatewayError> {
        let span = tracing::info_span!(
            "gen_ai.chat",
            gen_ai.operation.name = "chat",
            seiva.route = route,
            gen_ai.response.model = tracing::field::Empty,
            gen_ai.usage.input_tokens = tracing::field::Empty,
            gen_ai.usage.output_tokens = tracing::field::Empty,
            seiva.outcome = tracing::field::Empty,
        );
        let mut report = CallReport::default();
        let outcome = self
            .run(caller, route, &request, &mut report)
            .instrument(span.clone())
            .await;
        span.record(
            "seiva.outcome",
            outcome.as_ref().map_or_else(|f| f.0.code(), |_| "ok"),
        );
        let (response, masked_answer, masked_request) = match outcome {
            Ok(pair) => pair,
            Err(failed) => {
                let (e, masked_request) = *failed;
                // A refusal is audited too; its error stands whatever the audit does.
                let _ = self
                    .record(
                        caller,
                        route,
                        &request,
                        masked_request.as_ref(),
                        None,
                        &report,
                        Err(&e),
                    )
                    .await;
                return Err(e);
            }
        };
        span.record("gen_ai.usage.input_tokens", report.usage.total_input());
        span.record("gen_ai.usage.output_tokens", report.usage.output_tokens);
        span.record("gen_ai.response.model", response.model.as_str());
        self.finish_audit(
            caller,
            route,
            &request,
            Some(&masked_request),
            Some(&masked_answer),
            &mut report,
        )
        .await?;
        Ok(Gated { response, report })
    }

    async fn finish_audit(
        &self,
        caller: &Caller,
        route: &str,
        request: &ChatRequest,
        masked_request: Option<&ChatRequest>,
        masked_answer: Option<&ChatResponse>,
        report: &mut CallReport,
    ) -> Result<(), GatewayError> {
        report.stages.push(Stage::Audit);
        match self
            .record(
                caller,
                route,
                request,
                masked_request,
                masked_answer,
                report,
                Ok(()),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if self.inner.audit_mode == AuditMode::Required => {
                Err(GatewayError::AuditFailed(e))
            }
            Err(e) => {
                tracing::error!(route, error = %e, "the audit record could not be written");
                Ok(())
            }
        }
    }

    /// Identify, authorize, reserve, mask and guard the input.
    async fn prepare(
        &self,
        caller: &Caller,
        route: &str,
        request: &ChatRequest,
        report: &mut CallReport,
    ) -> Result<Prepared, GatewayError> {
        let inner = &self.inner;
        let principal = &caller.principal;

        // Identify.
        if principal.subject.is_empty() {
            return Err(GatewayError::Unauthenticated);
        }
        report.stages.push(Stage::Identify);

        // Authorize: the route, the caller's residency, every tool offered.
        let info = inner
            .router
            .route_info(route)
            .ok_or_else(|| GatewayError::Model(LlmError::UnknownRoute(route.to_owned())))?;
        if let Some(permission) = info.permission() {
            if !principal.has(permission) {
                return Err(GatewayError::Forbidden {
                    route: route.to_owned(),
                    permission: permission.to_owned(),
                });
            }
        }
        if let Some(area) = &caller.area {
            if inner.in_country_areas.contains(area) && info.residency() == Residency::Abroad {
                return Err(GatewayError::Residency {
                    route: route.to_owned(),
                    area: area.clone(),
                });
            }
        }
        for tool in &request.tools {
            match inner.tools.get(&tool.name) {
                Some(p) if principal.has(p) => {}
                _ => {
                    return Err(GatewayError::ToolNotAllowed {
                        tool: tool.name.clone(),
                    })
                }
            }
        }
        let abroad = info.residency() == Residency::Abroad;
        report.stages.push(Stage::Authorize);

        // Budget: hold the estimate in every budget of this caller.
        let mut keys = vec![BudgetKey::new(
            BudgetScope::Principal,
            principal.subject.clone(),
        )];
        if let Some(t) = &principal.tenant {
            keys.push(BudgetKey::new(BudgetScope::Tenant, t.clone()));
        }
        if let Some(a) = &caller.area {
            keys.push(BudgetKey::new(BudgetScope::Area, a.clone()));
        }
        let estimate = (request_chars(request) as u64) / 4 + 1 + u64::from(request.max_tokens);
        let reservation = inner
            .budget
            .reserve(&keys, estimate)
            .await
            .map_err(|e| match e {
                BudgetError::Exceeded(key) => GatewayError::BudgetExceeded {
                    scope: key.to_string(),
                },
                BudgetError::Unavailable(m) => GatewayError::BudgetUnavailable(m),
            })?;
        report.reserved = estimate;
        report.stages.push(Stage::BudgetReserve);

        // Mask, then label untrusted content.
        let refuse = inner.credentials == CredentialPolicy::Refuse;
        let mut vault = Vault::default();
        let masked = if abroad || inner.mask_in_country {
            match inner.masker.mask_request(request, &mut vault, refuse) {
                Ok(m) => m,
                Err(_) => return Err(self.release(&reservation, credential_blocked()).await),
            }
        } else {
            if refuse && carries_credential(&inner.masker, request) {
                return Err(self.release(&reservation, credential_blocked()).await);
            }
            request.clone()
        };
        let mut masked = masked;
        report.untrusted_labelled = label_untrusted(&mut masked);
        report.masked = vault.counts().clone();
        report.stages.push(Stage::Mask);

        // Input guardrails.
        let ctx = GuardContext { route, principal };
        for g in &inner.guards {
            match g.check_input(&ctx, &masked).await {
                Ok(Verdict::Allow) => {}
                Ok(Verdict::Block(reason)) => {
                    return Err(self
                        .release(
                            &reservation,
                            GatewayError::Blocked {
                                stage: g.name().to_owned(),
                                reason,
                            },
                        )
                        .await)
                }
                Err(reason) => {
                    return Err(self
                        .release(
                            &reservation,
                            GatewayError::GuardFailed {
                                guard: g.name().to_owned(),
                                reason,
                            },
                        )
                        .await)
                }
            }
        }
        report.stages.push(Stage::GuardInput);
        Ok(Prepared {
            masked,
            vault,
            reservation,
        })
    }

    /// Give the held units back and pass `error` on.
    async fn release(&self, reservation: &Reservation, error: GatewayError) -> GatewayError {
        let _ = self.inner.budget.settle(reservation, 0).await;
        error
    }

    /// Output guards and the tool rule on the masked answer.
    async fn check_output(
        &self,
        route: &str,
        principal: &Principal,
        request: &ChatRequest,
        answer: &ChatResponse,
    ) -> Result<(), GatewayError> {
        let offered: HashSet<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
        if let Some((_, name, _)) = answer
            .tool_uses()
            .into_iter()
            .find(|(_, n, _)| !offered.contains(n))
        {
            return Err(GatewayError::Blocked {
                stage: "tool_policy".into(),
                reason: format!("the model called `{name}`, which it was not offered"),
            });
        }
        let ctx = GuardContext { route, principal };
        for g in &self.inner.guards {
            match g.check_output(&ctx, answer).await {
                Ok(Verdict::Allow) => {}
                Ok(Verdict::Block(reason)) => {
                    return Err(GatewayError::Blocked {
                        stage: g.name().to_owned(),
                        reason,
                    })
                }
                Err(reason) => {
                    return Err(GatewayError::GuardFailed {
                        guard: g.name().to_owned(),
                        reason,
                    })
                }
            }
        }
        Ok(())
    }

    fn reveals_to(&self, principal: &Principal) -> bool {
        match &self.inner.reveal {
            RevealPolicy::Caller => true,
            RevealPolicy::Requires(p) => principal.has(p),
            RevealPolicy::Never => false,
        }
    }

    #[allow(clippy::type_complexity)]
    async fn run(
        &self,
        caller: &Caller,
        route: &str,
        request: &ChatRequest,
        report: &mut CallReport,
    ) -> Result<(ChatResponse, ChatResponse, ChatRequest), Box<(GatewayError, Option<ChatRequest>)>>
    {
        let Prepared {
            masked,
            vault,
            reservation,
        } = self
            .prepare(caller, route, request, report)
            .await
            .map_err(|e| Box::new((e, None)))?;
        let inner = &self.inner;

        // The model.
        let answer = match inner.router.complete(route, &masked).await {
            Ok(a) => a,
            Err(e) => {
                let _ = inner.budget.settle(&reservation, 0).await;
                return Err(Box::new((GatewayError::Model(e), Some(masked))));
            }
        };
        report.upstream = answer.upstream.clone();
        report.usage = answer.usage;
        report.stages.push(Stage::Model);
        let spent = answer.usage.total_input() + answer.usage.output_tokens;

        // Output guards: on refusal the spend still counts.
        if let Err(e) = self
            .check_output(route, &caller.principal, request, &answer)
            .await
        {
            let _ = inner.budget.settle(&reservation, spent).await;
            return Err(Box::new((e, Some(masked))));
        }
        report.stages.push(Stage::GuardOutput);

        // Unmask.
        let response = if self.reveals_to(&caller.principal) {
            unmask_response(&answer, &vault)
        } else {
            answer.clone()
        };
        report.stages.push(Stage::Unmask);

        // Settle.
        if let Err(e) = inner.budget.settle(&reservation, spent).await {
            let message = match e {
                BudgetError::Unavailable(m) => m,
                BudgetError::Exceeded(k) => format!("settling {k}"),
            };
            return Err(Box::new((
                GatewayError::BudgetUnavailable(message),
                Some(masked),
            )));
        }
        report.stages.push(Stage::BudgetSettle);
        Ok((response, answer, masked))
    }

    /// Write the audit entry. Never a value: counts, names, codes — and the
    /// masked text only when the product opted in.
    #[allow(clippy::too_many_arguments)]
    async fn record(
        &self,
        caller: &Caller,
        route: &str,
        request: &ChatRequest,
        masked_request: Option<&ChatRequest>,
        masked_answer: Option<&ChatResponse>,
        report: &CallReport,
        outcome: Result<(), &GatewayError>,
    ) -> Result<(), String> {
        let code = match outcome {
            Ok(()) => "ok",
            Err(e) => e.code(),
        };
        let p = &caller.principal;
        let mut entry = AuditEntry::new(p.subject.clone(), "llm.call", code)
            .in_tenant(p.tenant.clone())
            .on_object(Some(route.to_owned()))
            .with_request_id(caller.request_id.clone());
        if let Some(area) = &caller.area {
            entry = entry.with_detail("area", area.clone());
        }
        if let Some(up) = &report.upstream {
            entry = entry.with_detail("upstream", up.clone());
        }
        let u = report.usage;
        entry = entry
            .with_detail("usage_in", u.input_tokens.to_string())
            .with_detail("usage_out", u.output_tokens.to_string())
            .with_detail("usage_cache_read", u.cache_read_tokens.to_string())
            .with_detail("usage_cache_write", u.cache_write_tokens.to_string());
        if !report.masked.is_empty() {
            let masked = report
                .masked
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",");
            entry = entry.with_detail("masked", masked);
        }
        if !request.tools.is_empty() {
            entry = entry.with_detail(
                "tools_offered",
                request
                    .tools
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        if let Some(answer) = masked_answer {
            let called: Vec<&str> = answer.tool_uses().into_iter().map(|(_, n, _)| n).collect();
            if !called.is_empty() {
                entry = entry.with_detail("tools_called", called.join(","));
            }
        }
        if let Err(e) = outcome {
            entry = entry.with_detail("reason", e.to_string());
        }
        if self.inner.store_masked_text {
            if let Some(last) = masked_request.and_then(|r| r.messages.last()) {
                entry = entry.with_detail("request_masked", cut(&text_of(&last.content), 2000));
            }
            if let Some(a) = masked_answer {
                entry = entry.with_detail("answer_masked", cut(&a.text(), 2000));
            }
        }
        self.inner
            .audit
            .record(&entry.redacted())
            .await
            .map_err(|e| e.to_string())
    }

    /// A streamed answer from `route`, through every stage.
    ///
    /// Text is held back by up to 256 characters (and never released in the
    /// middle of a placeholder) so a credential can be caught before it reaches
    /// the caller; the arguments of a tool call stream masked and arrive unmasked
    /// in [`StreamEvent::Completed`], after the output guards ran on the whole
    /// answer. An error ends the stream.
    pub async fn stream(
        &self,
        caller: &Caller,
        route: &str,
        request: ChatRequest,
    ) -> Result<EventStream, GatewayError> {
        let mut report = CallReport::default();
        let prepared = match self.prepare(caller, route, &request, &mut report).await {
            Ok(p) => p,
            Err(e) => {
                let _ = self
                    .record(caller, route, &request, None, None, &report, Err(&e))
                    .await;
                return Err(e);
            }
        };
        let upstream = match self.inner.router.stream(route, &prepared.masked).await {
            Ok(s) => s,
            Err(e) => {
                let _ = self.inner.budget.settle(&prepared.reservation, 0).await;
                let e = GatewayError::Model(e);
                let _ = self
                    .record(
                        caller,
                        route,
                        &request,
                        Some(&prepared.masked),
                        None,
                        &report,
                        Err(&e),
                    )
                    .await;
                return Err(e);
            }
        };
        let reveal = self.reveals_to(&caller.principal);
        let state = StreamState {
            gateway: self.clone(),
            caller: caller.clone(),
            route: route.to_owned(),
            request,
            prepared,
            report,
            upstream,
            reveal,
            pending: BTreeMap::new(),
            out: VecDeque::new(),
            done: false,
        };
        Ok(Box::pin(futures_util::stream::unfold(
            state,
            |mut st| async move {
                loop {
                    if let Some(next) = st.out.pop_front() {
                        return Some((next, st));
                    }
                    if st.done {
                        return None;
                    }
                    st.step().await;
                }
            },
        )))
    }
}

/// How much text the stream holds back.
const HOLD: usize = 256;

struct Pending {
    /// Masked text not yet released.
    text: String,
    /// The last masked text released, to catch a credential across the boundary.
    tail: String,
}

struct StreamState {
    gateway: Gateway,
    caller: Caller,
    route: String,
    request: ChatRequest,
    prepared: Prepared,
    report: CallReport,
    upstream: EventStream,
    reveal: bool,
    pending: BTreeMap<usize, Pending>,
    out: VecDeque<Result<StreamEvent, LlmError>>,
    done: bool,
}

impl StreamState {
    fn release(&mut self, index: usize, all: bool) {
        let Some(p) = self.pending.get_mut(&index) else {
            return;
        };
        let mut cut_at = if all {
            p.text.len()
        } else {
            p.text.len().saturating_sub(HOLD)
        };
        while !p.text.is_char_boundary(cut_at) {
            cut_at -= 1;
        }
        // Never release half a placeholder.
        if !all {
            if let Some(open) = p.text[..cut_at].rfind('‹') {
                if !p.text[open..cut_at].contains('›') {
                    cut_at = open;
                }
            }
        }
        if cut_at == 0 {
            return;
        }
        let chunk: String = p.text.drain(..cut_at).collect();
        p.tail.push_str(&chunk);
        if p.tail.len() > HOLD {
            let mut start = p.tail.len() - HOLD;
            while !p.tail.is_char_boundary(start) {
                start += 1;
            }
            p.tail.drain(..start);
        }
        let text = if self.reveal {
            self.prepared.vault.unmask(&chunk)
        } else {
            chunk
        };
        self.out
            .push_back(Ok(StreamEvent::TextDelta { index, text }));
    }

    async fn fail(&mut self, error: GatewayError, spent: u64) {
        let _ = self
            .gateway
            .inner
            .budget
            .settle(&self.prepared.reservation, spent)
            .await;
        let _ = self
            .gateway
            .record(
                &self.caller,
                &self.route,
                &self.request,
                Some(&self.prepared.masked),
                None,
                &self.report,
                Err(&error),
            )
            .await;
        self.out.clear();
        self.out
            .push_back(Err(LlmError::Decode(format!("gateway: {error}"))));
        self.done = true;
    }

    async fn step(&mut self) {
        match self.upstream.next().await {
            None => {
                let e = GatewayError::Model(LlmError::Transport(
                    "the stream ended without an answer".into(),
                ));
                self.fail(e, self.prepared.reservation.units).await;
            }
            Some(Err(e)) => {
                self.fail(GatewayError::Model(e), self.prepared.reservation.units)
                    .await
            }
            Some(Ok(StreamEvent::TextDelta { index, text })) => {
                let p = self.pending.entry(index).or_insert_with(|| Pending {
                    text: String::new(),
                    tail: String::new(),
                });
                p.text.push_str(&text);
                let window = format!("{}{}", p.tail, p.text);
                if self.gateway.inner.masker.has_credential(&window) {
                    let e = GatewayError::Blocked {
                        stage: "credential_leak".into(),
                        reason: "the answer carries a credential".into(),
                    };
                    self.fail(e, self.prepared.reservation.units).await;
                    return;
                }
                self.release(index, false);
            }
            Some(Ok(StreamEvent::Completed(answer))) => self.complete(answer).await,
            // Tool starts and argument fragments pass through (masked).
            Some(Ok(other)) => self.out.push_back(Ok(other)),
        }
    }

    async fn complete(&mut self, answer: ChatResponse) {
        let gw = self.gateway.clone();
        let spent = answer.usage.total_input() + answer.usage.output_tokens;
        self.report.upstream = answer.upstream.clone();
        self.report.usage = answer.usage;
        self.report.stages.push(Stage::Model);
        if let Err(e) = gw
            .check_output(&self.route, &self.caller.principal, &self.request, &answer)
            .await
        {
            self.fail(e, spent).await;
            return;
        }
        self.report.stages.push(Stage::GuardOutput);
        let indexes: Vec<usize> = self.pending.keys().copied().collect();
        for i in indexes {
            self.release(i, true);
        }
        let response = if self.reveal {
            unmask_response(&answer, &self.prepared.vault)
        } else {
            answer.clone()
        };
        self.report.stages.push(Stage::Unmask);
        if let Err(e) = gw
            .inner
            .budget
            .settle(&self.prepared.reservation, spent)
            .await
        {
            let message = match e {
                BudgetError::Unavailable(m) => m,
                BudgetError::Exceeded(k) => format!("settling {k}"),
            };
            self.fail(GatewayError::BudgetUnavailable(message), 0).await;
            return;
        }
        self.report.stages.push(Stage::BudgetSettle);
        let mut report = std::mem::take(&mut self.report);
        if let Err(e) = gw
            .finish_audit(
                &self.caller,
                &self.route,
                &self.request,
                Some(&self.prepared.masked),
                Some(&answer),
                &mut report,
            )
            .await
        {
            self.out.clear();
            self.out
                .push_back(Err(LlmError::Decode(format!("gateway: {e}"))));
            self.done = true;
            return;
        }
        self.out.push_back(Ok(StreamEvent::Completed(response)));
        self.done = true;
    }
}

fn credential_blocked() -> GatewayError {
    GatewayError::Blocked {
        stage: "mask".into(),
        reason: "the request carries a credential".into(),
    }
}

fn carries_credential(masker: &Masker, request: &ChatRequest) -> bool {
    let mut vault = Vault::default();
    masker.mask_request(request, &mut vault, true).is_err()
}

fn text_of(parts: &[Content]) -> String {
    parts
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } => Some(text.as_str()),
            Content::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
