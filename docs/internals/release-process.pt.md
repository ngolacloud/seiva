# O processo de release

## Versionamento

Os crates versionam em conjunto na linha `0.1` por agora. O `seiva-ports` — a
fronteira só-de-traits contra a qual a camada de aplicação codifica — é mantido
estável em SemVer por política, para que a `application` de um módulo possa
depender dele sem churn; quebrar as portas é uma mudança maior deliberada e
documentada.

## Publicar os crates

Um projecto gerado depende do framework por versão (`delonix-seiva = "0.1"`, …) e
compila de forma autónoma através de um `[patch.crates-io]` provisório até os
crates estarem no crates.io. Publicar (só o dono, `cargo publish`) por ordem de
dependência (`seiva-ports` → `seiva-macros` → … → `seiva-cli`) é o que retira o
patch para os utilizadores finais. O estado e o conjunto pronto a publicar são
seguidos em [publicação](../publishing.md).

## Notas de release

As mudanças são registadas à medida que aterram (o log de engenharia é
[progress.md](../progress.md)); as notas por release viverão em
[releases](../releases/index.md) assim que a linha `0.1` for publicada. Uma
mudança arquitectural é também captada num [ADR](adrs.md).

## Relacionado

- [Estado da publicação](../publishing.md) · [Notas de release](../releases/index.md) · [ADRs](adrs.md)

---

*Parte da [documentação do Seiva](../index.md).*
