# Publicação dos crates — estado e caminho

Para um projecto gerado por `seiva project new` compilar **standalone** (fora da
árvore do framework), os crates `seiva-*`/`delonix-seiva` têm de estar
disponíveis por versão (crates.io). O scaffold **já emite deps por versão**
(`delonix-seiva = { version = "0.1", … }`, `seiva-http = "0.1"`, …; as camadas
intra-projecto ficam `path`), por isso só falta **publicar** os crates para um
projecto gerado compilar sozinho.

## Estado de publish-readiness (medido 2026-11-04)

**Prontos (`cargo package` passa):** `seiva-ports`, `seiva-config`, `seiva-auth`,
`seiva-webhooks`, `seiva-macros`, `seiva-cqrs`, `seiva-openapi`, `delonix-seiva`,
`seiva-testing`. Metadata completa; deps internas já carregam `version` via
`[workspace.dependencies]`.

**DESBLOQUEADOS nesta campanha** (demo extraída → `cargo package` passa):
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
descritor. `seiva-grpc` continua `publish = false` sem bloquear o projecto gerado,
cujo grafo é agora inteiramente publicável.

**`seiva-cli` — marcado `publish = false` (decisão b):** depende dos exemplos
pelos comandos-demo; instala-se via `cargo install --git …/seiva seiva-cli` até
esses comandos serem extraídos. Projetos gerados **não** dependem do CLI, por
isso isto não os afecta. Publicá-lo em crates.io fica para quando os comandos-
demo saírem (decisão de produto, abaixo).

**O único passo que falta para o projecto gerado compilar sozinho:** publicar os
11 crates de biblioteca (owner-only). O scaffold já referencia a versão.

**Decisão de produto diferida (não bloqueia nada do acima):**
- `seiva-cli` depende dos exemplos (`notes-facade`, `accounts-*`) porque os
  comandos-demo `user create`/`openapi`/`proto` operam sobre eles. Decisão de
  produto: esses comandos são demo-only (extrair para exemplos/testes), feature-
  gated, ou removidos do CLI shippable? **Nota:** os projectos gerados **não**
  dependem do `seiva-cli` (é instalado como binário), por isso isto não bloqueia o
  projecto gerado — só a publicação do próprio CLI em crates.io (instala via
  `cargo install --git` entretanto).

## O que falta (release pass dedicado)

1. **Decidir `seiva-grpc`** (publicar vs tirar do módulo gerado) e **`seiva-cli`**
   (destino dos comandos-demo), conforme acima.
2. ~~Extrair as demos de `seiva-http`/`seiva-server`~~ ✅ feito nesta campanha.
   Resta (opcional) a divisão `seiva-runtime` demo-free vs demo, se se quiser um
   binário de exemplo `accounts-server` separado.
3. **(histórico) `seiva-cli`**: o caminho `user create`/`openapi`/`proto` deixa de depender dos
     exemplos (ou move a demonstração para um exemplo).
2. **Decisão de distribuição (do dono):** crates.io (deps por `version`) **ou**
   git tag (`{ git = "…seiva", tag = "v0.1.0" }`). Recomendo **crates.io** para o
   getting-started de 5 min ser `cargo add`/scaffold sem git.
3. **Scaffold:** substituir `FRAMEWORK_CRATES` (path placeholder) pela forma
   escolhida em (2) — deps por `version` (crates.io) ou `git`+`tag`.
4. **Publicar** por ordem topológica (owner-only, precisa de conta crates.io e é
   irreversível):
   `seiva-ports` → `seiva-macros` → `seiva-testing` → `seiva-messaging` →
   `delonix-seiva` → `seiva-openapi` → `seiva-auth` → `seiva-config` →
   `seiva-webhooks` → `seiva-cqrs` → `seiva-http` → `seiva-server` → `seiva-cli`.
   (`cargo publish -p <crate>` por crate, de baixo para cima.)

## Nota

A publicação em si (`cargo publish`) é externa e irreversível e exige a conta
crates.io do dono — não é feita automaticamente. Este documento é o pré-requisito
e o plano; os passos 1–3 são trabalho de código, o passo 4 é operação do dono.
