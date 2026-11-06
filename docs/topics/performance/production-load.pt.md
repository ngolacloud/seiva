# Benchmark de carga do caminho de produção

Este é o companheiro a que a [nota do perfil de build](llvm-profile.md) remete:
mantém o **caminho seguro ligado** — autenticação por token, uma leitura/escrita
com âmbito de inquilino através de **DORM sobre um PostgreSQL real**, cabeçalhos de
segurança na resposta e o limitador de taxa por-IP — e reporta percentis de
latência, throughput e códigos de estado. Um binário mais pequeno é uma propriedade
de build; isto mede o serviço em execução a fazer trabalho real.

O harness é [`benchmarks/production/run.sh`](../../../benchmarks/production/run.sh).
Constrói o serviço `admin-data-e2e` (o módulo CRUD `Catalog` do adendo da ADR 0009:
cinco operações sobre uma tabela `catalog_items` via `DormUnitOfWork`), corre **duas
réplicas como dois processos a partilhar um PostgreSQL** (o modelo de réplica de
estado partilhado) e conduz [`oha`](https://github.com/hatoo/oha) contra elas.

## O que foi medido

- Host: um só x86-64 de 20 núcleos, rede de loopback, PostgreSQL 16 sobre o socket
  Unix (peer auth). `oha 1.16.0`, 50 ligações, 10 s por caso.
- Serviço: perfil release (ThinLTO, `codegen-units = 1`, strip). Pool do DORM
  `pool_max = 8` por réplica. Autenticação **ligada** (`clerk-token` → um
  `Principal` no inquilino `acme`). Cada leitura tem âmbito de inquilino
  (`WHERE tenant = $1`) e mascara a coluna `cost` por permissão; cada escrita é um
  `INSERT ... RETURNING` real.

Dois cenários, ambos corridos tal como estão — nada suprimido:

### Cenário 1 — Protegido (a opção segura por omissão: 300 req/min por IP)

Uma enxurrada de 50 ligações contra uma réplica com o rate limit entregue
(`RateLimit::default` = 300 pedidos / 60 s, chaveado por IP do cliente). Toda a
carga vem de um IP, por isso, depois do orçamento de 300 pedidos, o limitador
descarta o resto:

| Caso | req/s | p50 | p95 | p99 | 2xx | 429 |
|---|---:|---:|---:|---:|---:|---:|
| `GET /v1/catalogs` | 255 955 | 0.16 ms | 0.42 ms | 0.74 ms | 298 | 2 563 826 |

O ponto **não** é o número de throughput — é que a protecção contra DoS rejeita uma
enxurrada em tempo sub-milissegundo (p99 0.74 ms) depois de admitir exactamente o
seu orçamento. O limitador é o trabalho aqui; a base de dados mal é tocada.

### Cenário 2 — Caminho de trabalho (rate limit elevado para não prender)

O mesmo caminho com o limitador erguido para fora do caminho
(`ADMIN_DATA_E2E_RATE_LIMIT`), para que a medição seja o **custo da auth + a
leitura/escrita na BD com âmbito de inquilino**, não o limitador:

| Caso | Réplica | req/s | p50 | p95 | p99 | 2xx |
|---|---|---:|---:|---:|---:|---:|
| `GET /v1/catalogs` (auth + scoped SELECT) | r1 | 48 156 | 1.00 ms | 1.35 ms | 1.74 ms | 481 639 |
| `GET /v1/catalogs` (auth + scoped SELECT) | r2 | 46 511 | 1.00 ms | 1.52 ms | 2.40 ms | 465 310 |
| `POST /v1/catalogs` (auth + INSERT) | r1 | 10 630 | 3.36 ms | 8.24 ms | 32.10 ms | 106 274 |

- **Leitura, agregada pelas duas réplicas: ≈ 94,7 k req/s** de leituras
  autenticadas, com âmbito de inquilino e suportadas por PostgreSQL, p99 abaixo de
  2,5 ms.
- **Escrita: ≈ 10,6 k req/s** de inserts autenticados que persistem no PostgreSQL;
  o p99 de escrita (32 ms) é dominado pelo commit do PostgreSQL + o pool de 8
  ligações sob contenção de 50 vias, exactamente onde deve aparecer.
- Um punhado de `errors` reportados por caso (≈ 50 = um por ligação) são os resets
  de ligação no encerramento, negligenciáveis contra centenas de milhares de
  pedidos.

### Memória

Uma réplica, medida a partir do `VmRSS` de `/proc/<pid>/status`:

| Estado | RSS |
|---|---:|
| Ocioso (pronto, sem tráfego) | 6 MiB |
| Pico sob carga de 50 ligações | 15 MiB |

## Ressalvas honestas — o que isto é e o que não é

- **Um só host, loopback.** Sem rede, sem distribuição de cliente real, sem salto
  de terminação TLS. Os números são indicativos do *custo do caminho nesta
  máquina*, não um SLA de capacidade. Volta a medir no teu alvo.
- **Dois processos modelam duas réplicas.** Partilham genuinamente um PostgreSQL (o
  verdadeiro ponto de contenção), mas não há balanceador de carga nem latência
  entre hosts.
- **Rate limit em-processo aqui, não o distribuído.** Este binário usa o limitador
  de janela fixa por-processo. O limitador Redis distribuído
  (`seiva-ratelimit-redis`, ADR 0014) é o que faz o orçamento por-IP aguentar
  *entre* réplicas; está provado pelo seu próprio teste ao vivo, não exercitado
  nesta corrida.
- **A idempotência não está no caminho quente deste serviço.** O armazenamento
  durável de idempotência (ADR 0012) está provado pelo teste ao vivo multi-processo
  do `seiva-data-dorm`; este caminho de escrita do `Catalog` não anexa uma
  `Idempotency-Key`, por isso estes números de escrita são o custo cru do INSERT,
  não o custo de escrita de-duplicada.
- **Isto não é o microbenchmark de overhead.** A nota do perfil de build desliga
  deliberadamente a auth para isolar o overhead do framework; esta mantém todo o
  caminho seguro ligado. Respondem a perguntas diferentes — não compares os seus
  números de throughput directamente.

## Reproduzir

```sh
createdb seiva_prod_bench   # once
PROD_BENCH_DURATION=10s PROD_BENCH_CONNECTIONS=50 bash benchmarks/production/run.sh
# raw oha JSON lands in benchmarks/production/results/
```

## Multi-réplica containerizado e balanceado (via `delonix`)

O modelo de dois-processos-numa-máquina acima partilha uma base de dados mas não
tem balanceador de carga nem orçamento de taxa *partilhado* — o limitador
em-processo conta por processo, por isso duas réplicas atrás de um balanceador
round-robin admitiriam cada uma um orçamento completo (≈2× o tecto pretendido). O
deployment containerizado fecha exactamente essa lacuna.

[`benchmarks/production/delonix/`](../../../benchmarks/production/delonix/) levanta,
com **`delonix compose up`** (sem daemon Docker), uma pilha de cinco serviços:
PostgreSQL 16 + Redis 7 + **duas** réplicas `admin-data-e2e` + um balanceador de
carga **HAProxy** (`option forwardfor`, round-robin, verificações `/health/ready`).
Cada réplica corre o limitador de taxa Redis **distribuído** (ADR 0015) chaveado no
IP de cliente reencaminhado, por isso o orçamento por-cliente é contado num único
armazenamento partilhado.

### Prova A — UM orçamento de taxa entre réplicas

Com o limite por-cliente definido em **120 req/min** e um burst de 4000 pedidos
conduzido através do balanceador de carga (ambas as réplicas a servi-lo
round-robin):

| Admitidos (2xx) | Rejeitados (429) | Esperado se NÃO partilhado |
|---:|---:|---:|
| **119** | 3881 | ~240 (2× orçamento) |

As duas réplicas admitiram **119** — um orçamento, não dois. Um limitador
por-processo atrás do mesmo balanceador teria admitido ≈240. Esta é a propriedade
que a corrida num só host não conseguia demonstrar, e é imposta pelo contador Redis
partilhado (`INCR`/`PEXPIRE`, uma ida atómica ao servidor por pedido).

### Prova C — idempotência durável entre réplicas

Com uma pilha fresca, **40 POSTs idênticos concorrentes** que carregam a **mesma
`Idempotency-Key`** são disparados através do balanceador de carga (a aterrar em
ambas as réplicas round-robin). O armazenamento durável (ADR 0011, um CAS
`INSERT … ON CONFLICT DO NOTHING` reivindicado dentro da transacção de escrita)
admite exactamente um:

| POSTs idênticos concorrentes (mesma chave) | Linhas criadas |
|---:|---:|
| 40 | **1** |

Uma repetição reproduz a resposta armazenada; uma chave reutilizada com conteúdo
*diferente* é um `409` (provado pelo teste `live_catalog_create_is_idempotent`). A
reivindicação faz commit na mesma transacção que o insert, por isso um rollback
des-reivindica a chave — uma chave com commit tem sempre o seu efeito, entre
réplicas.

### Prova B — throughput através do balanceador de carga containerizado

Limite erguido para fora do caminho; tráfego conduzido no frontend do HAProxy (que
distribui para ambas as réplicas):

| Caso | req/s | p50 | p95 | p99 | 2xx |
|---|---:|---:|---:|---:|---:|
| `GET /v1/catalogs` | 9 415 | 1.36 ms | 4.43 ms | 72.50 ms | 94 125 |
| `GET /v1/catalogs` (2ª corrida) | 9 525 | 1.46 ms | 5.95 ms | 72.35 ms | 95 211 |
| `POST /v1/catalogs` | 6 527 | 2.26 ms | 68.11 ms | 72.00 ms | 65 236 |

Estes são **throughput mais baixo e latência de cauda mais alta do que os números
do host acima**, e isso é esperado e honesto: cada pedido atravessa agora a ponte
do container, afunila por um único ponto de ligação HAProxy **e** paga uma ida ao
Redis para o limitador distribuído. Esse é o custo de um *orçamento partilhado
correcto entre réplicas* — o deployment troca latência pela garantia que a Prova A
demonstra. Os números do host isolam o custo do caminho da BD; estes isolam o do
caminho orquestrado.

### Reproduzir

```sh
bash benchmarks/production/delonix/run.sh   # builds the binary, delonix compose up, proves A+B, tears down
```

### Idempotência no caminho quente de escrita — feito

A escrita do `Catalog` honra agora uma `Idempotency-Key` de ponta a ponta (ADR
0011): a junção `&Ctx` carrega a chave validada + o método/caminho do pedido, o
handler dobra uma chave com âmbito + uma impressão digital canónica do corpo, e
reivindica-a através do armazenamento durável **dentro da transacção de escrita**.
Numa reivindicação fresca insere e armazena a resposta; uma repetição reproduz-na;
uma chave reutilizada com conteúdo diferente é um `409`. A Prova C acima mostra que
isto se aguenta entre réplicas; o teste `live_catalog_create_is_idempotent` prova a
repetição + o `409` contra um PostgreSQL real.

### Controlos da consola + concorrência optimista — feito

A lista da consola tem agora **pesquisa** conduzida pelo servidor, **ordenação**
(cabeçalhos de coluna clicáveis) e **paginação**, a partir das allow-lists
registadas (ADR 0009), e **concorrência optimista** na edição: as linhas carregam
uma `version`, uma edição obsoleta é um `409`. Provado por testes ao vivo e pelo
E2E de browser (`e2e/admin/catalog.mjs`).
