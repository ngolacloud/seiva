# Parte 3: Eventos entre módulos

Os módulos da loja nunca se chamam uns aos outros. Cooperam através de **eventos de
domínio**: um módulo publica um facto sobre os seus próprios dados, e outros
módulos reagem. É isto que os mantém independentes — e o que torna possível a
separação em microserviços na Parte 7.

## Um caso de uso, uma transacção, um evento

Cada caso de uso de escrita corre dentro de uma única `UnitOfWork::transaction`, e
publica o seu evento **dentro dessa transacção**. Com a camada de dados DORM
(Parte 4) o evento aterra num **outbox** transaccional na *mesma* transacção que a
mudança de negócio, por isso os dois fazem commit juntos ou de todo. Aqui está
`orders.place`, de `shop-orders-application`, condensado:

```rust
self.uow.transaction(|| async {
    self.repository.save(&order).await?;
    self.events.publish("orders.placed", &event).await   // same transaction
}).await?;
```

A camada de aplicação depende apenas da **porta** `EventPublisher` — não sabe se o
evento vai para um gravador em memória (testes), para um log ou para um broker.
Essa decisão pertence ao composition root.

## O fluxo da encomenda

Fazer uma encomenda desencadeia uma cadeia. Cada seta é um módulo a reagir ao
evento de outro módulo — nunca uma chamada directa:

```text
orders.place
   └─▶ orders.placed ──▶ inventory.reserve  ──▶ inventory.reserved
                                              └▶ payments.charge   ──▶ payments.captured
                                                                    └▶ notifications.send ─▶ notifications.sent
```

- `orders` publica **`orders.placed`** com as linhas da encomenda.
- `inventory` reage: reserva stock para cada linha e publica
  **`inventory.reserved`** (ou falha com `InsufficientStock`).
- `payments` reage a uma encomenda reservada: cobra o total e publica
  **`payments.captured`** (ou **`payments.declined`**).
- `notifications` reage a um pagamento cobrado: envia uma confirmação e publica
  **`notifications.sent`**.

Cada módulo só alguma vez toca nos seus próprios dados. O fluxo é uma sequência de
*factos*, não uma call stack.

## Quem liga as reacções?

O **composition root** (`apps/shop-api`) é dono da ligação. Subscreve cada módulo
reactivo aos eventos que lhe importam — um pequeno *process manager* que transforma
`orders.placed` numa chamada a `inventory.reserve`, e assim por diante. Os módulos
mantêm-se alheios; troca a ligação e os mesmos módulos comportam-se de forma
diferente.

Esta é também a costura para a Parte 7: quando `payments` se torna o seu próprio
serviço, nada no módulo `payments` muda — apenas *onde* a reacção `payments.charge`
corre e *como* o evento lhe chega (um bus em processo torna-se um tópico de
broker).

Ver [Mensageria e eventos](../../topics/messaging-and-events.md) para as opções de
broker e o envelope CloudEvents, e
[Workers em segundo plano](../../topics/background-workers.md) para o relay de
outbox que encaminha os eventos com commit.

## A seguir

[Parte 4: A camada de dados](part-4-the-data-layer.md).

---

*Parte da [documentação do Seiva](../../index.md).*
