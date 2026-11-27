# Agents

> **Status: phase G5 of [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md)**, in
> `seiva-agent`.

An agent is a loop: the model answers or asks for a tool; the tool runs; its result goes
back; repeat. `seiva-agent` gives you that loop with the guardrails already in place, and
leaves the parts that are yours — tools, approvals, hooks, memory — as traits.

## Build one

```rust
let agent = Agent::builder(gateway, "support-agent")
    .system("You help the support desk.")
    .tool(Arc::new(FnTool::read("customer_lookup", "Find a customer by id.", schema, lookup)))
    .tools(facades::tools(&payments_toolset))
    .approver(Arc::new(AskInTheApp))
    .build()?;

let run = agent.run(&caller, Some("conversation-42"), "Refund order 1043", &cancel).await?;
println!("{} ({:?} after {} steps)", run.answer, run.stop, run.steps);
```

Every model call goes through the [gateway](language-models.md): the caller's identity,
budget, masking, guardrails and audit apply to each step. The gateway must allow the tools
offered (registered with `.tool(name, permission)`, or `any_tool`).

## Bounded

`Limits` caps the steps, the tokens, the wall time and each tool's run time; a `Cancel`
stops a run from outside. A limit ends the run with its `StopCause` (`StepLimit`,
`TokenLimit`, `TimeLimit`) and what the model said so far — never an endless loop.

## Tools

| Source | How |
|---|---|
| A function | `FnTool::read(..)` / `FnTool::write(..)` |
| Module facades, in-process | `facades::tools(&tool_set)` — the `#[tool]` methods of [agent tools](agent-tools.md), called as the person, their `#[requires]` checked |
| An MCP server | `mcp::tools(client, &pins, trust_on_first_use)` |

A write tool runs only if the `Approver` agrees. The default, `DenyWrites`, refuses every
write: an agent with nobody to ask changes nothing. `Hook`s see each call before (and may
stop it) and after.

## MCP servers, pinned

Only configured servers are reached, through the `HttpTransport` port — so the
deployment's outbound guard applies. When a server lists its tools, each tool's name,
description, input schema and annotations are hashed. A tool whose hash is not the pinned
one is **not offered** to the model: a new tool waits for approval (unless
`trust_on_first_use`), and a tool whose description **changed** since it was approved
always waits — the "rug pull", where a server swaps a harmless description for
instructions, does not reach the model. A tool the server does not mark read-only is
treated as a write. Results are data: the gateway labels them as untrusted.

## Memory

`Memory` keeps conversations per principal and conversation id; another person never
loads one. `InMemoryMemory` ships; a persistent store implements the trait.

## Related

- [Language models](language-models.md) · [Agent tools (MCP)](agent-tools.md)

---

*Part of the [Seiva documentation](../index.md).*
