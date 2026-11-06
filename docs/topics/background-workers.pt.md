# Trabalho em segundo plano e o runtime de workers

Algum trabalho acontece fora de um pedido: drenar a outbox, fazer polling, tarefas
periódicas. O runtime de workers do Seiva (`delonix_seiva::worker`) aloja-os ao lado
de — ou em vez de — o servidor HTTP.

## O runtime de workers

Um worker é um ciclo que o runtime conduz: faz uma unidade de trabalho num intervalo,
reporta métricas (um `WorkerReport`: passagens, processados, erros), e pára
limpamente ao sinal de encerramento. O `seiva-server` pode alojar workers e a
aplicação HTTP em conjunto, ou um worker pode ser o seu próprio `main`.

```rust
run_worker(WorkerConfig { interval, .. }, shutdown_signal(), || async {
    // one pass of work; return how many items were processed
}).await
```

## O consumidor de destaque: o relay da outbox

O relay da outbox transaccional (`seiva-outbox-relay`) é um worker: cada passagem
drena a outbox e reencaminha eventos para o broker, acompanhando um cursor e contando
erros, até ao encerramento. É isto que torna a entrega de eventos fiável — ver
[mensageria e eventos](messaging-and-events.md).

## Quando usar um worker

- Reencaminhar a outbox para um broker (o relay).
- Manutenção periódica (expiração, reconciliação, aquecimento de cache).
- Fazer polling de um sistema externo num intervalo.

Para reagir aos eventos de outro serviço, prefere um consumidor de broker (conduzido
pela camada de mensageria) ao polling; usa um worker quando a cadência é baseada em
tempo ou a fonte não tem push.

## Encerramento gracioso

Os workers param no mesmo `SIGTERM` que drena o servidor HTTP — o runtime resolve o
futuro de encerramento, o ciclo termina a sua passagem actual, e o processo sai
limpamente. Um rollout não larga um pedido nem abandona uma passagem a meio.

## Relacionado

- [Mensageria e eventos](messaging-and-events.md) · [A aplicação e a raiz de composição](transport/application-and-composition.md)

---

*Parte da [documentação do Seiva](../index.md).*
