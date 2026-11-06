# Parte 3: Persistir com a camada de dados

Até aqui a `polls` mantém as suas perguntas e opções em memória. Nesta parte
movemo-las para uma base de dados real com a camada de dados DORM — contra
PostgreSQL, MySQL ou qualquer base de dados PG-wire. Para a história completa da
camada de dados, ver a [secção DORM](../../dorm/index.md).

## Configurar a camada de dados

```console
$ seiva db init                 # PostgreSQL (default); or --backend mysql | sqlite
```

`db init` gera `data/` (o projecto DORM: schema, migrações, os binários de
migração), escreve `data/dorm.schema` a partir do `schema/polls-site.schema` do teu
projecto — por isso os `namespace`s dos módulos e os seus modelos comandam-no — e
define o provider da datasource para o backend escolhido.

O schema é uma pequena DSL ao estilo Prisma — um bloco `datasource` mais um
`namespace` por módulo. No fim desta parte o `namespace` `polls` lê-se assim:

```schema
datasource db {
  provider = "postgresql"
}

namespace polls {
  model Question {
    id    Uuid   @id @default(uuid())
    text  String
  }

  model Choice {
    id          Uuid   @id @default(uuid())
    question_id Uuid
    text        String
    votes       Int    @default(0)
  }
}
```

## Escrever e aplicar uma migração

Aponta o `DELONIX_ORM_DSN` para a tua base de dados e corre o dev loop (o
equivalente no Seiva a `makemigrations` + `migrate`):

```console
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=polls sslmode=disable"
$ seiva db dev initial
generated src/models.rs from dorm.schema (2 model(s))
created migration 1 (initial) — 2 statement(s) in migrations/
applied version(s): [1]
```

Isso criou as tabelas `questions` e `choices` do módulo `polls` (cada uma com
`id`, o seu texto, e a contagem `votes` e o `question_id` da opção) e o registo de
migrações. `seiva db dev <name>` sincroniza sempre o schema, regenera os modelos e
escreve+aplica a migração seguinte; `seiva db migrate` implanta a cadeia registada
(produção). Ver [Migrações](../../topics/data-layer/migrations.md).

## Uma base de dados diferente — os mesmos comandos

O caminho de dados é agnóstico ao DSN. Para MySQL:

```console
$ seiva db init --backend mysql
$ DELONIX_ORM_DSN='mysql://root:pw@127.0.0.1:3306/polls' seiva db dev initial
```

PostgreSQL, CockroachDB (PG-wire) e MySQL são todos backends ao vivo — ver
[múltiplas bases de dados](../../topics/data-layer/multiple-databases.md).

## Ligar o adaptador DORM (próximo passo)

O módulo ainda usa o `PollRepository` em memória no composition root; trocá-lo pelo
adaptador suportado por DORM (atrás da feature `dorm`) é a ligação de produção —
coberta no [guia da camada de dados](../../topics/data-layer/index.md). Os casos de
uso não mudam: já programam contra a porta `PollRepository`, e uma transacção por
caso de uso (ADR 0004) significa que um voto e o seu evento `polls.voted` fazem
commit juntos ou de todo.

## Brownfield

Já tens uma base de dados? `seiva db pull` faz a introspecção dela para blocos de
modelo que dobras nos teus módulos — ver [a referência da CLI](../../reference/cli.md).

## A seguir

[Parte 4: Autenticação e permissões](part-4-authentication.md).

---

*Parte da [documentação do Seiva](../../index.md).*
