//! The loop: ask, run the tools the model calls, feed back, repeat — bounded.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use seiva::contract::ToolEffect;
use seiva_ai_gateway::{Caller, Gateway, GatewayError};
use seiva_llm::{ChatRequest, Content, Message, Role, ToolSpec, Usage};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::tool::{AgentCtx, Tool, ToolCall, ToolOutput};

/// The bounds of one run (OWASP LLM06, LLM10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Model calls per run.
    pub max_steps: u32,
    /// Tokens per run (input, cached and output together).
    pub max_tokens: u64,
    /// `max_tokens` of each model call.
    pub tokens_per_call: u32,
    /// Wall time of the whole run.
    pub max_duration: Duration,
    /// Time budget of one tool call.
    pub tool_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 8,
            max_tokens: 200_000,
            tokens_per_call: 4_096,
            max_duration: Duration::from_secs(300),
            tool_timeout: Duration::from_secs(60),
        }
    }
}

/// Stops a run from outside (a closed tab, a cancel button). Cheap to clone.
#[derive(Clone, Default)]
pub struct Cancel(Arc<(AtomicBool, Notify)>);

impl Cancel {
    /// A signal that has not fired.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fire it.
    pub fn cancel(&self) {
        self.0 .0.store(true, Ordering::SeqCst);
        self.0 .1.notify_waiters();
    }

    /// Whether it fired.
    pub fn is_cancelled(&self) -> bool {
        self.0 .0.load(Ordering::SeqCst)
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.0 .1.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// A hook's verdict before a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    /// Go on.
    Continue,
    /// Do not run it; the model is told why.
    Deny(String),
}

/// Sees every tool call: before (and may stop it) and after.
#[async_trait]
pub trait Hook: Send + Sync {
    /// Before the call. Allows by default.
    async fn before_tool(&self, _ctx: &AgentCtx<'_>, _call: &ToolCall) -> HookDecision {
        HookDecision::Continue
    }

    /// After the call, with what it returned.
    async fn after_tool(&self, _ctx: &AgentCtx<'_>, _call: &ToolCall, _output: &ToolOutput) {}
}

/// Decides whether a write may run. The host asks the person.
#[async_trait]
pub trait Approver: Send + Sync {
    /// Whether `call` of `tool` may run now.
    async fn approve(&self, ctx: &AgentCtx<'_>, tool: &ToolSpec, call: &ToolCall) -> bool;
}

/// The default: no write runs (an agent without a person to ask changes nothing).
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyWrites;

#[async_trait]
impl Approver for DenyWrites {
    async fn approve(&self, _: &AgentCtx<'_>, _: &ToolSpec, _: &ToolCall) -> bool {
        false
    }
}

/// Every write runs. For tests and for unattended jobs whose writes are
/// pre-approved by design — an explicit choice.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproveWrites;

#[async_trait]
impl Approver for ApproveWrites {
    async fn approve(&self, _: &AgentCtx<'_>, _: &ToolSpec, _: &ToolCall) -> bool {
        true
    }
}

/// Where conversations are kept, per owner (the principal) and conversation id.
#[async_trait]
pub trait Memory: Send + Sync {
    /// The conversation so far (empty if new).
    async fn load(&self, owner: &str, conversation: &str) -> Result<Vec<Message>, String>;
    /// Replace it.
    async fn save(
        &self,
        owner: &str,
        conversation: &str,
        messages: &[Message],
    ) -> Result<(), String>;
}

/// An in-process [`Memory`]. One process only.
#[derive(Clone, Default)]
pub struct InMemoryMemory(Arc<Mutex<Conversations>>);

/// Conversations by `(owner, conversation id)`.
type Conversations = HashMap<(String, String), Vec<Message>>;

impl InMemoryMemory {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Memory for InMemoryMemory {
    async fn load(&self, owner: &str, conversation: &str) -> Result<Vec<Message>, String> {
        let map = self.0.lock().map_err(|_| "memory poisoned".to_owned())?;
        Ok(map
            .get(&(owner.to_owned(), conversation.to_owned()))
            .cloned()
            .unwrap_or_default())
    }

    async fn save(
        &self,
        owner: &str,
        conversation: &str,
        messages: &[Message],
    ) -> Result<(), String> {
        let mut map = self.0.lock().map_err(|_| "memory poisoned".to_owned())?;
        map.insert(
            (owner.to_owned(), conversation.to_owned()),
            messages.to_vec(),
        );
        Ok(())
    }
}

/// Why a run stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopCause {
    /// The model answered without calling a tool.
    Answered,
    /// It used every step.
    StepLimit,
    /// It used its token budget.
    TokenLimit,
    /// It ran out of time.
    TimeLimit,
}

/// What happened to one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallOutcome {
    /// It ran and succeeded.
    Ok,
    /// It ran and failed.
    Failed,
    /// A hook stopped it.
    Denied(String),
    /// The approver refused the write.
    NotApproved,
    /// No such tool.
    Unknown,
    /// It outlived its time budget.
    TimedOut,
}

/// One tool call, for the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    /// The step it happened in.
    pub step: u32,
    /// The tool.
    pub tool: String,
    /// Read or write (`None` for an unknown tool).
    pub effect: Option<ToolEffect>,
    /// What happened.
    pub outcome: CallOutcome,
}

/// The result of a run.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AgentRun {
    /// The model's last text.
    pub answer: String,
    /// Why it stopped.
    pub stop: StopCause,
    /// Model calls made.
    pub steps: u32,
    /// Tokens used, all calls together.
    pub usage: Usage,
    /// Every tool call, in order.
    pub calls: Vec<CallRecord>,
    /// The whole conversation, as saved.
    pub messages: Vec<Message>,
}

/// Why a run failed outright.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentError {
    /// The gateway refused or failed a model call.
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    /// The run was cancelled.
    #[error("the run was cancelled")]
    Cancelled,
    /// The memory store failed.
    #[error("memory: {0}")]
    Memory(String),
}

/// The agent could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot build the agent: {0}")]
pub struct BuildError(String);

/// A configured agent. Cheap to clone; one agent serves many runs and callers.
#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

struct Inner {
    gateway: Gateway,
    route: String,
    system: Option<String>,
    tools: BTreeMap<String, Arc<dyn Tool>>,
    hooks: Vec<Arc<dyn Hook>>,
    approver: Arc<dyn Approver>,
    memory: Arc<dyn Memory>,
    limits: Limits,
}

/// Builds an [`Agent`].
pub struct AgentBuilder {
    gateway: Gateway,
    route: String,
    system: Option<String>,
    tools: Vec<Arc<dyn Tool>>,
    hooks: Vec<Arc<dyn Hook>>,
    approver: Arc<dyn Approver>,
    memory: Arc<dyn Memory>,
    limits: Limits,
}

impl AgentBuilder {
    /// The system prompt.
    #[must_use]
    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Add a tool.
    #[must_use]
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Add tools.
    #[must_use]
    pub fn tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Add a hook; hooks run in the order added.
    #[must_use]
    pub fn hook(mut self, hook: Arc<dyn Hook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Who approves writes ([`DenyWrites`] by default).
    #[must_use]
    pub fn approver(mut self, approver: Arc<dyn Approver>) -> Self {
        self.approver = approver;
        self
    }

    /// Where conversations are kept ([`InMemoryMemory`] by default).
    #[must_use]
    pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = memory;
        self
    }

    /// The bounds of each run.
    #[must_use]
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Build it. Two tools with the same name are refused.
    pub fn build(self) -> Result<Agent, BuildError> {
        let mut tools = BTreeMap::new();
        for t in self.tools {
            let name = t.spec().name;
            if tools.insert(name.clone(), t).is_some() {
                return Err(BuildError(format!("tool `{name}` is defined twice")));
            }
        }
        if self.limits.max_steps == 0 {
            return Err(BuildError("max_steps must be at least 1".into()));
        }
        Ok(Agent {
            inner: Arc::new(Inner {
                gateway: self.gateway,
                route: self.route,
                system: self.system,
                tools,
                hooks: self.hooks,
                approver: self.approver,
                memory: self.memory,
                limits: self.limits,
            }),
        })
    }
}

impl Agent {
    /// An agent that asks `route` through `gateway` — every model call goes
    /// through the gateway's stages (identity, budget, masking, guardrails,
    /// audit). The gateway must allow the tools offered (registered, or
    /// `any_tool`).
    pub fn builder(gateway: Gateway, route: impl Into<String>) -> AgentBuilder {
        AgentBuilder {
            gateway,
            route: route.into(),
            system: None,
            tools: Vec::new(),
            hooks: Vec::new(),
            approver: Arc::new(DenyWrites),
            memory: Arc::new(InMemoryMemory::new()),
            limits: Limits::default(),
        }
    }

    /// The tools offered to the model.
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.inner.tools.values().map(|t| t.spec()).collect()
    }

    /// Run once for `caller`: `input` is the person's message. With a
    /// `conversation` id the run continues that conversation (kept per caller —
    /// another principal never loads it) and saves it at the end.
    pub async fn run(
        &self,
        caller: &Caller,
        conversation: Option<&str>,
        input: impl Into<String>,
        cancel: &Cancel,
    ) -> Result<AgentRun, AgentError> {
        let inner = &self.inner;
        let owner = caller.principal.subject.clone();
        let mut messages = match conversation {
            Some(id) => inner
                .memory
                .load(&owner, id)
                .await
                .map_err(AgentError::Memory)?,
            None => Vec::new(),
        };
        messages.push(Message::user(input));
        let deadline = Instant::now() + inner.limits.max_duration;
        let specs = self.tool_specs();
        let mut usage = Usage::default();
        let mut calls = Vec::new();
        let mut answer = String::new();
        let mut steps = 0;
        let stop = loop {
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            if steps >= inner.limits.max_steps {
                break StopCause::StepLimit;
            }
            if usage.total_input() + usage.output_tokens >= inner.limits.max_tokens {
                break StopCause::TokenLimit;
            }
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
            else {
                break StopCause::TimeLimit;
            };
            steps += 1;
            let mut request = ChatRequest::new(inner.limits.tokens_per_call);
            if let Some(s) = &inner.system {
                request = request.system(s.clone());
            }
            for m in &messages {
                request = request.message(m.clone());
            }
            for s in &specs {
                request = request.tool(s.clone());
            }
            let gated = tokio::select! {
                r = tokio::time::timeout(left, inner.gateway.complete(caller, &inner.route, request)) => match r {
                    Err(_) => break StopCause::TimeLimit,
                    Ok(r) => r?,
                },
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            };
            let response = gated.response;
            usage += response.usage;
            answer = response.text();
            messages.push(response.to_message());
            let tool_calls: Vec<ToolCall> = response
                .tool_uses()
                .into_iter()
                .map(|(id, name, input)| ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    input: input.clone(),
                })
                .collect();
            if tool_calls.is_empty() {
                break StopCause::Answered;
            }
            let ctx = AgentCtx {
                caller,
                step: steps,
            };
            let mut results = Vec::new();
            for call in &tool_calls {
                let (output, record) = tokio::select! {
                    r = self.run_tool(&ctx, call) => r,
                    _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                };
                calls.push(record);
                results.push(Content::tool_result(
                    call.id.clone(),
                    output.content,
                    output.is_error,
                ));
            }
            messages.push(Message::new(Role::User, results));
        };
        if let Some(id) = conversation {
            inner
                .memory
                .save(&owner, id, &messages)
                .await
                .map_err(AgentError::Memory)?;
        }
        Ok(AgentRun {
            answer,
            stop,
            steps,
            usage,
            calls,
            messages,
        })
    }

    async fn run_tool(&self, ctx: &AgentCtx<'_>, call: &ToolCall) -> (ToolOutput, CallRecord) {
        let inner = &self.inner;
        let record = |effect: Option<ToolEffect>, outcome: CallOutcome| CallRecord {
            step: ctx.step,
            tool: call.name.clone(),
            effect,
            outcome,
        };
        let Some(tool) = inner.tools.get(&call.name) else {
            return (
                ToolOutput::error(format!("unknown tool `{}`", call.name)),
                record(None, CallOutcome::Unknown),
            );
        };
        let effect = tool.effect();
        for hook in &inner.hooks {
            if let HookDecision::Deny(reason) = hook.before_tool(ctx, call).await {
                let out = ToolOutput::error(format!("not run: {reason}"));
                return (out, record(Some(effect), CallOutcome::Denied(reason)));
            }
        }
        if effect == ToolEffect::Write && !inner.approver.approve(ctx, &tool.spec(), call).await {
            let out = ToolOutput::error("not run: the person did not approve this change");
            for hook in &inner.hooks {
                hook.after_tool(ctx, call, &out).await;
            }
            return (out, record(Some(effect), CallOutcome::NotApproved));
        }
        let (out, outcome) = match tokio::time::timeout(
            inner.limits.tool_timeout,
            tool.call(ctx, call.input.clone()),
        )
        .await
        {
            Err(_) => (
                ToolOutput::error(format!(
                    "the tool took longer than {} seconds",
                    inner.limits.tool_timeout.as_secs()
                )),
                CallOutcome::TimedOut,
            ),
            Ok(out) if out.is_error => (out, CallOutcome::Failed),
            Ok(out) => (out, CallOutcome::Ok),
        };
        for hook in &inner.hooks {
            hook.after_tool(ctx, call, &out).await;
        }
        tracing::debug!(tool = %call.name, step = ctx.step, ?outcome, "tool call");
        (out, record(Some(effect), outcome))
    }
}
