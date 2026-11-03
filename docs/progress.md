# Progress — Delonix Seiva

> Tracker canónico do Seiva. Uma sessão nova começa por ler o prompt único e
> este ficheiro. O lado DORM da campanha tem o seu tracker em
> `../DORM/docs/progress-dorm-seiva.md`.

## Estado (2026-11-03)

**Fundação + núcleo + facade + lint(×2) + scaffolding + módulo ligado + CI + gateway DORM.**

### `UnitOfWork` por escopo + ligado ao DORM (ADR 0004, decisão do dono)
- **Decisão #2 do dono:** o port `UnitOfWork` passa a ser **por escopo** —
  `transaction(|| async {…})`, commit em `Ok`, rollback em `Err` (dispensa
  `on_commit`/DORM-B3). Resolve o desencontro com o `db::atomic` (closure) do DORM.
- Implementado em toda a pilha: `seiva-ports` (port RPITIT genérico), `seiva-testing`
  (`InMemoryUnitOfWork` por escopo), `examples/notes/application` (valida no domínio,
  depois `transaction`), e o gateway (`DormUnitOfWork` → `with_session`+`db::atomic`).
- `NoteRepository::save` passou a `-> impl Future + Send` (o future tem de ser Send
  para correr dentro do `transaction`). ADR 0002 emendada; ADR 0004 regista a resolução.
- Falta só o port `EventPublisher` no gateway (precisa do `dorm-outbox` na mesma transacção).

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
