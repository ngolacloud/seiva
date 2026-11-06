# Internos do Seiva

Como o Seiva funciona por dentro, e como trabalhar nele — o material dos
*internos* e do *projecto open-source*.

- [Arquitectura](../architecture.md) — a regra de dependência, o contrato, como as
  peças encaixam
- [O contrato e as macros](the-contract-and-macros.md) — como `#[facade]`,
  `#[controller]` e `#[derive(WireSchema)]` se expandem (sem magia escondida)
- [Architecture Decision Records](adrs.md) — a história do desenho, uma decisão
  por ADR
- [Contribuir](contributing.md) — a regra de dependência, as funções de fitness, o
  catálogo de anti-padrões, o fluxo ADR-first
- [O processo de release](release-process.md) — versionamento, publicação, notas
  de release

## Filosofia de desenho

1. **O contrato é a fonte da verdade.** Declaras uma fachada; os transportes, o
   documento OpenAPI e as verificações de permissões são derivados — nunca
   escritos à mão.
2. **Sem magia escondida.** Cada macro tem um equivalente documentado escrito à
   mão; corre `seiva expand` para o ver.
3. **O compilador impõe a arquitectura.** As camadas são o grafo de crates; não a
   consegues violar e ainda assim fazer build.
4. **Honesto por omissão.** Os contratos gerados correspondem ao que o servidor
   aceita; os placeholders estão rotulados; as capacidades são seguidas no log e
   nos ADRs.
5. **O núcleo nunca depende da camada de dados.** O DORM entra apenas através de
   uma feature.

---

*Parte da [documentação do Seiva](../index.md).*
