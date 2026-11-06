# Contribuir

Como trabalhar no próprio framework.

## A regra de dependência é lei

Um módulo são cinco crates de camada e o compilador impõe a direcção
(`interface → application → domain`; `infrastructure → application`; a fachada é o
contrato). Não consegues violar as camadas e ainda assim fazer build. Mantém-no
assim: a camada de aplicação não nomeia nenhum item de transporte ou base de
dados; a interface nunca depende de infraestrutura.

## As funções de fitness guardam invariantes

As regras arquitecturais são testes, não convenções. Corre-as localmente:

```console
$ seiva lint        # the architecture fitness functions + anti-pattern catalogue
$ cargo test        # unit + integration tests
```

Quando adicionas um invariante, adiciona-lhe uma função de fitness — uma que
*exercite* a coisa onde for possível (compila-a/corre-a), não apenas afirme
strings. Ver [como escrever uma função de fitness](../howto/write-a-fitness-function.md)
e o catálogo `seiva-pattern-fitness`.

## Uma mudança arquitectural aterra primeiro num ADR

Uma mudança que introduz ou altera um padrão, fronteira ou contrato público é
registada num [ADR](adrs.md) (contexto, decisão, alternativas, consequências)
antes de ser implementada — para que o *porquê* sobreviva. A checklist
`seiva-community-adr` cobre isto.

## Seguro por omissão, medido

As mudanças a `seiva-http`, `seiva-auth`, `seiva-webhooks` ou ao portão de
segurança passam pela checklist do OWASP API Top 10 (`seiva-security-review`).
Prova o comportamento conduzindo-o, não apenas com testes unitários — as
viagens de ida-e-volta ao vivo neste repositório (autenticação, o caminho de
dados DORM, a administração) são o modelo.

## Formatação e estilo

`cargo fmt` e `cargo clippy` limpos antes de committar. Acompanha a densidade de
comentários e o idioma do código à volta; os templates de código gerado são
formatados à mão de propósito.

## Relacionado

- [Arquitectura](../architecture.md) · [ADRs](adrs.md) · [O contrato e as macros](the-contract-and-macros.md)

---

*Parte da [documentação do Seiva](../index.md).*
