# Transacções e a unidade de trabalho

Um caso de uso que altera dados fá-lo dentro de uma **transacção**, através da
porta `UnitOfWork` — por isso a camada de aplicação exprime a atomicidade sem
nomear a base de dados.

## A porta `UnitOfWork`

`seiva-ports` define uma unidade de trabalho baseada em âmbito: a aplicação corre
as suas alterações dentro de `transaction(|| async { … })`. O corpo que devolve
`Ok` faz commit; devolver `Err` faz rollback. A aplicação depende apenas do trait;
o gateway do DORM implementa-o.

```rust
unit_of_work.transaction(|| async {
    repository.save(&record).await?;          // business change
    events.publish("notes.created", &data).await?;  // event, same tx
    Ok(())
}).await?;
```

## A unidade de trabalho suportada por DORM

O gateway mapeia a porta no executor de âmbito-de-closure do DORM: `with_session`
(uma sessão por módulo — o modo estrito exige uma) mais `db::atomic` (uma
transacção real de base de dados). O erro de fronteira é traduzido para um erro do
DORM para que o `atomic` faça rollback e de volta para o `BoxError` da porta à
saída (ADR 0004). Provado ao vivo: uma transacção que faz commit persiste; uma que
falha não deixa nada para trás.

## A outbox transaccional

A porta `EventPublisher`, quando suportada por DORM, escreve o evento numa
**outbox** transaccional *na mesma transacção* da alteração de negócio — para que o
evento e a alteração façam commit juntos ou nenhum deles (sem eventos perdidos ou
fantasma). Um worker de relay reencaminha os eventos da outbox para o broker depois.
É por isto que `publish` tem de ser invocado dentro de `transaction`; ver
[mensagens e eventos](../messaging-and-events.md).

## Interactiva vs baseada em âmbito

A porta é baseada em âmbito (uma closure) em vez de um handle que passas de um lado
para o outro, o que é o que torna a camada de aplicação testável (uma unidade de
trabalho falsa apenas corre o corpo) e mantém a fronteira da transacção explícita e
lexical.

## Relacionado

- [Fazer consultas](queries.md) · [Mensagens e eventos](../messaging-and-events.md)
- [A camada de dados](index.md) · [ADR 0004 — gateway DORM](../../adr/0004-dorm-data-layer-gateway.md)

---

*Parte da [documentação do Seiva](../../index.md).*
