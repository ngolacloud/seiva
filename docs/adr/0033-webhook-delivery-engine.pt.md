# ADR 0033 — Um motor de entrega de webhooks durável e com retry por cima da assinatura já existente do `seiva-webhooks`

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0011 (idempotência durável), ADR 0021 (barramento de eventos in-process e coreografia), ADR 0026 (controlo de carga por rota), ADR 0034 (fila de jobs durável)

## Contexto

O Seiva já assina webhooks correctamente. O `seiva-webhooks` implementa o esquema
[Standard Webhooks] — um `HMAC-SHA256` sobre `{id}.{timestamp}.{payload}`, os headers
`webhook-id` / `webhook-timestamp` / `webhook-signature`, uma janela de replay com
relógio de confiança, rotação de chaves multi-assinatura. Essa é a metade
*criptográfica* difícil, e está feita. **A assinatura não é a lacuna.**

O que falta é tudo em volta da assinatura: pegar num evento de domínio, torná-lo numa
chamada HTTP de saída para o endpoint de um tenant, e depois fazer essa chamada de forma
**fiável** — porque um receptor está em baixo, lento ou instável muito mais vezes do que
está saudável-e-instantâneo. Hoje um produto que quer «disparar um webhook quando X
acontece» tem de fazer à mão o loop de entrega: a quem enviar, retry na falha, quantas
vezes, com que backoff, para onde vão as falhas quando o endpoint está morto por uma
hora, e como impedir que uma parede de retries contra um endpoint partido afogue os
outros. O delonix-meet faz exactamente isto à mão. Esse loop é um *mecanismo* horizontal
— um motor de entrega durável e com retry — e o Seiva já possui todas as primitivas de
que ele precisa (`seiva-webhooks` para assinar, `seiva-resilience` para recuar,
`seiva-jobs` para o tornar durável, o barramento de eventos para o alimentar). Só não foi
montado.

As forças:

- **Estender, não reinventar.** A assinatura HMAC fica exactamente como o
  `seiva-webhooks` a entrega. Este ADR acrescenta a *entrega* à volta dela; não muda um
  byte da assinatura.
- **A entrega é durável, fora do caminho de pedido.** Um evento tem de ser registado
  como devido, depois entregue de forma assíncrona — um receptor lento nunca pode manter
  um pedido de API aberto (o oposto do descarte do ADR 0026). Esse trabalho durável
  assenta na fila de jobs (ADR 0034), pelo que um crash nunca perde uma entrega
  pendente.
- **At-least-once com idempotência.** Uma entrega repetida pode chegar duas vezes; cada
  uma leva uma idempotency key estável (o id da mensagem) para que o receptor deduplique
  (ADR 0011). O motor nunca finge exactly-once sobre a rede.
- **Isolamento por endpoint.** O endpoint morto de um tenant não pode esfomear a entrega
  a todos os outros. Os retries precisam de backoff limitado + jitter, um dead-letter
  após N tentativas, e um **circuit breaker por endpoint** para que um alvo persistentemente
  a falhar deixe de ser martelado.
- **Seguro quanto ao egress.** Uma URL fornecida pelo tenant é um vector de SSRF; o
  endereço de saída é validado contra gamas privadas/metadata antes da chamada (o guard
  formalizado no ADR 0045 — até ele chegar, uma verificação conservadora dentro do
  motor).

## Decisão

Adicionar um crate de **motor de entrega**, `seiva-webhook-delivery`, que compõe as
primitivas existentes; o `seiva-webhooks` fica a biblioteca de assinatura, intocado.

- **Um ledger `deliveries`** — uma linha por entrega `(subscription, event)`, com
  `status` (`pending` → `in_flight` → `delivered` | `failed` | `dead`), a contagem de
  `attempt`, `next_attempt_at`, o último status HTTP e um excerto da resposta. O ledger é
  a fonte de verdade durável do que é devido e do que aconteceu.
- **Alimentado pelo barramento de eventos.** Um subscritor de `seiva-events` (ou um dreno
  de outbox, ADR 0021) torna um evento de domínio publicado numa linha `pending` por
  subscrição correspondente, **dentro da mesma transacção** da escrita do ledger — para
  que um evento confirmado tenha sempre as suas entregas enfileiradas, exactamente como a
  reserva de idempotência (ADR 0011).
- **Conduzido pela fila de jobs.** A entrega é um job no `seiva-jobs` (ADR 0034): um
  worker reclama as linhas `pending` devidas sob lease, assina o payload com
  `seiva-webhooks`, valida o egress, faz POST, e regista o resultado. Correr na fila
  significa que tudo é durável, com lease e seguro a N réplicas por construção.
- **Retry sobre `seiva-resilience`.** Uma falha transitória (timeout, 5xx, erro de
  conexão) agenda a próxima tentativa com **backoff exponencial + jitter**
  (`RetryPolicy`), `timeout()` limitado por POST, até uma contagem máxima de tentativas;
  passado o tecto a linha vai para **dead-letter** (`dead`) para inspecção/replay em vez
  de tentar para sempre.
- **Circuit breaker por endpoint.** Um `CircuitBreaker` indexado pelo endpoint de destino
  abre após falhas sustentadas, para que um endpoint morto seja saltado (as suas linhas
  adiadas) em vez de consumir workers — isolando um receptor partido do resto.
- **Idempotency key por entrega.** O `webhook-id` serve de idempotency key do receptor;
  uma tentativa reentregue leva o mesmo id para que o receptor deduplique (ADR 0011). A
  entrega é explicitamente at-least-once.
- **Validação de egress.** Antes de cada POST o destino é verificado contra uma política
  de allow/deny (bloquear loopback, link-local, RFC 1918, metadata de cloud
  `169.254.169.254`) — o guard SSRF que o ADR 0045 tornará uma primitiva partilhada.

## Alternativas consideradas

- **Um novo esquema de assinatura agrupado com a entrega.** Rejeitado de imediato: o
  `seiva-webhooks` já implementa Standard Webhooks correctamente, revisto. O motor
  compõe-no; rederivar HMAC bifurcaria uma primitiva resolvida e sensível à segurança.
- **Entregar de forma síncrona dentro do pedido que emite o evento.** Rejeitado: amarra a
  latência da API à saúde de um endpoint de terceiros e perde a entrega num crash. Enfileirar
  de forma durável + entrega assíncrona é a única forma que sobrevive a um receptor em
  baixo e a um reinício de processo.
- **Um loop de retry feito à medida em vez de `seiva-jobs` + `seiva-resilience`.**
  Rejeitado: rederiva leases, backoff, dead-letter e segurança multi-réplica que o
  framework já possui. O motor é montagem de primitivas existentes, não maquinaria nova.
- **Best-effort fire-and-forget (sem ledger).** Rejeitado: sem um registo durável do que é
  devido não há retry, nem dead-letter, nem auditoria do que um tenant foi (ou não)
  avisado — o oposto de uma superfície de integração fiável.

## Consequências

- **Performance.** A entrega corre fora do caminho de pedido na fila de jobs, pelo que
  emitir um evento é uma escrita durável barata; o worker agrupa as linhas devidas e um
  receptor lento nunca bloqueia um chamador de API. O crate é opt-in — um serviço que não
  dispara webhooks nada paga.
- **Segurança.** Os payloads são assinados pelo `HMAC-SHA256` já existente do
  `seiva-webhooks`; o destino é **validado quanto ao egress** contra gamas de SSRF (ADR
  0045) antes de cada POST; segredos e assinaturas nunca são registados; a entrega é sobre
  TLS. O ledger regista resultados sem guardar o segredo de assinatura.
- **Resiliência.** Retry com backoff exponencial limitado + jitter, um timeout limitado
  por POST, **dead-letter** após N tentativas, e um **circuit breaker por endpoint** para
  que um receptor morto não esfomeie o resto; entrega at-least-once com o `webhook-id` como
  idempotency key do receptor (ADR 0011); durabilidade e segurança a N réplicas herdadas da
  fila de jobs (ADR 0034) e do outbox (ADR 0021).
- O delonix-meet mantém o seu **catálogo de eventos e as subscrições por tenant** como
  política e apaga o seu loop de entrega escrito à mão; «que eventos disparam e para quem»
  fica no produto, «entregá-lo de forma fiável e prová-lo» torna-se do framework.
- **Não** se torna um broker de mensagens genérico nem um receptor de webhooks de entrada:
  é o motor de entrega de saída para o catálogo de eventos do próprio produto, assente na
  fila e no assinador que o framework já tem.

[Standard Webhooks]: https://www.standardwebhooks.com/
