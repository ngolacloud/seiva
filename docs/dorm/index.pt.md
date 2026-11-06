# DORM — o ORM da Delonix

O **DORM** é a camada de dados por omissão do Seiva: um ORM nativo em Rust, com
tipagem em tempo de compilação. É um produto por direito próprio — o seu próprio
repositório, crates no crates.io, a sua própria CLI e o seu livro — que o Seiva
*consome*. Esta secção é o retrato completo do DORM tal como um programador de
Seiva o encontra: a linguagem de esquema, a API de consultas tipada, as
migrações, as transacções, os backends, as ferramentas, e exactamente como o
Seiva o liga.

> Se só queres a visão ao nível do framework — as portas contra as quais a tua
> aplicação programa e como os dados ficam de fora dos teus casos de uso — lê
> primeiro o [guia temático da camada de dados](../topics/data-layer/index.md).
> Esta secção desce um nível mais fundo, até ao próprio DORM.

## O que é o DORM

Descreves os teus dados uma única vez, num ficheiro `.schema` declarativo. O DORM
compila isso em structs de modelo em Rust com `#[derive(Model)]` e, a partir dos
*mesmos* metadados, gera o esquema SQL, as migrações e um construtor de consultas
preguiçoso, encadeável e tipado (`QuerySet`). O esquema é a única fonte de
verdade; o código do modelo é gerado e não editado à mão.

```rust
// Typed, lazy, reusable — nothing runs until you await a terminal method.
let newest = Author::objects()
    .filter(author::name.icontains("ada"))
    .order_by([author::name.asc()])
    .limit(10)
    .all()
    .await?;
```

Duas coisas tornam o DORM distinto, e ambas são estruturais em vez de estilísticas:

- **Seguro por construção.** Cada *valor* viaja como parâmetro vinculado (`$1`,
  `$2`, …), e cada *identificador* é um `&'static str` que a macro de derivação
  emitiu para dentro do binário. Não há caminho de código que formate um valor
  dentro de SQL nem forma de um chamador nomear uma tabela ou coluna que o
  compilador ainda não conhecesse. A injecção de SQL não é um bug que consigas
  escrever.
- **Um compilador, duas frentes.** A API tipada (`author::name.eq("ada")`) e uma
  API dinâmica indexada por strings (`.filter_str("name__icontains", "ada")`)
  constroem o *mesmo* plano de consulta. A dinâmica é segura para expor a uma REPL
  ou a uma consola de administração porque só consegue resolver nomes que a macro
  registou.

## Porquê um ORM nativo em Rust

O DORM é **nativo em Rust de uma ponta à outra**: sem recolector de lixo, sem
fronteira de função estrangeira entre um motor de consultas e o teu código,
segurança de tipos em tempo de compilação até ao binário, e implantação de
binário único. Os metadados que as consultas lêem são os metadados que o gerador
de migrações lê, por isso o esquema que o teu código assume e o esquema que a tua
base de dados tem não podem divergir em silêncio.

O DORM é também deliberadamente *independente*. As suas crates não carregam
lógica de aplicação nem de negócio; qualquer programa Rust pode depender só de
`dorm-core`. É essa independência que permite ao Seiva tratar o DORM como uma
camada de fonte de dados substituível, em vez de um framework a que está fundido.

## Onde o DORM assenta sob o Seiva

O núcleo do Seiva **nunca depende do DORM**. O acoplamento vive num único sítio —
o gateway `seiva-data-dorm`, compilado só quando ligas a funcionalidade `dorm`. A
camada de aplicação de um módulo programa contra *portas* só-trait (`UnitOfWork`,
`EventPublisher`, `Clock`, `IdGenerator`, e o seu próprio trait de repositório);
o gateway é o adaptador que satisfaz essas portas com o DORM.

```
application ──(RecordRepository / UnitOfWork ports)──▶ infrastructure
                                                         ├─ in-memory adapters   (default, DORM-free)
                                                         └─ DORM-backed adapters  (feature = "dorm")
```

O workspace público do Seiva é **livre de DORM por omissão**: um projecto cujos
repositórios usam todos uma fonte de dados externa não puxa ORM algum. Vê
[Usar o DORM a partir do Seiva](using-from-seiva.md) para o gateway, a flag de
funcionalidade e os comandos `seiva db *` que delegam no DORM.

## As crates

| Crate | O que é |
|---|---|
| `dorm-core` | O runtime: `QuerySet`/`Manager`, o compilador de consultas, `Value`, o registo de modelos, o pool de ligações e as transacções |
| `dorm-derive` | `#[derive(Model)]` — transforma uma struct num modelo e emite os seus field handles tipados e o `ModelMeta` |
| `dorm-schema` | O parser de `.schema` e o gerador de código (esquema declarativo → modelos Rust) |
| `dorm-migrations` | Geração de DDL, o autodetector de esquema e o migrador versionado |
| `dorm-sqlite` | O executor de SQLite |
| `dorm-shell` | A REPL `dorm-shell` — enumera os modelos ligados e corre consultas tipadas por string ao vivo |
| `dorm-studio` | O DORM Studio — a UI web sobre a tua base de dados viva |
| `dorm-cli` | O gestor de projectos instalável (`cargo install dorm-cli`, o binário `dorm`) |

Crates opcionais de suporte transacional — `dorm-outbox`, `dorm-jobs`,
`dorm-eventstore` — implementam portas que a camada de cima possui; o `dorm-core`
nunca depende delas. Essa fronteira está fixada na ADR 0054 do próprio DORM.

## Nesta secção

- **[Esquema e modelos](schema-and-models.md)** — a DSL `.schema`
  (`datasource`, `namespace`, `model`, campos, atributos, enums, relações) e o
  código de modelo que ela gera.
- **[Fazer consultas](queries.md)** — o `QuerySet` tipado: CRUD, filtros,
  ordenação, paginação, agregados, projecções.
- **[Relações](relations.md)** — chaves estrangeiras, filtrar através delas e
  juntar as linhas relacionadas numa só ida ao servidor.
- **[Migrações](migrations.md)** — gerar e aplicar o esquema, o livro-razão, e
  como o fluxo de trabalho aflora através do Seiva.
- **[Transacções](transactions.md)** — `atomic`, savepoints, modo só-leitura, e
  como a porta `UnitOfWork` do Seiva mapeia para o DORM.
- **[SQL cru](raw-sql.md)** — a válvula de escape parametrizada e as suas
  protecções.
- **[Backends](backends.md)** — PostgreSQL, MySQL, SQLite, CockroachDB, e o
  caminho de dados agnóstico ao DSN, TLS e pooling.
- **[A CLI e o Studio](cli-and-studio.md)** — a superfície de comandos `dorm`, a
  REPL e a GUI do Studio.
- **[Usar o DORM a partir do Seiva](using-from-seiva.md)** — o gateway, o relay
  do outbox, os comandos `seiva db *` e a funcionalidade `dorm`.

---

*Parte da [documentação do Seiva](../index.md).*
