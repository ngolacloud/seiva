# Fachadas: o contrato

Uma **fachada** é o contrato público versionado de um módulo: um trait anotado com
`#[seiva::facade]`. É a única fonte de verdade — as rotas HTTP, o documento
OpenAPI, o serviço proto e as verificações de permissão derivam-se todos dela,
nunca são escritos à mão.

## Declarar uma fachada

```rust
use delonix_seiva::{facade, WireSchema};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, WireSchema)]
pub struct CreateNote { pub title: String }

#[derive(Debug, Clone, serde::Serialize, WireSchema)]
pub struct NoteDto { pub id: u128, pub title: String }

#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
}
```

- **`#[facade(version = "v1")]`** — marca o trait como um contrato numa versão. As
  versões vivem lado a lado (`/v1/…`, `/v2/…`).
- **`#[requires("notes.create")]`** — a permissão que um chamador tem de deter para
  este método. Lida do descriptor e imposta antes de o handler correr.
- **Os DTOs** são os tipos do wire: derivam `serde` e
  [`WireSchema`](../../reference/wire-schema.md) para que o documento OpenAPI
  carregue os seus campos reais (não um placeholder opaco).

## O que gera (sem magia escondida)

A macro apenas *acrescenta* um `NotesFacade` de tamanho zero cujo `DESCRIPTOR`
lista o nome, a versão e os métodos (cada um com o seu `requires` e assinatura), e
regista-o num inventário em tempo de link. O equivalente escrito à mão está
documentado em `delonix_seiva::contract`; corre `seiva expand` para o veres. Nada é
reflectivo — o descriptor é conhecido em tempo de compilação.

Como cada fachada se regista a si própria, o `/openapi.json` de um serviço em
execução (e `seiva openapi`/`proto`) lista **cada módulo montado** sem registo
central para manter — fechando o OWASP API9 (inventário inadequado).

## As camadas à volta dela

A fachada é o topo das cinco camadas de um módulo: `domain` (entidades/regras),
`application` (o caso de uso sobre portas), `infrastructure` (adaptadores) e
`interface` (o controlador). Os DTOs e o trait da fachada são o que os outros
módulos e os transportes vêem; a implementação fica privada. Ver
[Arquitectura](../../architecture.md).

## Relacionado

- [Controladores e encaminhamento](controllers-and-routing.md) · [Validação](../validation.md)
- [O descriptor do contrato](../../reference/contract-descriptor.md) · [ADR 0007 — WireSchema](../../adr/0007-wire-schema-contract-fidelity.md)

---

*Parte da [documentação do Seiva](../../index.md).*
