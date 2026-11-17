# RUST_ENGINEERING_AUDIT — Seiva

- **Alvo:** `seiva` @ `main 985cfa1` (árvore suja: ADR 0027 observability/trace em curso, não committado).
- **Papel:** Plataforma (framework de serviços; competência dominante = distributed systems).
- **Data:** 2026-11-15. **Modo:** read-only (auditoria não altera código).

## 1. Metodologia

Seis lanes independentes, cada uma read-only, com evidência `ficheiro:linha` ancorada no SHA
e distinção **Confirmado** (caminho lido ponta-a-ponta) vs **Plausível** (inferido):

- **Lane A** — async / concorrência / runtime boundaries (`seiva-http`, `-server`, `-cqrs`, `-events`, `-messaging`).
- **Lane B** — auth / identity / zero-trust / multi-tenant / rate-limit.
- **Lane C** — contratos / macros / API pública / SemVer / ergonomia (lente de adopção).
- **Lane D** — dados / eventos / outbox / consistência.
- **Lane E** — observabilidade / operação / config / prontidão produção.
- **Lane F** — adopção pela comunidade Rust + varreduras quantitativas transversais.

Consolidação feita por quem **não** gerou as lanes (evita auto-validação).

## 2. Veredicto honesto

A engenharia Rust **não é o problema do Seiva**. É disciplinada: `cargo clippy --workspace
--all-targets` dá **0 warnings**, 21/25 crates declaram `#![forbid(unsafe_code)]`, há **1**
bloco `unsafe` em todo o código (e está num teste), 348 testes, e o CI impõe
`clippy -D warnings` + fmt + `cargo-deny` + rustdoc `-D warnings`. A base de auth, outbox de
escrita, backpressure e observabilidade está **correcta e provada**.

O que o impede de ser adoptado hoje:

1. **Um bug crítico real** no consumo de eventos in-process (perda silenciosa de efeito) —
   ver §3.
2. **Uma fragilidade** que contraria a própria promessa de "framework de alta criticidade":
   contrato sem `#[non_exhaustive]` nos tipos declarados "SemVer-stable". *(A segunda suspeita —
   a macro `#[seiva::controller]` fazer panic no arranque, C2 — foi **refutada** na implementação:
   ver §3.1.)*
3. **O último quilómetro open-source:** não publicado em crates.io/docs.rs, sem
   governança (CONTRIBUTING/CHANGELOG/SECURITY), `main` atrás do local e árvore suja.

> **Um Rustacean criterioso não adoptaria hoje — por distribuição e maturidade de
> projecto, não por qualidade de engenharia.**

## 3. O achado que mais importa: convergência independente A1 ⇔ D-1

Duas lanes cegas uma à outra (A, async; D, dados/eventos) aterraram **no mesmo defeito**, no
mesmo ficheiro. É o sinal mais forte da auditoria.

**Perda silenciosa de efeito no consumo de eventos in-process** — `crates/seiva-events/src/lib.rs`
(loop `begin → handle → finish` em `dispatch_to_subscribers`, ~`:233-241`; `drain_in_process`,
~`:345-364`).

- **Mecanismo (provado por leitura):** `begin` persiste a reivindicação do inbox **antes** de
  `handle` e **fora de transacção**. Se `handle` falha: passagem 1 propaga `Err`, o claim já
  committou por autocommit, o cursor não avança; passagem 2 relê o evento, `begin` devolve
  `InProgress`, o laço trata como "já consumido" e devolve `Ok`, e o cursor **avança**. Uma
  falha transitória (timeout de rede, erro de BD momentâneo) vira **perda permanente** do
  efeito `(subscriber, evento)` até ao TTL de 24h.
- **Porque é grave:** contradiz os invariantes que o próprio crate documenta
  ("exactly-one effect per (subscriber, event)") e é o comportamento **por omissão** do
  worker gerado pelo scaffold (`delonix-seiva/src/scaffold.rs:1233-1278`). A docstring promete
  uma transacção "Fase 2" que o código não implementa.
- **O desenho da correcção já existe:** **ADR 0028 §2** (Proposed, 2026-11-15) —
  claim→efeito→mark numa só `UnitOfWork::transaction`.
- **Teste mascara o bug (D-5):** `seiva-events/tests/choreography.rs` /
  `a_failing_subscriber_does_not_advance_the_cursor` corre **1 só passagem** e dá verde; nunca
  exercita a reentrega após falha. Snapshot ≠ correcto.

## 3.1 Refutação na implementação: C2 (macro `controller`) **não se reproduz**

A Lane C classificou como Alto um alegado panic no arranque de `#[seiva::controller]` ("um helper
privado no `impl` gera uma rota cujo `.expect()` rebenta"). Na fase de implementação **refutei-o**,
com o compilador:

1. Num `impl Trait for Type` do Rust, todo método pertence ao trait — um método inerente dá
   `error[E0407]: method is not a member of trait` (reproduzido num crate de teste isolado).
2. `#[seiva::facade]` regista **todos** os `TraitItem::Fn` no descritor
   (`seiva-macros/src/lib.rs:92-180`); logo `descriptor.method(name)` nunca é `None` para um método
   do `impl`, e o `.expect()` em `seiva-macros/src/lib.rs:979` é **inalcançável**.

→ **Sem alteração de código.** Reclassificado para 🟢 Baixo (asserção defensiva). Registo isto como
exemplo da disciplina de cepticismo do arnês: um "Alto" aparente que não sobrevive à verificação.

## 4. Síntese por severidade

| Severidade | Achados |
|---|---|
| 🔴 Crítico | D-1/A1 (perda de efeito no consumo in-process) |
| 🟠 Alto | D-2 (in-process sem dead-letter/retry) · C1 (contrato sem `#[non_exhaustive]`) |
| 🟡 Médio | B-1 (tenant opt-in, Global default) · D-4 (statement_timeout descartado) · E2 (falha de bind do `/metrics` engolida) · E4 (JWKS sem refresh) · E1 (shutdown sem prazo) · A5 (rate-limiter in-process O(n)) · B-2 (rate-limit Redis fail-open) · A2 (tasks detached) · D-3 (relay sem SKIP LOCKED) · E5/E6 (gRPC placeholder / worker no-op) · C6 (MSRV não verificado no CI) |
| 🟢 Baixo | **C2 (refutado, ver §3.1)**, A3, B-3, B-4, B-5, D-6..D-9, C7..C12, E7..E11 |

## 5. Forças reais (verificadas, não assumidas)

- `#![forbid(unsafe_code)]` em 21/25 crates; **1** `unsafe` total (em teste). Clippy limpo.
- **Outbox de escrita correcto:** mudança de negócio e evento fazem commit juntos
  (`seiva-data-dorm/src/lib.rs:187-208`); `publish` **recusa** correr fora de transacção (`:233-266`).
- **Auth sólida:** alg-confusion fechado (allowlist RS256/ES256, rejeita `none`/`HS*`);
  X-API-Key SHA-256 em repouso + `revoke`; Argon2id com salt CSPRNG, PHC, upgrade-on-login,
  throttle de login limitado. BOLA fechado por tipo (`Scoped<T>` sem `into_inner`/`Deref`).
- **Zero locks mantidos através de `.await`** (verificado caso a caso em 5 crates).
- **Backpressure correcto** (504 timeout / 503 `try_acquire` sem fila, permit por drop).
- **Observabilidade de baixa cardinalidade** (route-template, tenant nunca é label,
  `<unmatched>` colapsado); `/health` live/ready isento de métricas; trace propagation provada.

## 6. Resultado desta auditoria (acção)

O P0 real (D-1/A1) foi implementado nesta mesma remessa seguindo o ADR 0028 §2 — ver
[IMPROVEMENT_ROADMAP.md](IMPROVEMENT_ROADMAP.md) e o commit da sessão. O segundo candidato a P0
(C2) foi refutado e não exigiu alteração (§3.1). Os restantes achados ficam priorizados P1–P3; a
barreira de adopção está destilada em [COMMUNITY_ACCEPTANCE.md](COMMUNITY_ACCEPTANCE.md).
