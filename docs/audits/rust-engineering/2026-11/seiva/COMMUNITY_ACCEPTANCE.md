# COMMUNITY_ACCEPTANCE — os 6 pontos para romper a barreira de adopção

A auditoria concluiu que **a engenharia não é o travão** do Seiva — é a maturidade e a
distribuição. Estes são os **6 pontos** que, satisfeitos, levam um Rustacean criterioso de
"fecho a aba" a "dou-lhe uma oportunidade". Cada um tem um **critério de pronto observável** (não
"parece bom"). Ordenados por alavancagem.

---

## 1. Instalável e documentado do registo oficial (crates.io + docs.rs)

**Porquê:** a primeira coisa que um Rustacean faz é `cargo add` / abrir docs.rs. Hoje o núcleo é
publicável mas **não publicado**, e `seiva-cli` é `publish = false` (C8) — a porta de entrada
"estilo Django/FastAPI" não corre do registo.

**Pronto quando:**
- [ ] `cargo add seiva` (ou o nome publicado) resolve de crates.io; os crates centrais
      (`seiva-ports`, `seiva-contract`, `seiva-http`, `seiva-server`, `delonix-seiva`, `seiva-macros`)
      têm uma versão publicada.
- [ ] docs.rs constrói cada crate publicado a verde, com `[package.metadata.docs.rs]` a fixar as
      features documentadas (C7).
- [ ] `cargo install seiva-cli` (ou equivalente) instala o CLI do registo — ou o CLI é
      explicitamente marcado como "via git" na doc, sem o anunciar como comando de arranque.

## 2. Hello-world coerente que compila como está escrito

**Porquê:** a documentação inteira usa `#[seiva::facade]`, que **nunca compila** como escrito — os
exemplos reais usam `#[facade]` com `use delonix_seiva::facade` (C3, C4). Três nomes
(`delonix-seiva` / `seiva` / `delonix_seiva`) tropeçam antes do primeiro `cargo build`.

**Pronto quando:**
- [ ] Um único namespace decidido e usado de ponta a ponta (docs + exemplos + README).
- [ ] Todos os blocos de código da doc do "getting started" são **doctests** ou testados em CI, por
      isso não podem divergir do que compila.
- [ ] O exemplo canónico mandado copiar (`notes`) usa a macro-bandeira `#[controller]` (C10), para
      quem copia ver o headline a funcionar.
- [ ] As macros aceitam `crate = "..."` (ou resolvem via `proc_macro_crate`), para re-exports e
      workspaces guarda-chuva não partirem a compilação (C3).

## 3. Contrato estável a sério (`#[non_exhaustive]` + política SemVer)

**Porquê:** as crates declaradas "SemVer-stable by policy" não marcam **nenhum** tipo
`#[non_exhaustive]` (C1/G3); adoptar hoje é aceitar que qualquer `0.x → 0.y` pode partir todo o
código gerado. Num framework cuja proposta é "o contrato é a fonte de verdade", a instabilidade
está precisamente nos tipos do contrato.

**Pronto quando:**
- [ ] `Problem`, `ValidationError`, `Scope`, `IdempotencyOutcome`, `RateLimitDecision`, `AuditEntry`
      são `#[non_exhaustive]`.
- [ ] Os descritores construídos por literal no crate do utilizador (`Param`/`MethodDescriptor`/
      `FacadeDescriptor`) têm política documentada: ou construtores `const fn`/builder, ou
      declaração explícita de que são a parte versionada da macro (bump acoplado).
- [ ] `#![deny(missing_docs)]` nas crates estáveis e na flagship (`seiva-ports`, `seiva-contract`,
      `delonix-seiva`) (C5), e um gate `cargo public-api` (ou equivalente) que falha num breaking
      change não-intencional.

## 4. Garantias de entrega provadas e **iguais** monólito↔microserviço

**Porquê:** é a proposta de valor central ("o mesmo código de subscriber, as mesmas garantias, por
composição"). A auditoria encontrou aqui o **único bug crítico** (D-1/A1, perda silenciosa de
efeito — **corrigido nesta remessa** via compensação de claim) e uma **divergência de garantias**:
o caminho in-process não tem dead-letter/retry que o broker tem (D-2), nem o relay tem
`SKIP LOCKED` para escalar em segurança (D-3, ADR 0028 §1).

**Pronto quando:**
- [x] O consumo in-process não perde efeito numa falha transitória: teste de reentrega-após-falha
      verde (prova em `seiva-events/tests/choreography.rs`).
- [ ] O driver in-process tem a mesma política de falha do broker (max-attempts, backoff,
      dead-letter) — teste de paridade (D-2).
- [ ] O relay é seguro a N réplicas (`FOR UPDATE SKIP LOCKED`/lease, ADR 0028 §1) — teste de dois
      workers concorrentes a despachar cada evento uma vez (D-3).
- [ ] A atomicidade claim-effect-mark completa do ADR 0028 §2 no caminho DORM (migração de schema
      aplicada por `seiva db migrate`), cobrindo o crash entre efeito e mark.

## 5. Segurança por **default**, não por opção

**Porquê:** a base de auth é sólida (alg-confusion fechado, BOLA por tipo, Argon2id bem feito), mas o
**isolamento multi-tenant é opt-in**: `Scope::Global` é o default embarcado e 7/8 exemplos correm
nele (B-1/G5). Quem copia o scaffold herda zero isolamento row-level — o oposto do que um framework
"de alta criticidade" deve entregar por omissão.

**Pronto quando:**
- [ ] A pipeline CQRS deriva o scope de um `Ctx`/`Principal` ambiente e **falha fechada**, em vez de
      fixar `Scope::Global` (`seiva-cqrs/src/lib.rs:476`).
- [ ] Os módulos de exemplo realizam o padrão `accounts` (tenant-scoped ponta-a-ponta), não `Global`.
- [ ] Um lint de fitness (ADR 0029 BOLA) falha num `Scope::Global` literal fora de uma allowlist de
      manutenção; a doc declara `#[model(tenant_scoped)]` como obrigatório para isolamento.
- [ ] Teste live 2-tenant sobre um módulo tipo `notes`: o tenant A **não** lê linhas do tenant B.

## 6. Governança, CI reprodutível e testes adversariais

**Porquê:** um projecto open-source credível mostra como se contribui, como evolui, e que se leva a
sério a correcção sob entradas hostis. Faltam `CONTRIBUTING/CHANGELOG/SECURITY`; o `main` público
está atrás do local e com árvore suja; o MSRV 1.75 é declarado mas **não testado** (C6); e não há
`trybuild`/proptest/fuzz num framework macro-pesado e orientado a dados.

**Pronto quando:**
- [ ] `CONTRIBUTING.md`, `CHANGELOG.md` (keep-a-changelog), `SECURITY.md` presentes;
      `CODE_OF_CONDUCT.md` opcional.
- [ ] `main` pushado, à frente ou a par do trabalho local, com árvore limpa (nada por committar nos
      crates centrais).
- [ ] Job de CI que compila com a MSRV declarada (`cargo +1.75.0 check --workspace`) **ou** o
      `rust-version` subido ao floor real medido (C6).
- [ ] `trybuild` a cobrir os erros de compilação das macros (fail-closed, spans), e property-based
      (`proptest`) nos parsers/`WireSchema`; fuzz nos pontos que lêem bytes externos.

---

## Veredicto de adopção (honesto)

| Hoje | Depois dos 6 pontos |
|---|---|
| **Não adoptaria** — contrato frágil, onboarding incoerente, não publicado, 1 bug crítico (agora corrigido) | **SIM condicional** — proposta diferenciada (arquitectura forçada pelo compilador + contrato único → HTTP/gRPC/OpenAPI + segurança de origem), com a maturidade que lhe falta hoje |

A proposta técnica do Seiva é **diferenciada** face a axum/actix (que são bibliotecas, não
frameworks opinados) e disputa o mesmo espaço de loco.rs/Pavex — mas sem a maturidade de ecossistema
de nenhum deles. Fechar estes 6 pontos não torna o Seiva popular; **remove as razões para o
rejeitar à primeira vista**, que é o pré-requisito para a popularidade poder sequer começar.
