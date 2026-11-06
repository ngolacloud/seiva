# O descritor de contrato

`#[seiva::facade]` transforma um trait num **descritor** legível por máquina — o
valor que cada gerador (OpenAPI, proto) e verificação (permissões, inventário) lê.
É conhecido em tempo de compilação; não há reflexão.

## Os tipos (`delonix_seiva::contract`)

```rust
pub trait Facade { const DESCRIPTOR: FacadeDescriptor; }

pub struct FacadeDescriptor {
    pub name: &'static str,         // the trait identifier, e.g. "Notes"
    pub version: &'static str,      // e.g. "v1"
    pub methods: &'static [MethodDescriptor],
}

pub struct MethodDescriptor {
    pub name: &'static str,         // the method name
    pub requires: &'static str,     // the #[requires("…")] permission, or ""
    pub params: &'static [Param],   // the parameters (no &self)
    pub output: &'static str,       // the return type, rendered canonically
}

pub struct Param { pub name: &'static str, pub ty: &'static str }
```

`MethodDescriptor::is_guarded()` é `!requires.is_empty()`.

## Descoberta em tempo de link

`#[seiva::facade]` emite também um `inventory::submit!` de um `FacadeRegistration`,
para que cada fachada ligada a um binário seja descobrível através de
`registered_facades()` — ordenada por `(name, version)` — sem lista mantida à mão.
É assim que o `/openapi.json` de um serviço em execução lista cada módulo montado
(OWASP API9), e como `seiva openapi`/`proto` documentam o projecto.

## Sem magia escondida

A macro apenas *adiciona* o descritor; escrevê-lo à mão é o equivalente exacto
(documentado no módulo `contract`). Corre `seiva expand` para ver o código gerado
para qualquer fachada.

## Como os consumidores o lêem

- `seiva-openapi` transforma descritores num documento OpenAPI 3.1 — resolvendo a
  forma real de cada tipo nomeado a partir do [inventário de wire-schema](wire-schema.md)
  quando o DTO deriva `WireSchema`, recorrendo a um placeholder honesto caso
  contrário.
- `seiva-grpc` transforma-os num serviço proto3.
- `#[seiva::controller]` lê o mesmo descritor para montar rotas e impor o
  `requires` de cada método.

## Relacionado

- [Fachadas (guia de tópico)](../topics/transport/facades.md) · [Wire schema](wire-schema.md)
- [O contrato e as macros (internos)](../internals/the-contract-and-macros.md)

---

*Parte da [documentação do Seiva](../index.md).*
