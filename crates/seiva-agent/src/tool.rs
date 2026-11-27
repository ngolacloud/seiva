//! Tools: what the agent may call.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use seiva::contract::ToolEffect;
use seiva_ai_gateway::Caller;
use seiva_llm::ToolSpec;
use serde_json::Value;

/// What a tool sees of the run: who it acts for, and which step it is.
#[derive(Debug, Clone, Copy)]
pub struct AgentCtx<'a> {
    /// The person the agent acts for.
    pub caller: &'a Caller,
    /// The step of the loop (from 1).
    pub step: u32,
}

/// One call the model asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The call id (echoed in the result).
    pub id: String,
    /// The tool.
    pub name: String,
    /// Its arguments.
    pub input: Value,
}

/// What a tool returns to the model: text, and whether it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// The text the model reads (the gateway labels it as untrusted data).
    pub content: String,
    /// Whether the call failed.
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful result.
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    /// A failed one.
    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

/// A tool the agent may call. Implement it for anything; [`FnTool`] wraps a
/// function, `facades::tools` wraps module facades, `mcp::tools` wraps the tools
/// of an MCP server.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The name, description and input schema the model sees.
    fn spec(&self) -> ToolSpec;

    /// Whether calling it changes something. A write goes through the agent's
    /// approver first.
    fn effect(&self) -> ToolEffect;

    /// Run it. Failures are a [`ToolOutput::error`], fed back to the model.
    async fn call(&self, ctx: &AgentCtx<'_>, input: Value) -> ToolOutput;
}

type BoxFn = dyn Fn(Caller, Value) -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
    + Send
    + Sync;

/// A tool from a function.
pub struct FnTool {
    spec: ToolSpec,
    effect: ToolEffect,
    f: Arc<BoxFn>,
}

impl FnTool {
    /// A tool that only reads.
    pub fn read<F, Fut>(name: &str, description: &str, input_schema: Value, f: F) -> Self
    where
        F: Fn(Caller, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self::new(name, description, input_schema, ToolEffect::Read, f)
    }

    /// A tool that changes something (it goes through the approver).
    pub fn write<F, Fut>(name: &str, description: &str, input_schema: Value, f: F) -> Self
    where
        F: Fn(Caller, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self::new(name, description, input_schema, ToolEffect::Write, f)
    }

    fn new<F, Fut>(
        name: &str,
        description: &str,
        input_schema: Value,
        effect: ToolEffect,
        f: F,
    ) -> Self
    where
        F: Fn(Caller, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self {
            spec: ToolSpec::new(name, description, input_schema),
            effect,
            f: Arc::new(move |caller, input| Box::pin(f(caller, input))),
        }
    }
}

#[async_trait]
impl Tool for FnTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn effect(&self) -> ToolEffect {
        self.effect
    }

    async fn call(&self, ctx: &AgentCtx<'_>, input: Value) -> ToolOutput {
        match (self.f)(ctx.caller.clone(), input).await {
            Ok(text) => ToolOutput::ok(text),
            Err(text) => ToolOutput::error(text),
        }
    }
}

/// Module facades as in-process tools: the `#[tool]` methods of a
/// `seiva_mcp_server::ToolSet`, called as the person the agent acts for (their
/// `#[requires]` checked, their `Ctx`). No HTTP hop in a monolith.
#[cfg(feature = "facades")]
pub mod facades {
    use std::sync::Arc;

    use async_trait::async_trait;
    use seiva::contract::ToolEffect;
    use seiva_llm::ToolSpec;
    use seiva_mcp_server::ToolSet;
    use serde_json::Value;

    use super::{AgentCtx, Tool, ToolOutput};

    struct FacadeTool {
        set: ToolSet,
        spec: ToolSpec,
        effect: ToolEffect,
    }

    #[async_trait]
    impl Tool for FacadeTool {
        fn spec(&self) -> ToolSpec {
            self.spec.clone()
        }

        fn effect(&self) -> ToolEffect {
            self.effect
        }

        async fn call(&self, ctx: &AgentCtx<'_>, input: Value) -> ToolOutput {
            match self
                .set
                .invoke(&self.spec.name, &ctx.caller.principal, input)
                .await
            {
                Ok(v) => ToolOutput::ok(v.to_string()),
                Err(e) => ToolOutput::error(e.to_string()),
            }
        }
    }

    /// Every tool of `set`.
    pub fn tools(set: &ToolSet) -> Vec<Arc<dyn Tool>> {
        set.tools()
            .map(|t| {
                Arc::new(FacadeTool {
                    set: set.clone(),
                    spec: ToolSpec::new(t.name.clone(), t.description, t.input_schema.clone()),
                    effect: t.effect,
                }) as Arc<dyn Tool>
            })
            .collect()
    }
}
