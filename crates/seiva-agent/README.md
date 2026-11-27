# seiva-agent

The agent harness of [Delonix Seiva](https://github.com/ngolacloud/seiva) — phase G5 of
[ADR 0048](https://github.com/ngolacloud/seiva/blob/main/docs/adr/0048-ai-gateway-and-llm-harness.md).
A small loop a developer extends rather than rewrites: ask the model through the AI
gateway, run the tools it calls, feed the results back, repeat — bounded.

```rust
let agent = Agent::builder(gateway, "support-agent")
    .system("You help the support desk.")
    .tool(Arc::new(FnTool::read("customer_lookup", "Find a customer by id.", schema, lookup)))
    .tools(facades::tools(&payments_toolset))      // module facades, in-process
    .tools(github_tools)                            // an allowlisted MCP server
    .approver(Arc::new(MyDialog))                   // who approves writes (default: nobody)
    .limits(Limits { max_steps: 8, ..Limits::default() })
    .build()?;

let run = agent.run(&caller, Some("conversation-42"), "Refund order 1043", &cancel).await?;
```

| Piece | What it does |
|---|---|
| Bounds | `Limits`: steps, tokens, wall time, each tool's timeout; `Cancel` stops a run. A limit ends the run with its `StopCause` (OWASP LLM06, LLM10) |
| The gateway | every model call is a `seiva-ai-gateway` call: identity, budget, masking, guardrails and audit apply to each step, and tool results reach the model labelled untrusted |
| Tools | `FnTool` (a function), `facades::tools` (module facades in-process, their `#[requires]` checked as the person), `mcp::tools` (an MCP server) |
| Writes | a write tool runs only if the `Approver` agrees; the default `DenyWrites` refuses them all |
| Hooks | `Hook::before_tool` (may deny) and `after_tool` see every call |
| Memory | `Memory`, kept per principal: another person never loads a conversation |
| MCP client | allowlisted servers through the `HttpTransport` port (the egress guard applies); JSON or SSE responses, sessions; each tool's description and schema **pinned by hash** — a new or changed tool is not offered until approved; a tool not marked read-only is a write |

## License

MIT OR Apache-2.0.
