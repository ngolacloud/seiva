# Progress — Delonix Seiva

> Tracker canónico do Seiva. Uma sessão nova começa por ler o prompt único e
> este ficheiro. O lado DORM da campanha tem o seu tracker em
> `../DORM/docs/progress-dorm-seiva.md`.

## Estado (2026-11-08) — bus de comandos + eventos in-process (ADR 0020/0021)

Dois padrões nativos a seguir ao CQRS, em `feat/cqrs-bus-and-inprocess-events`
(PR #30 sobre main). **Fase 1 + Fase 2 feitas e integradas, workspace verde**
(`fmt`/`clippy -D warnings`/`cargo test --workspace` = 134 grupos `ok`, 0 falhas).

- **[ADR 0020](adr/0020-command-bus-and-pipeline-behaviors.md) — Command bus +
  pipeline de behaviors (opt-in).** Liga o `seiva-cqrs` (estático/zero-cost, antes
  desligado do caminho HTTP) a um pipeline tipado de behaviors.
  - **F1** (`seiva-cqrs`): `Validate` (pré-tx, via trait `Validated`) e
    `Transactional` (handler dentro de `UnitOfWork::transaction`; `Trace` reusa o
    `Instrument`).
  - **F2** (`seiva-http::bus`): `Authorize` (fail-closed, ADR 0019; partilha
    `require_permission` com o guard tower) e `Idempotent` (intra-tx, via `Ctx`:
    `scoped_key`+`fingerprint`, `begin`/`finish`, replay/`409`). `From<BoxError>`
    passou a ser *downcast-aware* → erros das behaviors são status-bearing (403/409
    não remapeados para 500; erros opacos continuam 500 no-leak). Prova:
    `tests/bus_pipeline.rs` (cadeia `Authorize→Validate→Transactional{Idempotent}`).
  - **F3 (FEITA)** — o módulo real `orders` despacha `place` pela **cadeia do bus**.
    Desenho (resolve a inversão de tx + camadas): facade `place(&ctx, cmd)` (o
    `&Ctx` é saltado do wire pela macro); `OrdersService::place_work` = a lógica
    **sem** abrir tx (o `place` directo = `uow.transaction(place_work)` fica p/
    compat); a cadeia monta-se na **interface** (que já depende de seiva-http):
    `Validate → Transactional{ Idempotent → handler(place_work) }` com key, ou
    `Validate → Transactional{ handler }` sem. Como o facade passa a exigir `&Ctx`,
    quem o implementa é o **controller** (não a application) — mesmo padrão do
    `admin-data-e2e` catalog. `OrdersController<R,E,S>` ganha `with_idempotency`;
    `409` status-bearing via `OrdersError::Conflict`. Prova:
    `apps/shop-api/tests/orders_idempotency_e2e.rs` (mesma key = 1 order + 1 evento;
    key+corpo diferente = 409) + testes de interface. Substitui o boilerplate manual
    de idempotência (que o catalog ainda faz à mão) por uma cadeia declarativa.
  - **Ainda por fazer:** activar os seams `#[cached]`/`#[policy]` na macro é o
    default-no-controller que o ADR 0020 adiou (ADR futuro reavalia).
- **[ADR 0021](adr/0021-in-process-event-bus-and-choreography.md) — Event bus
  in-process + choreography.** Fecha o lado que faltava do outbox (já construído).
  - **F1** (`seiva-events`, DORM-free): `EventSubscriber`, `SubscriberRegistry`,
    `dispatch` com dedup de inbox.
  - **F2** (`seiva-events` + `seiva-outbox-relay`): `OutboxReader` + driver
    `drain_in_process` (gémeo DORM-free do `drain`); `InMemoryOutbox` (feature
    `testing`); prova **offline** `orders.placed → inventory.reserved`
    (`tests/choreography.rs`: cursor, dedup de inbox sob redelivery, retry sem
    avançar cursor). Adapter `DormOutboxReader` em `seiva-outbox-relay` compila; o
    teste live `live_drain_in_process_over_the_dorm_outbox` está **gated** em
    `SEIVA_DORM_TEST_DSN`.
  - **F3 (FEITA)** — choreography **viva** no `apps/shop-api`, 100% offline:
    `InMemoryOutboxPublisher` (EventPublisher→CloudEvent no outbox, feature
    `testing`); `OrdersController<R,E>` genérico com `with_events` injecta o
    publisher (substitui o `NoopEventPublisher`); subscriber `inventory` na
    composition root (`orders.placed`→`Inventory::reserve`, stock partilhado via
    `StockRepository for Arc<R>`); `apps/shop-api` partido em `lib.rs`
    (`build_app() -> BuiltApp`) + binário. Prova e2e `tests/choreography_e2e.rs`:
    POST order → `drain_in_process` → stock 10→7; `redeliver`+drain mantém 7 (dedup
    de inbox). 137 grupos de teste `ok`.
  - **F2b (por fazer):** prova **live** sobre PostgreSQL/DORM (`DormOutboxReader`,
    gated em `SEIVA_DORM_TEST_DSN`); worker/loop de fundo sobre `drain_in_process`.

## Estado (2026-11-07) — ids no fio como strings (ADR 0018)

- **`delonix_seiva::WireId`** — um `u128` que serializa no fio como **string** JSON
  (`WireSchema`=string), para nenhum cliente JS perder precisão acima de 2^53.
  Decisão em [ADR 0018](adr/0018-wire-ids-as-strings.md); adoptado pelo
  `examples/shop` (DTOs de id usam `WireId`; path params ficam `u128`).

## Estado (2026-11-04) — transportes · CQRS · server · módulos · docs

**Framework ligado ponta-a-ponta e verde no CI. 14 crates por agentes, 54 grupos de testes.**

### Fundação de assinaturas + transportes gerados de um descritor (3.º fan-out)
- **`FacadeDescriptor` enriquecido:** `#[seiva::facade]` passa a captar as assinaturas —
  `Param { name, ty }` + `MethodDescriptor { params, output }` (tipos renderizados dos tokens
  do trait, normalizados). Desbloqueia os geradores sem reflexão.
- **`seiva-grpc`** — `proto_document(&FacadeDescriptor) -> String` (proto3) + mapa Rust→proto
  auditável; `&Ctx` não é campo de fio; `Result<T,E>`→`T`. Feature `tonic` mapeia erro→`Status`.
- **`#[seiva::controller]`** (em `seiva-macros`, reexportado por `delonix-seiva`) — gera
  `<trait>_router`→axum lendo versão+permissão do descritor; provado em `seiva-controller-test`.
- **`seiva-openapi`** — `openapi_document(ApiInfo, &[FacadeDescriptor])` → OpenAPI 3.1 de modelo
  tipado; `x-seiva-requires`+bearer nos guardados; schemas-placeholder honestos p/ tipos opacos.
- **`seiva-http` `/openapi.json`** passou a servir o documento **gerado** (não stub).

### CQRS + CLI (itens 1-2)
- **`seiva-cqrs`** — bus de aplicação: `Command`/`Query` + handlers, `Mediator<H>` estático
  (handler em falta = erro de compilação), middleware tipado `Instrument` + `CacheQueries`
  (costura p/ `#[cached]`/`#[policy]`).
- **CLI `seiva`** +3 comandos: `openapi`, `proto` (documentos gerados da fachada demonstração),
  `user create` (lê password de stdin).

### `seiva-server` — o substituto do `dorm-api` (decisão do dono: "servidor que monta tudo")
- `serve(config, shutdown)` liga TcpListener + `seiva_http::router` (controllers + `/openapi.json`
  + `/health`), Health `Starting→Ready→Draining`, `axum::serve` graceful; host de workers sobre
  `delonix_seiva::worker::run_worker` (passo demo; relay DORM atrás da feature `dorm`); config
  12-factor (`SEIVA_HTTP_ADDR`>`PORT`>`127.0.0.1:8080`, `APP_ENV`+guarda); bin tokio multi-thread
  ctrl_c+SIGTERM. SEM `[features]` de propósito (clippy `--all-features` não pode puxar excluídos).

### Módulo `accounts` (5 camadas) + `seiva user create` ponta-a-ponta
- **`examples/accounts`** sobre `seiva-auth`: nunca guarda plaintext (prova `verify_password`
  ✓/✗), DTO sem segredos, duplicado rejeitado, evento `accounts.user_created`. 11 testes.
  ADR 0003 respeitada (interface só sobre a fachada; composição em testes/app).
- **`seiva user create`** passou a compor o `AccountsService` na CLI (ports in-process) e a correr
  o caso de uso real (Argon2id + regras + evento), imprimindo `{id, username}` sem segredo.

### Scaffold a emitir transportes (item 1 do 2.º pedido)
- `seiva module new` gera agora módulos com `#[seiva::facade]` + `#[seiva::controller]` (adaptador
  CONCRETO — o genérico não dá `Send` p/ axum) + helpers `openapi`/`proto` do descritor; app
  transport-free; repo in-memory DORM-free. Agente **provou a compilação** do gerado (POST→201).

### Pendências (gestos do dono)
- **PR #30 (DORM)** — depreciar `dorm-messaging`: verificado pronto (MERGEABLE/CLEAN, 3 checks
  verdes); `gh pr merge` BLOQUEADO pelo classificador → merge à mão.
- **Token de CI (item 4)** — CI tem job GATED **`dorm-gateway`** que fmt/clippy/testa os 4 crates
  excluídos contra o DORM privado **só se existir o segredo `DORM_TOKEN`** (sem ele = no-op verde).
  Adicionar o segredo activa-o e desbloqueia ligar a feature `dorm` ao server/gateway.

### Docs
- README reescrito (saiu da fase fundacional), `docs/architecture.md` e `docs/quickstart.md` novos,
  ancorados nos comandos reais (`seiva project new`/`module new`, `openapi`/`proto`/`user create`,
  `seiva-server`, env vars).

---

## Estado (2026-11-04)

**+ 2.º fan-out de 3 agentes: seiva-auth, seiva-config, seiva-webhooks (membros, DORM-free).**

### Segundo lote de agentes especialistas (integrados, membros)
- **`seiva-auth`** — Argon2id em PHC (`hash_password`/`verify_password`), `prompt_password`
  sem eco, `Permission`/`PermissionSet`, `AuthError` compacto. Parte DORM-A (contas, a sair
  do dorm-cli; construído fresco e DORM-free). 6 testes.
- **`seiva-config`** — config tipada 12-factor, **zero deps** (só std): `AppEnv`
  (recusa atalhos de dev em produção via `reject_dev_shortcut`), `ConfigError` que agrega
  todos os problemas, readers tipados. 12 testes.
- **`seiva-webhooks`** — Standard Webhooks: assinar/verificar `HMAC-SHA256({id}.{ts}.{payload})`,
  comparação em tempo constante, janela anti-replay, `now` injectado. Bate no **test vector
  oficial**. 14+1+2 testes.
- Fan-out paralelo (crates disjuntos, `[workspace]` próprio, sem raiz/git); integrei como
  membros. 32 grupos de testes; clippy `--all-features` + doc `-D warnings` limpos; workspace
  continua DORM-free.

## Estado (2026-11-03)

**+ Fan-out de 3 agentes especialistas: HTTP, RAG worker, MCP-HTTP (itens 1-3).**

### Três crates por agentes especialistas em paralelo (integrados)
- **`seiva-http`** (membro, DORM-free) — transporte HTTP axum/tower: erros RFC 9457
  `problem+json`+request_id, extractor `Valid<T>`, guarda de permissão **conduzida pelo
  `MethodDescriptor.requires`** da fachada, health live/ready (Starting/Ready/Draining),
  demonstração a montar `Notes` (versão+permissão vêm do `NotesFacade::DESCRIPTOR`). 7 testes.
- **`seiva-rag-worker`** (excluído, git-coupled) — `run_rag_worker` = `run_worker` sobre
  `dorm_rag::worker::run_once` (genérico sobre `Embedder`). Espelha o relay.
- **`seiva-mcp`** (excluído, git-coupled) — `POST /mcp` com **auth/zero-trust do Seiva** à
  frente, despacho para `dorm_mcp::handle_message`; tenant/writable por pedido vêm do principal.
  8 testes. Só usa a API pública do `dorm-mcp` (sem mudar o DORM).
- **Orquestração:** cada agente num crate disjunto, `[workspace]` próprio p/ build isolado,
  sem tocar no Cargo.toml raiz nem no git; eu integrei (removi `[workspace]`, registei
  membro/exclude, gate do workspace completo). 25 grupos de testes; clippy `--all-features` +
  doc `-D warnings` limpos; build do workspace **continua DORM-free**.
- **Item 4 (ligar `dorm` feature ao gateway) RETIDO:** acopla o build ao DORM privado + exige
  token de CI (decisão/segredo do dono).


**Fundação + núcleo + facade + lint(×2) + scaffolding + módulo ligado + CI + gateway DORM.**

### Runtime de workers (`delonix-seiva::worker`)
- **Ligado ao relay:** `seiva-outbox-relay::run_relay_worker(worker, shutdown, consumer,
  topic, relay_cfg, publisher)` usa o `run_worker` para drenar o outbox em ciclo —
  percurso messaging COMPLETO ponta-a-ponta (write→outbox→worker→broker). Compila
  contra o DORM real; `delonix-seiva` entra no `seiva-outbox-relay` por path (é DORM-free).

- `run_worker(config, shutdown, pass)` — o **ciclo** genérico, DORM-free, que conduz
  qualquer função de **uma passagem** (`forward`/`drain` do relay, `run_once` do RAG,
  leases do dorm-jobs): intervalo, *graceful shutdown* (futuro de shutdown), métricas
  (`WorkerReport{passes,processed,errors}`), e fast-drain (corre já a seguir quando a
  passagem encheu). Um `Err` numa passagem é contado e registado, **nunca fatal**.
- É a metade de framework da fronteira (brief Parte B): as funções de 1 passagem são da
  camada de dados; o ciclo/tempo/shutdown/métricas são do Seiva. Testável com um closure.
- 2 testes (sem BD, sem tempo real: intervalo 0 + shutdown por oneshot). `tokio`
  (time/macros) + `tracing` entraram no `delonix-seiva`.

### Parte DORM-A — 1.ª extracção: `dorm-messaging` → `seiva-messaging`
- **Movido do DORM com histórico preservado** (`git filter-repo` num clone temporário
  → merge de históricos não-relacionados). 6 commits do DORM vieram junto.
- **Adaptado e DORM-free:** erro próprio (`Error::Config`/`Broker`, `Result`), removido
  o `OutboxBus` (ponte outbox→broker, acoplada ao `dorm-outbox`/`db` — vai para um
  worker de relay do Seiva depois). Ficou CloudEvent 1.0 + `Publisher` + backends
  NATS/AMQP/Kafka atrás de features (off por omissão).
- **Dependência oculta revelada pela mudança:** o backend Kafka usava
  `rskafka::chrono::Utc::now()` (feature `clock` do chrono, antes ligada por
  unificação no workspace do DORM); liguei `chrono`/`clock` sob a feature `kafka`.
- Membro do workspace Seiva; testes live acoplados ao outbox removidos (voltam como
  testes do relay-worker). 22 grupos de testes; clippy `--all-features` e doc `-D
  warnings` limpos.
- **Relay-worker re-homed:** o antigo `OutboxBus` (ponte outbox→broker) voltou como
  **`seiva-outbox-relay`** — `forward`/`drain` (uma passagem) sobre o `dorm-outbox`
  (git rev `f43d873`) + `Publisher` do `seiva-messaging`. Compila contra o DORM real;
  excluído do workspace (git-coupled, como o gateway). O ciclo/intervalo/shutdown é o
  runtime de workers do Seiva (por cima destas funções de 1 passagem).
- **FALTA (PR à parte no DORM):** remover/depreciar o `dorm-messaging` no DORM com nota
  de redirecção. ⚠️ dono activo no DORM main (`288fcbc`, 13 PRs merged).

### `UnitOfWork` por escopo + ligado ao DORM (ADR 0004, decisão do dono)
- **Decisão #2 do dono:** o port `UnitOfWork` passa a ser **por escopo** —
  `transaction(|| async {…})`, commit em `Ok`, rollback em `Err` (dispensa
  `on_commit`/DORM-B3). Resolve o desencontro com o `db::atomic` (closure) do DORM.
- Implementado em toda a pilha: `seiva-ports` (port RPITIT genérico), `seiva-testing`
  (`InMemoryUnitOfWork` por escopo), `examples/notes/application` (valida no domínio,
  depois `transaction`), e o gateway (`DormUnitOfWork` → `with_session`+`db::atomic`).
- `NoteRepository::save` passou a `-> impl Future + Send` (o future tem de ser Send
  para correr dentro do `transaction`). ADR 0002 emendada; ADR 0004 regista a resolução.
- **EventPublisher FEITO no gateway:** `DormOutboxPublisher` escreve o evento no outbox
  (`dorm-outbox`) na transacção corrente (padrão transactional-outbox); o relay-worker
  reencaminha depois. O gateway implementa agora **os 4 ports**. `examples/notes` ajustado:
  save+publish dentro do `transaction()` (outbox na mesma tx).

### Travessia da fronteira — `seiva-data-dorm` (ADR 0004)
- **DORM integrado:** os 4 PRs (DORM-C, B1×2, B2) mergeados; DORM `main` em `f43d873`.
- **Gateway `seiva-data-dorm`** (ADR 0004): todo o acoplamento ao DORM num só crate,
  dependente do DORM **por git rev `f43d873`** (nunca path). **Compila e testa contra
  o DORM real** (dorm-core v0.6.0 via git). Implementa `Clock`/`IdGenerator`; `connect`
  constrói o `Database` (connect_with) e **impõe os invariantes B1**: strict sessions ON
  + cursor key configurada. 3 testes (connect com deadpool lazy, sem PG).
- **EXCLUÍDO do workspace** por agora (CI público não tem acesso ao DORM privado sem token)
  → o CI do Seiva continua verde e DORM-free; build do gateway com `CARGO_NET_GIT_FETCH_WITH_CLI=1`.
- **Achado (ADR 0004):** `UnitOfWork`/`EventPublisher` adiados — modelo de transacção do DORM
  é por closure (`db::atomic`), o port `seiva-ports` é por handle (begin/commit). Resolve-se
  a seguir: port com escopo (`run<F>`) **ou** primitiva handle-based do DORM (`on_commit`, B3).
- **Decisões do dono por fazer:** ligar o `dorm` feature → gateway (acopla build + exige token
  de CI para o DORM privado); e a forma final do port `UnitOfWork`.

### Repo publicado + 2.ª fitness function + CI
- **Repo no GitHub:** github.com/ngolacloud/seiva (privado), `main` publicado.
- **`seiva lint` ganhou a 2.ª fitness function — regra de dependência entre camadas**
  (`lint::check_layers`, pura): lê os `Cargo.toml` das 5 camadas de cada módulo e
  recusa dependências proibidas (domain só kernel; application sem framework/data
  layer/camadas inferiores/outro módulo; facade=contrato; infrastructure↛interface;
  interface↛infrastructure/domain). Complementa a imposição do compilador (grafo de
  crates). 4 testes novos (10 no total em `lint`).
- **CI do repo** (`.github/workflows/ci.yml`, espelha o DORM): job `check`
  (fmt + clippy `--all-features -D warnings` + `test --workspace` + smoke `seiva
  doctor`) e job `docs` (`cargo doc -D warnings`). Valide localmente antes de push
  (armadilha docs CI do DORM aprendida).
- 20 grupos de testes verdes; clippy limpo; `cargo doc -D warnings` limpo.

### Módulo de referência ligado ao framework — `examples/notes` (ADR 0003)
- Módulo canónico de 5 camadas no workspace do Seiva, ligado aos crates **reais**
  (`delonix-seiva`, `seiva-ports`, `seiva-testing`): prova de integração + alvo
  que o gerador vai emitir. (Path-deps porque nada do Seiva está publicado ainda.)
- **ADR 0003 (Accepted):** fixa **onde vive a implementação da fachada** — na
  `application` (hexagonal: serviço de aplicação implementa o port de entrada),
  porque `interface` não pode depender de `domain` e `facade` não se implementa a
  si própria. A regra de camadas é sobre dependências **directas**. Grafo:
  `facade→delonix-seiva`; `domain→∅`; `application→domain,facade,seiva-ports`;
  `infrastructure→application,domain,seiva-ports`; `interface→application,facade`.
- **Provado:** `notes-domain` (regra título não-vazio, 1 teste); `notes-application`
  (`NotesService` implementa `Notes`, usa `IdGenerator`/`UnitOfWorkFactory`/
  `EventPublisher`, port `NoteRepository`; 2 testes com os dublês do `seiva-testing`
  + repo falso — sem servidor/broker/BD); facade com `#[seiva::facade]` real;
  `infrastructure` (`InMemoryNoteRepository`); `interface` (`NotesApi<S: Notes>`).
- **Erro mapeado na fronteira:** dentro da app, `PortResult`/`BoxError`; no trait
  da fachada, `-> Result<_, String>` (sem vazar detalhe interno).

### Scaffolding — `scaffold` + `seiva project new`/`module new`
- **`scaffold`** (`delonix-seiva`): geração **pura** (`caminho → conteúdo`,
  testável sem disco) — `project_files`, `module_files` — mais edições puras de
  registação (`add_members_to_workspace`, `register_module_in_seiva_toml`,
  `append_namespace_block`) e `validate_name` (snake_case ASCII). 6 testes.
- **`seiva project new <nome>`**: gera `seiva.toml`, workspace `Cargo.toml`
  (gerido pela CLI, com âncora de membros), `apps/<nome>-api`, `shared/kernel`,
  `schema/<nome>.schema`, `.gitignore`, README.
- **`seiva module new <nome>`**: gera as **5 camadas** (`facade`, `domain`,
  `application`, `infrastructure`, `interface`) + `module.toml`, e **regista** nos
  membros do workspace, no `seiva.toml` e com um bloco `namespace` no schema.
  `--data external` ainda reporta não-implementado (fase da camada de dados).
- **Provado ponta-a-ponta** (projecto gerado `shop` + módulos `notes`/`identity`):
  - `cargo build` do workspace gerado (12 crates) → verde;
  - **regra de camadas imposta pelo compilador**: `domain` a importar
    `application` → `error[E0433] unresolved module` (o grafo de crates recusa);
  - **zero-trust** no projecto gerado: `uses` sem `allow` → exit 1; com `allow` → 0;
  - nomes inválidos (`Bad`, `order-items`) recusados.
- **Corte honesto:** os crates gerados compilam **autónomos** (sem dep do
  framework ainda). Ligar ao runtime (macro `#[seiva::facade]`, ports, HTTP/gRPC,
  DI) é fase seguinte — anotado nos ficheiros gerados.

### Zero-trust de módulos — `manifest` + `lint` (1.ª fitness function)
- **`manifest`** (`delonix-seiva`): parse tipado do `module.toml`
  (`[module]` name/facade/owners, `[[uses]]`, `[[allow]]`) com `serde`+`toml`;
  erros marcam o caminho. 3 testes.
- **`lint::check_contracts`** (puro, sem I/O): impõe a regra do brief — todo
  `[[uses]]` precisa de `[[allow]]` correspondente no fornecedor, versões de
  fachada coincidem, nomes únicos; **tudo negado por omissão**. Regras:
  `uses-without-allow`, `method-not-allowed`, `uses-unknown-module`,
  `facade-version-mismatch`, `duplicate-module-name`. 6 testes.
- **`lint::lint_project`** varre `modules/*/module.toml` (ordem determinística).
- **`seiva lint` deixou de ser stub**: chama a biblioteca (regra CLI-fina).
  Provado ao vivo: contrato coincidente → exit 0; sem `[[allow]]` →
  `uses-without-allow` exit 1; sem `modules/` → nada a verificar exit 0.
- **Falta** (fases seguintes do lint): regra de dependência entre camadas
  (`domain`↛`infrastructure`, etc.), sem FK/leitura entre módulos, verificar que
  os métodos de `[[uses]]` existem no `FacadeDescriptor`, `--owners`, orçamentos
  de latência, análise de segurança.

### Contrato Rust-first — macro `#[seiva::facade]` (ADR 0001)
- **Crate novo `seiva-macros`** (proc-macro; só `syn`/`quote`/`proc-macro2`).
  `#[facade(version = "v1")]` sobre um trait: re-emite o trait **inalterado**
  (consome os atributos auxiliares `#[requires(..)]`, `#[policy(..)]`,
  `#[cached(..)]`) e **adiciona** um ZST `<Trait>Facade` que implementa
  `contract::Facade` com um `DESCRIPTOR` legível por máquina (nome, versão,
  métodos, permissão de cada um).
- **Módulo `contract` no `delonix-seiva`**: `Facade`, `FacadeDescriptor`,
  `MethodDescriptor`. É a fonte que as próximas fases lêem para gerar
  `.proto`/gRPC e impor o zero-trust `[[uses]]`/`[[allow]]` em compile-time.
- **Corte delimitado e honesto:** a macro ainda **não** gera `.proto`/gRPC —
  isso assenta no descritor numa fase seguinte. Extrai o contrato, nada mais.
- **"Sem magia escondida":** equivalente escrito à mão documentado no doc-test
  de `contract.rs` (passa). `delonix-seiva` re-exporta `facade`; usa
  `extern crate self as delonix_seiva` para os caminhos absolutos gerados
  funcionarem também nos testes do próprio crate.
- Teste de integração `tests/facade.rs` (3) + doc-test: descritor com
  nome/versão/3 métodos, `requires` vazio em método sem guarda, consulta por
  nome, e o trait continua implementável e executável.

### Núcleo — ADR 0002 + `seiva-testing`
- **ADR 0002 (Accepted):** fixa os 4 ports do núcleo (`Clock`, `IdGenerator`,
  `EventPublisher`, `UnitOfWork`/`UnitOfWorkFactory`), o modelo de erro
  (`PortResult`/`BoxError` neutro), DI resolvida na compilação (sem service
  locator; porta sem provider = erro de build), e a política de estabilidade do
  `seiva-ports` (SemVer estrito, só traits, única dep `async-trait`). Ports de
  camada de dados (`Connection`, `OutboxStore`, `JobStore`) adiados para a fase
  da camada de dados.
- **Crate novo `seiva-testing`** (dep só de `seiva-ports`; `dev-dependency` dos
  módulos): um dublê por porta — `FixedClock`, `SequenceIdGenerator`,
  `RecordingEventPublisher`, `InMemoryUnitOfWorkFactory`. 6 testes.
- **Prova do payoff:** teste `use_case_runs_against_the_four_doubles_only` corre
  um caso de uso completo só com os 4 dublês — sem servidor, broker ou BD.
- **Defeito de desenho corrigido:** `UnitOfWork` passou de `Send + Sync` para só
  `Send` (uma transacção é movida/consumida por uma tarefa, nunca partilhada; os
  hooks `FnOnce+Send` não são `Sync`). A factory, essa, mantém `Send + Sync`.
- Semântica `on_commit` provada: corre no commit, descartado no rollback. O hook
  é síncrono e, no framework real, **acorda o relay** (não publica inline).

### Fundação (sessão anterior desta campanha)
- Repo `projects/seiva` criado (git `main`), lado a lado com `DORM/`.
- Workspace Cargo com 3 crates, a compilar, testar, clippy e fmt verdes:
  - `seiva-ports` — boundary só de traits, sem runtime/macros/data layer. Ports:
    `Clock`, `IdGenerator`, `EventPublisher`, `UnitOfWork` + `UnitOfWorkFactory`,
    tipo `Id([u8;16])` sem dependência de UUID. 1 teste. (`#![forbid(unsafe_code)]`)
  - `delonix-seiva` — entrypoint do framework; re-exporta `ports`; `VERSION` e
    `DORM_ENABLED`. Feature `dorm` ligada por omissão.
  - `seiva-cli` — binário `seiva` (clap). Árvore de comandos declarada
    (`project new`, `module new/split/merge`, `db dev/migrate`, `lint`, `expand`,
    `start`, `doctor`); só `doctor` e `--version` fazem trabalho real; o resto
    reporta a fase que o implementa (exit 64), sem fingir sucesso.
- ADR 0001 (Accepted): repositório próprio + dependência num só sentido;
  contrato Rust-first (`#[seiva::facade]` → `.proto`); barramento NATS JetStream
  (Kafka/AMQP atrás de feature).
- README, LICENSE-MIT/APACHE (iguais à família), `.gitignore`.

### Provas vivas
- `cargo build/test/clippy/fmt --workspace` → tudo verde.
- `seiva doctor` com feature `dorm` → "DORM"; `--no-default-features` →
  "external". **Decisão 5 ancorada no código**: o núcleo não depende do DORM.
- `seiva project new shop` → mensagem honesta de "not implemented yet" (exit 64).

### Decisões tomadas
- Crates fundadores: `seiva-ports` (SemVer-estável por política), `delonix-seiva`,
  `seiva-cli`. Nenhum depende do DORM; o DORM só entrará por `seiva-data-dorm`
  atrás da feature `dorm` (ADR 0001).
- Dependência do DORM será por git com revisão fixada (`ad5073d`) até o DORM
  publicar a API de biblioteca — **ainda não ligada** (honra decisão 5 + build offline).
- `Id` sem dependência de UUID no boundary; adapters renderizam como UUID/ULID.

## Próximos passos (propostos)
1. ~~Núcleo: ADR dos ports + dublês~~ ✅ (ADR 0002 + `seiva-testing`).
2. ~~Macro `#[seiva::facade]` (1.º corte: descritor de contrato)~~ ✅
   (`seiva-macros` + `contract`). **Falta** sobre o descritor: geração de
   `.proto` + cliente gRPC + testes de contrato; e `seiva expand` real.
3. **Gateway `seiva-data-dorm`**: ligar o DORM por git rev fixada (`ad5073d`),
   implementar `UnitOfWork`/`OutboxStore` sobre o `dorm-outbox`. Adicionar então
   os ports de camada de dados (`Connection`, `OutboxStore`, `JobStore`).
4. **Scaffolding**: `seiva project new` e `seiva module new` (5 camadas + `module.toml`).
5. Pendente: recuperar a lista numerada de fases do prompt (truncada em
   «Precedência (12…») para alinhar a ordem canónica.

## Dependência no DORM (lado DORM da campanha)
Para o Seiva avançar além do núcleo precisa de: API de biblioteca do DORM
(DORM-B4), namespaces (DORM-B2), primitivas transaccionais (DORM-B3) e a saída
do `dorm-messaging` → `seiva-messaging` (DORM-A). Ver `../DORM/docs/progress-dorm-seiva.md`.

## Auditoria de segurança S1 (2026-11-04) — OWASP API Top 10 + gate CVE

Fase S1 da auditoria de segurança. Controlos acrescentados ao transporte e ao CI:

- **API8** — `seiva-http::security`: headers de segurança por omissão (CSP, HSTS,
  nosniff, X-Frame-Options, Referrer-Policy) + CORS por allowlist explícita.
- **API4** — `seiva-http::rate_limit`: rate limiting por IP, janela fixa, relógio
  injetado; `seiva-server` serve com `ConnectInfo` para a chave ser o IP real.
- **API1/BOLA** — `seiva-http::guard::Principal` ganha `tenant` + `owns`/
  `require_owns` (verificação de posse object-level onde o recurso é conhecido).
- **API2** — `seiva-auth::LoginThrottle`: lockout/throttling de tentativas.
- **Gate CVE/KEV** — `deny.toml` + job `audit` no CI (cargo-deny). Gate duro sobre
  features por omissão (limpo hoje); passo informativo `--all-features`.

### Dívida rastreada — advisories dos backends de broker opcionais de `seiva-messaging`

O gate `--all-features` expõe 8 advisories RUSTSEC, **todos** em backends opcionais
(features `nats`/`amqp`/`kafka`, `default = []`; um build Seiva por omissão não os
puxa). O fix real é subir os majors dos clientes de broker (trabalho separado,
precisa de validação da crate `seiva-messaging`):

- `async-nats 0.45` → `rustls-webpki 0.102.8`: **5 vulnerabilidades** de verificação
  de certificados — RUSTSEC-2026-0049, -0098, -0099, -0104, -0382. Subir `async-nats`.
- `lapin 2.5` → `instant` (RUSTSEC-2024-0384) e `rustls-pemfile` (RUSTSEC-2025-0134),
  ambos *unmaintained*. Subir `lapin`/`tcp-stream`.
- `rskafka 0.5` → `zstd-safe 6` (RUSTSEC-2026-0403). Subir `rskafka`.

**Decisão do dono pendente:** (a) manter o gate duro sobre default + fix dos brokers
numa sessão dedicada [recomendado]; ou (b) gate `--all-features` com as 8 exceções
datadas em `deny.toml` até ao bump. Hoje está em (a).

## S3 — Cloud native real (2026-11-04)

- **App gerado = servidor real**: `apps/<name>-api/main.rs` usa `serve_router` +
  `observability::init` + `Health` + graceful shutdown (fim do stub `println!`).
- **Observabilidade**: `seiva_server::observability::init()` instala subscriber
  `tracing` JSON para stdout (12-factor; fecha a lacuna do S0).
- **Packaging**: o scaffold emite `Delonixfile` (multi-stage, distroless
  `cc-debian12:nonroot`, não-root), `compose.yaml` e `.dockerignore`.
- **`seiva start`**: corre o app em dev; hot-reload via `cargo watch` se presente.
- **Fitness**: teste `project_is_cloud_native_ready` (app real + Delonixfile
  distroless/multi-stage/não-root + compose mapeia porta).

### Prova (docker, 2026-11-04)
A receita do Delonixfile aplicada ao `seiva-server` real:
- imagem **28.2 MB**, **sem `/bin/sh`** (distroless), **`User=nonroot`**;
- contentor responde `/health/ready` **200**, `/health/live` **200**, `/docs` **200**;
- `/openapi.json` descobre `/v1/accounts/*` + `/v1/notes` (API9 em runtime);
- logs **JSON** em stdout: `{"timestamp":...,"level":"INFO","fields":{"message":"starting",...}}`.

### Dívida / pendente de S3
- **Projeto gerado não compila standalone**: `FRAMEWORK_CRATES` é path placeholder
  (profundidades app vs módulo inconsistentes). Resolve quando os crates do
  framework publicarem (crates.io/git). A prova acima usa o `seiva-server` real.
- **`seiva-server` arrasta a demo `accounts`**: os pedaços de runtime
  (`serve_router`/`observability`/`shutdown_signal`/`ServerConfig`) deviam viver
  num crate demo-free `seiva-runtime`; `seiva-server` = `seiva-runtime` + demo.
- **Verbos CLI em falta**: `stack up/down`, `cluster on/off`, `logs -f` (açúcar
  sobre `delonix`/compose; o `compose.yaml` gerado já funciona com `delonix stack
  up`/`docker compose up` diretamente).

## S4 — Limpeza do anti-pattern da Parte C (2026-11-04)

Único anti-pattern real da Parte C: `interface -> infrastructure` em produção
(scaffold + accounts), contra a regra do `seiva lint`, não apanhado porque o
`lint_project` só varre `modules/` (os exemplos não têm `module.toml`).

**Decisão do dono: MANTER a regra.** Controller passou a genérico sobre os ports
(`interface` depende só de `application`); a infra é injetada pela raiz de
composição (`seiva-server`); nos exemplos a infra foi para `[dev-dependencies]`.
Scaffold gera módulos lint-clean. Fitness nova
(`generated_interface_does_not_depend_on_infrastructure`) guarda o gerador.
Regra do lint inalterada. (Os exemplos continuam fora do scan do lint por não
terem `module.toml` — a fitness do gerador cobre o artefacto que importa.)

## S5 — Skills e agents (Parte G) + caminho para referência da comunidade (2026-11-04)

Instalados em `.claude/` (versionados via exceção no `.gitignore`; o resto de
`.claude/` fica local): 5 skills (`seiva-security-review`, `seiva-pattern-fitness`,
`seiva-cloud-native-check`, `seiva-dx-parity`, `seiva-community-adr`) + 5 agents
(`security-reviewer`, `rust-framework-architect`, `cloud-native-validator`,
`dx-advocate`, `community-maintainer`). Ver `.claude/README.md`. A auditoria passa
a gate permanente: uma sessão nova usa-os sem reler o prompt original.

### Console dxjs — ADR 0016 (Proposed, 2026-11-06)
[ADR 0016](adr/0016-dxjs-admin-console.md) propõe servir o console `/admin` por uma
SPA **dxjs** (`github.com/ngolacloud/dxjs`, nascida da auditoria StillJS→Seiva),
como mount **opt-in** (`/admin/dx`, módulo externo same-origin sob `script-src
'self'` — CSP estrita mantida) a par do console inline hash-pinned (ADR 0008), que
fica como default/fallback. Cutover completo = ADR futuro, só após paridade. Já
provado: o `examples/seiva-console` dxjs renderiza o contrato real deste backend
(`admin-e2e`) via proxy same-origin, sob CSP estrita, sem violações.

### Hashing de passwords plugável + fluxo de credenciais — ADR 0017 (2026-11-06)
[ADR 0017](adr/0017-pluggable-password-hashing.md): `seiva-auth` ganha o modelo
`PASSWORD_HASHERS` do Django — `trait PasswordHasher` + registo ordenado
`PasswordHashers` (preferido p/ hashes novos, verifica qualquer algoritmo
registado, `needs_rehash` p/ upgrade-on-login), com `Argon2idHasher` default e as
funções livres a delegar (API inalterada). E `authenticate_password(throttle,
hashers, …)` — o fluxo de referência que **põe o `LoginThrottle` em serviço**
(lockout OWASP API2). 17 testes. security-review GO, lint 10/10, cargo deny ok.

### Framework padrão da comunidade — fase futura (fora do âmbito desta auditoria)
Falta, para o Seiva ser referência pública ao nível de axum/loco.rs/shuttle:
- **Getting-started de 5 minutos** executável (quando os crates do framework
  publicarem e um projeto gerado compilar standalone — ver dívida de S3).
- **Site de documentação próprio** (mdBook/rustdoc publicado; hoje há `docs/` + Pages de API).
- **Canal de anúncio de breaking changes** (RFCs/discussions + política de SemVer).
- **`CHANGELOG.md`** seguindo *Keep a Changelog* (hoje ausente).
- **Publicação dos crates** (crates.io ou git tag) — desbloqueia o getting-started
  e o `seiva start` ponta-a-ponta num projeto gerado.

## Brokers: advisories RESOLVIDOS (2026-11-04)

Decisão de S1 fechada na opção (a). Subidos os clientes de broker opcionais de
`seiva-messaging`: `async-nats` 0.45→0.51, `lapin` 2.5→4 (um ajuste de API:
`basic_publish` passou a receber `ShortString`), `rskafka` 0.5→0.6. Os 8
advisories RUSTSEC desapareceram (incl. as 5 vulnerabilidades de verificação de
certificados do `rustls-webpki` 0.102 via `async-nats`). Novo license transitivo
`CDLA-Permissive-2.0` (bundle de certificados CA `webpki-root-certs`, dados)
adicionado à allowlist. O passo `--all-features` do CI passou a **gate duro**
(fim do `continue-on-error`); `deny.toml` sem exceções. Workspace verde com
`cargo test --workspace --all-features`.

## Autorização object-level / BOLA fail-closed (2026-11-04, ADR 0005)

`permission_guard` só cobre API5 (function-level). Para API1/BOLA havia
`Principal::require_owns`, mas era **consultivo**: um handler que o esquecesse
servia dados de outro tenant em silêncio. ADR 0005 fecha a porta por tipo:
`Scoped<T>` (em `seiva-http::guard`) embrulha um recurso com o seu tenant e só
deixa ler o valor via `owned_by(&Principal) -> Result<T, Problem>` — sem
`into_inner`/campo público/`Deref`, logo esquecer a verificação é **erro de
compilação**, não fuga em runtime. `require_owns` fica como escotilha explícita
(ex.: lista já filtrada por `WHERE tenant = $1`).

Dívida aceite: adoção é por-módulo e opt-in; e o primitivo vive em `seiva-http`
(construído na costura do interface), por isso a `application` ainda não devolve
`Scoped<T>` diretamente — mover o primitivo para um crate sem transporte é o
refactor que o primeiro módulo multi-tenant real vai forçar.

## Identidade / autenticação (bloqueador #1 do audit) — ADR 0006 (2026-11-04)

Novo crate transport-free `seiva-identity`: um `Principal` único (subject, tenant,
permissions), o trait `Authenticator` (deny-by-default), `TokenAuthenticator`
(dev/test) e `JwtAuthenticator` (verificação OIDC/JWT contra JWKS injetado —
RS256/ES256 só, nunca HS*/`none`; valida iss/aud/exp/nbf; mapeia claims→Principal
com claims de tenant/permissões configuráveis e fallback `scope`). `seiva-http`
passou a importar o `Principal` daqui (re-exportado; `require_owns` virou o trait
`PrincipalExt`) e ganhou o middleware `authn` + `router_with_auth`: sem bearer →
anónimo (rotas públicas servem, guardadas dão 403); bearer inválido → 401 com
`WWW-Authenticate`; bearer válido → `Principal` dos claims verificados, à frente
do `permission_guard`. Provado por crypto real (fixtures RSA+JWKS) e integração
401/403/200.

**Dívida/ressalvas (datadas 2026-11-04):**
- **Fetch/rotação de JWKS e allowlist de emissor (SSRF, API7) ficam de fora** — o
  `JwtAuthenticator` consome um `JwkSet` estático; um porto de refresh vem depois.
  Emissão de tokens é do IdP, não do Seiva.
- **Composição `merge`-depois-`layer`**: em axum, `router(health).merge(módulo)`
  NÃO aplica as camadas do framework (authn/security/request-id) às rotas do
  módulo mescladas depois. Para a autenticação ser efetiva nas rotas reais, a
  composition root tem de mesclar os módulos ANTES de aplicar as camadas (ou usar
  um helper do framework). Fica como follow-up prioritário (toca o scaffold e o
  `serve_router`), relacionado com o bloqueador #7.
- **Unificação do `seiva-mcp`** sobre o `seiva-identity` (o seu `Principal` tem
  `can_write`, não `permissions`, e está acoplado ao DORM) fica como follow-up
  deste mesmo tema; o crate partilhado já existe para o efeito.

### Identidade efetiva ponta-a-ponta (atualização 2026-11-04)

Resolvida a ressalva da composição `merge`-depois-`layer`: `seiva-http` ganhou
`router_with_modules(health, security, auth, modules)`, que mescla as rotas dos
módulos ANTES de aplicar as camadas — a única ordem que faz a authn/headers/limite
valerem nas rotas de módulos (provado por teste: rota de módulo mesclada → bad
bearer dá 401, não é contornada). O `seiva-server` passou a ler o OIDC do ambiente
(`SEIVA_OIDC_JWKS_FILE`/`_ISSUER`/`_AUDIENCE` [+ claims]; trio parcial falha o
boot) e constrói um `JwtAuthenticator` a partir do ficheiro JWKS, montando o
`authn` via `router_with_auth`. Provado AO VIVO: servidor real com JWKS, sem token
→ 200 (anónimo), bad bearer → 401 + `WWW-Authenticate`, JWT RS256 válido assinado
com a chave da fixture → 200.

Pendente (não bloqueia este tema): adotar `router_with_modules` no `main` gerado
pelo scaffold (ligado ao bloqueador #2, publicação+compilação do projeto gerado);
unificação do `seiva-mcp`.

## Fidelidade do contrato OpenAPI (bloqueador #3) — ADR 0007 (2026-11-05)

O descriptor do facade carrega os tipos como *nomes* (string), por isso o gerador
OpenAPI produzia um contrato infiel de duas formas: (1) o corpo de um write era
embrulhado como objeto chaveado pelo nome do parâmetro (`{"cmd":…}`), quando o
`#[controller]` desserializa o corpo diretamente como o DTO (`Valid<CreateNote>`,
e proíbe >1 param de corpo) — um cliente que seguisse o doc era rejeitado; (2) o
schema de qualquer DTO era `{"type":"string","x-seiva-rust-type":"…"}`, afirmando
*string* onde o fio leva um objeto.

Resolvido com `WireSchema` (ADR 0007), sem reflexão: um DTO descreve-se via
`#[derive(WireSchema)]` (trait `delonix_seiva::schema::WireSchema` + impls dos
escalares/`Vec`/`Option`/`&T`), que implementa o schema (objeto composto campo-a-
campo; `Option` fora de `required`) E regista o tipo por nome num inventário
link-time — o mesmo mecanismo que o `#[facade]` já usa para descobrir facades. O
gerador resolve o nome via `registered_schema` ANTES do placeholder (tipo sem
derive degrada graciosamente). **Aditivo**: o descriptor não muda (`ty: &str`
mantém-se), por isso o gerador gRPC e os checks de contrato ficam intocados. O
corpo de um write passa a ser o schema do DTO diretamente (sem envelope). Exemplos
(notes/accounts) e o template do scaffold derivam `WireSchema`.

Provado por efeito: `seiva openapi` documenta o create de `notes` com corpo
`{title}` e resposta `{id,title}`, **0** `x-seiva-rust-type` no documento real.
Testes: gerador 11/11 (corpo-direto, resposta-do-inventário, placeholder p/ tipo
não registado), schema 4/4, fitness que COMPILA o projeto gerado (DTOs com derive)
a passar, workspace inteiro verde.

**Ressalvas (datadas 2026-11-05):**

- **gRPC/proto continua com placeholder** (lê `ty`, mapeamento próprio) — enriquecer
  a mensagem proto a partir do `WireSchema` fica como follow-up.
- **`Vec<u8>`** compõe como array de inteiros (não o `format:"byte"` do mapeador por
  string) quando chega por campo composto — divergência menor, documentada.
- **Colisão de nomes curtos** entre DTOs de crates diferentes (first-match) — herdada
  dos descriptors chaveados por nome curto.
