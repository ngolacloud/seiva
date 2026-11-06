# O Seiva num relance

O Seiva permite-te escrever um backend como um **binário Rust compilado**:
tratamento rápido de pedidos com uma única fonte de verdade e documentação gerada,
mais as *baterias* que esperas — uma camada de dados, um sistema de autenticação,
segurança por omissão e um gerador de projectos.

## A grande ideia

**Um módulo é dono dos seus dados e expõe um único contrato legível por máquina.
Todo o transporte é derivado desse contrato — nunca escrito à mão.**

Escreves uma *fachada* — um trait versionado anotado com `#[seiva::facade]`:

```rust
#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
}
```

A partir dessa única declaração, o Seiva deriva:

- a **rota HTTP** (`POST /v1/notes`) via `#[seiva::controller]`;
- o **documento OpenAPI 3.1** — com as formas *reais* de pedido/resposta, porque
  os DTOs derivam [`WireSchema`](../reference/wire-schema.md) (sem envelope `cmd`,
  sem marcadores opacos);
- a **verificação de permissão** (`notes.create`), imposta antes de o handler
  correr;
- um documento de serviço **proto3** (`seiva proto`).

Sem reflection: está tudo conhecido em tempo de compilação, e cada macro tem um
equivalente documentado escrito à mão («sem magia escondida»).

## Um módulo são cinco crates

O compilador impõe a arquitectura — não consegues violar a regra de dependência e
mesmo assim compilar (ver [Arquitectura](../architecture.md)):

```
interface ─▶ application ─▶ domain
   │             │
   └─▶ facade ◀──┘        infrastructure ─▶ application
```

- **facade** — o contrato versionado + os DTOs de wire (`serde`, `Validate`, `WireSchema`).
- **domain** — entidades e regras; não depende de nada além do kernel partilhado.
- **application** — o caso de uso sobre *portas* só-trait; livre de transporte e de
  camada de dados, por isso testa-se em unidade sem servidor nem base de dados.
- **infrastructure** — adaptadores (em memória por omissão; suportados por DORM via
  uma feature).
- **interface** — o `#[seiva::controller]` e a geração do OpenAPI.

## O que recebes

- **Linguagem / runtime** — Rust, sem GC, um único binário compilado.
- **OpenAPI + UI de docs automáticos** — fiéis, gerados a partir do contrato.
- **Validação orientada a tipos** — `serde` + `Validate`, problemas RFC 9457.
- **ORM / camada de dados** — DORM, opcional e protegido por feature.
- **Geração de projecto** — `seiva project new` / `seiva module new`.
- **Autenticação incorporada** — JWT/OIDC verificado via `seiva-identity`, negação por omissão.
- **Build cloud-native** — distroless, não-root, `/health`.
- **Migrações e um admin** — em curso (ver [progresso](../progress.md)).

A aposta do Seiva: **uma experiência de desenvolvimento ergonómica e com baterias
incluídas, sem GC, implantada como um único binário distroless.** Para um boletim
honesto e medido contra outras stacks, ver o
[panorama competitivo](../competitive-landscape.md).

## O que existe hoje

O Seiva está em fase de fundação. A funcionar hoje: o pipeline
facade/controller/OpenAPI (com schemas `WireSchema` fiéis), autenticação OIDC/JWT
com negação por omissão, autorização ao nível do objecto, o gerador de cinco
camadas, um projecto gerado que compila de forma autónoma, e o empacotamento
distroless. Em curso: a migração para a camada de dados DORM, as migrações e um
admin. O [registo de engenharia](../progress.md) e os [ADRs](../internals/adrs.md)
acompanham tudo honestamente.

---

*Parte da [documentação do Seiva](../index.md). A seguir:
[Instalação rápida](quick-install.md).*
