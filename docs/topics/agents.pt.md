# Agentes

> **Estado: fase G5 do [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md)**, no
> `seiva-agent`.

Um agente é um ciclo: o modelo responde ou pede uma ferramenta; a ferramenta corre; o
resultado volta; repete. O `seiva-agent` dá-te esse ciclo com as protecções já postas, e
deixa como traits as partes que são tuas — ferramentas, aprovações, hooks, memória.

## Construir um

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

Cada chamada ao modelo passa pelo [gateway](language-models.md): a identidade, o orçamento,
o mascaramento, os guardrails e a auditoria aplicam-se a cada passo. O gateway tem de
permitir as ferramentas oferecidas (registadas com `.tool(nome, permissão)`, ou o
`any_tool`).

## Com limites

O `Limits` limita os passos, os tokens, o tempo total e o tempo de cada ferramenta; um
`Cancel` pára uma execução de fora. Um limite termina a execução com a sua `StopCause`
(`StepLimit`, `TokenLimit`, `TimeLimit`) e o que o modelo disse até ali — nunca um ciclo sem
fim.

## Ferramentas

| Origem | Como |
|---|---|
| Uma função | `FnTool::read(..)` / `FnTool::write(..)` |
| Facades de módulos, no processo | `facades::tools(&tool_set)` — os métodos `#[tool]` das [ferramentas para agentes](agent-tools.md), chamados como a pessoa, com o `#[requires]` verificado |
| Um servidor MCP | `mcp::tools(client, &pins, trust_on_first_use)` |

Uma ferramenta de escrita só corre se o `Approver` concordar. O valor por omissão,
`DenyWrites`, recusa todas as escritas: um agente sem ninguém a quem perguntar não muda
nada. Os `Hook` vêem cada chamada antes (e podem pará-la) e depois.

## Servidores MCP, com as ferramentas fixadas

Só se chega aos servidores configurados, pelo port `HttpTransport` — por isso aplica-se a
guarda de saída do deployment. Quando um servidor lista as ferramentas, o nome, a
descrição, o esquema de entrada e as anotações de cada uma levam um hash. Uma ferramenta
cujo hash não é o fixado **não é oferecida** ao modelo: uma ferramenta nova espera por
aprovação (salvo com `trust_on_first_use`), e uma cuja descrição **mudou** depois de
aprovada espera sempre — a «rug pull», em que um servidor troca uma descrição inofensiva
por instruções, não chega ao modelo. Uma ferramenta que o servidor não marca como só de
leitura é tratada como escrita. Os resultados são dados: o gateway marca-os como não
confiáveis.

## Memória

A `Memory` guarda as conversas por principal e por id; outra pessoa nunca carrega uma
conversa alheia. Vem o `InMemoryMemory`; uma base persistente implementa o trait.

## Relacionadas

- [Language models](language-models.md) · [Agent tools (MCP)](agent-tools.md)

---

*Parte da [documentação do Seiva](../index.md).*
