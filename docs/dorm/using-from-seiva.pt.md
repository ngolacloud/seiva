# Usar o DORM a partir do Seiva

O DORM e o Seiva são dois produtos. O Seiva *consome* o DORM, através de uma
pequena crate de gateway e atrás de uma flag de funcionalidade, e a dependência é
estritamente de sentido único: o Seiva depende do DORM, nunca o contrário. Esta
página mostra toda a costura — como um build de Seiva liga o DORM, que crate detém
o acoplamento, e como os comandos `seiva db *` se relacionam com a CLI `dorm`.

## O workspace público é livre de DORM por omissão

O núcleo do Seiva nunca depende do DORM. O ponto de entrada do framework reporta-o
sem rodeios: um build que não active a funcionalidade `dorm` não carrega ORM, e
`seiva doctor` aflora uma flag `DORM_ENABLED` para que um operador veja num relance
que camada de dados um binário liga.

Isto é deliberado. Um módulo cujos repositórios falam todos com uma fonte de dados
externa não deve arrastar um ORM, e os adaptadores em memória por omissão permitem
construir e correr um serviço completo **hoje sem base de dados** — que é também o
que mantém os casos de uso testáveis em unidade. Desligar o DORM é simplesmente
*não compilar a crate de gateway*.

## A funcionalidade `dorm` e o gateway

Todo o acoplamento ao DORM vive numa única crate — o gateway `seiva-data-dorm` —
compilada só quando activas a funcionalidade `dorm`. O resto do Seiva depende só
das portas só-trait em `seiva-ports`. A camada `infrastructure` de um módulo usa o
gateway para construir o seu handle DORM e para satisfazer as portas que a sua
`application` declarou.

```
application  ──(ports: UnitOfWork, EventPublisher, Clock, IdGenerator, RecordRepository)──▶
                                                                                      infrastructure
                                                                                        └─ seiva-data-dorm (feature = "dorm")
```

O gateway depende do DORM por versão de crate publicada e implementa as quatro
portas centrais:

- **`Clock`** — o relógio real do sistema.
- **`IdGenerator`** — um gerador de ids respaldado por UUIDv4.
- **`UnitOfWork`** — via `DormUnitOfWork`, mapeando a porta baseada em âmbito para
  o `with_session` + `db::atomic` do DORM (vê
  [transacções](transactions.md#how-seivas-unitofwork-maps-onto-dorm)).
- **`EventPublisher`** — via `DormOutboxPublisher`, escrevendo cada evento no
  outbox transacional *na transacção actual*.

## `connect` — e os dois invariantes que impõe

Um framework entrega ao gateway um `DormConfig` (por módulo: o seu DSN, réplicas de
leitura, esquema PostgreSQL, credenciais), mapeado para as próprias
`ConnectOptions` do DORM. `connect(&DormConfig)` constrói o `Database` do módulo
e, crucialmente, impõe os dois invariantes de fronteira que um framework não pode
esquecer:

- **Sessões estritas ligadas** — sem `DEFAULT_DB` global ao processo em silêncio.
  Uma consulta sem sessão é um erro tipado, por isso cada pedido corre numa sessão
  explícita por módulo (`with_session`), não num handle ambiente.
- **A chave de cursor por keyset configurada** — sem valor por omissão
  falsificável. A chave de assinatura para os [cursores por keyset](queries.md#keyset-pagination-for-lists)
  é definida por programação, por isso os cursores de paginação não podem ser
  falsificados.

```rust
use seiva_data_dorm::{connect, DormConfig};

let db = connect(&DormConfig {
    dsn: module_dsn,
    replicas: vec![],
    // …per-module schema, pool, TLS
})?;
```

A própria biblioteca DORM lê **nenhuma** variável de ambiente neste caminho — o
gateway fornece tudo por API. (A variável de ambiente `DELONIX_ORM_DSN` é para a
via de CLI abaixo e para um programa DORM autónomo.)

## O relay do outbox

O `EventPublisher` respaldado pelo DORM só escreve no outbox; algo tem de
encaminhar essas linhas para o broker. Esse é o worker `seiva-outbox-relay`. Uma
passagem drena um lote do outbox em base de dados e publica cada evento através de
um `Publisher` de mensagens, **pelo-menos-uma-vez**: o cursor avança só depois de
um publish bem-sucedido, por isso um publish falhado faz retry, e após um limite o
evento vai para dead-letter.

A divisão de trabalho corresponde à regra de fronteira do DORM: *consumir* o
outbox em base de dados é do DORM (`dorm-outbox` oferece um `relay` de uma
passagem), enquanto o **ciclo** à sua volta — intervalo, encerramento gracioso,
métricas, eleição de líder — pertence ao runtime de workers do framework. É por
isto que `publish` tem de ser chamado dentro de uma `transaction`: o evento e a
mudança de negócio fazem commit em conjunto, e o relay entrega depois. Vê o guia
de [mensagens e eventos](../topics/messaging-and-events.md) do framework.

## Os comandos `seiva db *`

No dia a dia não chamas o `dorm` directamente; a CLI `seiva` envolve-o. Cada
comando delega na CLI `dorm` (encontrada no `PATH`, ou via `SEIVA_DORM_BIN`):

| Comando | Delega em | Faz |
|---|---|---|
| `seiva db init [--backend postgres\|mysql\|sqlite]` | `dorm init` | Faz o scaffold da camada de dados DORM do projecto sob `data/` (esquema, migrações, os binários de migração). Corre uma vez. Define a funcionalidade DORM e o `provider` do esquema. |
| `seiva db dev [name]` | `dorm migrate dev` | O ciclo de dev: sincroniza `data/dorm.schema` a partir do esquema do teu projecto, regenera modelos, autoriza + aplica a próxima migração. |
| `seiva db migrate` | `dorm migrate` | A via de implantação: aplica a cadeia registada por ordem. Idempotente; não autoriza nada. |
| `seiva db pull [--out] [--dsn]` | `dorm db pull` | Brownfield: introspecciona uma base de dados viva para blocos de modelo que dobras nos `namespace`s dos teus módulos. |
| `seiva db studio [--write] [--port] [--dsn]` | `dorm studio` | O administrador de dados no navegador, só-leitura salvo `--write`, só em loopback. |

O DSN flui directamente — `seiva db dev`/`migrate` lêem `DELONIX_ORM_DSN`;
`pull`/`studio` recebem `--dsn` ou recorrem a ele. Vê
[Migrações](migrations.md#the-seiva-workflow) para o fluxo de trabalho de autoria e
[A CLI e o Studio](cli-and-studio.md) para a superfície `dorm` que estes envolvem.

## Porque a fronteira está onde está

O DORM é a camada de fonte de dados: uma base de dados, uma transacção, um
processo. As coisas cuja garantia vem da comunicação entre serviços, do tempo, da
identidade do chamador, ou de política **não** são do ORM — são do Seiva. É por
isso que uma API REST gerada, contas de utilizador, clientes de broker e ciclos de
worker de longa duração que outrora viveram no DORM são agora responsabilidade do
framework (ADR 0054 do DORM), e porque o gateway, o ciclo de relay e a sessão com
âmbito de pedido vivem todos do lado do Seiva da linha. O resultado: um utilizador
simples de CLI do DORM não perde nada, e um serviço Seiva obtém o ORM como uma
dependência substituível e bem cercada.

## Relacionado

- [Transacções](transactions.md) · [Migrações](migrations.md) ·
  [Backends](backends.md) · [A CLI e o Studio](cli-and-studio.md)
- Os guias temáticos da camada de dados:
  [visão geral](../topics/data-layer/index.md),
  [transacções](../topics/data-layer/transactions.md)
- [ADR 0004 — o gateway da camada de dados DORM](../adr/0004-dorm-data-layer-gateway.md)

---

*Parte da [documentação do Seiva](../index.md).*
