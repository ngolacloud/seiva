//! Why the gateway refused or stopped a call.

use seiva_llm::LlmError;

/// A call the gateway did not let through, or could not finish.
///
/// Every stage fails closed: a variant here means nothing was returned to the
/// caller, and — except for [`GatewayError::Model`] and the output stages —
/// nothing was sent to a model. No variant carries a prompt, an answer, a
/// masked value or a credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GatewayError {
    /// The caller is anonymous. A model call always has a person or a service
    /// behind it.
    #[error("the caller is not authenticated")]
    Unauthenticated,
    /// The caller may not use this route.
    #[error("the caller lacks `{permission}` for route `{route}`")]
    Forbidden {
        /// The route.
        route: String,
        /// The permission it requires.
        permission: String,
    },
    /// The request offers a tool the caller may not call, or one the gateway
    /// does not know.
    #[error("tool `{tool}` is not allowed for this caller")]
    ToolNotAllowed {
        /// The tool.
        tool: String,
    },
    /// The caller's area keeps its data in the country, and the route does not.
    #[error("area `{area}` keeps its data in the country; route `{route}` sends it abroad")]
    Residency {
        /// The route.
        route: String,
        /// The caller's area.
        area: String,
    },
    /// A budget is spent.
    #[error("the {scope} budget is spent")]
    BudgetExceeded {
        /// Which budget: `principal:<id>`, `tenant:<id>`, `area:<id>`.
        scope: String,
    },
    /// The budget store could not answer. An unmetered call is spend nobody
    /// approved, so the call does not go out.
    #[error("the budget store is unavailable: {0}")]
    BudgetUnavailable(String),
    /// A stage blocked the call: a credential in the request, a guardrail, a tool
    /// the model was not offered.
    #[error("blocked by {stage}: {reason}")]
    Blocked {
        /// The stage or guardrail that blocked it.
        stage: String,
        /// Why, without the offending content.
        reason: String,
    },
    /// A guardrail failed to run. It counts as a block (fail closed).
    #[error("guardrail `{guard}` failed: {reason}")]
    GuardFailed {
        /// The guardrail.
        guard: String,
        /// Its error.
        reason: String,
    },
    /// The audit record could not be written, so the answer is withheld.
    #[error("the audit record could not be written: {0}")]
    AuditFailed(String),
    /// The model call itself failed (after the input stages passed).
    #[error(transparent)]
    Model(#[from] LlmError),
}

impl GatewayError {
    /// A short, stable code for the audit record and for metrics.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden { .. } => "forbidden",
            Self::ToolNotAllowed { .. } => "tool_not_allowed",
            Self::Residency { .. } => "residency",
            Self::BudgetExceeded { .. } => "budget_exceeded",
            Self::BudgetUnavailable(_) => "budget_unavailable",
            Self::Blocked { .. } => "blocked",
            Self::GuardFailed { .. } => "guard_failed",
            Self::AuditFailed(_) => "audit_failed",
            Self::Model(_) => "model_error",
        }
    }
}
