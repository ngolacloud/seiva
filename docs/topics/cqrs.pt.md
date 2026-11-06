# Despacho CQRS

O Seiva oferece um mediador comando/consulta ligado estaticamente (`seiva-cqrs`)
para a camada de aplicação — com middleware tipado — **quando a complexidade de um
módulo o justifica**. É uma ferramenta, não um mandato.

## O que é

Um comando/consulta é uma mensagem tipada; um handler processa-a; um mediador
despacha mensagem → handler. As preocupações transversais (instrumentação, cache de
consultas) são **middleware tipado** no pipeline de despacho, ligadas em tempo de
compilação — sem reflexão em tempo de execução, sem localizador de serviços. É
independente da camada de dados: o despacho é sobre a camada de aplicação,
independentemente de como os dados são armazenados.

```rust
// a command + its handler, dispatched through the mediator with middleware
let dto = mediator.send(CreateRecord { title }).await?;
```

## Quando usá-lo — e quando não

Usa CQRS quando um módulo tem **muitos** casos de uso, ou caminhos de leitura e
escrita que diferem o suficiente para querer modelos separados ou cache nas leituras.
Para um módulo que é CRUD simples sobre um único modelo, o método de fachada a chamar
directamente o serviço de aplicação é mais simples e mais claro — forçar CQRS sobre
CRUD trivial é um anti-padrão que o catálogo `seiva-pattern-fitness` assinala.

O Seiva não impõe CQRS: um módulo começa com uma chamada directa fachada→serviço, e
adopta o mediador apenas se a sua complexidade o justificar.

## Middleware

O middleware do pipeline é tipado e componível — p. ex. uma camada de instrumentação
que abarca cada comando, ou uma camada de cache de consultas com chave no valor da
consulta. Adicionas exactamente o comportamento transversal de que o módulo precisa,
verificado pelo compilador.

## Relacionado

- [Fachadas](transport/facades.md) · [A arquitectura](../architecture.md)
- [Workers em segundo plano](background-workers.md)

---

*Parte da [documentação do Seiva](../index.md).*
