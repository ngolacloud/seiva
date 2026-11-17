# ADR 0027 — Observabilidade nativa: métricas Prometheus e tracing OpenTelemetry como primitivo opt-in do framework, com o contexto de rasto a atravessar a fronteira dos eventos

- Estado: Proposto
- Data: 2026-11-15
- Decisores: NgolaCloud
- Segue: ADR 0012 (saúde e limites cloud-native), ADR 0021 (barramento de eventos in-process e coreografia), ADR 0015 (rate limit distribuído)

## Contexto

O Seiva já faz bem o logging estruturado: um fluxo de eventos JSON para stdout, um
objecto por evento, nível via `RUST_LOG`, inicializado antes de tudo o resto para
que até um erro de configuração seja uma linha parseável (`crates/seiva-server/src/lib.rs`,
`observability::init`). Isto satisfaz o 12-Factor XI (logs como fluxos de eventos).
É também a **única** telemetria que o framework emite.

Faltam duas peças, e ambas mordem exactamente a forma de deployment que o Seiva
vende:

- **Sem métricas.** Não há endpoint `/metrics`, nem exporter Prometheus, nem uma
  fachada `metrics` em parte alguma de `crates/`. Os controlos de carga por rota
  (ADR 0026), o rate limiter (ADR 0015), a guarda de autorização (ADR 0019) e o
  worker tomam todos decisões — admitir/descartar/permitir/negar, lotes drenados,
  atraso — que um operador não consegue ver. O worker até calcula um `WorkerReport`
  de contadores e depois **deita-o fora numa linha de log**
  (`crates/delonix-seiva/src/worker.rs`).
- **Sem tracing distribuído.** Não há exporter OpenTelemetry/OTLP nem propagação de
  contexto de rasto. Isto é tolerável num monólito mas **quebra a proposta de valor
  central**: no momento em que `seiva module split` transforma uma coreografia
  (`orders.placed → inventory.reserved → payments.captured`) em serviços separados,
  uma única operação lógica torna-se N ilhas de log desconexas, sem forma de a
  seguir através do salto HTTP *nem* do salto evento/outbox.

As forças:

- **O núcleo tem de permanecer sem-DORM e leve de runtime.** A observabilidade não
  pode arrastar um runtime pesado para dentro de crates de biblioteca que hoje não
  ligam nenhum. Como o rate limit (ADR 0015) e os controlos de rota (ADR 0026), o
  mecanismo vive no framework mas a **política e os exporters são ligados no
  composition root**, atrás de uma feature cargo, para que um build que não opte
  não pague nada.
- **A cardinalidade é um perigo de produção, não um pormenor.** Uma métrica rotulada
  por `tenant`, por caminho de pedido cru, ou por id de objecto faz explodir a
  contagem de séries do Prometheus e pode derrubar o alvo de scrape. O framework
  tem de emitir séries de **baixa cardinalidade** por construção (*template* de
  rota, *classe* de estado, desfecho) e tem de fazer do inquilino um *atributo* de
  rasto, nunca um *rótulo* de métrica.
- **O contexto de rasto tem de atravessar as duas fronteiras que o Seiva detém.** A
  fronteira HTTP (W3C `traceparent` à entrada e à saída) *e* a fronteira assíncrona
  — o envelope CloudEvents que leva um evento de domínio pelo outbox transaccional,
  pelo broker e até um subscritor (ADR 0021, `seiva-messaging`). Um rasto que pára
  no outbox é inútil para o caso de módulo-partido, que é precisamente o caso que
  dele precisa.
- **Tem de compor com o que existe.** A camada de correlação por request-id
  (`crates/seiva-http/src/request_id.rs`) é o lugar natural para também estabelecer
  um span e extrair/injectar o `traceparent`; as probes de saúde têm de ficar
  isentas do ruído de instrumentação tal como estão isentas do rate limiter.

## Decisão

Introduzir um novo crate `seiva-observability`, **membro do workspace e sem-DORM**,
e ligá-lo, opt-in, através do `seiva-server` e do `seiva-http`.

**Métricas (Prometheus).**

- Depender da fachada `metrics` (uma abstracção fina de recorder, leve em alocação)
  nos crates de biblioteca, e de `metrics-exporter-prometheus` apenas em
  `seiva-observability`/`seiva-server`. Os crates de biblioteca registam contra a
  fachada; nunca ligam um exporter.
- O `seiva-http` regista, por pedido: um contador de pedidos e um histograma de
  latência indexados por **template de rota** (a regra correspondida, p.ex.
  `/v1/projects/:id`, nunca o caminho concreto) e por **classe de estado**
  (`2xx`/`4xx`/`5xx`); um gauge de in-flight; e as decisões de descarte/negação dos
  controlos de rota (`503`/`504`), do rate limiter (`429`) e da guarda (`403`). O
  inquilino **não** é rótulo.
- O worker regista o seu `WorkerReport` como contadores/gauges (lotes drenados,
  itens reencaminhados, falhas, atraso do outbox) em vez de só os registar em log.
- O `seiva-server` expõe `/metrics` atrás da feature `metrics`, servido num **bind
  separado** (uma porta de administração, p.ex. `SEIVA_METRICS_ADDR`) para que a
  superfície de scrape não seja a superfície pública da API, e **isento de
  autenticação e de rate limiting** como as probes de saúde.

**Tracing (OpenTelemetry/OTLP).**

- Atrás de uma feature `tracing` (OTLP), o `seiva-observability` instala uma camada
  `tracing-opentelemetry` com um exporter OTLP configurado pelo ambiente
  (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`, amostragem via
  `OTEL_TRACES_SAMPLER*`). Sem a feature, o subscritor JSON existente fica
  inalterado.
- **Fronteira HTTP:** a camada de request-id extrai o `traceparent`/`tracestate`
  W3C do pedido de entrada para o contexto do span actual e injecta o contexto
  resultante na resposta, para que um rasto a montante continue através do serviço.
- **Fronteira de eventos:** o envelope CloudEvents em `seiva-messaging` ganha uma
  **extensão de contexto de rasto** padrão (`traceparent`/`tracestate` como
  atributos do envelope). O publicador injecta o contexto actual quando um evento é
  escrito no outbox; o despachante in-process e o worker de dreno do broker
  extraem-no ao processar, para que o span do caso de uso que *emitiu* o evento seja
  o pai do span que o *processa* — o rasto sobrevive ao salto outbox→broker→subscritor
  e a um `module split`.

**Política no root.** O `serve()` do `seiva-server` ganha ganchos opt-in
(`with_metrics(addr)`, `with_tracing(config)`); um serviço que não chame nenhum
emite exactamente os logs de hoje e não liga exporter nenhum. As probes de
saúde/prontidão ficam isentas do ruído de métricas e tracing, coerente com o
ADR 0012.

## Consequências

- Um operador passa finalmente a ver as decisões que o framework já toma: taxas de
  descarte/negação, latência por rota, concorrência in-flight, atraso do outbox —
  os sinais necessários para operar os controlos por rota (ADR 0026) e o rate limit
  distribuído (ADR 0015) com limiares reais em vez de palpites.
- A história de módulo-partido torna-se operável: um `traceparent` segue um pedido
  pelo salto HTTP e, crucialmente, pelo salto evento/outbox, de modo que uma
  coreografia se lê como um único rasto distribuído, quer corra num monólito quer
  como N serviços. Isto fecha a maior lacuna operacional para usar o Seiva como
  camada de serviço de uma plataforma de cloud.
- A cardinalidade fica limitada por construção (template de rota + classe de estado;
  o inquilino é atributo de span, não rótulo de métrica), pelo que o alvo de scrape
  se mantém saudável.
- O núcleo permanece sem-DORM e de custo-zero quando não usado: os crates de
  biblioteca registam contra a fachada `metrics` (um no-op sem recorder) e os
  exporters/tracing vivem atrás de features ligadas só no root.
- **Não** substitui uma stack de observabilidade de plataforma (colector, Prometheus,
  Tempo/Jaeger, Grafana): o Seiva emite OTLP e uma superfície de scrape Prometheus;
  o cluster opera os backends. Isto é o piso de instrumentação in-process, não um
  produto de monitorização.

## Alternativas consideradas

- **Só logs JSON `tracing`, correlacionar por request-id (statu quo).** Rejeitado
  como a decisão inteira: os logs não agregam em séries de latência/débito e um
  request-id não se propaga entre serviços, pelo que o caso de módulo-partido — o
  diferenciador — não tem vista distribuída.
- **Cozer o OpenTelemetry directamente nos crates do núcleo, sempre ligado.**
  Rejeitado: ligaria um exporter pesado e um runtime em bibliotecas que hoje não
  ligam nenhum, violando o invariante sem-DORM/custo-zero-quando-não-usado que o
  rate limit e os controlos de rota estabeleceram. Com feature no root, o build por
  omissão fica enxuto.
- **O inquilino como rótulo de métrica para painéis por inquilino.** Rejeitado:
  cardinalidade ilimitada é uma falha auto-infligida. A visibilidade por inquilino
  pertence aos rastos (atributo de span) e aos logs, não ao espaço de séries do
  Prometheus.
- **Um formato/endpoint de métricas próprio.** Rejeitado: scrape Prometheus + OTLP
  são a língua franca cloud-native; um formato próprio exigiria um colector próprio
  e não compraria nada.
