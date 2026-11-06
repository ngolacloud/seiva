# Transacções

O DORM dá-te uma unidade de trabalho **baseada em âmbito**: corres um closure
dentro de uma transacção, e o resultado decide se ela faz commit. Nunca passas um
handle de ligação pelas tuas chamadas — o DORM acompanha a transacção activa num
task-local, por isso cada consulta dentro do âmbito corre nela automaticamente.

> Numa aplicação Seiva alcanças isto através da *porta* `UnitOfWork`, não da
> função `db::atomic` directamente. A última secção mapeia as duas; a visão de
> framework está em [Transacções e a unidade de trabalho](../topics/data-layer/transactions.md).

## `atomic`

`db::atomic` corre um closure dentro de uma transacção, fazendo commit em `Ok` e
rollback em `Err`:

```rust
use dorm_core::prelude::*;

db::atomic(|_tx| async move {
    let ada = Author::objects().create(NewAuthor { name: "Ada".into() }).await?;
    Post::objects()
        .create(NewPost { author: ada.as_fk(), slug: "on-engines".into() })
        .await?;
    Ok(())
})
.await?;
```

Tudo lá dentro corre na ligação da transacção — **não** passas um handle a cada
chamada.

## O aninhamento é por savepoint

Um `atomic` dentro de outro torna-se um `SAVEPOINT`: se o bloco interior falhar
desfaz só o seu próprio trabalho, e a transacção exterior ainda pode fazer commit.

```rust
db::atomic(|_| async move {
    do_the_required_part().await?;

    // An optional part that may fail without losing the required part.
    let _ = db::atomic(|_| async move { do_the_optional_part().await }).await;

    Ok(())
})
.await?;
```

## O que acontece num pânico

Um retorno de erro faz rollback explicitamente. Um pânico não consegue fazer
`.await` a um `ROLLBACK`, por isso o DORM tira antes a ligação **do pool** no
caminho de descida — é fechada em vez de reciclada, e o PostgreSQL faz rollback de
uma transacção cuja ligação cai. O mesmo mecanismo cobre o processo a ser morto:
nada que um âmbito começou mas não fez commit fica alguma vez escrito.

## Modo só-leitura

Marca todo o processo como só-leitura e cada escrita recusa *antes* de chegar ao
SQL, nomeando a operação que bloqueou:

```rust
dorm_core::db::set_read_only(true);

Post::objects().filter(post::id.eq(1)).delete().await?;
// Err: operation `delete` refused — the session is read-only
```

A sessão é *também* aberta `READ ONLY` no servidor (a protecção real); a flag
existe para que a recusa nomeie a operação em vez de aflorar um erro cru da base
de dados. Tanto a [shell](cli-and-studio.md) como o [Studio](cli-and-studio.md#dorm-studio)
partilham esta protecção, e é por isso que são só-leitura salvo se arrancados com
`--write`.

## Transacções interactivas (a shell)

`atomic` emoldura um único futuro. A shell (`dorm shell --write`) também expõe
transacções **interactivamente**, mantidas abertas através dos comandos:

```text
dorm*> :begin
dorm*> insert Author name=Ada
dorm*> insert Author name=Grace
dorm*> Author                 # the session sees its own uncommitted rows
dorm*> :commit                # …persisted atomically  (or :rollback to discard)
```

`:begin` abre uma transacção numa ligação dedicada; cada comando seguinte corre
nela, por isso uma leitura vê o trabalho não feito commit e uma instrução falhada
aborta a transacção. `:commit` persiste tudo de uma vez; `:rollback` — ou deixar a
shell com uma transacção aberta — descarta-a.

## Como o `UnitOfWork` do Seiva mapeia para o DORM

A porta `UnitOfWork` do Seiva é baseada em âmbito pela mesma razão que a do DORM: a
aplicação exprime atomicidade como um closure sem nomear a base de dados, o que
mantém os casos de uso testáveis (uma unidade de trabalho falsa apenas corre o
corpo).

```rust
unit_of_work.transaction(|| async {
    repository.save(&record).await?;                 // business change
    events.publish("notes.created", &data).await?;   // event, same tx
    Ok(())
}).await?;
```

O gateway `seiva-data-dorm` implementa essa porta mapeando-a directamente para o
executor de âmbito-closure do DORM: uma sessão por módulo (`with_session` — o modo
estrito exige uma) mais `db::atomic` (uma transacção real de base de dados). O erro
de fronteira da porta é ligado a um erro DORM para que `atomic` faça rollback na
falha, e de volta ao tipo de erro da porta no caminho de saída. Uma transacção que
faz commit persiste; uma que falha não deixa nada para trás.

### O outbox transacional

Quando respaldada pelo DORM, a porta `EventPublisher` escreve o evento num
**outbox** transacional *na mesma transacção* que a mudança de negócio — para que
o evento e a mudança façam commit em conjunto ou não façam de todo (sem eventos
perdidos nem fantasmas). `publish` tem, portanto, de ser chamado dentro de
`transaction`; o outbox do DORM recusa um publish fora de uma. Um worker de relay
separado encaminha os eventos do outbox para o broker depois — vê
[Usar o DORM a partir do Seiva](using-from-seiva.md#the-outbox-relay) e o
[guia de mensagens](../topics/messaging-and-events.md) do framework.

## Uma nota sobre concorrência optimista

O DORM tem um desenho para uma coluna de concorrência optimista (incrementa uma
`version` em cada escrita, rejeita uma escrita obsoleta com um conflito), e o
scaffold do Seiva marca essa coluna com `@version`. O parser de esquema ainda não
reconhece o atributo, por isso hoje a coluna `version` existe mas a *imposição*
não está ligada através do caminho de esquema — trata-a como pendente. Para um
contador atómico em linha que está vivo hoje, usa uma expressão `F` num `update`
(vê [actualizar linhas](queries.md#updating-rows)).

## Relacionado

- [Fazer consultas](queries.md) · [SQL cru](raw-sql.md) ·
  [Backends](backends.md)
- [Usar o DORM a partir do Seiva](using-from-seiva.md) ·
  [ADR 0004 — gateway DORM](../adr/0004-dorm-data-layer-gateway.md)

---

*Parte da [documentação do Seiva](../index.md).*
