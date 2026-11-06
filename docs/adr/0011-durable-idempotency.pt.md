# ADR 0011 — Idempotência de negócio durável como porta, reutilizando o DORM

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0002 (portas), ADR 0004 (gateway DORM)

## Contexto

Um comando com efeitos — um `POST` que cria um registo, um job, uma acção de
administração — pode ser repetido após um timeout ou uma resposta perdida. Sem
uma garantia de idempotência, a repetição corre o efeito duas vezes. O brief (§7)
exige uma garantia de idempotência **reutilizável, durável e distribuída**, e é
específico:

- A base do framework **não** pode depender de uma base de dados em particular;
  reutilizar as primitivas DORM/outbox existentes através de uma porta.
- A concorrência tem de ser arbitrada por uma unicidade/CAS durável, **não** por
  um lock em memória (que não consegue coordenar entre réplicas).
- A reserva, a mutação de domínio e a escrita do outbox devem fazer commit numa
  **só** transacção, para que uma chave committada tenha sempre o seu efeito e
  vice-versa.
- Mesma chave + pedido diferente → `409` sem executar; concluída → reproduzir o
  resultado armazenado sem re-executar; em curso → espera limitada ou conflito.
- Um teste com `HashMap` em memória **não** demonstra a garantia distribuída.

O DORM já entrega a primitiva (`dorm_core::idempotency`): `begin(key,
request_hash, ttl)` reclama uma chave com um atómico `INSERT … ON CONFLICT DO
NOTHING RETURNING` e devolve `Fresh`/`Replay(json)`/`InProgress`; uma chave
reutilizada com um hash diferente é `IdempotencyMismatch`; `finish` armazena a
resposta; as linhas participam na sessão/transacção DORM ambiente.

## Decisão

**Definir uma porta `IdempotencyStore` em `seiva-ports` e implementá-la no
gateway DORM delegando em `dorm_core::idempotency`. A base do framework é
codificada contra a porta; a garantia durável e entre réplicas vive no
adaptador.**

- `seiva_ports::IdempotencyStore` (sem dependências, orientada a bytes como o
  `EventPublisher`): `begin(scoped_key, fingerprint, ttl) -> IdempotencyOutcome`
  (`Fresh` | `Replay(bytes)` | `InProgress` | `Mismatch`) e `finish(scoped_key,
  response_bytes)`. A chave é **delimitada pelo chamador** (inquilino + identidade
  + operação + versão dobrados dentro) antes de chegar ao store, para que as
  chaves de inquilinos nunca colidam; o `fingerprint` fixa a chave aos bytes
  semanticamente relevantes de um pedido. `Mismatch` é um desfecho, não um erro,
  para que o chamador responda `409` sem executar.
- `seiva_data_dorm::DormIdempotencyStore` mapeia a porta sobre
  `dorm_core::idempotency::{begin, finish}` e re-exporta o seu DDL como
  `idempotency_migrations()`. Chamada **dentro** de `DormUnitOfWork::transaction`,
  a reserva faz commit na mesma transacção que a mutação de domínio e a escrita do
  outbox (`DormOutboxPublisher`) — a atomicidade que o brief exige. Os bytes
  opacos de resposta são armazenados sem perdas na coluna JSONB como um array de
  bytes.
- `seiva_testing::InMemoryIdempotencyStore` é um duplo de processo único apenas
  para testes de **lógica**; está documentado como NÃO sendo uma prova
  distribuída.

### Prova (a garantia distribuída, contra uma base de dados real)

O `live_idempotency_one_effect_under_concurrency` de `seiva-data-dorm`
(condicionado a `SEIVA_DORM_TEST_DSN`) conduz **20 conexões concorrentes** — o
modelo de réplica — sobre uma chave, cada uma a correr `begin → effect → finish`
numa só transacção via o gateway. Porque a reclamação é um CAS de base de dados e
o lock de linha serializa os duplicados, exactamente uma conexão é `Fresh` e a
tabela de efeito secundário termina com exactamente **uma** linha; um fingerprint
diferente é `Mismatch` (sem efeito); a chave concluída `Replay`a os exactos bytes
armazenados; uma chave diferente é independente. Passa contra PostgreSQL 16 vivo —
não um modelo em memória.

## Consequências

**Positivo.** Uma garantia de idempotência reutilizável e durável, arbitrada pela
base de dados (funciona entre réplicas); atómica com a mutação e o outbox; a base
mantém-se agnóstica à BD por trás da porta; `Mismatch`/`Replay`/`InProgress` são
de primeira classe; provada contra uma base de dados real.

**Negativo / custo.** A prova viva precisa de um PostgreSQL (com token na CI, como
os outros jobs DORM). A resposta é armazenada como um array de bytes JSONB
(simples, sem perdas; ligeiramente maior do que base64 — rever se corpos grandes
se tornarem comuns).

### Integração HTTP (entregue)

A metade de transporte vive em `seiva_http::idempotency`: um extractor
`OptionalIdempotencyKey` (valida o cabeçalho — comprimento limitado, ASCII
visível; ausente é `None`, malformado é `400`), `scoped_key` (dobra inquilino +
subject + método + path na chave para que inquilinos/identidades nunca colidam ou
se reproduzam entre si), `fingerprint` (SHA-256 canónico de método + path + query
+ o corpo como JSON de chaves ordenadas), e
`mismatch_problem`/`in_progress_problem` (`409`). Um **middleware exterior
genérico é deliberadamente evitado**: corre acima de qualquer transacção e por
isso não conseguiria manter a reserva atómica com a mutação + outbox. Em vez
disso, a aplicação conduz `IdempotencyStore::begin` dentro da sua transacção
`UnitOfWork`; um teste ponta-a-ponta (`seiva-http/tests/idempotency_http.rs`)
mostra o padrão completo (um efeito em `Fresh`, reprodução na repetição, `409` em
mismatch, `400` numa chave malformada).

### Inbox do consumidor & limpeza por TTL (entregue)

- **Dedup do inbox.** `seiva_ports::inbox_key(consumer, event_id)` compõe a chave
  do store para deduplicação do lado do consumidor; um consumidor reclama-a
  através do mesmo `IdempotencyStore::begin` dentro da transacção de consumo
  (efeito + marca juntos, ack após o commit), de modo que uma reentrega do broker
  é consumida uma só vez. Provado vivo (`live_inbox_dedup_and_purge`: o mesmo
  evento entregue duas vezes → um efeito) e offline (`seiva-testing`).
- **Limpeza por TTL.** `IdempotencyStore::purge_expired` (DORM → SQL
  `purge_expired`) mais `seiva_server::run_idempotency_purge`, um worker agendado
  sobre `run_worker` que limpa dentro de uma transacção `UnitOfWork`. Provado vivo
  (uma chave `ttl=0` é limpa e depois reclamável `Fresh`) e a cola do worker é
  testada com os duplos em memória.

## Alternativas consideradas

- **Uma nova tabela de idempotência detida pelo Seiva.** Rejeitada: o DORM já tem
  a primitiva; o brief diz para a reutilizar através de uma porta, e duplicar o
  schema bifurcaria a garantia.
- **Coordenação Redis/em memória.** Rejeitada como autoridade: não pode ser
  atómica com a mutação PostgreSQL + outbox, que é exactamente a garantia exigida.
  (Pode mais tarde servir de cache/dica de coordenação, não a autoridade.)
- **Reserva numa transacção separada do efeito.** Rejeitada para a omissão:
  admite uma chave committada sem efeito (uma falha pelo meio) a precisar de
  reconciliação. Uma só transacção faz "chave committada ⇒ efeito committado"
  valer por construção; `InProgress` mantém-se disponível para desenhos que os
  separam.

## Adenda (2026-11-05) — ligado num caminho quente de escrita via a costura `&Ctx`

A porta é agora exercida ponta a ponta numa escrita HTTP real (o `Catalog.create`
de `apps/admin-data-e2e`), provando que a metade de transporte se junta à metade
durável sem um middleware feito à medida e sem um novo parâmetro da macro do
controller:

- `seiva_http::Ctx` (a costura do lado do servidor que já levava o `Principal`
  para o âmbito de inquilino, ADR 0006/0009) leva agora também o **método + path**
  do pedido e a **`Idempotency-Key`** opcional e validada, populados no seu
  `FromRequestParts`. Uma chave presente-mas-malformada é tratada como ausente (um
  create simples e não deduplicado) e nunca armazenada — a validação continua a
  guardar a chave de armazenamento. Um handler que queira um `400` rígido usa o
  extractor `OptionalIdempotencyKey` directamente.
- O handler dobra `scoped_key(principal, method, path, key)` e um `fingerprint`
  canónico `(method, path, query, body)` (o corpo via `serde_json::to_value` do
  comando tipado — canónico, de chaves ordenadas) e chama `begin`/`finish`
  **dentro** da transacção `DormUnitOfWork`: em `Fresh` insere e armazena os bytes
  de resposta; em `Replay` devolve a resposta armazenada; em `InProgress`/`Mismatch`
  devolve `409` sem executar. O `409` é carregado por um erro de fachada portador
  de estado, por isso não é remapeado para `422`.

Provado vivo: `live_catalog_create_is_idempotent` (a repetição reproduz a resposta
armazenada idêntica; uma chave reutilizada com conteúdo diferente é `409`;
exactamente uma linha), e a Prova C de `benchmarks/production/delonix/` — 40 POSTs
idênticos concorrentes com uma chave, através do balanceador de carga para duas
réplicas, inserem exactamente uma vez.
