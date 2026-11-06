# Múltiplas bases de dados e backends

> Este é o tópico das **múltiplas bases de dados** do Seiva — a história é moldada
> pelo DORM, a camada de dados.

## O Seiva é agnóstico de base de dados na junção

Nada no Seiva fixa em código uma base de dados. Os comandos de dados passam uma DSN
directamente ao DORM, e o `connect` do gateway também recebe a DSN:

- `seiva db dev` / `migrate` lêem `DELONIX_ORM_DSN`;
- `seiva db pull` / `studio` recebem `--dsn` (ou `DELONIX_ORM_DSN`);
- o `DormConfig.dsn` do gateway `seiva-data-dorm` é a DSN do módulo.

Portanto, com que base de dados um serviço Seiva fala é uma escolha de deployment
(a DSN), não uma escolha de código. O *alcance* dessa escolha é definido pelo
runtime do DORM.

## Escolher um backend

`seiva db init --backend <postgres|mysql|sqlite>` (por omissão `postgres`) define a
feature do DORM e o `provider` do esquema; `seiva db dev`/`migrate` passam então a
visar esse backend, lendo a DSN de `DELONIX_ORM_DSN`.

## O que funciona hoje

O **PostgreSQL** é o backend principal, de primeira classe.

O **MySQL** funciona através do driver `mysql` do DORM. Provado ao vivo de ponta a
ponta através do Seiva:

```console
$ seiva db init --backend mysql
$ DELONIX_ORM_DSN='mysql://root:pw@127.0.0.1:3306/app' seiva db dev initial
```

criou a tabela `records` do módulo (`id char(36)` chave primária — `Uuid` mapeia
para `CHAR(36)` — `title text`, `version int`) e o livro-razão de migrações no
MySQL 8, a partir do modelo do módulo sem SQL escrito à mão.

O **CockroachDB** funciona com **zero configuração além da DSN** — é compatível ao
nível do wire com o PostgreSQL, por isso o backend `postgres` serve-o. Provado ao
vivo: `seiva db init` + `seiva db dev` contra um nó CockroachDB criou lá a mesma
tabela `records`. Qualquer base de dados PostgreSQL-wire (CockroachDB, e Postgres
geridos como o Neon/Supabase) é alcançável por esta via.

O **SQLite** está disponível da mesma forma (`--backend sqlite`), para uso
local/embebido.

## O que ainda não está ao vivo

**SQL Server, Oracle e MongoDB** estão no roadmap do DORM (ADR 0011): o compilador
de consultas carrega perfis de dialecto, e a junção do backend-trait (`Driver`) que
tornou o MySQL/SQLite vivos estende-se a eles à medida que o DORM acrescenta cada
driver. Quando o DORM entregar um driver, o Seiva alcança-o através do mesmo
`--backend`/DSN sem alteração aqui — o lado do Seiva já é agnóstico de backend.

## Relacionado

- [A camada de dados (DORM)](index.md) · [Migrações](migrations.md)
- [Panorama competitivo](../../competitive-landscape.md) (a linha das múltiplas bases de dados)

---

*Parte da [documentação do Seiva](../../index.md).*
