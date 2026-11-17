# COMPETENCY_MATRIX — Seiva @ 985cfa1

Classe por competência: **Correct · Partial · Incorrect · Missing · N/A**. A classe reflecte
o estado do subsistema, não de um achado isolado; a coluna "Achados" aponta onde falha.

| Competência (distributed systems) | Classe | Achados | Evidência-chave |
|---|---|---|---|
| Fronteiras async (Send/Sync/'static, lock-através-de-await) | **Correct** | — | 5 crates verificados caso a caso; zero locks através de `.await` (lane A §STRENGTHS) |
| `unwrap/expect/panic` em caminho quente | **Correct** | A3 (baixo) | ≈54 em lib-path, infalíveis/poison; 0 sobre input não-confiável |
| Supervisão de tasks / graceful shutdown | **Partial** | A2, E1 | `seiva-server/src/lib.rs:626-665` tasks detached; `:518-524` shutdown sem prazo |
| Back-pressure por rota | **Correct** | — | `backpressure.rs` (504/503, permit por drop), testado |
| CQRS / mediator | **Correct** | — | dispatch monomorfizado, pipeline `Authorize→Validate→Transactional` fixado por teste |
| Event dispatch / consistência (consume-path) | **Incorrect** | **D-1/A1 🔴**, D-2, D-5, D-6 | `seiva-events/src/lib.rs:233-241` claim não-atómico; teste mascara (`choreography.rs`) |
| Outbox (write-path) | **Correct** | — | commit conjunto negócio+evento; `publish` recusa fora de tx (`seiva-data-dorm:233-266`) |
| Entrega / dead-letter / retry (in-process) | **Missing** | D-2 | broker tem `max_attempts=5`+backoff+dead-letter; in-process não tem nada |
| Concorrência de relay (multi-réplica) | **Partial** | D-3 | sem `FOR UPDATE SKIP LOCKED`/lease (`seiva-outbox-relay/src/lib.rs:191-250`) — ADR 0028 §1 |
| Integração de BD (pool/timeouts) | **Partial** | D-4 | `statement_timeout` descartado (`seiva-data-dorm/src/lib.rs:82-94`) |
| JWT / bearer | **Correct** + Missing | B-3 | alg-confusion fechado (`jwt.rs:29,131,141`); sem revogação/introspecção |
| API-key (X-API-Key) | **Correct** + Missing | B-4 | SHA-256 em repouso + `revoke`; sem gerador seguro |
| Multi-tenant / zero-trust (SG1) | **Partial** | **B-1 🟡** | mecanismo correcto; `Scope::Global` default (`seiva-cqrs/src/lib.rs:476`, 7/8 exemplos) |
| Rate-limit | **Correct** (algoritmo) + Partial | B-2, A5 | Lua atómico; Redis fail-open (`rate_limit.rs:397`); in-process O(n) sob flood |
| Segredos / CORS / headers / timing | **Correct** | — | headers seguros em toda resposta; CORS allowlist; auditoria redige segredos |
| Injecção SQL | **N/A** (sem superfície) | — | 3 `format!`+SQL só interpolam nomes de tabela `const`; valores via `$1..$n` |
| Password hashing | **Correct** | B-5 (baixo) | Argon2id+salt CSPRNG+PHC+upgrade; corre inline (usar `spawn_blocking`) |
| Observabilidade (métricas/trace) | **Correct** | E2, E8 | baixa cardinalidade; falha de bind engolida (`seiva-server:646-667`) |
| Config / 12-factor | **Correct** + Partial | E3 | fail-fast exemplar; sem segredos por `*_FILE` |
| Contratos / SemVer-stability | **Partial** | **C1 🟠** | `ports`/`contract`/descritores sem `#[non_exhaustive]` (`seiva-macros:198-228`) |
| Proc-macros (qualidade/hygiene) | **Correct** | C2 (refutado 🟢), C12 | erros spanned de qualidade; C2 "panic no arranque" **refutado** (E0407 + descritor regista todos os métodos → `.expect()` inalcançável) |
| API pública / ergonomia / DX | **Partial** | C3, C4, C8, C10 | 3 nomes confundem hello-world; doc mostra `#[seiva::facade]` que não compila |
| Documentação | **Partial** | C5, C7 | boa à mão, mas `missing_docs` não imposto nas crates estáveis; sem `docs.rs` metadata |
| Publicabilidade (crates.io/docs.rs) | **Missing** | C8, F | núcleo publicável; `seiva-cli` `publish=false`; 2 crates com dep `git` ao DORM |
| Testes adversariais (fuzz/property/trybuild/loom) | **Missing** | F, D-5 | 348 testes mas sem proptest/fuzz/trybuild; teste de eventos mascara D-1 |
| `unsafe` / segurança de memória | **Correct** | — | 1 `unsafe` (teste); 21/25 `#![forbid(unsafe_code)]` |

## Leitura da matriz

- **Núcleo de correcção sólido** (async, CQRS, outbox-escrita, auth, headers).
- **Um buraco de correcção real:** event dispatch no consume-path (Incorrect → D-1/A1).
- **Fragilidades de contrato/macro** que minam a narrativa de "alta criticidade" (C1, C2).
- **Lacunas de maturidade**, não de código: publicabilidade e testes adversariais (Missing).
