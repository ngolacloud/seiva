# Relações

Os modelos do DORM relacionam-se entre si através de **chaves estrangeiras**. Uma
chave estrangeira é um campo tipado `ForeignKey<Target>`; a macro de derivação
regista o alvo, a acção de apagamento e o `related_name` inverso, e o compilador
de consultas usa esses metadados para resolver os joins por ti.

> Dentro do Seiva, as relações ficam dentro de um módulo. O esquema do DORM
> [proíbe uma relação que atravesse um `namespace`](schema-and-models.md#namespaces-the-module-boundary),
> por isso os modelos de um módulo não podem referenciar os de outro — o mesmo
> isolamento que a camada de fachada impõe do lado da API.

## Declarar uma chave estrangeira

No esquema, um campo tipado como outro modelo é uma relação para-um; o campo
`<Model>[]` do outro lado nomeia a relação de retorno e não produz coluna:

```
model Author {
  id    Int    @id @default(autoincrement())
  name  String @unique
  posts Post[]
}

model Post {
  id     Int    @id @default(autoincrement())
  author Author @relation(onDelete: Cascade)
  slug   String @db.VarChar(80)
}
```

No código de modelo gerado isto é um campo `ForeignKey<Author>` que carrega os
metadados `foreign_key`, `on_delete` e `related_name`:

```rust
#[field(foreign_key = "Author", on_delete = "cascade", related_name = "posts")]
author: ForeignKey<Author>,
```

`on_delete` aceita `cascade`, `restrict`, `set_null`, ou `do_nothing`
(as grafias no esquema são `Cascade`, `Restrict`, `SetNull`, `NoAction`).

## Inserir através de uma relação

Referencia o pai através da sua chave. Cada modelo ganha um `as_fk()` gerado,
atalho para `ForeignKey::from_id(parent.pk())`:

```rust
let ada = Author::objects().create(NewAuthor { name: "Ada".into() }).await?;
Post::objects()
    .create(NewPost { author: ada.as_fk(), slug: "on-engines".into() })
    .await?;
```

## Filtrar através de uma relação

Atravessa a chave estrangeira chamando o campo relacionado no handle da relação; o
compilador resolve o join — nunca escreves SQL nem nomeias uma tabela de join:

```rust
// posts whose author's name contains "ada"
Post::objects()
    .filter(post::author.author().name().eq("Ada Lovelace"))
    .all()
    .await?;
```

A forma dinâmica soletra a mesma travessia com `__`:
`.filter_str("author__name", "Ada Lovelace")`. As relações inversas resolvem-se
pelo seu `related_name`, por exemplo um autor filtrado por um campo dos seus posts.

## Obter o pai numa só ida ao servidor

`select_related` junta o pai e anexa-o a cada linha, por isso não há uma segunda
consulta por linha — o problema do N+1 não surge:

```rust
for post in Post::objects().select_related("author").all().await? {
    if let Some(author) = post.author.get() {
        println!("{} by {}", post.slug, author.name);
    }
}
```

Este é o mecanismo por trás da garantia de "sem N+1" do Seiva para o lado para-um:
uma instrução, uma ida ao servidor. (Para o lado *para-muitos* — carregar os posts
de cada autor sem uma consulta por autor — o roteiro de produção do DORM adiciona
`prefetch_related`, que emite uma consulta extra por relação em vez de uma por
linha; o `select_related` para-um é o caminho entregue e documentado hoje.)

## Contar o lado inverso

`annotate` com um `Count` sobre o `related_name` dá uma contagem por linha da
relação inversa:

```rust
use dorm_core::prelude::*;

// each author, with how many posts they have
Author::objects()
    .annotate(Aggregate::new("posts", AggKind::Count, "posts"))
    .all()
    .await?;
```

## Relacionado

- [Esquema e modelos](schema-and-models.md) · [Fazer consultas](queries.md)
- [Migrações](migrations.md) — como a restrição `REFERENCES … ON DELETE …` é
  gerada.

---

*Parte da [documentação do Seiva](../index.md).*
