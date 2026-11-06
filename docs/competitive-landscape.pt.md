# Panorama competitivo — onde o Seiva (e o DORM) ganham, e onde temos de melhorar

Um cartão de pontuação honesto do Seiva contra o **FastAPI** (+ fastapi-cli) e o
**Django**, e do **DORM** contra o **Prisma**, com as lacunas que temos de fechar
para entregar valor e competir. Escrito para ser usado, não para adular.

## Posicionamento numa linha

> **A experiência de programador do FastAPI + a completude do Django — como um
> binário distroless único, compilado e sem GC.** O FastAPI é uma biblioteca de
> routing+validação; o Django tem baterias incluídas mas em Python; o Seiva aspira
> a ter baterias incluídas *e* a ser nativo.

## Cartão de pontuação

Legenda: ✅ forte · 🟡 parcial/em curso · ✖ em falta.

Legenda para a coluna Seiva: um ⬆︎ marca uma célula que se moveu na campanha P0–P1
(vê o [delta de progresso](#progress-delta) abaixo para a prova).

| Capacidade | FastAPI | Django | **Seiva** | DORM / Prisma |
| --- | --- | --- | --- | --- |
| OpenAPI auto + UI de docs ao vivo | ✅ | ✖ | ✅ fiel, **ciente do projecto** ⬆︎ | — |
| Validação dirigida por tipo/esquema | ✅ Pydantic | 🟡 formulários | ✅ `serde` + `Validate` | — |
| Fidelidade pedido→contrato | ✅ | n/a | ✅ `WireSchema` (sem envelope/placeholder) | — |
| Assíncrono, alto débito | ✅ (GC) | 🟡 (GIL/GC) | ✅ **sem GC, Rust** | — |
| Injecção de dependências / portas | ✅ `Depends` | 🟡 | ✅ portas por trait | — |
| CLI de dev (`dev`/reload) | ✅ fastapi-cli | ✅ runserver | ✅ `seiva start --watch` ⬆︎ | — |
| Esqueletização de projecto/app | ✖ | ✅ | ✅ `new` + **auto-wire** ⬆︎ | — |
| ORM / camada de dados | ✖ | ✅ | ✅ round-trip DORM provado ⬆︎ | ✅ Prisma |
| Migrações | ✖ | ✅ | ✅ `seiva db init/dev/migrate` ⬆︎ | ✅ migrate |
| Auth embutida (JWT/OIDC verificado) | 🟡 primitivos | ✅ sessões | ✅ negação por omissão, verificado | — |
| Autorização ao nível do objecto (BOLA) | ✖ | 🟡 | ✅ `Scoped<T>` em tempo de compilação | — |
| Segurança por omissão | 🟡 | ✅ | ✅ headers/authn/rate-limit/OWASP | — |
| GUI de admin | ✖ | ✅ **ex-líbris** | 🟡 consola de operações `/admin` (opt-in `seiva-console-admin`) + `seiva db studio` | 🟡 Studio (GUI de dados) |
| DSL de esquema + studio | — | — | ✅ sync de esquema + `db studio` ⬆︎ | ✅ Prisma |
| Introspecção de BD (brownfield) | ✖ | 🟡 inspectdb | ✅ `seiva db pull` ⬆︎ | ✅ `db pull` |
| Multi-base-de-dados | via SQLAlchemy | ✅ | ✅ PostgreSQL + CockroachDB + MySQL ao vivo (`--backend`) ⬆︎ | ✅ incl. Mongo |
| Implantação binário único / distroless | ✖ | ✖ | ✅ **embutido** | ✅ (vs Prisma npm+codegen) |
| Maturidade de hot reload / DX de editor | ✅ | ✅ | 🟡 hot-reload feito; extensão de editor + tempos de compilação em curso | — |
| Docs (Diátaxis) | ✅ excelente | ✅ **padrão de ouro** | 🟡 a estrutura espelha o Django; conteúdo em curso | ✅ |
| Ecossistema / mindshare | ✅ enorme | ✅ enorme | ✖ **nascente** | ✅ enorme |

## Delta de progresso {#progress-delta}

A campanha P0–P1 que se seguiu ao primeiro cartão de pontuação. Cada célula ⬆︎
acima aponta para uma fatia aqui, e cada fatia fechou apenas contra um teste de
fitness ou uma demonstração ao vivo reproduzível (por isso os movimentos são
ganhos, não asseverados). Os ramos estão empilhados e não empurrados; o dono
integra.

| Lacuna | O que aterrou | Prova | Commit |
| --- | --- | --- | --- |
| Compilação standalone | o projecto gerado compila via `[patch.crates-io]` interino; o main usa `router_with_modules` | o fitness compila um projecto gerado + módulo | `d145961` |
| Fidelidade do contrato | trait+derive+inventário `WireSchema`; corpo = DTO directamente, esquemas reais | `seiva openapi` → `{title}`/`{id,title}`, 0 placeholders | `feceb67` (ADR 0007) |
| Reload da CLI de dev | `seiva start --watch` / `--no-watch` | `resolve_reload` testado unitariamente | `561b437` |
| Descoberta de fachadas | `seiva module new` faz auto-wire; `seiva openapi` documenta o projecto | o fitness corre o binário → `/v1/widgets` | `a5ab52e` |
| Camada de dados | gateway DORM sobre deps de crate publicado; round-trip de módulo | `Accounts::create_user` ao vivo → persist+outbox+relay vs PG | `dbdd93a`, `06dd60b` |
| Introspecção brownfield | `seiva db pull` (delega `dorm db pull`) | ao vivo → `model CatalogItem` fiel a partir de uma tabela PG | `f203cba` |
| Migrações | `seiva db init/dev/migrate`; esquema de módulo → migração | ao vivo → `public.records` a partir do modelo `notes` | `006b9a8`, `82a7418` |
| GUI de admin | consola de operações `/admin` (a partir do contrato), opt-in `seiva-console-admin` (ADR 0008; a base dá 404) + `seiva db studio` (dados) | consola montada → `/admin` 200 (CSP por hash); base → `/admin` 404; `db studio` serviu linhas | `2da7c64`, `c5fe05f` (+ DORM `552eaaf`), ADR 0008 |
| Proto ciente do projecto | `seiva proto` documenta o projecto (SEIVA_EMIT=proto) | fitness: o binário gerado emite `service Widgets` | `09c3772` |
| Multi-base-de-dados | `seiva db init --backend mysql\|sqlite\|postgres` (costura `Driver` do DORM) | ao vivo: `records` criadas em **MySQL 8** e **CockroachDB** v26.3, mesmo modelo | `ef32082` |

**Em falta** (a fronteira honesta): **P0-4** publicar os crates (só-dono) — larga os
patches interinos; **mais backends** — PostgreSQL, CockroachDB e MySQL estão ao vivo
(`--backend`); SQL Server, Oracle e MongoDB aguardam que o DORM adicione cada
`Driver` (ADR 0011), após o que o Seiva os alcança através do mesmo `--backend`/DSN
sem alteração; **P2** — conteúdo de docs restante, uma extensão para VS Code, e um
benchmark publicado.

## Onde somos genuinamente melhores

1. **Desempenho nativo, sem GC, um binário.** Contra ambas as frameworks Python
   esta é a vitória estrutural: maior débito, menor latência de cauda, sem
   interpretador/GIL, e uma imagem distroless, não-root e sem shell pronta a usar
   (a disciplina `seiva-cloud-native-check`). Implantar é `COPY um-binário`, não
   "entregar um runtime Python/Node + deps". Isto é **medido, não asserido** — vê
   [Benchmarks](benchmarks.md): numa carga trivial sem BD o Seiva serviu ~3–5× o
   débito do FastAPI e ~15× o do Django, com p99 sub-milissegundo, na máquina de
   referência (reproduz com `benchmarks/run.sh`; os números dependem do ambiente).
2. **Fidelidade de contrato por construção.** O documento OpenAPI é *gerado a partir
   do mesmo contrato que o servidor impõe*, com esquemas reais (`WireSchema`, ADR
   0007) — sem desvio entre docs e realidade. O FastAPI consegue isto a partir do
   Pydantic; nós igualamo-lo e estendemo-lo a permissões (`x-seiva-requires`) e a
   inventário em tempo de link (cada fachada montada é listada — OWASP API9).
3. **Segurança que o sistema de tipos impõe.** JWT/OIDC verificado com negação por
   omissão (só RS256/ES256, nunca `alg:none`), e **autorização ao nível do objecto
   como preocupação de tempo de compilação** (`Scoped<T>`). O Django é seguro por
   omissão em runtime; o Seiva empurra garantias-chave para tempo de compilação.
4. **Arquitectura que não consegues violar.** A regra de dependência de cinco
   camadas é o grafo de crates; as fitness functions falham o build nas violações.
   Esta é uma história de correcção e manutenibilidade que nem o FastAPI nem o
   Django impõem.
5. **A tese de Rust-nativo do DORM é validada pela retirada do Prisma.** O Prisma
   passou anos num motor de consultas em Rust e, no Prisma 7 (Nov 2025), removeu-o —
   porque a **fronteira FFI Rust↔JS** era o estrangulamento. O DORM é Rust nativo de
   ponta a ponta: sem FFI, sem GC, segurança de tipos em tempo de compilação até ao
   binário. Devíamos dizer isto em voz alta.

## Onde temos de melhorar (priorizado)

> O plano para fechar todas as lacunas abaixo — com critérios de pronto
> verificáveis — é o [roadmap](roadmap.md).

**P0 — mínimos para sermos levados a sério vs FastAPI/Django:**

1. **Entregar a camada de dados (cutover DORM).** "ORM opcional, cutover em curso" é
   a maior lacuna de credibilidade vs Django e a principal razão para escolher uma
   *framework*. Termina o gateway `dorm`, a UX de migrações, e um caminho "modelo →
   repositório" de primeira classe. *Este é o desbloqueio de valor #1.*
2. **Ciclo de dev com hot-reload.** `fastapi dev` / `runserver` definem a fasquia:
   editar-e-ver. O `seiva start` precisa de watch+rebuild (e builds incrementais
   rápidos). Os tempos de compilação Rust são o imposto de DX; investe nisso
   (integração cargo-watch, um reload fino).
3. **Descoberta de fachadas em runtime para `seiva openapi`/`proto`.** Hoje a CLI
   documenta apenas a fachada de demonstração embutida; tem de fazer introspecção do
   projecto do utilizador para que o contrato gerado seja o do projecto.
4. **Publicar os crates no crates.io.** A compilação standalone funciona via um
   `[patch.crates-io]` interino; a adopção real precisa de `cargo add seiva-*`. (Vê
   [publicação](publishing.md).)

**P1 — as "baterias" que definem a categoria Django:**

5. **Uma superfície de admin / auto-CRUD.** O ex-líbris do Django; nenhum
   concorrente o iguala. Mesmo uma API de leitura/actualização gerada + uma UI web
   mínima a partir dos descritores de fachada seria um diferenciador (já temos os
   descritores).
6. **Introspecção de BD (brownfield).** A geração estilo `db pull` baixa
   enormemente a fricção de adopção — crítica para equipas com um esquema existente.
7. **Maturidade de migrações** — detecção de desvio, reversibilidade, um ciclo de
   dev com shadow-DB (o `migrate dev` do Prisma é a referência).
8. **Amplitude multi-base-de-dados** para além do PostgreSQL (pelo menos MySQL; o
   roadmap existe).

**P2 — alcance e confiança:**

9. **Conteúdo de documentação.** A *estrutura* agora espelha o Django (esta árvore);
   a fasquia de padrão-de-ouro são tutoriais + referência *preenchidos*. Continua a
   escrever; é a alavanca de adopção mais barata.
10. **DX de editor/ferramentas** — uma história de extensão para VS Code (esquema +
    autocompletamento de fachada) para rivalizar com as do Prisma e do FastAPI.
11. **Ecossistema & exemplos** — templates do mundo real (o template "full-stack"
    que puxou a adopção do FastAPI), integrações, um benchmark que publicamos.
12. **Polimento da qualidade do scaffold** — `Vec<u8>`/`format:byte` nos esquemas,
    fidelidade de proto (seguimento da ADR 0007), e a checklist de "primeiro dia"
    `seiva-dx-parity`.

## As três apostas a pressionar no go-to-market

1. **"Nativo, não interpretado."** Visa equipas em FastAPI/Django a bater em muros
   de desempenho, latência de cauda ou custo de cloud. Lidera com a história de
   binário único, sem GC, distroless + um benchmark publicado.
2. **"Contratos honestos e impostos."** Vende correcção: os docs coincidem com o
   servidor, a autorização é em tempo de compilação, a arquitectura não apodrece.
   Este é um argumento de CTO/staff que as frameworks Python não conseguem fazer.
3. **"DORM: o ORM que o Prisma gostava de ter conseguido entregar."** Rust nativo,
   sem fronteira FFI — exactamente a coisa que o Prisma removeu. Uma camada de dados
   autónoma que também se aguenta sozinha.

---

*Vê também [Seiva num relance](getting-started/seiva-at-a-glance.md) e o
[registo de engenharia](progress.md) para o estado actual honesto. Esta análise
baseia-se no FastAPI, fastapi-cli, Prisma ORM (incl. o lançamento sem Rust do Prisma
7, Nov 2025) e Django tal como em 2026-11.*
