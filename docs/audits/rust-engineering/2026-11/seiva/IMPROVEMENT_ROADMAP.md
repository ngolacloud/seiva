# IMPROVEMENT_ROADMAP — Seiva @ 985cfa1

Priorização por **risco × alavancagem**. Cada item tem critério de pronto observável.
P0 implementado nesta remessa; P1–P3 por fazer.

## P0 — Correcção (bloqueia "alta criticidade")

| Item | Achado | Estado / Critério de pronto |
|---|---|---|
| Consumo in-process atómico (claim→efeito→mark numa `UnitOfWork::transaction`, ADR 0028 §2) | G1 (D-1/A1) 🔴 | **FEITO** — teste de reentrega-após-falha que **falha sem o fix** e passa com ele (efeito corre exactamente 1×); `cargo test -p seiva-events` verde |
| ~~`#[seiva::controller]` sem panic no arranque~~ | G4 (C2) | **REFUTADO** — o panic não se reproduz (E0407 impede método inerente no `impl`; descritor regista todos os métodos → `.expect()` inalcançável). Sem alteração de código. Ver AUDIT §3.1 |

## P1 — Consistência & segurança por default

| Item | Achado | Critério de pronto |
|---|---|---|
| Dead-letter/retry/backoff no driver in-process (paridade com broker) | G2 (D-2) 🟠 | Subscriber que falha N≥max_attempts → evento dead-lettered + cursor avança; teste de paridade com o broker |
| Scope derivado do `Ctx` fail-closed na pipeline CQRS + lint BOLA (ADR 0029) contra `Scope::Global` literal | G5 (B-1) 🟡 | Teste live 2-tenant sobre módulo tipo `notes`: tenant A **não** lê linhas de B; lint falha num `Scope::Global` fora de allowlist |
| `statement_timeout_ms` (+ connect/acquire) em `DormConfig` → `ConnectOptions` | P2 (D-4) 🟡 | `statement_timeout_ms=100` + `pg_sleep(1)` → erro de timeout mapeado |
| Falha de bind do `/metrics` falha o boot (contrato documentado) | E2 🟡 | Porta ocupada no exporter → `serve` devolve erro, não arranca "saudável" |
| Prazo de drenagem no graceful shutdown | E1 🟡 | `with_graceful_shutdown` com timeout; ligação presa não bloqueia além do prazo |

## P2 — Estabilidade de contrato & operação

| Item | Achado | Critério de pronto |
|---|---|---|
| `#[non_exhaustive]` nos tipos "SemVer-stable" (`Problem`, `Scope`, `IdempotencyOutcome`, `RateLimitDecision`, `AuditEntry`) + política para descritores | G3 (C1) 🟠 | `cargo public-api`/revisão confirma tipos non_exhaustive; política de descritor documentada |
| JWKS com refresh / `jwks_uri` | E4 (S3) 🟡 | Rotação de chave no IdP reflecte sem restart; teste de refresh |
| Rate-limit Redis fail-open **configurável** (fail-closed opcional) | S2 (B-2) 🟡 | Store a devolver `Err` → 200 (open) vs 429/503 (closed) conforme config |
| `spawn_blocking` no Argon2 do fluxo de referência | S5 (B-5) 🟢 | Hashing não corre na thread do executor; nota no scaffold |
| `ApiKey::generate()` CSPRNG | S4 (B-4) 🟢 | Helper devolve chave ≥128-bit + digest; teste de entropia/formato |
| Tasks auxiliares aguardadas no `serve` (drain determinístico) | A2 🟡 | `JoinHandle` do exporter/fan-out aguardado antes de `serve` retornar |
| Job CI de MSRV (`cargo +1.75.0 check`) ou subir `rust-version` ao floor real | C6 🟡 | CI verde em 1.75 **ou** `rust-version` actualizado ao medido |

## P3 — Adopção (os 6 pontos → [COMMUNITY_ACCEPTANCE.md](COMMUNITY_ACCEPTANCE.md))

| Item | Achado | Critério de pronto |
|---|---|---|
| Completar/experimental-marcar gRPC a partir do WireSchema; cablar worker real | G7 (E5/E6) 🟡 | gRPC com paridade de DTO ou doc "experimental"; worker não-no-op |
| `FOR UPDATE SKIP LOCKED`/lease no relay (ADR 0028 §1) | G6 (D-3) 🟡 | Dois workers concorrentes → cada evento despachado 1×; **nova sessão** |
| Publicação crates.io + docs.rs; governança; `main` pushado/limpo; testes adversariais | Ver COMMUNITY_ACCEPTANCE | Os 6 critérios de aceitação da comunidade satisfeitos |

## Sequência recomendada

1. **P0** (feito) — tira o Seiva da zona "contrato frágil + macro que rebenta".
2. **P1** — fecha a divergência de garantias e o default inseguro de tenant (as duas coisas que um
   avaliador de "alta criticidade" testa primeiro).
3. **P2/P3** — estabilidade de contrato e o último quilómetro de adopção, em paralelo.
