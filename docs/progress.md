# Progress — Delonix Seiva

> Tracker canónico do Seiva. Uma sessão nova começa por ler o prompt único e
> este ficheiro. O lado DORM da campanha tem o seu tracker em
> `../DORM/docs/progress-dorm-seiva.md`.

## Estado (2026-11-03)

**Fase fundadora concluída** — repo `ngolacloud/seiva` arrancado.

### Feito nesta sessão
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
1. **Núcleo**: ADR do núcleo que fixa a forma definitiva dos ports + dublês em
   memória para teste; macro `#[seiva::facade]` (crate `seiva-macros`).
2. **Gateway `seiva-data-dorm`**: ligar o DORM por git rev fixada, implementar
   `UnitOfWork`/`OutboxStore` sobre o `dorm-outbox`.
3. **Scaffolding**: `seiva project new` e `seiva module new` (5 camadas + `module.toml`).
4. Pendente: recuperar a lista numerada de fases do prompt (truncada em
   «Precedência (12…») para alinhar a ordem canónica.

## Dependência no DORM (lado DORM da campanha)
Para o Seiva avançar além do núcleo precisa de: API de biblioteca do DORM
(DORM-B4), namespaces (DORM-B2), primitivas transaccionais (DORM-B3) e a saída
do `dorm-messaging` → `seiva-messaging` (DORM-A). Ver `../DORM/docs/progress-dorm-seiva.md`.
