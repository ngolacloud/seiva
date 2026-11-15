# ADR 0026 — Controlo de carga por rota: um timeout (504) e backpressure por concorrência (503) como primitivo composável do framework

- Status: Aceite
- Data: 2026-11-15
- Decisores: NgolaCloud
- Segue: ADR 0015 (rate limit distribuído), ADR 0019 (autorização fail-closed)

## Contexto

O transporte HTTP do Seiva já traz três defesas OWASP API4 (Consumo de Recursos
Sem Restrição), cada uma a uma granularidade diferente:

- um **limite de tamanho do corpo** que delimita um pedido (`DefaultBodyLimit`),
- um **timeout global de pedido** (30 s → `408`) que impede um cliente slowloris
  de prender um worker no serviço inteiro (`tower_http::timeout`),
- um **rate limit por cliente** (→ `429`) que delimita a que ritmo um chamador
  emite pedidos, por processo ou partilhado entre réplicas via store (ADR 0015).

Nenhuma delas delimita o **trabalho em curso numa rota concreta**. Um endpoint de
relatório que leva segundos, ou uma escrita que estagna numa dependência lenta,
acumula pedidos até esgotar o pool — e as rotas *saudáveis* começam também a
falhar. O timeout global de 30 s é grosseiro demais para proteger uma rota cujo
orçamento é 200 ms, e o rate limit limita o *ritmo* de chegada, não a
*concorrência*: uma rajada dentro da janela corre toda ao mesmo tempo.

As forças:

- O limite é propriedade da **rota**, não do serviço: `/v1/reports/export` pode
  merecer 5 s e 4-em-curso enquanto `/v1/projects` merece 200 ms e 64. Logo o
  primitivo tem de ser **por rota**, não um botão global.
- Uma rota saturada tem de **falhar depressa** — descartar o excedente de imediato
  para a latência ficar limitada para os pedidos admitidos — em vez de enfileirar
  sem limite (o que só desloca o colapso para jusante).
- Tem de ser **opcional e composto na raiz**, como o rate limit distribuído (ADR
  0015): o framework oferece o mecanismo, a raiz de composição define a política.
  O núcleo mantém-se DORM-free e não arrasta runtime nenhum.
- Os códigos de estado têm de ser **honestos para o operador**: um prazo de rota
  excedido é um gateway timeout (`504`), uma rota na capacidade é uma
  indisponibilidade transitória (`503`) — distintos do `408` do serviço inteiro e
  do `429` por cliente.

## Decisão

Acrescentar ao `seiva-http` um primitivo de 1.ª classe, opcional:

- **`RouteLimit`** — os limites de uma rota: um `timeout` opcional (violação →
  `504 Gateway Timeout`) e um `max_in_flight` opcional de concorrência (violação →
  `503 Service Unavailable`). `max_in_flight(0)` é fixado em `1` para um lapso
  nunca trancar a rota.
- **`RouteControls`** — um conjunto de regras `prefixo → RouteLimit`; no pedido
  ganha o prefixo **mais longo** que casa, para uma regra específica se sobrepor a
  uma ampla. Cada limite de concorrência é materializado uma vez num
  `tokio::sync::Semaphore` partilhado, para todos os pedidos da rota beberem do
  MESMO orçamento.
- **`with_route_controls(router, controls)`** — embrulha um router no middleware
  que as aplica (uma camada `from_fn` do `axum`, no estilo do rate-limiter). Um
  `RouteControls` vazio não acrescenta camada e não custa nada.

O middleware, por pedido: casa o caminho com uma regra; se houver limite de
concorrência, `try_acquire` de uma licença (**nunca enfileira** — descarta com
`503` se falhar) e mantém-na durante todo o atendimento; depois delimita o handler
com `tokio::time::timeout`, devolvendo `504` na violação. Ambas as respostas são
`Problem` RFC 9457 com o id de correlação do pedido; o `503` leva um `Retry-After`
curto (a violação é transitória — a licença liberta-se assim que um pedido em
curso termina).

**Composição.** A raiz aplica `with_route_controls` ao sub-router dos **módulos**
*antes* de `router_with_modules` lhe embrulhar as camadas do framework. Assim o
controlo corre **dentro** da autenticação (um chamador admitido) mas **à volta** do
trabalho de cada rota — a ordem correcta: autenticar, depois delimitar a rota.

O `tokio` entra no `seiva-http` como dependência `default-features = false` só com
as folhas `time` e `sync` — não se liga runtime nenhum à biblioteca.

## Consequências

- Uma rota lenta ou pesada fica isolada: o seu timeout e o seu limite de
  concorrência impedem-na de esgotar o pool partilhado, e o resto do serviço
  mantém-se responsivo.
- A granularidade por rota coloca a política junto do deployment, de forma
  declarativa:
  `RouteControls::new().route("/v1/projects", RouteLimit::new().timeout(..).max_in_flight(..))`.
- O descarte fail-fast mantém limitada a latência dos pedidos admitidos sob
  sobrecarga, e um `503` + `Retry-After` é a deixa para o balanceador tentar
  noutro lado ou recuar.
- O mecanismo está no framework; a **política é da raiz de composição**. Um serviço
  que não define controlos fica intacto (sem camada, sem custo), tal como um
  deployment de instância única que nunca adere ao rate limit distribuído.
- **Não** substitui um gateway de borda: um deployment que precise de load
  shedding global, circuit breaking ou filas com prioridade continua a pôr uma
  borda dedicada à frente. Isto é o chão em processo, não o tecto.

## Alternativas consideradas

- **Uma pilha tower `ConcurrencyLimitLayer` + `LoadShedLayer` + `TimeoutLayer`.**
  Idiomática em tower, mas introduz erros de `tower::Service` que o axum obriga a
  mapear de volta com uma camada `HandleError`, e é desconfortável de chavear por
  rota (a camada embrulha um serviço inteiro). O middleware `from_fn` com uma
  tabela de regras casada por caminho é mais simples, alinha com o rate-limiter
  existente e produz problemas RFC 9457 directamente.
- **Um único botão global de timeout/concorrência.** Rejeitado: o orçamento certo
  é propriedade da rota, e um valor global é apertado demais para rotas lentas ou
  largo demais para rápidas.
- **Enfileirar o excedente em vez de descartar.** Rejeitado: uma fila sem limite só
  adia o colapso e inflaciona a latência de cauda; o descarte fail-fast limitado é
  o sinal de backpressure sobre o qual um chamador pode agir.
