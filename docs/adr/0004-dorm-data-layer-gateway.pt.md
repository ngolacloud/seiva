# ADR 0004 — O gateway da camada de dados DORM

- Estado: Aceite
- Data: 2026-11-03
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0002, ADR 0003

## Contexto

O DORM é a camada de dados por omissão, e a Parte C do brief exige que **todo** o
acoplamento ao DORM viva numa só crate, de modo que desligar a camada de dados
seja simplesmente não compilar essa crate (o padrão Gateway). O resto do Seiva —
o núcleo, as crates `domain`/`application`/`facade`/`interface` do módulo — tem
de conhecer apenas `seiva-ports`.

O DORM já mesclou a campanha de que este gateway depende (fronteira DORM-C, B1
sessões estritas + chave de cursor por API + `connect_with`, B2 namespaces), em
`main` `f43d873`. A regra do brief mantém-se até o DORM publicar a API de
biblioteca: o Seiva depende do DORM **por uma revisão git fixada, nunca por
caminho local**.

## Decisão

### 1. Uma crate de gateway, fixada por revisão git

A crate `seiva-data-dorm` é o único lugar que nomeia o DORM. Depende de
`dorm-core` por `{ git = "…/dorm", rev = "f43d873…" }` e de `seiva-ports`. A
crate `infrastructure` de um módulo usa-a para construir o seu handle DORM e para
satisfazer as portas que a sua `application` declarou; mais nada importa o DORM.

### 2. Ainda não é membro do workspace

Construir o gateway precisa de acesso ao repositório privado do DORM. A CI do
Seiva não o tem sem um segredo. Por isso o gateway é **excluído** do workspace
por agora e construído por si só (com `CARGO_NET_GIT_FETCH_WITH_CLI=1`), o que
mantém a CI do framework verde e sem DORM. Ligá-lo à feature `dorm` do framework
— o que faz o build por omissão puxar o DORM e exige um token de CI (deploy key /
PAT) — é um passo deliberado e posterior, decisão do dono.

### 3. O gateway impõe os invariantes DORM-B1 no connect

`connect(DormConfig)` constrói o `Database` de um módulo com `connect_with` (DSN,
réplicas, schema, pool por API — DORM-B1), e antes de o devolver liga as
**sessões estritas** (`set_strict_sessions(true)` — uma statement sem sessão é
um erro, nunca o predefinido global do processo) e **configura a chave de
assinatura do cursor** (`cursor::set_cursor_key` — sem predefinição falsificável).
Um framework não as pode esquecer: o gateway é dono delas.

### 4. As portas que mapeiam de forma limpa são implementadas já

`Clock` (`SystemClock`) e `IdGenerator` (`UuidIdGenerator`, UUIDv4 → `Id`) estão
implementadas. Não precisam de DORM, mas vivem aqui para que a composição de um
módulo tenha um só fornecedor de camada de dados para ligar.

### 5. O `UnitOfWork` é agora baseado em âmbito (a incompatibilidade do modelo de tx, resolvida)

A primeira descoberta de fronteira: o `UnitOfWork` de `seiva-ports` era
**baseado em handle** (`begin()` → `commit()`/`rollback()`), mas a API de
transacção do DORM é **delimitada por closure** (`db::atomic(|tx| async { … })`,
que faz commit quando a closure retorna). Um `commit()` baseado em handle chamado
mais tarde não consegue conduzir uma closure que já retornou.

**Decisão (o dono escolheu isto):** tornar o `UnitOfWork` **baseado em âmbito** —
`transaction(|| async { … })` faz commit em `Ok`, rollback em `Err`. O brief
permite ajustar a forma da porta ("ajusta os detalhes na ADR do núcleo"), e isto
é honesto quanto a onde a transacção começa e termina. Está implementado por toda
a pilha: a porta (`seiva-ports`), o duplo em memória (`seiva-testing`), o módulo
de exemplo (`examples/notes`), e o gateway (`DormUnitOfWork`, que o mapeia
directamente sobre `with_session` + `db::atomic`). Isto evita precisar de uma
primitiva DORM baseada em handle (o `on_commit` do DORM-B3), que deixa de estar
no caminho crítico. A necessidade de "publicar após um commit bem-sucedido" é
satisfeita agindo sobre o `Ok` que a chamada devolve — sem hook `on_commit`.

`EventPublisher` é a única porta que ainda falta: o outbox transaccional
(`dorm-outbox`) escreve na mesma transacção, por isso é ligada assim que esse
estiver dentro.

## Consequências

- A travessia da fronteira está provada: `seiva-data-dorm` compila e testa contra
  o DORM fixado em `f43d873`, com `connect` a construir um handle e a impor
  sessões estritas + a chave de cursor (o deadpool é lazy, por isso o teste não
  precisa de base de dados).
- O núcleo mantém-se sem DORM; o `cargo build --workspace` por omissão não puxa
  DORM nenhum.
- A forma da porta `UnitOfWork` é agora uma decisão conhecida e delimitada, em
  vez de uma acidental.

## Alternativas consideradas

- **Acoplar já todo o workspace ao DORM** (tornar o gateway membro e dar à CI um
  token do DORM). Rejeitada para este passo: é decisão do dono e tornaria a CI
  verde e auto-contida numa que precisa de um segredo.
- **Re-exportar tipos DORM através da API pública do gateway.** Evitado para além
  do inevitável (`Database`, `dorm_core::Error` em `connect`): o gateway é a
  costura do DORM, mas as portas mantêm o DORM fora de todas as outras crates.
