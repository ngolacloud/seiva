# seiva-mcp-server

Module facades as [MCP](https://modelcontextprotocol.io) tools, on behalf of a person —
phase G4 of [ADR 0048](https://github.com/ngolacloud/seiva/blob/main/docs/adr/0048-ai-gateway-and-llm-harness.md).
DORM-free.

```rust
#[seiva::facade(version = "v1")]
pub trait Payments {
    /// Refund a captured payment, fully or partly.
    #[requires("payments.refund")]
    #[tool(effect = "write")]
    async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<RefundDto, PaymentsError>;
}

#[seiva::tools]                         // next to #[seiva::controller]
impl Payments for PaymentsService { /* … */ }

let tools = ToolSet::new().with(payments_tools(service))?;
let mcp = McpServer::builder(tools, authenticator, audit_sink)
    .resource("https://api.example.com/mcp")
    .build();
let app = mcp.router();
```

| What | How |
|---|---|
| The tool | name `payments_refund`, description = the doc comment, input schema = the parameter's `WireSchema`, permission = `#[requires]`, effect = `#[tool(effect)]` — all from the facade descriptor |
| Nothing by default | only `#[tool]` methods are tools; the effect and a description are compile-time requirements; a write tool cannot be `#[public]` |
| On behalf of the person | the caller is the person whose token the MCP client presents (the same `Authenticator` as the HTTP API); `#[requires]` is enforced here; the method runs with their `Ctx`, so the tenant scope applies as over HTTP |
| Writes wait | the first call returns an approval request; the app shows it (`GET /mcp/approvals/{id}`) and the person confirms it (`POST`, their token); the model's next call with the same arguments and `"_approval": id` runs it, once. `auto_approve(tool, permission)` is the audited exception |
| Audit | every call, approval and refusal; a write is recorded before it runs and does not run if that fails; arguments as a digest only |
| Transport | Streamable HTTP, JSON responses, protocol `2025-06-18`: `initialize`, `ping`, `tools/list`, `tools/call`; `Origin` checked; 401 points to `/.well-known/oauth-protected-resource` (RFC 9728) |

## License

MIT OR Apache-2.0.
