# Parte 6: CQRS — separar leituras de escritas

Uma escrita que coloca uma encomenda e uma leitura que lista encomendas têm
formatos diferentes, custos diferentes e regras de cache diferentes. **CQRS**
(Command/Query Responsibility Segregation) torna essa separação explícita: um
**comando** muda o estado, uma **query** lê-o, e um **mediator** encaminha cada
mensagem para o seu handler. O Seiva entrega-o como `seiva-cqrs` — tipado
estaticamente e de custo zero (o mediator é monomorfizado sobre o handler que
contém, por isso não há dispatch dinâmico).

Não precisas de CQRS para CRUD simples. Recorre a ele quando os caminhos de leitura
e escrita divergem — o que, para `orders`, acontece: colocar uma encomenda corre
uma transacção e emite um evento, enquanto ler uma é uma procura barata que vale a
pena pôr em cache.

## O comando e a query

Em `examples/shop/orders/application/src/cqrs.rs` o módulo orders expõe um comando
`PlaceOrder` e uma query `GetOrder`, cada um realizado por um handler:

```rust
use seiva_cqrs::{Command, Query, Mediator, Instrument, CacheQueries};

// A command changes state; its Output is what the handler returns.
pub struct PlaceOrderCommand { pub customer: String, pub lines: Vec<Line> }
impl Command for PlaceOrderCommand { type Output = OrderDto; }

// A query reads state; it is cacheable by key.
pub struct GetOrder { pub id: u128 }
impl Query for GetOrder { type Output = OrderDto; }
```

Um único `OrdersHandler` implementa `CommandHandler<PlaceOrderCommand>` e
`QueryHandler<GetOrder>` sobre o `OrdersService` do módulo — o mesmo caso de uso da
Parte 1, agora alcançável através do barramento.

## Ligar o mediator com middleware tipado

O mediator é construído uma vez, envolvendo o handler em middleware tipado — aqui
instrumentação mais uma cache de leitura directa (read-through) para a query
`GetOrder`:

```rust
let mediator = Mediator::new(
    Instrument::new(
        CacheQueries::<_, GetOrder>::new(OrdersHandler::new(service)),
    ),
);

let placed = mediator.send(PlaceOrderCommand { customer, lines }).await?; // command
let fetched = mediator.query(GetOrder { id: placed.id }).await?;          // query (cached)
```

`CacheQueries` memoiza apenas o lado da leitura — um segundo `GetOrder` para o mesmo
id é servido da cache, nunca re-executando o handler; um `PlaceOrder` corre sempre e
nunca é posto em cache. O dispatch é resolvido em tempo de compilação: uma mensagem
sem handler registado é um *erro de compilação*, não uma surpresa em tempo de
execução.

## Onde vive o código

```text
modules/orders/
├── application/
│   ├── src/lib.rs          # the OrdersService use case (Part 1)
│   └── src/cqrs.rs         # ← PlaceOrderCommand, GetOrder, OrdersHandler, the mediator
```

O CQRS fica na camada de **aplicação**; `seiva-cqrs` é uma dependência de
aplicação, e a camada continua a não nomear nenhum ponto de entrada de transporte
ou framework (`seiva lint` impõe isto).

## Construir e testar

```console
$ cargo test -p shop-orders-application           # runs the CQRS tests
$ cargo run -p seiva-cli --bin seiva -- lint      # the layering stays honest
```

Os testes do módulo enviam o comando através do mediator e consultam-no de volta,
afirmando que a cache é atingida (o segundo `GetOrder` não re-executa o handler) e
que um comando inválido falha através do barramento.

## Live-reload enquanto trabalhas

```console
$ seiva start --watch        # rebuilds + restarts on save (needs cargo-watch)
```

## Seguinte

[Parte 7: Documentação de API auto-gerada & Swagger](part-7-api-docs-and-swagger.md)
— vê cada comando e query aparecer no documento OpenAPI e na Swagger UI, sem
especificação escrita à mão.

---

*Parte da [documentação do Seiva](../../index.md).*
