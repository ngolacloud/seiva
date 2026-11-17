# Publicação dos crates — estado e caminho

Para um projeto gerado por `seiva project new` compilar **standalone** (fora da
árvore do framework), os crates `seiva-*`/`delonix-seiva` têm de estar
disponíveis por versão (crates.io). O scaffold **já emite deps por versão**
(`delonix-seiva = { version = "0.1", … }`, `seiva-http = "0.1"`, …; as camadas
intra-projeto ficam `path`).

## Estado: PUBLICADO em crates.io (v0.1.0, 2026-11-05)

Os **crates de biblioteca estão publicados** em crates.io sob licença dupla
MIT/Apache. Um projecto gerado compila standalone com `cargo add`/scaffold, sem
patches nem git. Verificado 2026-11-16:

| Crate | crates.io |
| --- | --- |
| `seiva-ports`, `seiva-macros`, `seiva-testing`, `seiva-messaging` | ✅ publicado |
| `delonix-seiva`, `seiva-openapi`, `seiva-auth`, `seiva-config` | ✅ publicado |
| `seiva-webhooks`, `seiva-cqrs`, `seiva-http`, `seiva-server` | ✅ publicado |
| `seiva-cli` | ✖ `publish = false` (ver abaixo) |

Para re-publicar uma nova versão, segue a ordem topológica no fim deste
documento.

### Histórico de publish-readiness (medido 2026-11-04, antes do corte)

**Prontos à data (`cargo package` passava):** `seiva-ports`, `seiva-config`,
`seiva-auth`, `seiva-webhooks`, `seiva-macros`, `seiva-cqrs`, `seiva-openapi`,
`delonix-seiva`, `seiva-testing`. Metadata completa; deps internas já carregam
`version` via `[workspace.dependencies]`.

**DESBLOQUEADOS na campanha** (demo extraída → `cargo package` passa):
- `seiva-http`: removido o demo `notes_router`/`notes-facade`; fica só com
  problem/valid/guard/health/security/rate_limit/docs + o builder de router. Os
  testes exercem os primitivos com uma rota inline (sem exemplo).
- `seiva-server`: `serve()` serve a superfície do framework; o `accounts` passou
  a `[dev-dependencies]` + testes (composição demo-free em produção).

**Não publicáveis por intenção:** os crates acoplados ao DORM privado
(`seiva-data-dorm`, `seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`),
excluídos do workspace.

**`seiva-grpc` — RESOLVIDO (decisão a):** o módulo gerado já **não** depende de
`seiva-grpc`; o `interface` gerado só gera o documento OpenAPI (`seiva-openapi`).
A geração proto fica como ferramenta de CLI (`seiva proto`), a partir do mesmo
descritor. `seiva-grpc` continua `publish = false` sem bloquear o projeto gerado,
cujo grafo é agora inteiramente publicável.

**`seiva-cli` — marcado `publish = false` (decisão b):** depende dos exemplos
pelos comandos-demo; instala-se via `cargo install --git …/seiva seiva-cli` até
esses comandos serem extraídos. Projetos gerados **não** dependem do CLI, por
isso isto não os afecta. Publicá-lo em crates.io fica para quando os comandos-
demo saírem (decisão de produto, abaixo).

**FEITO:** os 12 crates de biblioteca foram publicados em crates.io (v0.1.0,
2026-11-05). Um projecto gerado compila standalone com `cargo add`/scaffold. O
único crate por publicar é o `seiva-cli` (abaixo).

**Decisão de produto diferida (não bloqueia nada do acima):**
- `seiva-cli` depende dos exemplos (`notes-facade`, `accounts-*`) porque os
  comandos-demo `user create`/`openapi`/`proto` operam sobre eles. Decisão de
  produto: esses comandos são demo-only (extrair para exemplos/testes), feature-
  gated, ou removidos do CLI shippable? **Nota:** os projetos gerados **não**
  dependem do `seiva-cli` (é instalado como binário), por isso isto não bloqueia o
  projeto gerado — só a publicação do próprio CLI em crates.io (instala via
  `cargo install --git` entretanto).

## `seiva-cli` — destravado estruturalmente (2026-11-16)

O `seiva-cli` **deixou de ter `publish = false`**. Os comandos-demo foram
tratados pela decisão de produto **feature-gate** (branch `feat/seiva-cli-publishable`):

- As dependências de exemplo não publicáveis (`notes-facade`, `accounts-*`) e o
  `seiva-grpc` (ainda `publish = false`) **saíram** do `seiva-cli`. Mesmo como
  dependências `optional` atrás de uma feature desligada, bloqueavam o
  empacotamento: `cargo publish` **exige `version` em toda a dependência `path`,
  mesmo optional** — e essas crates não estão em crates.io.
- A demonstração passou a ser **auto-contida** em `crates/seiva-cli/src/demo.rs`,
  construída só a partir de crates publicados (`delonix-seiva`, `seiva-openapi`,
  `seiva-auth`): o facade `Notes` inline (que `seiva openapi` documenta) e um
  `user create` inline com as mesmas regras de domínio + Argon2id do `accounts`.
- Tudo isto atrás da feature `demo` (**default off**). O binário publicado é
  magro — scaffold/lint/db/start + `openapi`/`proto` **dentro de um projecto**
  (que correm na mesma, via subprocesso). `cargo install seiva-cli --features demo`
  reactiva os comandos de demonstração. O `proto` built-in fora-de-projecto foi
  removido (dependia de `seiva-grpc`, não publicado).

**Prova nesta sessão:** `cargo publish -p seiva-cli --dry-run --no-verify` passa
(empacota 6 ficheiros; o manifesto e as versões das deps resolvem). O
empacotamento deixou de estar bloqueado.

**O que ainda falta para publicar de facto (release do workspace, dono):** o
`cargo publish --dry-run` **completo** (com verificação de compilação) ainda
falha, mas **não por causa do `seiva-cli`** — falha porque compila o tarball
contra `delonix-seiva 0.1.0` de crates.io, que está **atrás do workspace local**
(a API `scaffold` ganhou `DataLayer`/`ProjectKind`/`add_app_member`/… desde a
publicação de 2026-11-05). É preciso **cortar uma nova versão e republicar**
`delonix-seiva` (e `seiva-openapi`/`seiva-auth` se mudaram) **antes** de publicar
o `seiva-cli`. Segue a ordem topológica abaixo.

## Ordem topológica para (re)publicar

Para cortar uma nova versão da biblioteca, publica de baixo para cima
(`cargo publish -p <crate>`), esperando cada crate propagar no índice antes do
seguinte:

`seiva-ports` → `seiva-macros` → `seiva-testing` → `seiva-messaging` →
`delonix-seiva` → `seiva-openapi` → `seiva-auth` → `seiva-config` →
`seiva-webhooks` → `seiva-cqrs` → `seiva-http` → `seiva-server`
(→ `seiva-cli`, uma vez destravado acima).

## Nota

A publicação em si (`cargo publish`) é externa e irreversível e exige a conta
crates.io do dono — não é feita automaticamente. Este documento é o pré-requisito
e o plano; os passos 1–3 são trabalho de código, o passo 4 é operação do dono.
