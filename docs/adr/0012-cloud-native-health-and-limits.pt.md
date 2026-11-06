# ADR 0012 — Sondas de saúde cloud-native e postura de rate-limit

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0001

## Contexto

O brief (§8) fixa regras operacionais que um backend tem de honrar sob um
orquestrador e um balanceador de carga:

- A **prontidão** (readiness) reflecte a inicialização e as *dependências
  críticas*, com verificações que são **limitadas/em cache** para que uma enxurrada
  de sondas não martele a dependência.
- A **vivacidade** (liveness) não pode desencadear tempestades de reinício: uma
  falha transitória de dependência não pode fazer o orquestrador matar um processo
  saudável.
- As **sondas de saúde não podem partilhar o orçamento de rate-limit dos
  clientes** — um balanceador de carga sonda `/health/*` constantemente, e muitos
  clientes podem sentar-se atrás de um IP de origem (um NAT, o próprio proxy).

O framework já tinha `/health/live` (sempre `200`) e `/health/ready` (`200` só
enquanto `Ready`), mas ambos estavam *dentro* do rate limiter em
`apply_framework_layers`, e a prontidão não conseguia reflectir uma dependência.

## Decisão

- **As sondas de saúde estão isentas do rate limiter.** O middleware `rate_limit`
  salta `/health/live` e `/health/ready` antes de indexar no cliente, de modo que
  o tráfego de sondas (ou muitos clientes atrás de um IP) nunca possa disparar um
  `429` e fazer a instância ser marcada como não saudável.
- **A prontidão pode levar uma verificação de dependência crítica em cache.**
  `health_router_with_dependency(health, check, ttl)` recebe um `ReadinessCheck`
  (uma closure que devolve um future em caixa). `/health/ready` é `200` só
  enquanto `Ready` **e** a verificação passa; o resultado é cacheado por `ttl`,
  por isso a verificação corre no máximo uma vez por janela por mais vezes que a
  sonda seja sondada. O `health_router` por omissão mantém o comportamento só de
  ciclo de vida.
- **A vivacidade nunca consulta uma dependência.** `/health/live` indexa só no
  estado do ciclo de vida, por isso um soluço transitório da base de dados faz a
  prontidão `503` (retirada de rotação) enquanto a vivacidade fica `200` (não
  reiniciada) — sem tempestade de reinício.
- **O rate limiter é por processo, e di-lo.** É um limiter de janela fixa, em
  memória, indexado por IP do cliente (a entrada `X-Forwarded-For` mais à direita
  confiável apenas quando um único edge confiável está configurado, nunca um
  booleano cego). Entre réplicas, cada processo impõe o seu próprio orçamento; um
  limite globalmente consistente precisa de um backend partilhado, documentado
  como uma integração posterior.

## Consequências

**Positivo.** As sondas sobrevivem a enxurradas e a clientes com NAT; a prontidão
rastreia uma dependência real sem a martelar; um soluço de dependência nunca
reinicia um processo saudável; a regra de confiança no proxy resiste a
falsificação de `X-Forwarded-For`.

**Negativo / custo.** O limiter por processo não é uma quota global entre réplicas
(aceitável para amortecer abuso; um backend distribuído é trabalho futuro). O
cache de prontidão significa que uma recuperação de dependência só é visível ao
fim de no máximo `ttl`.

## Alternativas consideradas

- **Montar a saúde fora de todas as camadas.** Rejeitada: a saúde deve mesmo assim
  levar os cabeçalhos de segurança e o id de correlação; só o *rate limit* (e o
  limite de corpo, irrelevante para um GET) tem de ser saltado — uma isenção de
  caminho dirigida é mais simples do que dividir toda a pilha de camadas.
- **A prontidão sonda sempre a dependência (sem cache).** Rejeitada: uma enxurrada
  de sondas tornar-se-ia uma enxurrada de dependência (§8).
- **A vivacidade sonda a dependência também.** Rejeitada categoricamente: é a
  clássica causa de tempestade de reinício que o brief nomeia.
