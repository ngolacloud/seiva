# ADR 0015 — Rate limiting distribuído como porta opcional

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0012 (saúde & limites cloud-native)

## Contexto

O rate limiter do framework (ADR 0012) é **em-processo**: cada instância mantém os
seus próprios contadores de janela fixa. Isso é correcto para uma só instância,
mas atrás de um balanceador de carga com N réplicas um cliente pode obter até N ×
o limite espalhando pedidos pelas instâncias. O brief (§8) pede para documentar a
limitação por processo e **oferecer uma integração distribuída quando
necessária** — sem fazer a base depender de um backend específico, e sem fingir
que um cache é uma garantia de negócio durável (o rate limiting é amortecimento de
abuso, não estado de negócio).

## Decisão

**Uma porta `RateLimitStore` exprime um backend de rate-limit partilhado e
atómico; um middleware `rate_limit_with_store` impõe-no; uma crate opcional
`seiva-ratelimit-redis` implementa-o sobre Redis. A base mantém o seu limiter
em-processo como omissão.**

- `seiva_ports::RateLimitStore::check(key, limit, window) -> RateLimitDecision`
  (`Permitted` | `Limited { retry_after }`). O contrato exige que o backend
  **incremente-e-expire atomicamente** (uma ida-e-volta), de modo que réplicas
  concorrentes não possam correr para lá do limite.
- `seiva_http::rate_limit_with_store` reutiliza a extracção exacta da chave de
  cliente (IP do par, ou o `X-Forwarded-For` mais à direita apenas atrás de um
  único edge confiável) e a isenção de sondas de saúde do limiter em-processo, e
  devolve o mesmo `429` + `Retry-After`. Se o store der erro (uma indisponibilidade
  transitória) o pedido é **permitido** — uma indisponibilidade do limiter não
  pode deixar cair tráfego legítimo.
- `seiva-ratelimit-redis::RedisRateLimitStore` corre um único script Lua atómico
  (`INCR`; `PEXPIRE` na primeira ocorrência da janela; ler `PTTL`). É **opcional**
  e não uma dependência da base; uma implantação multi-réplica adere.
- `seiva_testing::InMemoryRateLimitStore` duplica a porta para testes de lógica.

### Prova

Unitário: `rate_limit_with_store` permite até ao limite e depois `429`, e isenta a
saúde, conduzido pelo store em memória. **Vivo** (`seiva-ratelimit-redis`,
condicionado a `SEIVA_REDIS_TEST_URL`) contra um Redis real: 50 clientes
concorrentes partilham um orçamento e exactamente `limit` são permitidos (os
restantes `Limited` com um retry-after positivo) — a garantia entre réplicas que o
limiter em-processo não consegue dar. Correu verde contra o Redis 7.

## Consequências

**Positivo.** Uma opção distribuída real, atómica entre réplicas, com a mesma
política de chave/isenção/resposta que o limiter em-processo; a base mantém-se
agnóstica ao backend por trás da porta; uma indisponibilidade do store falha
aberto.

**Negativo / custo.** Uma ida-e-volta ao Redis por pedido com rate-limit (um proxy
de edge dedicado pode mesmo assim ser preferível a RPS muito altos — orientação
inalterada). A janela fixa do Redis é por-cliente-por-janela, como a em-processo
(não um token bucket mais suave).

## Alternativas consideradas

- **Rate limiting em PostgreSQL.** Rejeitada: uma ida-e-volta à BD por pedido
  acopla o rate limiting à base de dados de negócio e é um encaixe pior do que o
  Redis para contadores de alta frequência; a §8 também avisa contra sobrecarregar
  a autoridade SQL.
- **Substituir o limiter em-processo.** Rejeitada: o em-processo é a omissão certa
  para uma só instância e está bem testado; o store distribuído é aditivo e
  opcional.
- **Fazer do Redis a autoridade para a idempotência também.** Rejeitada (ADR
  0011): a idempotência de negócio durável precisa de atomicidade com a mutação
  PostgreSQL + outbox, que o Redis não pode providenciar; o rate limiting não tem
  tal requisito.
