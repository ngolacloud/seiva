# Campanha de evolução — evidência por SHA, matrizes e lacunas honestas

A evolução do Seiva num framework de backend cloud-native com a consola de
administração como extensão (o brief de seis fases do dono). Baseline: `main` em
`aed02d1f`. O trabalho aterrou no ramo **`fix/ci-green-baseline`** (não empurrado
— decisão de PR do dono). Cada commit foi verificado localmente; as provas
acopladas ao DORM e com base de dados ao vivo correram contra um PostgreSQL 16
real.

## Commits (por ordem)

| SHA | Fase | O quê | Prova |
|---|---|---|---|
| `3800bbd` | F1 | CI verde: `Problem` → `Box<str>` (128→88 B), allows globais de `result_large_err` removidos (portão restaurado), `fmt` do gateway DORM | 5 jobs de CI reproduzidos verdes localmente |
| `c82cf67` | F2 | Administração extraída para a opcional `seiva-console-admin`; base `/admin`→404; CSP baseado em hash (sem `unsafe-inline`) — ADR 0008 | teste de 404 na base; `cargo tree -p seiva-server` = 0 ocorrências; o hash da CSP corresponde aos bytes servidos |
| `2322ca1` | F3a | Registo de `AdminResource` por módulo; a consola mostra apenas recursos registados **e** autorizados — ADR 0009 | unitário + integração (registo filtrado por Principal); duplicado→conflito |
| `240816a` | F3b | `seiva module new --admin` faz scaffold do registo | teste de scaffold; `--admin` presente/ausente |
| `dc2a509` | F3c | `seiva ext add` + contrato de integração declarativo `seiva-extension.toml` — ADR 0010 | scaffold ao vivo→`ext add` idempotente; recurso a patch manual |
| `f28f666` | F3 fix | Revisão de segurança (OWASP): montar extensões DENTRO das camadas do framework para que `/admin` receba autenticação/cabeçalhos/rate-limit | teste de ponta-a-ponta: o `Principal` flui pelas camadas até ao registo |
| `156d6e7` | F4 | Porta `IdempotencyStore` durável + adaptador DORM (reutiliza `dorm_core::idempotency`) — mesma-tx com mutação+outbox — ADR 0011 | **PG ao vivo**: 20 ligações concorrentes → 1 efeito; Mismatch; bytes de Replay |
| `679f7ac` | F4 | Extractor HTTP de `Idempotency-Key` + chave com âmbito + impressão digital canónica + mapeamento 409 | unitário + ponta-a-ponta (Fresh uma vez / Replay / 409 / 400) |
| `b7f6755` | F4 | Dedup da inbox do consumidor (`inbox_key`) + worker `purge_expired` com TTL (`run_idempotency_purge`) | **PG ao vivo**: reentrega→1 consumo; ttl=0 purgado→reclamável |
| `5505053` | F5 | Sondas de saúde isentas do rate limiter; dependência de prontidão em cache; a vivacidade ignora dependências — ADR 0012 | inundação de sondas mantém-se 200; dep em baixo→503 ready/200 live; cache sondada uma vez/TTL |
| `d7c9c27` | F5 | Auditoria persistente de mutações de administração (`AuditSink`) com redacção de segredos; sink DORM — ADR 0013 | unitário (metadados, negados-também, leituras-saltadas, redacção) + **PG ao vivo** (password `[redacted]`, segredo nunca guardado) |
| `b0737fa` | F5 | Enums-unitárias WireSchema → string enum; testes de fidelidade (aninhado/array/bytes/Option); respostas de erro Problem no OpenAPI (401/403/422) — adenda ao ADR 0007 | testes de derive + openapi |
| `7c37b67` | §9 | Advisories de `cargo-deny` para os lockfiles excluídos e acoplados ao DORM; 3 advisories reais trazidos à superfície + seguidos — ADR 0014 | `cargo deny` verde para todos os 5 crates excluídos com excepções fundamentadas e datadas |
| `5828149` | §9 | Perfil de release LLVM afinado (ThinLTO, `codegen-units=1`, strip), sem `target-cpu=native` | medido: `seiva` 6334→1680 KiB (−73%), build +31% |

Novos ADRs: **0008–0015** (+ a adenda ao 0007). Commits posteriores (0015
rate-limit distribuído, CRUD de dados de administração, UI de CRUD por linha na
consola) continuam no ramo; ver `git log`. Indexados em
[ADRs](adrs.md).

## Matriz de critérios de aceitação

| Requisito (§10) | Onde | Estado | Evidência |
|---|---|---|---|
| Backend faz build/arranca/testa com Cargo, sem DX/administração | workspace | ✅ | `cargo test --workspace` verde; sem DX no grafo |
| `/admin` → 404 sem extensão; sem administração no `cargo tree` | `seiva-http`, `seiva-server` | ✅ | teste de 404 na base; árvore = 0 ocorrências |
| A instalação de extensões é explícita, sem hooks, precisa de recompilar | `seiva ext add`, ADR 0010 | ✅ | prova ao vivo; manifesto declarativo, sem build.rs |
| Instalação repetida não duplica nada | `add_extension_*`/`mount_*` | ✅ | testes de idempotência |
| Módulo `--admin` → registo listado/autorizado | ADR 0009 | 🟡 | registo + registo filtrado por autorização feitos; um módulo CRUD suportado por DORM (`apps/admin-data-e2e`, `Catalog`) prova as cinco operações ao vivo sobre PG; a UI de CRUD por linha da consola (tabela de listagem + criar + editar + eliminar) está construída e provada num browser (e2e/admin/catalog.mjs); o âmbito por inquilino + mascaramento de campos são impostos do lado do servidor através da costura `&Ctx` (provado ao vivo) |
| Módulo sem registo não é exposto | portão do registo | ✅ | ausente do registo → ausente da consola |
| Outro inquilino / sem permissão → sem acesso | filtro do registo + `Scoped<T>` (ADR 0005) + âmbito por inquilino `Ctx` (ADR 0009) | ✅ | o registo esconde os não autorizados; guarda BOLA na API; operações de `Catalog` com âmbito por inquilino + `cost` mascarado, provado ao vivo |
| Pedido de administração repetido → um efeito entre réplicas | idempotência (ADR 0011) | ✅ | prova em **PG ao vivo** com 20 ligações |
| Núcleo coerente após a extracção | docs/testes | ✅ | testes de base + docs alinhadas |
| Segurança: auditoria de todas as variantes de lockfile + testes negativos | ADR 0014, portões de deny | 🟡 | portão de advisories estendido aos lockfiles excluídos; licença/bans nos crates excluídos adiados |
| Qualidade: fmt, clippy, unitário/integração, doctests | CI | ✅ | tudo verde em cada commit |
| E2E + carga com limiares | `e2e/admin` (browser), `benchmarks/production` | ✅ | o E2E de browser de administração passa em Chrome headless (autenticar → listar autorizados → criar, sem fuga de segredos); CRUD por linha provado no browser sobre o módulo Catalog ao vivo; **benchmark de carga de produção sobre PostgreSQL** (caminho seguro, duas réplicas): ~94,7k req/s em leituras autenticadas+com âmbito por inquilino (p99 <2,5ms), ~10,6k req/s em inserções autenticadas (p99 32ms), descarte-429 a p99 0,74ms, RSS 6→15 MiB — `docs/topics/performance/production-load.md`. E2E de filtro/conflito-de-versão ainda pendente |

## Matriz de compatibilidade

| Componente | Versão / estado | Depende de | Notas |
|---|---|---|---|
| Base Seiva (`seiva-*`, `delonix-seiva`) | 0.1.0 | — (DORM só por trás da feature `dorm`) | agnóstica de DB; só portas |
| `seiva-console-admin` | 0.1.0 (extensão por adesão) | API pública da base Seiva (`axum`, `delonix-seiva::admin`, `seiva-identity`) | num só sentido; a base nunca depende dela (ADR 0008) |
| DORM | `dorm-core 0.6`, `dorm-outbox 0.1` (git/path, pré-publicação) | — | por trás da feature `dorm` / crates excluídos; os adaptadores de idempotência + auditoria vivem aqui |
| DX | ferramenta externa (`projects/dx`) | lê `seiva-extension.toml` (ADR 0010) | não exigida em compilação/runtime; o equivalente Cargo é `seiva ext add` |

## Lacunas honestas (remanescentes)

- **Cauda da F5.** Rate limiter distribuído — **feito** (ADR 0015): porta
  `RateLimitStore` + `rate_limit_with_store` + `seiva-ratelimit-redis` opcional,
  provado ao vivo contra Redis 7 (50 clientes concorrentes partilham um orçamento).
  O âmbito por inquilino do lado do servidor e o mascaramento de campos estão
  feitos através de uma costura `&Ctx` completa (o controlador injecta o
  Principal): as operações de `Catalog` fazem âmbito por inquilino e mascaram
  `cost` a menos que `catalog.cost.view` — provado ao vivo (isolamento de
  inquilino + máscara).
- **Cauda da F6.** E2E de browser da administração — **feito** (Playwright em
  Chrome do sistema): o fluxo de administração (`e2e/admin/admin.mjs`) e o ciclo
  completo de CRUD por linha sobre o módulo Catalog ao vivo (`e2e/admin/catalog.mjs`).
  Benchmark de carga de produção sobre PostgreSQL — **feito**
  (`benchmarks/production/run.sh`, `docs/topics/performance/production-load.md`):
  o caminho seguro (autenticação + DB com âmbito por inquilino) através de duas
  réplicas medido a ~94,7k req/s de leituras / ~10,6k req/s de inserções com
  p50/p95/p99 + códigos de estado + RSS reportados, e o limiter por-IP a descartar
  uma inundação como 429s em sub-ms. Ressalvas honestas enunciadas: host/loopback
  único, dois processos modelam as réplicas, limiter em-processo aqui (não o
  distribuído em Redis), idempotência fora do caminho quente desta escrita. O
  deploy totalmente conteinerizado, com balanceamento de carga e multi-réplica via
  `delonix` — **feito** (`benchmarks/production/delonix/`): `delonix compose up`
  corre PostgreSQL + Redis + duas réplicas de Catalog + HAProxy, e prova que o
  orçamento de rate por cliente é UM entre réplicas (limite 120/min, rajada de
  4000-req através do LB → 119 admitidos, não ~240) via o limiter distribuído em
  Redis, mais o throughput através do LB conteinerizado. **Idempotência no caminho
  quente de escrita — feito** (adenda ao ADR 0011): a costura `&Ctx` carrega a
  `Idempotency-Key` validada + método/caminho, e `Catalog.create` reclama uma
  chave com âmbito + impressão digital canónica do corpo através do store durável
  dentro da transacção de escrita — uma repetição reproduz, uma chave reutilizada
  com conteúdo diferente é `409`. A Prova C do deploy delonix dispara 40 POSTs
  idênticos concorrentes (uma chave) através do LB sobre duas réplicas e obtém
  exactamente UMA inserção; o teste `live_catalog_create_is_idempotent` prova a
  repetição + `409`. Os números publicados do perfil de build continuam a ser
  apenas de overhead e estão rotulados como tal.
- **gRPC** é apenas geração de proto; nenhuma viagem de ida-e-volta servida é
  reivindicada, por isso nenhuma é afirmada (adenda ao ADR 0007).
- **Consola de administração** faz agora **CRUD por linha** registado e autorizado
  (tabela de listagem + criar + editar + eliminar) sobre as operações de contrato
  de um módulo, provado no browser contra um módulo suportado por DORM ao vivo
  (`Catalog`). Impõe leituras com âmbito por inquilino e mascaramento de campos do
  lado do servidor (a costura `&Ctx`); **pesquisa, ordenação por cabeçalho
  clicável e paginação** (a partir das allow-lists registadas
  `search_fields`/`ordering`/`page_size`, validadas no servidor); e
  **concorrência optimista** (as linhas carregam uma `version`; uma edição
  obsoleta é um `409`). Provado por testes ao vivo
  (`live_catalog_list_search_order_paginate`, a afirmação de conflito de versão) e
  pelo E2E de browser (`e2e/admin/catalog.mjs`: a pesquisa filtra a tabela, um
  incremento concorrente seguido de um guardar obsoleto mostra o `409`). Âmbito
  futuro (não uma lacuna aberta no brief): acções em lote, widgets de filtro mais
  ricos.

Nada do acima é uma lacuna silenciosa: cada uma é enunciada aqui e no ADR
relevante.
