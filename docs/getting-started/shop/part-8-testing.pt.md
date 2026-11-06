# Parte 8: Testar a loja

Cinco módulos, e tudo continua a testar-se sem um servidor ou uma base de dados. Os
módulos da loja programam contra portas, por isso os seus casos de uso correm
contra fakes em memória do `seiva-testing` — os mesmos três níveis que o primeiro
tutorial introduziu, agora através de um sistema.

## Testar unitariamente cada caso de uso

O crate `application` de cada módulo testa o caminho feliz, uma rejeição de regra, e
que o evento certo foi emitido — com fakes para cada porta. Este é o verdadeiro
teste `reserve` de `shop-inventory-application`, condensado:

```rust
#[tokio::test]
async fn reserve_decrements_available_and_emits() {
    let svc = service();
    svc.receive(ReceiveStock { product_id: 1, qty: 10 }).await.unwrap();

    let stock = svc.reserve(Reserve { product_id: 1, qty: 3 }).await.unwrap();
    assert_eq!(stock.available, 7);
    assert_eq!(svc.events.topics(), ["inventory.received", "inventory.reserved"]);
}

#[tokio::test]
async fn reserve_more_than_available_is_refused() {
    let svc = service();
    svc.receive(ReceiveStock { product_id: 1, qty: 2 }).await.unwrap();
    assert!(svc.reserve(Reserve { product_id: 1, qty: 5 }).await.is_err());
    // nothing committed, no reservation event
    assert_eq!(svc.events.topics(), ["inventory.received"]);
}
```

`examples/shop` entrega **28 testes unitários** através das camadas de domínio e
aplicação dos cinco módulos, mais um **teste de composição por módulo** que conduz
o serviço através do seu adaptador real em memória via a fachada. Corre-os todos:

```console
$ cargo test
```

## O fluxo de eventos

O proveito do desenho de eventos (Parte 3) é que o fluxo da encomenda é testável
como uma sequência de eventos gravados. Com um `RecordingEventPublisher`
partilhado pelos módulos, um teste de composição afirma que colocar uma encomenda
produz `orders.placed`, depois `inventory.reserved`, depois `payments.captured`,
depois `notifications.sent` — por ordem, com nada emitido quando um passo falha
(uma encomenda sem stock nunca chega a `payments`).

## Funções de aptidão de arquitectura

`seiva lint` corre os invariantes que impedem cinco módulos de silenciosamente
ganharem dependências uns dos outros:

```console
$ seiva lint
```

Falha a build se o `interface` de um módulo depende da sua `infrastructure`, se uma
`application` nomeia `delonix_seiva` directamente, ou — o que mais importa com
vários módulos — se um módulo importa o crate de outro módulo. A fronteira da
Parte 2 é imposta, não esperada.

## Seguinte

[Parte 9: Do monólito aos microserviços](part-9-monolith-to-microservices.md).

---

*Parte da [documentação do Seiva](../../index.md).*
