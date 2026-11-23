# ADR 0031 — Real-time nativo: um hub WebSocket + SSE de 1ª classe como primitiva do framework, neutro ao protocolo e multi-pod

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0012 (saúde e limites cloud-native), ADR 0019 (autorização fail-closed), ADR 0021 (barramento de eventos in-process e coreografia), ADR 0026 (controlo de carga por rota)

## Contexto

O transporte HTTP do Seiva é estritamente **pedido/resposta**. O ADR 0026 até
limita uma rota descartando conexões (`503`) — o oposto de manter uma aberta. Não
há WebSocket, nem SSE, nem streaming em parte alguma do framework, e o roadmap não
menciona nenhum.

Mas um framework que ambiciona ser a base de **plataformas cloud-native** não pode
não ter um canal de *push*. Colaboração ao vivo, presença, dashboards que se
actualizam no lugar, notificações, streams de logs/eventos e a sinalização de
qualquer coisa WebRTC precisam todos que o servidor empurre para o cliente. O
*driver* aqui, o delonix-meet, constrói isto à mão: um `signaling.rs` de 10 000
linhas sobre `axum::ws`, auth por um room-token de curta duração, estado de
conexões em memória num `DashMap`, e Redis pub/sub (`pubsub.rs`, `redis_state.rs`,
`presence.rs`) para difundir eventos entre pods. É um hub genérico com um protocolo
de domínio aparafusado por cima — exactamente a forma que devia ser uma primitiva
do framework, construída uma vez, correctamente.

As forças:

- **Neutro ao protocolo.** O framework tem de transportar *frames*, não
  *significados*. Não pode saber nada de «offer/answer/ice»; o esquema de mensagens
  é a política do produto.
- **Uma conexão persistente é um recurso diferente de um pedido** e precisa dos seus
  próprios limites: um tecto de conexões por processo, um timeout de
  heartbeat/inactividade, um **buffer de envio limitado** para que um consumidor
  lento não faça a memória crescer sem fim, e um tecto por tenant. O semáforo de
  pedido do ADR 0026 não cobre sockets de vida longa.
- **Multi-pod por construção.** Um cliente no pod A tem de receber o que o pod B
  publica no mesmo tópico. O hub precisa de uma ponte pub/sub, não apenas fan-out
  local em memória, ou a escala horizontal deixa cair mensagens em silêncio.
- **Fail-closed e seguro quanto à origem.** O upgrade tem de autenticar *antes* de
  concluir (deny-by-default, ADR 0019), vincular o socket a um `Principal`+`Scope`,
  e verificar o `Origin` para travar o cross-site WebSocket hijacking (CSWSH). Um
  **connection-token** de curta duração (emitido por uma chamada REST autenticada
  normal) é a credencial certa — não um cookie de sessão de vida longa a viajar no
  upgrade.
- **Zero-cost quando não usado e DORM-free.** Um serviço que nada empurra não liga
  nenhum runtime e nada paga, como o rate limiter (ADR 0015) e o controlo de carga.

## Decisão

Adicionar um novo membro do workspace **`seiva-realtime`** — sem camada de dados,
atrás de feature, neutro ao protocolo.

- **`Hub`** — o registo de conexões: `ConnectionId → Connection`, agrupadas por um
  **`Topic`** tipado (uma string internada opaca, ex.: o `room:{id}` do produto).
  Fragmentado (`DashMap`) para subscrição/desinscrição/fan-out sem contenção de
  locks. `publish(topic, frame)` entrega a cada subscritor local **e** passa o frame
  à ponte para os outros pods.
- **`Frame`** — um payload opaco (`Bytes` ou valor JSON). O tipo de mensagem do
  produto implementa um trait fino `RealtimeMessage` (encode/decode); o hub nunca o
  interpreta. Sinalização, chat e deltas de presença são todos política do produto.
- **Transportes, compostos na raiz**, ambos a autenticar *antes* do upgrade:
  - **WebSocket** (feature `ws` do `axum`) para fluxos bidireccionais (sinalização).
  - **SSE** para fluxos só servidor→cliente (dashboards, notificações) — mais barato,
    amigo dos proxies, com reconexão automática.
- **`RealtimeAuth`** — um trait que valida o connection-token e devolve um
  `Principal`+`Scope` (reusa `seiva-identity`); a rejeição nega o upgrade. Uma
  allowlist de `Origin` é imposta no mesmo passo.
- **Backpressure por conexão** — cada socket possui uma fila de envio `mpsc`
  **limitada**; quando um consumidor lento a enche, a conexão é **fechada**
  (descarte de consumidor lento), nunca bufferizada sem limite. Um heartbeat
  ping/pong com timeout de inactividade ceifa sockets mortos.
- **Limites** — um semáforo `max_connections` por processo (violação → upgrade
  recusado, o análogo de vida longa do ADR 0026), um tecto por tenant, e um tamanho
  máximo de frame.
- **Ponte multi-pod** — um trait `BroadcastBackend` com um adapter Redis pub/sub (e
  um adapter NATS reusando `seiva-messaging`), mais um `PresenceBackend` para
  presença agregada. Só-local é o default (sem ponte, sem custo).
- **Ponte do barramento de eventos** — um subscritor de `seiva-events` (ou um dreno
  de outbox) pode fazer fan-out de um evento de domínio para um `Topic`, para que
  «algo aconteceu no domínio» se torne «os clientes ligados são avisados» sem o
  produto recablar transportes. Esta é a costura evento-interno → push-ao-cliente,
  tornada de 1ª classe.

O `tokio` entra com `default-features = false` (`sync`, `time`, `macros`); nenhum
runtime é ligado à biblioteca. As pontes Redis/NATS são features opcionais.

## Alternativas consideradas

- **Pôr o WebSocket dentro do `seiva-http`.** Rejeitado: uma conexão de vida longa
  tem um modelo de recurso e de falha diferente de um pedido (backpressure de envio,
  ceifa por inactividade, tectos por conexão). Um crate separado mantém limpo o
  raciocínio do caminho de pedido (ADR 0026) e deixa o real-time evoluir os seus
  próprios limites.
- **Só SSE.** Rejeitado: a sinalização é bidireccional; o SSE é só servidor→cliente.
  Entregamos **ambos** — SSE para push unidireccional, WS para bidireccional — para
  o produto escolher o transporte mais barato que sirva.
- **Deixar cada produto construir sobre `axum::ws` directamente (o status quo).**
  Rejeitado: isso volta a derivar o hub, o pub/sub, a presença, o backpressure e —
  o mais perigoso — a *segurança* (token, origem, tectos) em cada produto, cada um
  ligeiramente diferente. Os modos de falha aqui são subtis; construí-los uma vez,
  revistos uma vez, é o propósito de um framework.
- **Carregar mensagens tipadas de domínio no framework.** Rejeitado: soldaria o hub
  a um protocolo. Frames opacos + um `RealtimeMessage` do produto mantêm a linha
  mecanismo-vs-política (o princípio todo do upstreaming).

## Consequências

- **Performance.** Registo fragmentado para fan-out sem contenção; uma fila de envio
  limitada tecta a memória por conexão; SSE oferecido como o caminho unidireccional
  mais barato. O crate não liga runtime e é zero-cost quando um serviço opta por não
  o usar.
- **Segurança.** Upgrade deny-by-default (autenticar antes de concluir), socket
  vinculado a `Principal`+`Scope`, allowlist de `Origin` contra CSWSH,
  connection-tokens de curta duração em vez de cookies de vida longa, tamanho de
  frame limitado. Fail-closed, consistente com o ADR 0019.
- **Resiliência.** Heartbeat + ceifa por inactividade largam sockets mortos; o
  descarte de consumidor lento protege a memória perante um cliente parado; a ponte
  multi-pod torna a escala horizontal correcta em vez de silenciosamente com perdas;
  os tectos por processo e por tenant limitam o uso de recursos sob carga.
- O Seiva deixa de ser «um framework de APIs» e passa a «um framework de
  plataformas»: esta é a capacidade cuja ausência bloqueou a reescrita do
  delonix-meet e de que toda a plataforma real-time precisa.
- O delonix-meet reusa o hub e monta o seu protocolo de sinalização como política
  por cima, apagando o seu boilerplate de pub/sub, presença e canalização de
  conexões; o plano de media WebRTC fica fora do framework (não é transporte, é
  media).
- **Não** substitui um edge dedicado para fan-out extremo (milhões de subscritores):
  este é o chão de real-time in-process, com uma ponte pub/sub para escala
  horizontal normal — não um barramento de mensagens global.
