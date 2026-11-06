# Mensageria e eventos

Um módulo emite eventos de domínio através da porta `EventPublisher`; o framework
entrega-os de forma fiável via uma outbox transaccional e um worker de relay.

## A porta `EventPublisher`

A aplicação publica um evento por tópico e payload — não sabe nada de brokers ou
envelopes:

```rust
events.publish("notes.created", &serde_json::to_vec(&data)?).await?;
```

Como todas as portas, é um trait: a aplicação é testável com um fake, e o mecanismo
de entrega é um adaptador.

## Envelope CloudEvents

O `seiva-messaging` envolve os dados do evento num envelope **CloudEvents** (id,
source, type, data) — o formato de intercâmbio que os consumidores e o relay
compreendem. O `type` é o tópico; a gateway adiciona o id e o source na fronteira.

## Entrega fiável: a outbox transaccional

O publicador apoiado em DORM escreve o evento numa **outbox** transaccional na
*mesma transacção de base de dados* que a alteração de negócio
([transacções](data-layer/transactions.md)). Assim a alteração e o seu evento fazem
commit atomicamente — nunca uma alteração sem o seu evento, nunca um evento para uma
alteração que foi revertida. É por isso que o `publish` tem de correr dentro de uma
`UnitOfWork::transaction`; a outbox recusa um publish fora de uma.

Um **worker de relay** separado (`seiva-outbox-relay`) drena a outbox e reencaminha
cada evento para o broker, acompanhando o seu próprio cursor e repetindo — pelo que
uma indisponibilidade do broker atrasa a entrega mas nunca perde um evento.

## Backends de broker

Os brokers concretos são adaptadores feature-gated por trás do publicador/relay; o
caminho de desenvolvimento por omissão regista em log, e a produção liga um broker
real. O código da aplicação é inalterado em qualquer dos casos.

## Relacionado

- [Transacções e a unidade de trabalho](data-layer/transactions.md) · [Workers em segundo plano](background-workers.md)

---

*Parte da [documentação do Seiva](../index.md).*
