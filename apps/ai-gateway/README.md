# ai-gateway — the runnable AI gateway

The server shape of [ADR 0048](../../docs/adr/0048-ai-gateway-and-llm-harness.md) (G3):
one TOML file in, the OpenAI- and Anthropic-compatible endpoints out, on `seiva-server`
with `/health/live` and `/health/ready`. Every call — chat or embeddings — goes through the
gateway stages of `seiva-ai-gateway` (identity, route permission, residency, budget,
masking, guardrails, audit). Model calls go out through `seiva-llm-reqwest`.

```console
$ cargo run --release --manifest-path apps/ai-gateway/Cargo.toml -- check gateway.toml
$ head -c 32 /dev/urandom | base64 > desk.key          # issue a gateway key
$ seiva-ai-gateway hash-key < desk.key                 # its SHA-256 goes in the file
$ ANTHROPIC_API_KEY=… seiva-ai-gateway serve gateway.toml
```

## Configuration

```toml
listen = "0.0.0.0:4000"
audit = "/var/log/seiva-ai-gateway/audit.jsonl"   # or "stdout"; JSON lines, 0600

[limits]                      # optional
max_tokens_cap = 64000
body_limit_mb = 8

[budget]
period_secs = 86400
per_principal = 2000000
per_area = 20000000

[[providers]]
name = "anthropic"
kind = "anthropic"            # anthropic | openai | gemini | openai-compatible
residency = "abroad"          # abroad | in_country
# credential = "ANTHROPIC_API_KEY"   (the default; read from the environment)

[[providers]]
name = "local"
kind = "openai-compatible"    # vLLM, Ollama, or an OpenAI-compatible gateway already running
endpoint = "http://10.0.0.20:11434/v1"
plaintext = true
residency = "in_country"

[[routes]]
name = "support-chat"         # what clients send as `model`
residency = "abroad"
requires = "llm.support"
upstreams = ["anthropic/claude-sonnet-5-5", "local/llama3.3"]

[[routes]]
name = "hr-assistant"
residency = "in_country"      # can only use in_country providers
requires = "llm.hr"
upstreams = ["local/llama3.3"]

[[routes]]
name = "docs-embed"           # for POST /v1/embeddings
kind = "embedding"            # chat (default) | embedding | chat+embedding
residency = "in_country"
requires = "llm.embed"
upstreams = ["local/bge-m3"]  # openai, gemini, openai-compatible providers embed; anthropic does not

[gateway]
areas_in_country = ["hr"]                    # callers of these areas never reach abroad routes
tools = { customer_lookup = "customers.read" }
# any_tool = "llm.code"                     # coding agents offer the tools they run themselves
# reveal_requires = "pii.reveal"            # who gets masked values back (default: the caller)

[[keys]]
subject = "support-desk"
sha256 = "…"                                # never the key itself
permissions = ["llm.support", "customers.read"]
tenant = "acme"
area = "support"
```

Unknown fields are refused (a typo is an error, not a silently ignored setting). A file
with no `[[keys]]` is refused: every request would be.

## Pointing clients at it

- **OpenAI clients** (SDKs, Open WebUI, LibreChat…): base URL `http://<host>:4000/v1`, API
  key = the gateway key, model = a route name.
- **Embedding clients** (an OpenAI SDK's `embeddings.create`, a RAG indexer, `seiva-llm`'s
  `OpenAiCompatible` embedder): the same base URL and key, model = an **embedding** route.
  `GET /v1/models` lists each route's `capabilities` (`chat`, `embeddings`).
- **Anthropic clients**: base URL `http://<host>:4000`, the gateway key as `x-api-key` (or
  `Authorization: Bearer`), model = a route name. A coding agent such as Claude Code needs
  the model it requests mapped to a route name, and a key whose permissions include the
  `any_tool` permission — its tools run on the developer's machine.

The provider keys stay in the gateway's environment; clients never see them. Put TLS in
front (or terminate it with `seiva-server`'s mTLS, ADR 0047).
