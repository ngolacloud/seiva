# Backends

O DORM é genérico quanto ao backend na costura: o compilador de consultas carrega
um `Dialect`, a costura do runtime é um `Driver` que o `Database` detém, e cada
backend declara as capacidades que suporta, por isso o núcleo **degrada ou recusa
com um erro nomeado em vez de emitir SQL silenciosamente errado**. A base de dados
com que um serviço fala é uma decisão de implantação — o DSN — e não uma mudança
de código.

## O caminho de dados agnóstico ao DSN

Nada no Seiva fixa à força uma base de dados. Os comandos de dados passam um DSN
directamente ao DORM:

- `seiva db dev` / `seiva db migrate` lêem `DELONIX_ORM_DSN`;
- `seiva db pull` / `seiva db studio` recebem `--dsn` (ou `DELONIX_ORM_DSN`);
- o `DormConfig.dsn` do gateway `seiva-data-dorm` é o DSN do módulo.

Dentro do próprio DORM, o DSN vem do ambiente:

| Variável | Papel |
|---|---|
| `DELONIX_ORM_DSN` | O DSN que o DORM usa. Verificado primeiro. |
| `PAAS_DATABASE_URL` | Aceite como alternativa quando `DELONIX_ORM_DSN` está por definir. |

Tanto a forma de palavras-chave libpq como a forma de URL funcionam; se nenhuma
das variáveis estiver definida, a primeira consulta devolve um erro de
configuração que nomeia ambas, não um pânico.

```sh
export DELONIX_ORM_DSN='host=/var/run/postgresql user=me dbname=blog'
export DELONIX_ORM_DSN='postgresql://me:secret@localhost:5432/blog'
```

> **Numa aplicação Seiva** a biblioteca lê **nenhuma** variável de ambiente — o
> gateway entrega ao DORM a sua configuração (DSN, réplicas, pool, TLS, chave de
> cursor) por programação e por módulo. O DSN de ambiente acima é para a CLI e
> para um programa DORM autónomo. Vê
> [Usar o DORM a partir do Seiva](using-from-seiva.md).

## Escolher um backend

`seiva db init --backend <postgres|mysql|sqlite>` (por omissão `postgres`) define
a funcionalidade DORM e o `provider` do esquema; `seiva db dev`/`migrate` passam
então a visar esse backend através do DSN.

## O que funciona hoje

O **PostgreSQL** é o backend primário, de primeira classe — o alvo contra o qual o
DORM está mais endurecido e aquele onde todas as funcionalidades aterram primeiro.

O **MySQL** funciona através do executor MySQL do DORM. As diferenças de dialecto
são tratadas por ti: marcadores `?`, aspas de backtick, sem `RETURNING` (o DORM lê
de volta a chave via `last_insert_id`), booleanos `TINYINT(1)`. Provado ao vivo de
ponta a ponta através do Seiva — `Uuid` mapeia para `CHAR(36)`, e o livro-razão de
migrações é criado em MySQL 8 a partir do modelo do módulo sem SQL escrito à mão.

O **CockroachDB** funciona com **zero configuração para além do DSN**: é
compatível com o protocolo de fios do PostgreSQL, por isso o backend `postgres`
serve-o sem alterações. Qualquer base de dados com protocolo de fios PostgreSQL
alcançada desta forma — o CockroachDB, e ofertas de Postgres geridas — não precisa
de mudança de código.

O **SQLite** está disponível com `--backend sqlite`, para uso local e embebido. O
seu executor usa um pool de leitores com um único escritor sob WAL em vez de uma
ligação partilhada.

## O que ainda não está ao vivo

**SQL Server, Oracle e MongoDB** estão no roteiro multi-backend do DORM. A base é
a costura dialecto/driver/capacidade que pôs o MySQL e o SQLite ao vivo; cada novo
backend SQL é "implementar os ramos do dialecto + driver + mapeamento de tipos +
declaração de capacidade", localizado em vez de uma edição dispersa. O MongoDB não
é SQL e quebra o modelo `Plan → SQL`, por isso está explicitamente barrado atrás
de um spike e de uma decisão — *não* está comprometido. Quando o DORM entregar um
driver, o Seiva alcança-o através do mesmo `--backend`/DSN sem mudança do lado do
Seiva, que já é agnóstico ao backend.

## TLS

O TLS é escolhido pelo `sslmode` do DSN, ou sobreposto por `DELONIX_ORM_SSLMODE` /
`DELONIX_ORM_SSLROOTCERT`. O DORM nunca encripta pela metade — os modos libpq
oportunistas (`prefer`, `allow`) e um `sslmode` ausente significam texto simples;
pedes `require` ou `verify-full` para obteres TLS.

| `sslmode` | Significado | Estado |
|---|---|---|
| `disable` *(por omissão)* | Texto simples. | suportado |
| `require` | Encripta; **não** verifica o certificado. | suportado |
| `verify-full` | Encripta **e** verifica a cadeia e o hostname. Raízes de `sslrootcert`, senão as raízes Mozilla incluídas. | suportado |
| `verify-ca` | Verifica a cadeia, não o hostname. | ainda não — usa `verify-full` ou `require` |

```sh
export DELONIX_ORM_DSN='postgresql://me@db.internal:5432/blog?sslmode=verify-full&sslrootcert=/etc/ssl/ca.pem'
```

A implementação de TLS é uma funcionalidade Cargo do `dorm-core`: `tls-rustls`
(por omissão, Rust puro, compila offline) ou `tls-openssl` (libssl do sistema).
Sem nenhuma compilada, um DSN que peça TLS é um erro de configuração — nunca uma
degradação silenciosa.

## Pooling e a regra de um-só-runtime

O DORM possui um pool de ligações, afinado por ambiente (para a CLI) ou por
`ConnectOptions` (para um framework):

| Variável | Por omissão | Significado |
|---|---|---|
| `DELONIX_ORM_POOL_MAX` | 8 | Tecto de ligações no pool. |
| `DELONIX_ORM_POOL_ACQUIRE_TIMEOUT_MS` | 5000 | Quanto tempo um chamador espera por uma ligação antes de desistir. |
| `DELONIX_ORM_STATEMENT_TIMEOUT_MS` | 0 (desligado) | O `statement_timeout` do lado do servidor; um valor positivo cancela uma instrução que se prolonga e devolve `Error::Timeout`. Também limita o DDL, por isso deixa-o desligado enquanto migras, salvo se for mesmo essa a intenção. |

Como cada ligação é conduzida por uma tarefa de fundo, uma ligação no pool
pertence ao runtime tokio que a criou:

> **O processo tem de manter um runtime tokio vivo enquanto usar o DORM.**

Num serviço Seiva assíncrono isto é automático. Um programa síncrono a conduzir o
DORM através de `block_on` tem de construir o seu runtime **uma só vez** e
reutilizá-lo — um runtime por instrução funciona para exactamente uma instrução.

## Relacionado

- [Transacções](transactions.md) · [SQL cru](raw-sql.md) ·
  [Usar o DORM a partir do Seiva](using-from-seiva.md)
- Visão de framework: [Múltiplas bases de dados e backends](../topics/data-layer/multiple-databases.md)

---

*Parte da [documentação do Seiva](../index.md).*
