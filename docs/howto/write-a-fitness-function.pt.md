# Como escrever uma função de fitness

Uma **função de fitness** é um teste que guarda um invariante arquitectural e faz
o build falhar quando ele é quebrado — para que uma regra se mantenha ao longo do
tempo sem depender de revisão.

## Quando escrever uma

Quando decides que uma propriedade tem de se verificar *sempre*: uma regra de
camadas, uma postura de segurança, uma forma de contrato, um "nenhum stub deixado
para trás". Se importa o suficiente para ser enunciada, enuncia-a como um teste.

## A forma

Uma função de fitness afirma sobre um facto barato de calcular e inequívoco. As
mais fortes **exercitam** a coisa (compilam-na, correm-na, conduzem o fluxo) em
vez de afirmarem sobre strings — apanham desvios que uma verificação de texto não
apanha. Exemplos já presentes na árvore:

- o crate `interface` gerado **não** pode depender de `infrastructure` (a regra
  de camadas) — uma verificação de string no manifesto gerado;
- um projecto gerado tem de **compilar de forma autónoma** — o teste faz scaffold
  de um projecto + módulo e corre `cargo build`;
- o OpenAPI do corpo de uma escrita tem de ser o DTO directamente (sem envelope
  `cmd`) — afirmado contra o documento real gerado.

## Escreve-a como um teste normal

Põe-na junto ao código que guarda, como um `#[test]`/`#[tokio::test]`. Faz com
que a mensagem de falha explique o invariante, para que um futuro contribuinte
que nela tropece aprenda o *porquê*, não apenas o *quê*.

## Corre-as

```console
$ seiva lint        # the architecture fitness functions
$ cargo test        # the unit + integration tests, including yours
```

Ver o catálogo de anti-padrões `seiva-pattern-fitness` para as regras que o
`seiva lint` impõe, e [Testes](../topics/testing.md).

## Relacionado

- [Testes](../topics/testing.md) · [Arquitectura](../architecture.md)

---

*Parte da [documentação do Seiva](../index.md).*
