# Esquema e modelos

O DORM é **esquema-primeiro**. Descreves os teus dados num ficheiro `.schema`
declarativo; `dorm generate` compila isso em structs de modelo em Rust com
`#[derive(Model)]`. O esquema é a fonte de verdade, e o código de modelo gerado
não é editado à mão.

> Num projecto Seiva o esquema vive em `schema/<project>.schema`, e a camada de
> dados DORM gerada vive sob `data/`. `seiva db dev` mantém os dois em sintonia
> por ti — vê [Migrações](migrations.md) e
> [Usar o DORM a partir do Seiva](using-from-seiva.md). Esta página explica a
> linguagem por baixo desse fluxo de trabalho; também podes escrever modelos como
> structs Rust simples, o alvo para o qual o esquema compila.

## Um esquema completo

```
datasource db {
  provider = "postgresql"
  url      = env("DELONIX_ORM_DSN")
}

namespace catalog {
  model Author {
    id        Int      @id @default(autoincrement())
    name      String   @unique @db.VarChar(120)
    createdAt DateTime @default(now())
    posts     Post[]
  }

  model Post {
    id        Int      @id @default(autoincrement())
    author    Author   @relation(onDelete: Cascade)
    title     String   @index @db.VarChar(200)
    createdAt DateTime @default(now())
  }
}
```

`dorm generate` transforma cada `model` numa struct `#[derive(Model)]`.
`Post.author` torna-se um `ForeignKey<Author>`, com o seu `related_name` inverso
lido de `Author.posts Post[]`; o próprio `Author.posts` não produz coluna (só
nomeia a relação de retorno).

## Os blocos

| Bloco | Significado |
|---|---|
| `datasource <name> { provider = "…"  url = env("VAR") \| "…" }` | O backend e a ligação. `provider` selecciona o dialecto (o PostgreSQL é de primeira classe; vê [backends](backends.md)); `url` é lido pelo runtime, não cozido no binário. |
| `namespace <name> { … }` | Uma fronteira de módulo (vê abaixo). |
| `model <Name> { … }` | Uma tabela / uma struct Rust. |
| `enum <Name> { v1 v2 … }` | Um conjunto nomeado de variantes string, compilado para um enum nativo da base de dados. |
| `generator … { }` | Aceite e ignorado, para que um esquema escrito para uma toolchain em formato Prisma cole sem alterações. |

## Namespaces — a fronteira de módulo

Um bloco `namespace` agrupa os modelos que pertencem a um módulo Seiva em
processo. O DORM suporta namespaces nativamente e **proíbe uma relação que
atravesse um namespace**: `dorm validate` rejeita uma chave estrangeira cujo alvo
viva noutro namespace. Os dados de um módulo não podem, portanto, enredar-se com
os de outro módulo; só identificadores atravessam a fronteira, a mesma regra que a
camada de fachada do Seiva impõe do lado da API.

`seiva module new <m>` acrescenta um bloco `namespace <m>` com um modelo inicial.
Um esquema sem blocos `namespace` compila exactamente como antes — os namespaces
são aditivos.

## Campos e tipos

Um campo é `name Type [@attr …]`. O tipo determina o tipo da coluna; um `?` final
torna-o anulável (`Option<T>` em Rust).

| Esquema | Rust | SQL (PostgreSQL) |
|---|---|---|
| `Int` | `i32` | `INTEGER` — `@id @default(autoincrement())` promove-o a `i64` / `BIGSERIAL` |
| `BigInt` | `i64` | `BIGINT` |
| `String` | `String` | `TEXT` — `@db.VarChar(n)` define um comprimento máximo |
| `Boolean` | `bool` | `BOOLEAN` |
| `DateTime` | `DateTime<Utc>` | `TIMESTAMPTZ` |
| `Decimal` | `rust_decimal::Decimal` | `NUMERIC` |
| `Json` | `serde_json::Value` | `JSONB` |
| `Uuid` | `uuid::Uuid` | `UUID` |
| `Inet` / `Cidr` / `MacAddr` | `IpAddr` / `IpCidr` / `MacAddress` | `INET` / `CIDR` / `MACADDR` — só PostgreSQL |
| `<Model>` | `ForeignKey<Model>` | uma relação para-um; precisa de `@relation` |
| `<Model>[]` | *(sem coluna)* | uma relação de retorno para-muitos; nomeia o `related_name` |
| `<Enum>` | `String` | um enum nativo da base de dados (vê abaixo) |

O conjunto é deliberadamente **fechado**. Um conjunto aberto deixaria o gerador de
migrações incapaz de nomear o tipo de uma coluna e permitiria à API de consulta
dinâmica pedir uma coluna que o compilador não consegue descrever.

## Atributos de campo

| Atributo | Efeito |
|---|---|
| `@id` | chave primária |
| `@default(autoincrement())` | chave serial — `i64` / `BIGSERIAL`, preenchida pela base de dados |
| `@default(now())` | definido no insert (`auto_now_add`) |
| `@updatedAt` | refrescado em cada escrita (`auto_now`) |
| `@default(<literal>)` | um default SQL cru (`'active'`, `0`, `true`) |
| `@unique` | uma coluna única |
| `@index` | um índice secundário |
| `@db.VarChar(n)` / `@db.Text` | um comprimento máximo / um tipo de texto explícito |
| `@map("col")` | armazenar sob um nome de coluna diferente |
| `@relation(onDelete: Cascade\|Restrict\|SetNull\|NoAction)` | a acção de apagamento da chave estrangeira |

## Atributos de bloco

| Atributo | Efeito |
|---|---|
| `@@map("table")` | renomear a tabela |
| `@@id([a, b])` | uma chave primária composta |
| `@@unique([a, b])` | uma restrição única composta |
| `@@index([a, b])` | um índice composto (construído `CONCURRENTLY` por `migrate`/`push`) |
| `@@rag(…)` | optar pela camada de recuperação opcional do DORM para o modelo |

Uma coluna nomeada por `@@id` / `@@unique` / `@@index` tem de ser uma coluna real
do modelo, senão `dorm validate` nomeia a gralha.

### Uma nota sobre `@version`

O scaffold do Seiva marca uma coluna de concorrência optimista com `@version`. O
parser de esquema actual do DORM **não** aceita esse atributo: a sincronização da
camada de dados deixa-o cair e a coluna inteira `version` continua a ser criada,
mas a *imposição* da concorrência chega quando o DORM reconhecer o atributo. Trata
`@version` hoje como uma convenção mais uma coluna simples, não uma restrição
imposta. Vê [transacções](transactions.md).

## Enums

Um bloco `enum` de topo declara um conjunto nomeado de variantes string, compilado
para um **enum nativo da base de dados** — uma coluna `String` respaldada por
`CREATE TYPE … AS ENUM`:

```
enum OrderStatus {
  pending
  paid
  shipped
}

model Order {
  id     Int          @id @default(autoincrement())
  status OrderStatus
  note   OrderStatus?
}
```

O nome do tipo SQL é o snake_case do nome do enum (`OrderStatus` →
`order_status`); as variantes são emitidas exactamente como declaradas. O valor é
uma `String` simples em Rust, mas a base de dados rejeita qualquer valor fora do
conjunto. `dorm validate` rejeita um enum sem variantes, uma variante duplicada,
ou um nome que colida com um modelo. Adicionar uma variante a um enum vivo
(`ALTER TYPE … ADD VALUE`) ainda não é gerado como passo de migração.

## O modelo gerado

Para um `model Author`, a macro de derivação emite:

- `impl Model for Author`, carregando o seu `&'static ModelMeta`;
- `Author::objects()` — o ponto de entrada para as [consultas](queries.md);
- `mod author` — um handle de campo tipado, por isso `author::name.eq("ada")`;
- `struct NewAuthor` — a forma de insert, carregando **apenas** as colunas
  fornecidas pelo chamador. As colunas que a base de dados preenche (uma chave
  serial, um `@default`, uma marca `auto_now_add`) estão ausentes de propósito:
  não há valor para tu inventares.

```rust
use dorm_core::prelude::*;
use dorm_derive::Model;

#[derive(Model, Debug, Clone)]
#[model(table = "authors", ordering = "name")]
struct Author {
    #[field(primary_key)]
    id: i64,
    #[field(unique, max_length = 120)]
    name: String,
}
```

Um modelo não pode ser genérico e tem de ter campos nomeados — os seus metadados
são um `static`, e uma tuple struct não dá por onde nomear uma coluna. O atributo
`#[model]` carrega as escolhas ao nível da tabela (`table`, `ordering`,
`unique_together`, `check`, `tenant_scoped`); `#[field]` carrega as escolhas por
coluna que espelham os atributos de esquema acima (`primary_key`, `unique`,
`index`, `max_length`, `column`, `default`, `auto_now`, `foreign_key`,
`on_delete`, `related_name`).

## O que ainda não está na linguagem de esquema

`@@check` ao nível da tabela, ordenação por omissão, índices parciais/de expressão
e esquemas multi-ficheiro estão no roteiro. Um `@@index` composto adicionado a uma
tabela *existente* é aplicado por `push`/`sync` mas ainda não emitido como passo
de migração incremental versionado. `Cidr`/`MacAddr` funcionam num modelo escrito
à mão mas ainda não a partir de um tipo de esquema. Tudo isto é aditivo, e
escrever modelos Rust à mão continua plenamente suportado — o esquema é
simplesmente o caminho com ferramentas.

## Relacionado

- [Fazer consultas](queries.md) · [Relações](relations.md) ·
  [Migrações](migrations.md)
- A visão ao nível do framework: [Modelos e esquema](../topics/data-layer/models.md)

---

*Parte da [documentação do Seiva](../index.md).*
