# ADR 0021 — Um bus de eventos in-process fecha o ciclo do outbox; os módulos choreografam-se da mesma forma seja monólito ou microserviço

- Status: Aceite
- Data: 2026-11-08
- Decisores: NgolaCloud
- Segue: ADR 0002 (ports), ADR 0003 (camadas de módulo), ADR 0011 (idempotência durável / dedup de inbox)

## Contexto

A promessa central do Seiva é que **os módulos cooperam através de eventos, não de
chamadas**, e que o mesmo código de módulo corre como monólito modular ou como
microserviço *só por composição* — é isto que torna o `seiva module split` uma
mudança de composição em vez de uma reescrita (CLAUDE.md). O fluxo canónico está lá
declarado: `orders.placed → inventory.reserved → payments.captured →
notifications.sent`, montado pela raiz de composição a reagir a eventos.

Metade disto já está construída e provada:

- Um caso de uso de escrita publica o seu domain event **dentro** da sua transacção
  `UnitOfWork`, através do port `EventPublisher`; com DORM isso aterra num
  **outbox transaccional** (`DormOutboxPublisher`), pelo que dados e evento
  confirmam atomicamente (ADR 0011, `seiva-data-dorm`).
- O outbox é drenado para um broker **at-least-once** pelo `seiva-outbox-relay`
  (avançando o cursor só após uma publicação bem-sucedida), o envelope de fio é
  **CloudEvents 1.0** com backends NATS/AMQP/Kafka (`seiva-messaging`), e o lado
  consumidor desduplica com uma chave de **inbox** através do mesmo
  `IdempotencyStore` para que uma reentrega seja consumida uma só vez (ADR 0011).

A outra metade falta. **Não há bus de eventos in-process**: as apps ligam o
`NoopEventPublisher` (o evento é descartado), e **nenhuma choreography está montada**
em nenhuma app — o fluxo `orders → inventory` é aspiracional. Logo um módulo ainda
não consegue reagir ao evento de outro módulo *num único processo*, que é
exactamente o deployment em monólito que o framework afirma suportar de origem.

As forças:

- Um módulo publica um domain event; outro módulo **reage sem importar o seu crate**
  — preservando a independência que faz o `module split` funcionar.
- Num **monólito** a reacção acontece in-process; em **microserviços** acontece via
  broker. O *código do subscriber tem de ser byte-idêntico* nos dois casos, senão
  «o split é só composição» é falso.
- As garantias já estabelecidas — publicar-dentro-da-transacção, entrega
  at-least-once, consumir-uma-vez via inbox — têm de valer também para o caminho
  in-process, senão um monólito seria mais fraco do que a sua forma dividida.
- O desenvolvimento local tem de correr a choreography **sem levantar um broker**.

## Decisão

**Acrescentar um bus de eventos in-process na raiz de composição, e fazer com que a
*única* diferença entre monólito e microserviço seja o despachante que move os
eventos do outbox para os subscribers — não o publicador, não o subscriber, não as
garantias.** O outbox mantém-se o ponto único de publicação nos dois modos.

- **A publicação não muda.** Um caso de uso continua a publicar através do
  `EventPublisher` para o outbox transaccional, dentro da sua transacção. **Não**
  acrescentamos um caminho directo in-process `publish → chamar subscriber`, porque
  esse evento não seria durável (uma falha entre o commit e a entrega in-process
  perde-o) e divergiria do caminho do broker.
- **Dois despachantes, um contrato.** Um evento sai do outbox por uma de duas vias:
  - **despachante in-process** (novo): lê o outbox e invoca os subscribers
    registados localmente, marcando cada entrega uma só vez através do inbox
    (`inbox_key` + `IdempotencyStore`, ADR 0011) — o gémeo in-process do
    `seiva-outbox-relay::drain`;
  - **relay → broker** (já existe): o `seiva-outbox-relay` reencaminha para
    NATS/AMQP/Kafka, e o consumidor remoto drena com a mesma dedup de inbox.
  Porque ambos lêem o mesmo outbox e ambos desduplicam pelo mesmo inbox, um handler
  de subscriber é **idêntico** nos dois, e a entrega é at-least-once com
  exactamente-um *efeito* de qualquer forma.
- **Os subscribers são registados na raiz de composição, por tipo, não por import.**
  Um subscriber é um handler sobre um domain event (o CloudEvent desserializado para
  o seu payload tipado) que corre dentro da **sua própria** transacção
  `UnitOfWork::transaction` com uma reivindicação `inbox_key` (efeito + marca
  juntos, ack após commit) — o padrão de consumo que o ADR 0011 já define. Um módulo
  nunca nomeia o crate de outro módulo; a raiz de composição é onde `orders.placed`
  é encaminhado para o subscriber de inventário.
- **Prova por efeito.** Substituir o `NoopEventPublisher` numa app (o shop) e montar
  `orders.placed → inventory.reserved`, demonstrado primeiro sob o despachante
  in-process (sem broker) e depois sob o caminho relay→broker, com o mesmo código de
  subscriber e uma reserva por encomenda nos dois.

## Alternativas consideradas

- **Entrega directa in-process (o publish chama o subscriber após o commit, sem
  outbox).** Mais simples, mas rejeitada: não é durável (um evento perde-se se o
  processo morrer depois do commit mas antes da entrega), e diverge do caminho do
  broker, pelo que o subscriber se comportaria de forma diferente num monólito e
  depois de um split — quebrando a promessa central do framework.
- **Deixar os módulos importarem-se e chamarem-se uns aos outros (orders chama
  inventory directamente).** Rejeitada: reintroduz o acoplamento em compile-time que
  o `module split` existe para evitar; todo o ponto é que a cooperação sobrevive a
  ser puxada para processos separados sem alteração.
- **Correr um broker mesmo num monólito (nenhum caminho in-process).** Rejeitada:
  obriga cada deployment de processo único e cada `cargo run` local a levantar
  NATS/Kafka, contrariando «um monólito por composição» e prejudicando o ciclo de
  desenvolvimento.
- **Um bus in-process tipado com um mapa de subscribers `TypeId → Box<dyn Any>`.**
  Alguma indirecção em runtime na *fronteira de composição* é inevitável — os
  subscribers são ligados na raiz e o despachante desserializa cada CloudEvent para
  o tipo concreto do subscriber. Aceitamos essa indirecção, mas **confinamo-la à
  aresta de composição/despacho**: nunca toca no caminho de dispatch do próprio caso
  de uso, que se mantém estaticamente tipado (contraste com o ADR 0020). O handler
  do subscriber em si é um handler tipado normal.

## Consequências

- **Fica mais fácil:** o princípio declarado do framework — os módulos cooperam
  através de eventos — torna-se real e provado, não aspiracional; o `module split`
  passa a ser genuinamente uma troca de despachante (in-process ↔ relay+broker); o
  desenvolvimento local corre a choreography completa sem infra-estrutura.
- **Fica mais difícil / dívida (datada 2026-11-08):** o despachante in-process que lê
  o outbox e distribui pelos subscribers é código novo (parente próximo do relay
  existente); a ordenação de entrega, o retry/back-off e a back-pressure do caminho
  in-process têm de ser especificados, não assumidos. O registo de subscribers na
  raiz de composição é um passo de ligação em runtime (aceite, confinado à aresta
  como acima).
- **Imposição:** um teste de que o *mesmo* handler de subscriber produz um efeito por
  evento sob o despachante in-process e sob o drain do broker; uma prova viva de
  `orders.placed → inventory.reserved` na app shop; um check de fitness de que nenhum
  crate de módulo depende de outro crate de módulo (a independência de que o
  `module split` depende).

## Adenda — Fase 2: durabilidade (despacho transaccional, retry/back-off, dead-letter)

- Data: 2026-11-13
- Estado: Aceite (estende a decisão acima; altera as assinaturas públicas de
  `seiva_events::dispatch` / `drain_in_process` / `run_in_process_dispatch_worker`
  e acrescenta um método com default ao port `IdempotencyStore`)

O despachante da Fase 1 provou a coreografia offline mas deixou três lacunas que a
nota «Fica mais difícil / dívida (2026-11-08)» antecipava — o retry, o back-off e a
back-pressure estavam «especificados, não assumidos». A Fase 2 especifica-os e fecha
um bug latente de correcção.

**O bug.** O `dispatch` da Fase 1 reclama a chave de inbox (`begin` → `Fresh`), corre
o handler e depois faz `finish` — **sem transacção à volta dos três**. Se o handler
falha depois de um claim `Fresh`, o erro propaga-se e o driver deixa o evento pendente
(bom), mas o claim fica a meio: a redistribuição seguinte vê `InProgress`, não faz
**nenhum** efeito, devolve `Ok`, e o driver **avança o cursor para além do evento** —
descartando-o em silêncio. Uma falha transitória perde assim o evento. O caminho do
broker nunca teve este bug porque o seu claim e efeito já partilham uma transacção
`db::atomic` (`seiva-outbox-relay::drain`), por isso um handler falhado faz rollback do
claim e o retry volta a ser `Fresh`.

**Decisão.**

1. **O `dispatch` corre `claim → handle → finish` dentro de `UnitOfWork::transaction`**,
   uma transacção por subscriber (o padrão de consumo que a ADR 0011 já define, agora
   real para o caminho in-process). Um novo parâmetro obrigatório `uow: &U` transporta-a.
   Com o gateway DORM o claim, as escritas do handler e a marca commitam atomicamente e
   um rollback des-reclama a chave de graça — exactamente a garantia do caminho broker.
   A transacção corre sob `Scope::Global`: o claim de inbox é bookkeeping de
   infra-estrutura do próprio driver (o `scoped_key` já funde tenant + subscriber +
   evento conforme a ADR 0011), e o envelope `CloudEvent` não transporta tenant na Fase 2
   (o scoping por-evento fica adiado — ver abaixo).
2. **O `IdempotencyStore` ganha um `release(scoped_key)` com default.** Um store cujo
   `begin` entra na transacção do chamador (DORM) des-reclama no rollback
   automaticamente, por isso o seu `release` é o no-op default. Um store que **não**
   participa no `UnitOfWork` (a dupla in-memory, e qualquer store que um utilizador ligue
   sem transacção partilhada) sobrepõe-no para largar um claim não-terminado. O
   `dispatch` chama `release` só para um claim que **ele** obteve `Fresh` e cuja
   transacção falhou — nunca uma chave `Replay`/`InProgress`/`Mismatch` que não lhe
   pertence — por isso nenhum claim concorrente é perturbado. Isto mantém «um rollback
   des-reclama a chave» verdadeiro para todos os stores, não só o DORM, com um único
   corpo genérico de `dispatch`.
3. **O `drain_in_process` desvia um evento venenoso para dead-letter em vez de bloquear
   a fila.** Uma `RetryPolicy { max_attempts, base_backoff }` governa retries por-evento,
   dentro do passo, com back-off exponencial; um evento que continua a falhar após
   `max_attempts` é entregue a um `DeadLetterSink` e o cursor avança **para além** dele,
   para o evento seguinte progredir. Sem isto, a Decisão 1 (que faz os retries voltarem a
   correr mesmo) deixaria um evento permanentemente falhado a fazer retry para sempre e a
   matar à fome todos os eventos seguintes — trocando um descarte silencioso por uma
   paragem silenciosa. O `LoggingDeadLetterSink` é o default de produção; o
   `InMemoryDeadLetterQueue` (atrás da feature `testing`) torna o desvio observável
   offline.

**Alternativas consideradas (Fase 2).**

- *`release` explícito chamado sempre pelo `dispatch` em qualquer erro.* Rejeitado:
  dispararia também em `Mismatch` (uma chave que o `dispatch` nunca reclamou) e poderia
  correr em competição com um claim `InProgress` concorrente da mesma chave. Condicionar
  o `release` a «obtivemos `Fresh` nesta transacção» é preciso e sem corridas.
- *Um journal de transacção task-local a acoplar a UoW in-memory e o store* (espelhando a
  sessão ambiente do DORM). Rejeitado por ser mais pesado e por alterar o comportamento
  de todos os testes `InMemoryUnitOfWork` existentes; o `release` com default fica local
  ao único caminho que dele precisa.
- *Contagem durável de tentativas entre passos + back-off temporizado* (um evento
  venenoso repetido ao longo de passos do worker em vez de dentro de um). Adiado: exige
  um store durável de tentativas por-evento; o retry limitado dentro do passo +
  dead-letter já satisfaz «o evento venenoso deixa de bloquear os seguintes» e mantém o
  driver sem estado. O caminho broker mantém a sua própria semântica de retry.

**Consequências (Fase 2).**

- **Fica mais fácil:** uma falha transitória do handler é agora genuinamente repetida
  (não descartada); uma falha permanente é posta em quarentena (não uma paragem); a
  durabilidade do monólito iguala a do caminho broker.
- **Fica mais difícil / dívida (datada 2026-11-13):** os sleeps de back-off dentro do
  passo bloqueiam o único loop de drain enquanto um evento espera — aceitável para a
  escala do driver in-process, revisto se um deployment precisar de retry concorrente
  por-evento. O envelope `CloudEvent` ainda não transporta tenant, por isso a transacção
  do `dispatch` é `Scope::Global`; acrescentar um tenant ao envelope e fazer scoping
  por-evento é trabalho futuro.
- **Imposição:** testes offline de que (a) um handler que falha uma vez é re-corrido
  `Fresh` na redistribuição (o des-reclamar), e (b) um evento sempre-a-falhar aterra no
  `InMemoryDeadLetterQueue` enquanto o evento seguinte processa; mais a app shop viva a
  drenar via `run_in_process_dispatch_worker` montado em `build_app`.

## Relacionado

- [ADR 0002](0002-core-ports-and-test-doubles.md) · [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0020](0020-command-bus-and-pipeline-behaviors.md)

---

*Parte da [documentação do Seiva](../index.md).*
