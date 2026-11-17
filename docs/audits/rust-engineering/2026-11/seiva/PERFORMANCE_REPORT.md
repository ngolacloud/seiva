# PERFORMANCE_REPORT — Seiva @ 985cfa1

Foco: contenção, complexidade algorítmica, recursos (pool/timeouts), cardinalidade de métricas.
Nota: a maioria são limitações conhecidas com mitigação operacional, não defeitos de correcção.

## P1 🟡 — RateLimiter in-process: Mutex global + varrimento O(n) sob flood (A5)

- **Onde:** `crates/seiva-http/src/rate_limit.rs:186-235`, em especial `:194-211`.
- **Custo:** (1) **contenção** — cada request toma um único `std::sync::Mutex` global; sob RPS
  alto num runtime multi-thread, todos os pedidos serializam neste lock. (2) **Ironia API4** — ao
  atingir `MAX_TRACKED_CLIENTS` (100k), cada request novo sob flood de IPs distintos faz um
  `retain` O(n) e, se cheio, `min_by_key` O(n) para despejar o mais antigo — **O(n) por request**
  com o Mutex preso. O limitador fica lento no cenário de flood que deve defender.
- **Correcção não é deadlock:** o guard é correctamente largado antes de qualquer `.await` (é
  escalabilidade, não correcção).
- **Mitigação existente:** `DistributedRateLimit` (Redis, ADR 0015) é a via recomendada a escala;
  o in-process serve um nó.
- **Recomendação:** estrutura amortizada (despejo por amostragem) ou sharding do mapa por hash da
  chave; ou documentar o tecto de RPS do limitador in-process. Micro-bench de `check` com tabela
  cheia vs vazia prova o salto para O(n)/op.

## P2 🟡 — Gateway descarta `statement_timeout` → exaustão de pool (D-4)

- **Onde:** `crates/seiva-data-dorm/src/lib.rs:82-94` (`ConnectOptions { pool_max, ..default() }`);
  `DormConfig` (`:56-70`) sem campo de timeout. `dorm-core 0.6.1` `default()` →
  `statement_timeout_ms = 0` (desligado); `from_env()` **não** usado.
- **Custo:** nenhuma query tem `statement_timeout` server-side. Uma query descontrolada (lock,
  plano mau) prende a ligação indefinidamente; sob carga esgota o pool (`pool_max`) e derruba o
  módulo. Como o gateway usa `..default()` em vez de `from_env()`, **nem o env**
  `DORM_STATEMENT_TIMEOUT_MS` é respeitado — o valor é sempre 0.
- **Recomendação:** expor `statement_timeout_ms` (e connect/acquire timeouts) em `DormConfig` e
  propagá-los; ou basear em `ConnectOptions::from_env()`. Teste: `statement_timeout_ms=100` +
  `pg_sleep(1)` → asserir erro de timeout.

## P3 🟢 — Argon2id inline na thread do runtime async (B-5)

- **Onde:** `examples/accounts/interface/src/lib.rs:198-200`; `seiva-auth::hash_password` é síncrono.
- **Custo:** uma rajada de login/create-user (Argon2 default ~19 MiB, t=2) satura o executor. O
  `LoginThrottle` limita abuso por-chave mas não uma rajada distribuída por muitas chaves.
- **Recomendação:** `tokio::task::spawn_blocking` (ou pool de hash limitado) no fluxo de referência;
  notar no scaffold.

## P4 🟢 — Verificação de JWT antes do rate-limit shedding

- **Onde:** `seiva-http/src/lib.rs:286-293` — `authn` é mais exterior que `rate_limit`.
- **Custo:** correcto para a dimensão-tenant do limiter (precisa do `Principal`), mas um flood de
  tokens inválidos paga a verificação de assinatura (CPU) antes de ser limitado.
- **Recomendação:** considerar um shedding volumétrico por-IP anónimo antes do authn, ou aceitar
  como tradeoff documentado (baixo).

## Forças de performance (verificadas)

- **CQRS mediator monomorfizado** — dispatch estático, sem `TypeId`/`dyn`, sem modo de falha em
  runtime; behaviors compõem por nesting tipado (zero custo de dispatch).
- **Backpressure sem alocação de fila** — `try_acquire_owned` (503 imediato, nunca enfileira),
  permit por drop, `timeout` → 504.
- **Métricas de baixa cardinalidade** — label por route-**template** (não path), status-class,
  `<unmatched>` colapsado; **tenant nunca é label** (evita explosão de séries). Gauge drop-safe.
- **Trace context custo-zero** — `set_parent`/`inject_current` são no-ops até um serviço activar
  `seiva-observability/tracing`; o build offline não paga nada.
- **Comparações em tempo constante** — webhook HMAC (`ct_eq`), lookup de API-key por digest.
- **Redis limiter** — um único round-trip Lua atómico (`INCR`+`PEXPIRE`+`PTTL`), TTL por janela.
