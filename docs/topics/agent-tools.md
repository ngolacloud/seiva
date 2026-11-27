# Agent tools (MCP)

> **Status: phase G4 of [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md).** Facade
> methods become tools with `#[tool]` and `#[seiva::tools]`; `seiva-mcp-server` serves
> them over MCP.

A module's facade already drives its HTTP routes, its gRPC service and its OpenAPI
document. The same contract now drives its **agent tools**: an assistant — an IDE, a
corporate chat, an agent built on Seiva — calls a module the way a person would, with
that person's permissions, and changes nothing without them.

## Mark the methods

Nothing is a tool by default. A method opts in, and says what calling it does:

```rust
#[seiva::facade(version = "v1")]
pub trait Payments {
    /// Show a payment by its id.
    #[requires("payments.read")]
    #[tool(effect = "read")]
    async fn show(&self, ctx: &Ctx, id: String) -> Result<PaymentView, PaymentsError>;

    /// Refund a captured payment, fully or partly.
    #[requires("payments.refund")]
    #[tool(effect = "write")]
    async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<RefundDto, PaymentsError>;
}
```

The build fails when a `#[tool]` has no `effect`, when it has no description (the doc
comment, or `description = ".."`), and when a write tool is `#[public]`.

## Generate and serve

```rust
#[seiva::controller]
#[seiva::tools]
impl Payments for PaymentsService { /* … */ }

let tools = ToolSet::new().with(payments_tools(service.clone()))?;
let mcp = McpServer::builder(tools, authenticator, audit_sink)
    .resource("https://api.example.com/mcp")
    .authorization_server("https://auth.example.com")
    .build();
let app = app.merge(mcp.router());
```

The tool is `payments_refund`; its input schema is `RefundPayment`'s `WireSchema` (a
parameter that is not an object becomes one argument named after it); its permission is
the method's `#[requires]`.

## On behalf of the person

The MCP client presents **the person's own token**, checked by the same `Authenticator`
as the HTTP API. A tool the person may not use is neither listed nor callable — it is
reported as unknown. The method runs with that person's `Ctx`, so the tenant scope (ADR
0005) applies exactly as over HTTP. There is no broad agent token to steal or misuse.

## Writes wait for a person

The first call to a write tool does nothing: it returns an approval request. The app
that hosts the agent — never the model — shows it to the person
(`GET /mcp/approvals/{id}`: the tool, its description, the exact arguments) and, if they
agree, confirms it with their token (`POST /mcp/approvals/{id}`). The model then calls the
tool again with the same arguments and `"_approval": "<id>"`, and it runs once. Different
arguments, another person, an expired or reused approval: refused.

For a write that a role may run unattended, `auto_approve("payments_refund",
"payments.refund.auto")` is the explicit, audited exception.

## Audited

Every call, approval and refusal goes to the `AuditSink`. A write is recorded **before**
it runs and does not run if that record fails. Arguments appear only as a digest.

## Related

- [Language models](language-models.md) · [Authorization](authorization.md) · [Security](security.md)

---

*Part of the [Seiva documentation](../index.md).*
