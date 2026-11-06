# Migrações

As migrações do Seiva são as do DORM, conduzidas a partir do esquema do teu
projecto. Como um projecto Seiva é compatível com o DORM (`seiva db init`
configura a camada de dados sob `data/`), o fluxo corresponde ao
`migrate dev`/`deploy` do Prisma: um passo de autoria que gera a migração seguinte
e um passo de deploy que aplica a cadeia registada.

## Configurar a camada de dados uma vez

```console
$ seiva db init          # scaffolds data/ (dorm.schema, migrations/, the lanes)
```

`db init` escreve `data/dorm.schema` a partir do `schema/<project>.schema` do teu
projecto — os `namespace`s dos módulos e os seus modelos — e gera o código dos
modelos. O esquema do teu projecto é a fonte de verdade; `data/` é o motor de
migração.

## O ciclo de desenvolvimento

```console
$ seiva module new notes       # adds a `namespace notes { model Record … }`
$ seiva db dev add_notes       # sync schema → generate → author + apply migration
```

`seiva db dev <name>` faz três coisas por ordem:

1. **sync** — reescreve `data/dorm.schema` a partir de `schema/<project>.schema`,
   para que os modelos de cada módulo estejam actualizados;
2. **generate** — regenera o código dos modelos a partir desse esquema;
3. **migrate dev** — compara os modelos com a história registada em
   `data/migrations/`, escreve a nova migração e aplica-a.

Precisa de uma base de dados: define `DELONIX_ORM_DSN` (PostgreSQL, ou qualquer
motor PG-wire como o CockroachDB — ver [múltiplas bases de dados](multiple-databases.md)).

## Deploy

```console
$ seiva db migrate       # apply the recorded chain (prisma migrate deploy)
```

`seiva db migrate` aplica as migrações já redigidas em `data/migrations/`, por
ordem, registando cada uma no livro-razão `orm_schema_migrations`. Não redige nada
e não precisa de sincronização de esquema — é a via de produção, segura para correr
em cada deploy (é idempotente: as migrações já aplicadas são saltadas).

## O que fica registado

- `data/migrations/NNNN_<name>.json` — os ficheiros de migração ordenados (o SQL,
  um `down` reversível e um snapshot do esquema). Faz commit destes.
- `orm_schema_migrations` (na base de dados) — o livro-razão das versões aplicadas.

## Estado e rollback

As vias subjacentes do DORM também oferecem `migrate status` (o que a BD aplicou),
`migrate down <version>` (reverter, quando a migração é reversível) e um lint de
segurança para actualizações progressivas; estão disponíveis através do CLI `dorm`
em `data/` e vão aparecer como subcomandos `seiva db` à medida que o fluxo
amadurece.

## Relacionado

- [Modelos e esquema](models.md) · [A camada de dados](index.md)
- [Múltiplas bases de dados](multiple-databases.md) · [O CLI `seiva`](../../reference/cli.md)

---

*Parte da [documentação do Seiva](../../index.md).*
