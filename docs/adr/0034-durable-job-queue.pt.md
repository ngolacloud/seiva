# ADR 0034 — `seiva-jobs`: uma fila de jobs durável com leases por cima do loop do `seiva-worker`

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0011 (idempotência durável), ADR 0012 (saúde e limites cloud-native), ADR 0021 (barramento de eventos in-process e coreografia), ADR 0028 (segurança de concorrência do worker)

## Contexto

O Seiva tem um **loop** de worker mas não uma **fila** de worker. O
`seiva-worker::run_worker` é exactamente a metade que a arquitectura atribui ao
framework: um loop genérico que repete uma função de uma passagem com um intervalo,
drain rápido, shutdown gracioso e métricas — «as funções de uma passagem pertencem à
camada de dados … o loop é do framework». Mas as *passagens* que existem são todas
feitas à medida: o `forward` do relay de outbox, o `run_once` do RAG. **Não há fila
durável de uso geral** onde um produto possa enfileirar trabalho arbitrário e confiar
que ele corre o-suficiente-uma-vez, sobrevive a um crash, faz retry, e dead-letter.

Toda a plataforma precisa disso. O delonix-meet enfileira transcrição, transcodificação
ffmpeg, geração de exports e correio de saída — trabalho longo, falível, digno de retry,
que não deve correr no caminho de pedido e tem de sobreviver a um reinício. Hoje cada um
é um loop de poll de tabela feito à mão. O ADR 0028 já decidiu o modelo de **segurança**
dos workers competitivos (`FOR UPDATE SKIP LOCKED`, uma lease, claim-effect-mark
atómico); o que falta é a **fila** genérica que empacota esse modelo como uma primitiva
reusável: `enqueue`, `claim`, `ack`/`retry`/`fail`, dead-letter, agendamento. Este é o
mecanismo horizontal; os próprios jobs são a política do produto.

As forças:

- **O loop existe; a fila é nova.** O `run_worker` já carrega temporização, drain e
  shutdown. O `seiva-jobs` fornece a *passagem* que ele conduz: reclamar um lote de jobs
  devidos sob lease, entregar cada um a um handler, registar o resultado. Não pode
  rederivar o loop.
- **Uma lease, não uma marca.** Um job reclamado mantém um **visibility timeout**: se o
  worker morre a meio do tratamento, a lease expira e o job volta à fila e é
  re-reclamado — at-least-once, nunca preso (a propriedade de recuperação que o ADR 0028
  provou).
- **Seguro a N réplicas por construção.** Consumidores competitivos têm de partilhar a
  fila via `SKIP LOCKED` sem líder e sem estrangulamento de leitor único (ADR 0028), e
  cada efeito tem de ser idempotente pela inbox (ADR 0011), para que escalar o worker seja
  o default seguro, não uma armadilha.
- **Um port, não uma tabela.** A fila é definida como um port `JobQueue` com um backend
  DORM/`sqlx` como default e um backend Redis como opção — a camada de dados fica plugável
  (a mesma linha do ADR 0030). O núcleo fica DORM-free.
- **O agendamento faz parte do mecanismo.** Jobs atrasados (`run_at`) e jobs recorrentes
  (uma expressão cron) são genéricos; o produto não deve reimplementar um agendador para
  enviar um lembrete dentro de uma hora ou correr um export nocturno.
- **Drain gracioso.** No shutdown (ADR 0012) um worker tem de parar de reclamar, terminar
  ou libertar as suas leases em voo, e sair — nenhum job é perdido e nenhum fica trancado
  para além do drain.

## Decisão

Adicionar um novo membro do workspace **`seiva-jobs`** — a fila durável; o loop fica no
`seiva-worker`, o modelo de segurança fica o do ADR 0028.

- **`JobQueue`** — o port:
  - `enqueue(job: NewJob) -> JobId` — persistir um job com um `kind`, um payload opaco,
    um `Scope` (tenant carimbado na linha), um `run_at` opcional, um `max_attempts`, e uma
    idempotency key opcional (um enqueue duplicado sob a mesma key é um no-op, ADR 0011).
  - `claim(kinds, limit, lease) -> Vec<LeasedJob>` — seleccionar atomicamente até `limit`
    jobs **devidos** (`status = 'pending' AND run_at <= now`) com `FOR UPDATE SKIP
    LOCKED`, carimbar `owner` + `lease_expires_at`, marcar `in_flight`, commit (ADR 0028).
    Um job cuja lease expira volta a `pending`.
  - `ack(job)` — marcar `done`; `retry(job, backoff)` — reagendar com `next_attempt_at`
    e um `attempt` incrementado; `fail(job)` / dead-letter — ao esgotar `max_attempts`
    mover para `dead` para inspecção/replay.
- **A passagem de claim conduz o `run_worker`.** Um `job_pass(queue, handlers)` é uma
  função de uma passagem (claim → despachar cada um ao handler do seu `kind` dentro de uma
  `UnitOfWork::transaction`, o claim-effect-mark atómico do ADR 0028 → ack/retry) que
  devolve quantos tratou; o `seiva-worker` fá-lo em loop com drain rápido e shutdown
  gracioso. Nenhum loop novo é escrito.
- **Backends atrás do port:**
  - **`DormJobQueue`** (default, atrás de feature) — uma tabela `jobs`, claim por `SKIP
    LOCKED`, indexada em `(status, run_at)`; o store durável autoritativo.
  - **`RedisJobQueue`** (opcional) — para trabalho de menor latência e menor
    durabilidade.
  - O poll usa o drain rápido + intervalo do worker; um despertar `LISTEN/NOTIFY` é uma
    optimização sobre o busy-poll onde o backend o suporta.
- **Agendamento.** O `run_at` dá jobs atrasados; um `schedule(cron, kind, payload)`
  regista um job recorrente que uma passagem de agendador materializa em jobs devidos —
  genérico, para que o produto nunca escreva um loop de cron.
- **Handlers idempotentes.** Um handler de job reclama a sua inbox key (ADR 0011) dentro
  da mesma transacção, para que uma reentrega at-least-once corra o efeito uma vez.

O `tokio` entra `default-features = false` (`sync`, `time`); os backends DORM/Redis são
features opcionais, pelo que o build default não liga nenhum backend de fila.

## Alternativas consideradas

- **Recorrer a um broker externo (RabbitMQ/Kafka/SQS) como a primitiva.** Rejeitado como
  default: acrescenta uma dependência operacional para trabalho que a base de dados que o
  produto já corre consegue carregar correctamente via `SKIP LOCKED` + leases (ADR 0028).
  Um broker pode na mesma ser um backend `JobQueue` para um deployment que o queira — o
  port torna-o uma escolha, não um requisito.
- **Um segundo loop de worker dentro do `seiva-jobs`.** Rejeitado: o `run_worker` já
  carrega intervalo, drain rápido, shutdown e métricas. Duplicá-lo bifurcaria a semântica
  do loop; a fila fornece uma *passagem*, o worker fornece o *loop*.
- **Eleição de líder / fixar o worker a uma réplica.** Rejeitado pela mesma razão do ADR
  0028: tecta o throughput num worker e precisa de um coordenador que o framework não
  entrega. Consumidores competitivos por `SKIP LOCKED` escalam sem líder.
- **Um `tokio::spawn` fire-and-forget para trabalho de fundo.** Rejeitado: uma tarefa
  spawned morre com o processo, perde o job num crash, e não pode fazer retry nem
  dead-letter. O propósito todo é a durabilidade através de reinícios.

## Consequências

- **Performance.** O `claim` em lote com `SKIP LOCKED` e um índice em `(status, run_at)`
  particiona o trabalho pelas réplicas sem contenção; o caminho de drain rápido do worker
  esvazia um backlog sem esperas ociosas; o `LISTEN/NOTIFY` substitui o busy-poll onde
  disponível. O crate não liga backend a não ser que uma feature esteja ligada — zero-cost
  quando não usado.
- **Segurança.** Cada job leva o seu `Scope` (tenant carimbado na linha) para que um
  handler corra sob o tenant certo e não possa ler entre tenants; os payloads são do
  produto e devem ser não-sensíveis ou cifrados em repouso; a fila nunca regista o
  conteúdo dos payloads.
- **Resiliência.** Uma **lease com visibility timeout** recupera um job cujo worker morreu
  a meio do tratamento (at-least-once, nunca preso, ADR 0028); retry com backoff limitado e
  um **dead-letter** após `max_attempts` limitam o raio de explosão de um job veneno;
  handlers idempotentes pela inbox (ADR 0011) tornam a garantia at-least-once segura; o
  drain gracioso no shutdown (ADR 0012) termina ou liberta as leases em voo para que nada se
  perca ou fique trancado.
- O delonix-meet enfileira transcrição, ffmpeg, export e correio como **jobs** (a sua
  política — os handlers de `kind`) e apaga os seus loops de poll feitos à mão; a fila
  genérica, o leasing, o retry e o agendamento tornam-se do framework.
- **Não** substitui uma plataforma de processamento de streams para pipelines de eventos de
  alto throughput: esta é uma fila de tarefas durável para o trabalho assíncrono do próprio
  produto, sobre a base de dados que ele já corre — não um Kafka.
