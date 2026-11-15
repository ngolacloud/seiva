# SECURITY_REPORT — Seiva @ 985cfa1

Âmbito: auth, identity, zero-trust, multi-tenant, rate-limit, segredos, injecção, headers.
Base **muito sólida**; os achados são sobretudo *posture*/*defaults*, não vulnerabilidades
exploráveis directas. Legenda: Confirmado = caminho lido ponta-a-ponta.

## Achados

### S1 🟡 Médio — Isolamento de tenant opt-in, não default (B-1) · Partial · Confirmado
Ver [ARCHITECTURE_GAPS G5](ARCHITECTURE_GAPS.md). `seiva-cqrs/src/lib.rs:476` fixa `Scope::Global`
na pipeline genérica; 7/8 exemplos correm em Global → bypass `with_system_scope`
(`seiva-data-dorm/src/lib.rs:112`). Row-isolation exige ainda `#[model(tenant_scoped)]` (só o
`dorm-e2e` o faz). Quem copia o scaffold `notes` herda zero isolamento. **Correcção:** scope
derivado do `Ctx` fail-closed + lint BOLA (ADR 0029) contra `Scope::Global` literal.

### S2 🟡 Baixo–Médio — Rate-limiter distribuído faz fail-OPEN (B-2) · Partial · Confirmado
`seiva-http/src/rate_limit.rs:397-400` — em erro de `store.check`, o request é permitido. Quem
degradar/DoS o Redis desliga o limite global (API4). Tradeoff documentado, mas devia ser
**configurável** (fail-closed opcional para deploys sensíveis). Mitigação: o throttle de login é
in-process e não é afectado → credential-stuffing continua protegido.

### S3 🟡 Baixo–Médio — JWT bearer sem revogação/introspecção (B-3) · Missing · Confirmado
`seiva-identity/src/jwt.rs` valida assinatura/iss/aud/exp/nbf mas não tem deny-list nem
introspecção; `JwkSet` é por valor (rotação delegada ao wiring). Token comprometido vale até
`exp`; sem kill-switch; rotação de chave no IdP não reflecte até re-wire. Mitigação: a via
`X-API-Key` TEM `revoke` (`seiva-identity/src/lib.rs:260`) e `exp` curto limita exposição.
**Correcção:** hook opcional `jti` deny-list + seam de refresh JWKS (ver E4).

### S4 🟢 Baixo — Sem gerador seguro de API-key (B-4) · Missing · Confirmado
`ApiKeyAuthenticator` (`seiva-identity/src/lib.rs:233`) guarda digests SHA-256 e faz lookup
deny-by-default, mas não oferece helper para **cunhar** a chave; a entropia é problema do
integrador. **Correcção:** `ApiKey::generate()` (≥128-bit CSPRNG, `sk_live_<base64url>`),
devolvendo a chave clara uma vez + o digest.

### S5 🟢 Baixo — Argon2id inline na thread async (B-5) · Partial · Confirmado
Ver [PERFORMANCE_REPORT P3](PERFORMANCE_REPORT.md). Vector de DoS CPU (API4); usar `spawn_blocking`.

### S6 ℹ️ Info — Notas menores
- JWT leeway é o default do `jsonwebtoken` (60s), nunca explícito (`jwt.rs:60-67`) — tornar knob
  documentado.
- `require_permission` (`guard.rs:148`) é match exacto de string (sem wildcard/hierarquia) — SEGURO
  (deny-by-default), mas impede grants `notes.*`. Nota de desenho, não defeito.

## Positivos verificados (sem acção)

- **Alg-confusion fechado (Correct).** `ALLOWED_ALGORITHMS = [RS256, ES256]` (`jwt.rs:29`); alg do
  header verificado contra allowlist **antes** da assinatura (`:131`), verificação fixada ao alg do
  header (`:141-142`), chave do JWKS por `kid`. `alg:none` e `HS*` recusados — provado por testes
  (`a_disallowed_algorithm_is_rejected`, tampered/wrong-aud/wrong-iss/expired/unknown-kid).
- **Tokens/API-key em repouso (Correct).** Só digest SHA-256 guardado e lookup por digest
  (`seiva-identity/src/lib.rs:160,207,284`); timing side-channel só vaza bytes pre-image-resistant.
  Deny-by-default; `revoke` funciona. (SHA-256 rápido é adequado **porque** são tokens opacos de
  alta entropia, não passwords.)
- **Password hashing (Correct) — a "armadilha password-hash" bem resolvida.** Argon2id, salt
  OS-CSPRNG por hash, PHC string, registo plugável `PasswordHashers` (estilo Django),
  upgrade-on-login via `needs_rehash`, `LoginThrottle` (OWASP API2) com tabela limitada a 100k que
  sobrevive a flood de usernames distintos (API4). Rehash/wrong-hash é erro de servidor, **não**
  conta contra o throttle. Sem pepper hardcoded nem params fracos.
- **Sem injecção SQL (Correct / N/A).** Os 3 `format!`+SQL
  (`seiva-outbox-relay/src/lib.rs:194,204,225,257`, `seiva-data-dorm:360`) interpolam só nomes de
  tabela `const` de compile-time; todo valor de runtime é parâmetro `$1..$n`. O insert de auditoria
  (`seiva-data-dorm:417`) é parametrizado. Nenhum input de utilizador chega a format string.
- **Headers seguros por default + CORS (Correct).** `security_headers` (`security.rs:122`) põe
  nosniff, `X-Frame-Options: DENY`, CSP `default-src 'none'; frame-ancestors 'none'`, HSTS 2a,
  `Referrer-Policy: no-referrer` em **toda** a resposta incl. 429 (teste de regressão
  `security.rs:194`). CORS é allowlist exacta; vazia ⇒ sem camada CORS. Sem reflexão wildcard.
- **BOLA fechado por tipo (Correct).** `Scoped<T>` (`guard.rs:83`) sem `into_inner`/`Deref`/campo
  público, `#[must_use]`: o único acesso é `owned_by(principal)` — um tenant-check esquecido é erro
  de **compilação**. Tenant vem do `Principal` autenticado, nunca do corpo (API1).
- **Isolamento de idempotency key (Correct).** `scoped_key` (`idempotency.rs:113`)
  length-prefixa `(tenant, subject, method, path, key)` num SHA-256 — sem replay cross-tenant.
- **Sem fuga de segredos (Correct).** Zero credenciais hardcoded; entradas de auditoria `redacted()`
  (password/api_key/secret/token/cookie/key mascarados); `401`/`AuthError` genéricos; o gateway DORM
  preserva o `Problem` tipado em vez de achatar para 500.
- **Webhook HMAC (Correct).** `ct_eq` em tempo constante, chave vazia rejeitada
  (`seiva-webhooks/src/lib.rs:316`).

## Postura geral

Zero-trust bem aplicado onde conta (BOLA por tipo, alg-confusion, deny-by-default). A dívida de
segurança é de **defaults e operação** (tenant-scope default, fail-open configurável, revogação
JWT, rotação JWKS), não de primitivos criptográficos ou superfície de injecção.
