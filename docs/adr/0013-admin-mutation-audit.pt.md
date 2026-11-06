# ADR 0013 — Auditoria persistente de mutações administrativas

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0002 (portas), ADR 0006 (identidade), ADR 0008/0009 (administração)

## Contexto

O brief (§6) exige uma **auditoria persistente** das mutações administrativas:
actor, inquilino, operação, objecto, resultado, id de correlação e diferenças
**redigidas** — e **não** pode armazenar o corpo completo do pedido com os seus
segredos. A consola de administração chama as rotas normais e protegidas da API
(ADR 0008/0009), por isso "mutações de administração" são, no transporte, pedidos
mutadores de um principal autenticado.

Duas forças moldam o desenho. Primeiro, tal como a idempotência, a base não pode
depender de uma base de dados específica — o sumidouro de auditoria é uma porta.
Segundo, ao contrário da idempotência, a auditoria tem de capturar *tentativas*
incluindo recusas, e é conduzida a partir do transporte (onde um middleware vê
cada mutação e o seu desfecho) em vez de de dentro da transacção de um caso de uso
— por isso não pode depender da sessão DORM ambiente do pedido.

## Decisão

**Uma porta `AuditSink` regista valores `AuditEntry`; um middleware `audit`
regista os metadados de cada pedido mutador; o sumidouro suportado por DORM
persiste em `seiva_audit` na sua própria sessão. Os segredos são redigidos antes
de uma entrada ser sequer registada.**

- `seiva_ports::AuditEntry` (actor, inquilino, operação, objecto, desfecho,
  request_id, `details` redigidos) + `AuditSink::record`. `AuditEntry::redacted`
  mascara cada valor sensível de `details`, e `is_sensitive_field` (passwords,
  hashes, tokens, chaves, segredos, credenciais, autorização, cookies — amplo,
  insensível a maiúsculas) é a definição única que tanto o transporte como um
  handler usam.
- Middleware `seiva_http::audit`: para `POST`/`PUT`/`PATCH`/`DELETE` regista o
  actor (do `Principal` de authn), a operação (`METHOD path`), o estado do
  desfecho e o id de correlação — **apenas metadados, nunca o corpo**, para que
  nenhum segredo possa escapar por ele. Mutações recusadas (`403`) são registadas
  também. Montado dentro da camada de authn, recebe um sumidouro da raiz de
  composição. Uma escrita de auditoria falhada é registada em `warn`, nunca
  engolida, e nunca falha o pedido (a mutação já produziu a sua resposta).
- `seiva_data_dorm::DormAuditSink` acrescenta a `seiva_audit` (DDL re-exportado
  como `audit_migrations`). Abre a sua **própria** sessão por registo em vez de se
  juntar à transacção do pedido, de modo que o middleware de transporte — que
  corre acima de qualquer transacção — o possa usar. Um handler que queira detalhe
  ao nível do campo dentro da sua própria transacção constrói um `AuditEntry`
  redigido e regista-o aí em vez disso.
- `seiva_testing::RecordingAuditSink` duplica o sumidouro para asserções.

### Prova

Testes unitários: o middleware regista o actor/inquilino/operação/desfecho de uma
mutação, regista uma mutação recusada, e salta leituras; a redacção mascara
apenas valores sensíveis. Vivo (`live_audit_sink_persists_a_redacted_entry`,
condicionado a `SEIVA_DORM_TEST_DSN`) contra PostgreSQL real: uma entrada
registada é relida com os seus campos, o detalhe da password armazenado como
`[redacted]`, e o valor do segredo **nunca** presente na linha.

## Consequências

**Positivo.** Cada mutação administrativa/sensível (sucesso ou recusa) é
persistida com quem/o-quê/resultado/correlação; os segredos não podem escapar
pela auditoria; a base mantém-se agnóstica à BD por trás da porta; uma definição
de redacção é partilhada.

**Negativo / custo.** O middleware audita *metadados* automaticamente; um diff
redigido ao nível do campo é opcional por handler (registando uma entrada mais
rica). O sumidouro DORM escreve numa sessão separada, por isso uma falha entre o
commit da mutação e a escrita de auditoria pode perder um registo — aceitável
para auditoria (o desfecho, incluindo o estado pré-commit, continua a ser do
handler para registar transaccionalmente quando necessário).

## Alternativas consideradas

- **Auditar dentro da transacção do pedido (sempre).** Rejeitada como omissão: o
  middleware de transporte não tem sessão, e enfiar um sumidouro em cada handler é
  mais pesado do que um middleware de metadados. Handlers que precisem de
  auditoria transaccional continuam a poder (construir e registar uma entrada
  redigida no seu `UnitOfWork`).
- **Reutilizar `dorm-eventstore`.** Rejeitada: é event-sourcing versionado para
  agregados, não uma auditoria operacional append-only; uma tabela simples encaixa
  melhor.
- **Armazenar o corpo completo do pedido.** Rejeitada categoricamente pelo brief —
  persistiria segredos; metadados + detalhes redigidos opcionais é a regra.
