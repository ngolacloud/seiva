# Roadmap — fechar as lacunas P0–P2

Este plano transforma as lacunas do [panorama competitivo](competitive-landscape.md)
em workstreams concretos e **verificáveis**. A regra: uma lacuna está *fechada*
apenas quando a sua célula no cartão de pontuação passa de 🟡/✖ a ✅, sustentada por
uma fitness function ou uma demonstração reproduzível — para que a
[re-comparação](#re-running-the-comparison) no fim seja objectiva, não uma questão
de opinião.

O esforço é dimensionado por tamanho de t-shirt (S ≈ um dia, M ≈ alguns dias,
L ≈ 1–2 semanas, XL ≈ um mês ou mais).

## Mapa de dependências (o que condiciona o quê)

```
P0-4 publish crates ─────────────┐
P0-3 runtime facade discovery ───┤ (independent quick wins)
P0-2 hot-reload dev loop ────────┘

P0-1 DORM cutover (XL) ──┬─▶ P1-7 migrations maturity
                         ├─▶ P1-6 DB introspection
                         ├─▶ P1-8 multi-database breadth
                         └─▶ P1-5 admin / auto-CRUD ─▶ (reads descriptors + data)

P2-9 docs content ───────── (continuous, unblocks adoption)
P2-10 VS Code ext · P2-11 examples+benchmark · P2-12 scaffold/proto polish
```

Começa as três vitórias rápidas P0 em paralelo com a P0-1 (a estaca longa). A P1
espera na maior parte pela P0-1. A P2 corre continuamente.

---

## Fase 0 — mínimos (ser levado a sério na categoria)

### P0-1 · Entregar a camada de dados (cutover DORM) — **XL**

- **Objectivo:** um módulo gerado consegue persistir através do DORM, não só pelo
  adaptador em memória — a maior lacuna de credibilidade para uma framework de
  baterias incluídas.
- **Abordagem:**
  1. Trazer o gateway `seiva-data-dorm` a um estado compilável por trás da feature
     `dorm` (está actualmente excluído do workspace porque precisa da dep git privada
     do DORM + token de CI — decidir: caminho vendorizado, crate DORM publicado, ou
     um job de CI com token).
  2. Fornecer um adaptador `RecordRepository` assente em DORM no template de
     `infrastructure` do módulo, seleccionado na raiz de composição quando `dorm`
     está activo.
  3. Ligar `UnitOfWork`/transacções e o outbox através do gateway.
  4. Mapear a DSL de esquema (blocos `namespace` em `schema/<project>.schema`) para
     modelos DORM ponta-a-ponta.
- **Pronto (verificável):**
  - Um teste de fitness esqueletiza um projecto + módulo **com `--data dorm`**,
    corre-o contra um Postgres real (testcontainer ou o arnês PG:55432 ao vivo), e
    um registo faz round-trip `POST`→`GET` através do DORM. rc 0.
  - `seiva doctor` reporta a camada de dados DORM ligada.
  - Cartão de pontuação: *ORM/camada de dados* 🟡→✅.
- **Depende de:** decisão de acesso ao repositório DORM (dono).

### P0-2 · Ciclo de dev com hot-reload (`seiva start --watch`) — **M**

- **Objectivo:** paridade de editar-e-ver com o servidor de dev de uma framework
  dinâmica.
- **Abordagem:** adicionar um observador de ficheiros (estilo notify/cargo-watch) ao
  `seiva start` que recompila e reinicia o processo `apps/<name>-api` na mudança;
  debounce; faz stream dos logs; mantém a porta. Investir na velocidade de build
  incremental (afinação do `profile.dev` do workspace,
  `-Zthreads`/`cranelift` onde disponível) — o tempo de compilação é o imposto de DX.
- **Pronto:** `seiva start --watch` reinicia numa mudança a `.rs` em < poucos
  segundos para um build incremental; uma demonstração scriptada edita um handler e
  mostra a nova resposta sem um reinício manual. Cartão de pontuação: *CLI de dev*
  🟡→✅.
- **Depende de:** nada.

### P0-3 · Descoberta de fachadas em runtime para `seiva openapi`/`proto` — **S/M**

- **Objectivo:** a CLI documenta as fachadas *do projecto*, não só a demonstração
  embutida.
- **Abordagem:** o runtime já descobre cada fachada montada via o inventário em
  tempo de link (`registered_facades`, usado por `/openapi.json`). Dar à CLI um
  caminho que lê as fachadas compiladas *do projecto* — ou (a) ter o `seiva openapi`
  a invocar um pequeno binário gerado no projecto que imprime o documento a partir do
  próprio inventário do projecto, ou (b) emitir o documento a partir do servidor
  `seiva start` em execução (`GET /openapi.json`) e ter a CLI a buscá-lo/despejá-lo.
- **Pronto:** num projecto esqueletizado com um módulo, `seiva openapi` produz as
  operações desse módulo (não `NotesFacade`); a nota de limitação no stderr é
  removida. Um teste de fitness afirma que o documento gerado contém a rota do
  projecto. Cartão de pontuação: *(contribui para)* a história OpenAPI torna-se
  verdadeira ao projecto.
- **Depende de:** nada (assenta no WireSchema/ADR 0007 já feito).

### P0-4 · Publicar os crates da framework no crates.io — **S** (gated pelo dono)

- **Objectivo:** `cargo add delonix-seiva seiva-http …` funciona; largar o
  `[patch.crates-io]` interino dos projectos gerados.
- **Abordagem:** os crates estão prontos a publicar (metadata, deps de versão).
  Publicar por ordem de dependência (`seiva-ports` → `seiva-macros` → … →
  `seiva-cli`). Depois mudar o scaffold para deixar de acrescentar o patch assim que
  uma versão publicada exista; manter o caminho do patch como override opt-in de dev
  (`SEIVA_FRAMEWORK_DIR`).
- **Pronto:** um projecto gerado compila **sem** secção `[patch.crates-io]`
  (resolvendo do crates.io); o fitness de compilação-standalone corre nesse modo.
  Cartão de pontuação: *binário-único/standalone* totalmente ✅ para utilizadores
  finais.
- **Depende de:** o dono (a publicação é só-dono); vê [publicação](publishing.md).

---

## Fase 1 — as baterias que definem a categoria de baterias incluídas

### P1-5 · Superfície de admin / auto-CRUD — **L** (o diferenciador ex-líbris)

- **Objectivo:** o ex-líbris que poucos concorrentes neste espaço igualam —
  auto-CRUD a partir do modelo/contrato. Já temos os **descritores** de fachada;
  usa-os.
- **Abordagem (incremental):**
  1. Gerar uma **API de admin** de leitura/escrita a partir dos descritores de
     fachada + modelos DORM (list/get/create/update/delete por modelo), guardada por
     uma permissão `admin.*`.
  2. Uma **UI web** mínima e auto-hospedada (renderizada no servidor ou uma pequena
     SPA embutida) que conduz essa API — vistas de lista, formulários de
     detalhe/edição gerados a partir do `WireSchema` de cada DTO.
  3. Registar-e-esquecer: `seiva module new` torna um módulo visível no admin por
     omissão.
- **Estado (2026-11-05):** uma consola de **operações** (um formulário por operação
  de contrato, token bearer) está extraída para a extensão opcional
  `seiva-console-admin` (ADR 0008) — a base dá `404` em `/admin`, a consola monta-se
  explicitamente, CSP por hash (sem `unsafe-inline`). O **registo por módulo** também
  aterrou (ADR 0009): `delonix_seiva::register_admin!` declara um recurso; a consola
  mostra apenas recursos que estão registados *e* que o chamador está autorizado a
  ver (`/admin/registry.json` filtrado no servidor), com detecção de registo
  duplicado. Ainda **pendente**: scaffolding `seiva module new --admin` (3b), o
  contrato DX `dx add` + equivalente Cargo (3c), API de admin de leitura/escrita
  sobre modelos DORM, mascaramento de campo/tenant das linhas de dados, e o E2E
  Playwright. Cartão de pontuação: *GUI de admin* 🟡 (consola de operações
  registadas), ainda não ✅.
- **Depende de:** P0-1 (dados), P0-3 (descoberta).

### P1-6 · Introspecção de base de dados (adopção brownfield) — **M/L**

- **Objectivo:** geração estilo `db pull` do esquema a partir de uma base de dados
  existente — o maior redutor de fricção de adopção para equipas com um esquema já.
- **Abordagem:** um `seiva db pull` (introspecção DORM) que lê o
  `information_schema` e emite blocos `namespace`/modelo; depois esqueletizar módulos
  à volta deles.
- **Pronto:** contra um Postgres com dados semeados, `seiva db pull` produz um
  esquema que `seiva db migrate` faz round-trip com zero desvio. Cartão de pontuação:
  *introspecção* 🟡→✅.
- **Depende de:** P0-1.

### P1-7 · Maturidade de migrações — **M**

- **Objectivo:** igualar o fluxo de referência `migrate dev`/`deploy` do Prisma.
- **Abordagem:** detecção de desvio (shadow DB), migrações reversíveis, uma divisão
  clara dev (`seiva db dev`) vs deploy (`seiva db migrate`), integridade do histórico
  de migrações.
- **Pronto:** um teste de fitness: mudar um modelo → `seiva db dev` gera uma migração
  → aplicar → detectar zero desvio → reverter. Cartão de pontuação: *migrações*
  🟡→✅.
- **Depende de:** P0-1.

### P1-8 · Amplitude multi-base-de-dados — **M/L**

- **Objectivo:** para além do PostgreSQL — pelo menos MySQL (o roadmap multi-backend
  do DORM já existe).
- **Abordagem:** activar o backend MySQL do DORM através do gateway; uma matriz de
  backends no CI.
- **Pronto:** o fitness de round-trip DORM (P0-1) passa também contra MySQL. Cartão
  de pontuação: *multi-base-de-dados* 🟡→✅ (para PG+MySQL).
- **Depende de:** P0-1.

---

## Fase 2 — alcance e confiança (contínuo)

### P2-9 · Preencher o conteúdo da documentação — **L (contínuo)**

- **Objectivo:** mover as páginas "planeadas"/"rascunho" (esta árvore já segue o
  layout Diátaxis completo) para "publicadas" — a alavanca de adopção mais barata.
- **Pronto:** cada parte de tutorial e cada índice de tema está escrito e executável;
  uma verificação de links/âncoras e um fitness de "nenhum stub deixado em
  getting-started/topics" passam.

### P2-10 · Extensão para VS Code — **M**

- **Objectivo:** autocompletamento/validação de esquema + fachada para rivalizar com
  as ferramentas de editor de topo do Prisma.
- **Pronto:** uma extensão que realça a DSL de esquema e mostra diagnósticos de
  fachada/`requires`; publicada no marketplace (ou VSIX).

### P2-11 · Exemplos, templates e um benchmark publicado — **M**

- **Objectivo:** um template do mundo real (o tipo de starter que conduz a adopção
  de frameworks) + um benchmark reproduzível a sustentar a afirmação "nativo, não
  interpretado".
- **Pronto:** um `seiva project new --template <x>` que esqueletiza uma app não
  trivial; um arnês de benchmark commitado + números vs serviços equivalentes de
  frameworks interpretadas em `docs/benchmarks.md`.

### P2-12 · Polimento de scaffold / proto / DX — **S/M**

- **Objectivo:** fechar as pequenas lacunas conhecidas.
- **Pronto:** fidelidade proto3 a partir do `WireSchema` (o seguimento da ADR 0007);
  `Vec<u8>`→`format:byte` nos esquemas compostos; a checklist de "primeiro dia"
  `seiva-dx-parity` verde.

---

## Sequência de marcos (sugerida)

- **M1 (credibilidade rápida):** P0-3 + P0-4 + P0-2 — o projecto documenta-se a si
  próprio, os crates são instaláveis, e o ciclo de dev sente-se tão fluido como o de
  uma framework dinâmica. (Em paralelo, ~2–3 semanas.)
- **M2 (a camada de dados):** P0-1 — a estaca longa. Desbloqueia toda a P1.
- **M3 (baterias):** P1-7 + P1-6 + P1-8, depois P1-5 (admin) como a manchete.
- **M4 (alcance):** P2-9 contínuo ao longo de tudo; P2-10/11/12 à medida da
  capacidade.

## Voltar a correr a comparação {#re-running-the-comparison}

Quando as fases aterrarem, volta a correr o cartão de pontuação objectivamente:

1. Cada lacuna fechada tem de apontar para o seu **teste de fitness ou
   demonstração** (a linha "Pronto" acima). Nenhuma célula passa a ✅ sem um.
2. Volta a medir as três apostas de go-to-market com evidência: o **benchmark**
   (P2-11) sustenta "nativo, não interpretado"; o **admin + round-trip DORM**
   sustentam "baterias incluídas"; os testes de **fidelidade de contrato + authz em
   tempo de compilação** sustentam "contratos honestos e impostos".
3. Actualiza [competitive-landscape.md](competitive-landscape.md) no lugar e faz
   diff da tabela contra esta baseline — o delta é o valor entregue.

---

*Parte da [documentação do Seiva](index.md). Baseline:
[panorama competitivo](competitive-landscape.md) (2026-11).*
