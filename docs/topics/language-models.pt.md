# Modelos de linguagem

> **Estado: fase G1 do [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md).** O port do
> modelo, as rotas e os codecs dos fornecedores estão no `seiva-llm`; as etapas do gateway
> (identidade, orçamento, mascaramento, guardrails, auditoria) são a fase G2.

Um caso de uso que precisa de um modelo depende de uma **rota**, nunca de um fornecedor. A
rota diz que upstreams a podem servir, por ordem de fallback, e para onde os dados podem ir.

## Rotas e residência

```rust
use seiva_llm::{Residency, Route, Router};

let router = Router::builder()
    .provider(anthropic, Residency::Abroad)   // processes data outside the country
    .provider(local, Residency::InCountry)    // a model on the organisation's network
    .route(Route::new("hr-assistant", Residency::InCountry).requires("llm.hr").upstream("local", "llama3.3"))
    .route(
        Route::new("support-chat", Residency::Abroad)
            .requires("llm.support")
            .upstream("anthropic", "claude-sonnet-5-5")
            .upstream("local", "llama3.3"),
    )
    .build()?;
```

O `build()` recusa uma tabela errada em vez de falhar na altura do pedido:

- uma rota `in_country` com um upstream `abroad` — os dados pessoais poderiam sair do país;
- uma rota sem `.requires("permission")` nem `.public()` — a mesma regra fail-closed de um
  método de facade sem `#[requires]` ([ADR 0019](../adr/0019-fail-closed-authorization.md));
- um fornecedor desconhecido, uma rota sem upstream, um nome repetido.

## Chamar um modelo

```rust
use seiva_llm::{ChatModel, ChatRequest};

let chat = router.route("support-chat")?;          // a ChatModel
let answer = chat.complete(&ChatRequest::new(512).system("Be brief.").user("Olá")).await?;
println!("{} (served by {:?})", answer.text(), answer.upstream);
```

O `max_tokens` é obrigatório. O `stream(..)` devolve os pedaços e acaba sempre com
`StreamEvent::Completed`, com a resposta inteira. As ferramentas são `ToolSpec`; as chamadas
do modelo voltam como `Content::ToolUse`, e os resultados seguem como `Content::ToolResult`.

## Retry, timeouts e fallback

Cada tentativa tem um limite de tempo (`attempt_timeout`, 120 s por omissão; em streaming,
a espera por cada evento). As falhas transitórias — erro de transporte, timeout, `429`,
`5xx`, o `529` da Anthropic — repetem-se no mesmo upstream com o backoff de todo o framework
(`RetryPolicy`). Uma falha própria de um fornecedor (`401`, `403`, `404`) passa ao upstream
seguinte sem repetir. Um pedido mal formado (`400`, `422`) pára a rota: falharia em todo o
lado. Um stream só faz fallback antes do primeiro evento.

## Fornecedores

| Fornecedor | Construtor | Credencial |
|---|---|---|
| Anthropic | `Anthropic::new(transport, creds)` | `ANTHROPIC_API_KEY` |
| OpenAI | `OpenAiCompatible::openai(transport, creds)` | `OPENAI_API_KEY` |
| Google Gemini | `OpenAiCompatible::gemini(transport, creds)` | `GEMINI_API_KEY` |
| vLLM, Ollama, Azure, um gateway compatível com OpenAI | `OpenAiCompatible::new(name, endpoint, transport, creds)` | opcional, `.with_credential(..)` |

Um endpoint é `https`, a menos que a raiz de composição aceite um em texto claro com
`Endpoint::allow_plaintext(..)` — para um modelo numa rede que a organização controla.

## Credenciais e o transporte

As chaves dos fornecedores vêm de uma `CredentialSource` (`EnvCredentials` por omissão). Um
`Secret` imprime `***`, a chave segue num só cabeçalho marcado como sensível, e um erro nunca
a leva — mesmo quando um fornecedor ecoa o pedido.

Os codecs não ligam nenhum cliente HTTP: enviam pelo port `HttpTransport`. Em produção usa
o `seiva-llm-reqwest`, que nunca segue um redirect (levaria a chave para outro host). É
também nesse port que entra o cliente de saída guardado do
[ADR 0045](../adr/0045-egress-ssrf-guard.md).

## Testes

```rust
use seiva_testing::llm::ScriptedModel;   // feature "llm"

let model = Arc::new(ScriptedModel::new("fake").reply_text("Olá!"));
// … build the router over `model`, run the use case …
assert_eq!(model.calls()[0].request.messages.len(), 1);
```

O `ScriptedModel` toca um guião (respostas, chamadas a ferramentas, erros) e regista cada
pedido, para que um teste possa verificar o que o caso de uso enviou — e uma chamada a mais,
inesperada, falha.

## Relacionadas

- [ADR 0048 — gateway de IA e harness de LLM](../adr/0048-ai-gateway-and-llm-harness.md)
- [Configuration](configuration.md) · [Testing](testing.md) · [Security](security.md)

---

*Parte da [documentação do Seiva](../index.md).*
