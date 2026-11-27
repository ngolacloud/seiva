# Ferramentas para agentes (MCP)

> **Estado: fase G4 do [ADR 0048](../adr/0048-ai-gateway-and-llm-harness.md).** Os métodos
> de uma facade passam a ferramentas com `#[tool]` e `#[seiva::tools]`; o
> `seiva-mcp-server` serve-as por MCP.

A facade de um módulo já gera as rotas HTTP, o serviço gRPC e o documento OpenAPI. O mesmo
contrato gera agora as suas **ferramentas para agentes**: um assistente — um IDE, um chat
corporativo, um agente feito com o Seiva — chama um módulo como uma pessoa o faria, com as
permissões dessa pessoa, e não muda nada sem ela.

## Marcar os métodos

Nada é ferramenta por omissão. Um método opta por o ser e diz o que a chamada faz:

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

A compilação falha quando um `#[tool]` não tem `effect`, quando não tem descrição (o
comentário de documentação, ou `description = ".."`) e quando uma ferramenta de escrita é
`#[public]`.

## Gerar e servir

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

A ferramenta chama-se `payments_refund`; o esquema de entrada é o `WireSchema` de
`RefundPayment` (um parâmetro que não é um objecto passa a um argumento com o seu nome); a
permissão é o `#[requires]` do método.

## Em nome da pessoa

O cliente MCP apresenta **o token da própria pessoa**, verificado pelo mesmo
`Authenticator` da API HTTP. Uma ferramenta que a pessoa não pode usar não é listada nem
chamada — é dada como desconhecida. O método corre com o `Ctx` dessa pessoa, por isso o
scope do tenant (ADR 0005) aplica-se como no HTTP. Não há um token largo de agente para
roubar ou abusar.

## As escritas esperam por uma pessoa

A primeira chamada a uma ferramenta de escrita não faz nada: devolve um pedido de
aprovação. A aplicação que aloja o agente — nunca o modelo — mostra-o à pessoa
(`GET /mcp/approvals/{id}`: a ferramenta, a descrição, os argumentos exactos) e, se ela
concordar, confirma-o com o seu token (`POST /mcp/approvals/{id}`). O modelo chama então a
ferramenta outra vez com os mesmos argumentos e `"_approval": "<id>"`, e ela corre uma vez.
Argumentos diferentes, outra pessoa, uma aprovação expirada ou já usada: recusado.

Para uma escrita que um papel pode correr sem supervisão, `auto_approve("payments_refund",
"payments.refund.auto")` é a excepção explícita e auditada.

## Auditado

Cada chamada, aprovação e recusa vai para o `AuditSink`. Uma escrita é registada **antes**
de correr e não corre se esse registo falhar. Os argumentos aparecem só como um digest.

## Relacionadas

- [Language models](language-models.md) · [Authorization](authorization.md) · [Security](security.md)

---

*Parte da [documentação do Seiva](../index.md).*
