# Fazer consultas

Tudo começa em `Model::objects()`. Devolve um `Manager`, e a partir daí constróis
um **`QuerySet`** — preguiçoso e imutável. Cada método consome `self` e devolve um
novo, por isso nada chega à base de dados até fazeres `await` a um método terminal,
e uma consulta base é reutilizável em segurança.

```rust
let base   = Post::objects().filter(post::title.icontains("rust"));
let recent = base.clone().order_by([post::published_at.desc()]).limit(5);
let count  = base.count().await?;   // `base` is untouched by `recent`
```

Dois ramos construídos a partir da mesma base não se podem perturbar, porque
nenhum a mutou.

> Num módulo Seiva este construtor vive dentro do adaptador de repositório, atrás
> da porta de repositório da aplicação — o caso de uso pede à porta o que
> precisa e nunca nomeia `QuerySet`. Vê
> [Fazer consultas (visão de framework)](../topics/data-layer/queries.md).

## Ler linhas

| Terminal | Devolve | Notas |
|---|---|---|
| `.all()` | `Vec<T>` | Todas as linhas correspondentes. |
| `.first()` | `Option<T>` | A primeira, respeitando a ordenação. |
| `.last()` | `Option<T>` | A última; inverte a ordenação e toma uma. |
| `.get(q)` | `T` | Exactamente uma. Erra `DoesNotExist` se nenhuma, `MultipleObjectsReturned` se mais do que uma. |
| `.count()` | `i64` | `COUNT(*)` da correspondência. |
| `.exists()` | `bool` | Se algo corresponde, sem contar. |

```rust
let one   = Post::objects().get(post::id.eq(42)).await?;
let maybe = Post::objects().filter(post::slug.eq("on-engines")).first().await?;
let n     = Post::objects().filter(post::title.icontains("rust")).count().await?;
```

## Filtrar: `filter`, `exclude`, e `Q`

`filter` estreita; `exclude` é o seu complemento. Chamadas repetidas são
combinadas com `AND`:

```rust
Post::objects()
    .filter(post::title.icontains("rust"))
    .exclude(post::slug.startswith("draft-"))
```

Cada `post::field.<lookup>(value)` constrói um `Q`. Combina-os com `&` (e),
`|` (ou), `!` (não):

```rust
use dorm_core::prelude::*;

let q = (post::title.icontains("rust") | post::title.icontains("ferris"))
      & !post::slug.startswith("draft-");
Post::objects().filter(q).all().await?;
```

### Lookups de campo

Cada handle tipado carrega só os lookups que fazem sentido para o tipo da sua
coluna.

| Lookup | Método | SQL |
|---|---|---|
| exacto | `field.eq(v)` | `col = $1` |
| comparações | `field.gt(v)`, `.gte(v)`, `.lt(v)`, `.lte(v)` | `col > $1`, … |
| num conjunto | `field.is_in([a, b, c])` | `col IN ($1, $2, $3)` |
| num intervalo | `field.range(lo, hi)` | `col BETWEEN $1 AND $2` |
| é nulo | `field.isnull(true)` | `col IS NULL` |
| exacto insensível a maiúsculas | `field.iexact(v)` | `col ILIKE $1` *(texto)* |
| contém | `field.contains(v)`, `.icontains(v)` | `col LIKE '%'‖$1‖'%'` *(texto)* |
| começa por | `field.startswith(v)` | `col LIKE $1‖'%'` *(texto)* |

### A forma dinâmica (indexada por string)

A mesma consulta pode ser construída a partir de strings — a forma que uma REPL ou
uma consola de administração envia. Produz o plano idêntico, e só resolvem colunas
que a macro de derivação registou; um nome inventado é um erro, não uma consulta:

```rust
Post::objects().filter_str("title__icontains", "rust").all().await?;
```

## Ordenação, limite, offset, distinct

```rust
Post::objects()
    .order_by([post::published_at.desc(), post::slug.asc()])
    .offset(20)
    .limit(10)
    .distinct()
    .all()
    .await?;
```

- `order_by` **substitui** em vez de acrescentar — a última chamada vence.
- A forma por string é `.order_by_str(&["-published_at", "slug"])`.
- Sem `order_by`, aplica-se o `#[model(ordering = …)]` do modelo.

### Paginação por keyset para listas

A paginação por offset está disponível, mas para listas voltadas ao utilizador
prefere a paginação por **keyset** (cursor): um cursor sobre uma ordenação estável
é estável sob escritas concorrentes e não degrada em páginas fundas. O DORM
fornece cursores por keyset com um token de cursor **assinado** (a chave de
assinatura é configurada pelo host, sem valor por omissão falsificável), por isso
um cliente não consegue adulterar a posição. Num serviço Seiva a convenção da rota
`list` carrega os parâmetros de cursor e o repositório transforma-os numa leitura
limitada e ordenada; vê a [nota de paginação](../topics/data-layer/queries.md#pagination)
ao nível do framework. A configuração da chave de assinatura é parte do que o
[gateway](using-from-seiva.md) prepara.

## Criar linhas

Insere com a struct `New…` gerada, que omite as colunas preenchidas pela base de
dados. `create` devolve a linha tal como armazenada, com essas colunas
preenchidas:

```rust
let ada = Author::objects()
    .create(NewAuthor { name: "Ada Lovelace".into() })
    .await?;
println!("new id = {}", ada.id);   // filled by the database
```

Insere muitas numa só instrução, ou faz fetch-ou-insert idempotentemente sob uma
corrida (um insert perdido faz retry como leitura):

```rust
Author::objects()
    .bulk_create(vec![NewAuthor { name: "Ada".into() }, NewAuthor { name: "Grace".into() }])
    .await?;

let (author, created) = Author::objects()
    .get_or_create(author::name.eq("Ada"), NewAuthor { name: "Ada".into() })
    .await?;
```

## Actualizar linhas

`update` aplica um conjunto de atribuições a cada linha correspondente e devolve
quantas tocou. Constrói atribuições com `field.set(expr)` ou `Assignment::new`:

```rust
let n = Post::objects()
    .filter(post::slug.eq("on-engines"))
    .update(vec![post::title.set("On Analytical Engines")])
    .await?;
```

Uma expressão `F` calcula a partir da própria coluna — **atómica na base de
dados**, por isso dois chamadores concorrentes fazem ambos efeito onde um
read-modify-write perderia um:

```rust
use dorm_core::prelude::*;

// views = views + 1, inside the UPDATE
Post::objects()
    .filter(post::id.eq(42))
    .update(vec![Assignment::new("views", F::col("views") + 1)])
    .await?;
```

## Apagar linhas

```rust
let removed = Post::objects()
    .filter(post::slug.startswith("draft-"))
    .delete()
    .await?;   // u64: how many were deleted
```

## Agregados e projecções

`aggregate` calcula sobre toda a consulta e devolve uma linha de valores nomeados;
`annotate` adiciona um agregado por grupo (contar o lado inverso de uma relação é
o caso clássico — vê [relações](relations.md)):

```rust
use dorm_core::prelude::*;

let row   = Post::objects()
    .aggregate(vec![Aggregate::new("total", AggKind::Count, "id")]).await?;
let total = row.get("total");   // a Value
```

`AggKind` é `Count`, `Sum`, `Avg`, `Min`, `Max`; `count_of(path)` é atalho para um
único `COUNT`.

Para seleccionar colunas nomeadas em vez de structs inteiras, projecta com
`values` e lê com `rows` (mapas nome-para-`Value`). `values()` e `all()` são
mutuamente exclusivos — chamar `all()` numa consulta projectada é recusado em vez
de descodificar as colunas erradas:

```rust
let rows = Post::objects().values(&["id", "title"]).rows().await?;
for r in rows {
    println!("{} {}", r["id"], r["title"]);
}
```

## Ver o SQL — sem base de dados

Qualquer `QuerySet` consegue mostrar o SQL e os parâmetros que correria, com os
valores como marcadores vinculados. Isto não precisa de base de dados e é a base
dos testes de instantâneo do DORM:

```rust
let sql = Post::objects().filter(post::title.icontains("rust")).sql()?;
println!("{}", sql.text);          // SELECT … WHERE "posts"."title" ILIKE $1
println!("{}", sql.params.len());  // 1
```

## Relacionado

- [Relações](relations.md) · [Transacções](transactions.md) ·
  [SQL cru](raw-sql.md)
- [Esquema e modelos](schema-and-models.md) · [Backends](backends.md)

---

*Parte da [documentação do Seiva](../index.md).*
