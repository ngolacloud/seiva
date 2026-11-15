# ARCHITECTURE_GAPS — Seiva @ 985cfa1

Lacunas de arquitectura (não de estilo). Cada uma com impacto concreto e correcção ancorada.

## G1 🔴 — Consumo de eventos in-process não-atómico (perda silenciosa de efeito)

- **Onde:** `crates/seiva-events/src/lib.rs` — `dispatch_to_subscribers` loop `begin→handle→finish`
  (~`:233-241`); `drain_in_process` (~`:345-364`). Worker por omissão:
  `crates/delonix-seiva/src/scaffold.rs:1233-1278`.
- **Gap:** o claim do inbox é persistido **antes** do handler e **fora de transacção**. Um
  `handle` que falha deixa o claim durável; na reentrega, `begin` devolve `InProgress`, o laço
  salta e reporta `Ok`, e o cursor avança → **efeito perdido**. A docstring promete uma
  transacção "Fase 2" que o código não implementa.
- **Convergência:** detectado independentemente pela lane A (A1) e pela lane D (D-1).
- **Correcção (ADR 0028 §2):** claim→efeito→mark numa só `UnitOfWork::transaction`; rollback do
  claim em falha, para a reentrega voltar a ser `Fresh`. Teste de reentrega-após-falha
  (hoje ausente; o existente `choreography.rs` só faz 1 passagem — D-5).
- **Estado:** corrigido nesta remessa (ver IMPROVEMENT_ROADMAP P0).

## G2 🟠 — Garantia de entrega diverge entre monólito e microserviço (D-2)

- **Onde:** `seiva-events/src/lib.rs:345-364` (in-process) vs `dorm-outbox 0.1.0` `relay`
  (`max_attempts=5` + backoff + dead-letter).
- **Gap:** o caminho de broker tem política de falha (tentativas, backoff, veneno→dead-letter);
  o in-process **não tem nada**. Isto quebra a promessa central do Seiva — "as mesmas
  garantias, o mesmo código de subscriber, por composição" — assim que um módulo faz
  `module split` de monólito para microserviço.
- **Correcção:** dar ao driver in-process a mesma política (idealmente reutilizando
  `RelayConfig`), para o split não mudar a semântica.

## G3 🟠 — Contrato "SemVer-stable" sem `#[non_exhaustive]` (C1)

- **Onde:** `seiva-contract/src/lib.rs` (`Problem` todos campos `pub` `:110-124`,
  `ValidationError:40`), `seiva-ports/src/lib.rs` (`Scope:115`, `IdempotencyOutcome:229`,
  `AuditEntry:288`, `RateLimitDecision:356`), descritores em `delonix-seiva/src/contract.rs:44-92`.
- **Gap:** as crates declaradas "Kept SemVer-stable by policy" não marcam **nenhum** tipo
  público `#[non_exhaustive]`; adicionar uma variante/campo é breaking. Pior: os descritores são
  construídos por **literal de struct no crate do utilizador** (código gerado em
  `seiva-macros/src/lib.rs:198-228`), logo **não podem** ser `#[non_exhaustive]` — ficam
  permanentemente frágeis. Ironia: `#[non_exhaustive]` só aparece onde **não** é crítico
  (`seiva-server:351`, `observability/tracing.rs:46`).
- **Correcção:** `#[non_exhaustive]` nos tipos de contrato estáveis; para os descritores,
  construtores `const fn`/builder + política documentada (ou aceitar bump acoplado à macro).

## G4 🟢 — Macro `#[seiva::controller]`: `.expect()` defensivo (C2 **não se reproduz**)

- **Onde:** `seiva-macros/src/lib.rs:854-858` (itera os `ImplItem::Fn`) + `:974-979`
  (`descriptor.method(name).expect(...)`).
- **Avaliação original (Lane C):** classificou como Alto — "um helper privado no `impl` gera uma
  rota que faz `panic!` no arranque".
- **Reavaliação na implementação (verificada, read-only + compilador):** **o cenário não é
  reproduzível.** Prova:
  1. Num `impl Trait for Type` do Rust, **todos** os métodos pertencem obrigatoriamente ao trait
     — um método inerente dá `error[E0407]: method is not a member of trait` (reproduzido).
  2. A macro `#[seiva::facade]` regista **todos** os `TraitItem::Fn` no descritor
     (`seiva-macros/src/lib.rs:92-180`, sem saltar provided/default).
  3. Logo todo método que o `#[controller]` itera está no descritor, e `descriptor.method(name)`
     nunca é `None` para um método do `impl` → o `.expect()` é **inalcançável**.
  - Caso-limite (provided method do trait **não** sobreposto no `impl`): o controller simplesmente
    não gera rota para ele — rota declarada-não-servida (gap menor), **não** panic.
- **Estado:** **sem alteração de código** (não há bug). O `.expect()` fica como asserção defensiva.
  Melhoria opcional (Baixo): trocá-lo por `unreachable!` com nota, ou gerar a rota só para métodos
  cujo nome a macro reconhece. Não é P0.

## G5 🟡 — Isolamento multi-tenant é opt-in, não default (B-1)

- **Onde:** `seiva-cqrs/src/lib.rs:476` (pipeline genérica fixa `Scope::Global`); 7/8 módulos de
  exemplo bindam `Scope::Global` (`examples/notes`, `polls`, todo o `shop`); no gateway
  `Scope::Global` → `with_system_scope` (`seiva-data-dorm/src/lib.rs:112`), o bypass auditado.
- **Gap:** a garantia SG1 é "não podes esquecer-te de NOMEAR um scope" (o compilador exige o
  argumento), mas `Scope::Global` é sempre construível e é o default embarcado. Quem copia o
  scaffold `notes` herda **zero** isolamento row-level. Só `accounts` realiza o padrão
  ponta-a-ponta (`scope_of` fail-closed).
- **Correcção:** derivar o scope de um `Ctx`/`Principal` ambiente e **fail-closed** na pipeline
  CQRS; converter os exemplos ao estilo `accounts`; lint (ADR 0029 BOLA) que proíbe
  `Scope::Global` literal fora de allowlist; documentar que `#[model(tenant_scoped)]` é obrigatório.

## G6 🟡 — Concorrência de relay sem `SKIP LOCKED`/lease (D-3)

- **Onde:** `seiva-outbox-relay/src/lib.rs:191-250` + `dorm-outbox` `relay` — `SELECT … WHERE id
  > pos ORDER BY id LIMIT` **sem** lock de linha nem advisory lock.
- **Gap:** duas réplicas com o mesmo `(consumer, topic)` lêem o mesmo lote e ambas
  publicam/despacham. Correcção do efeito depende da idempotência do consumidor; o broker recebe
  duplicados. A doc delega leader-election ao chamador, mas a API aceita N instâncias sem aviso.
- **Correcção:** ADR 0028 §1 — competitive claim com `FOR UPDATE SKIP LOCKED` + lease. **Fora do
  escopo desta remessa** (nova sessão).

## G7 🟡 — Superfícies declaradas-mas-parciais (E5, E6)

- **gRPC** (`seiva-grpc`): colapsa todo DTO não-escalar num `string` placeholder, sem paridade
  com o OpenAPI/WireSchema.
- **Worker shipeado** no-op (`demo_pass`); o `run_idempotency_purge` DORM-free existe mas não é
  cablado no scaffold.
- **Correcção:** completar o codegen gRPC a partir do WireSchema; cablar o worker real ou marcar
  a superfície como experimental na doc.

## Resumo de acoplamento / extensibilidade

- **Eixo enum vs trait:** os defeitos G1/G2 derivam de a fronteira transaccional estar no sítio
  errado (loop sobre subscribers dentro de `dispatch`, sem seam para o chamador). ADR 0028 move o
  seam para dentro da transacção por-item — correcção estrutural, não cosmética.
- **Acoplamento ao DORM:** o núcleo publicável não tem deps `git`; só `seiva-mcp`/`seiva-rag-worker`
  dependem de `dorm-*` via git (periféricos). `delonix-seiva` tem feature `dorm` hoje no-op.
