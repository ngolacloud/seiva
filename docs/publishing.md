# Publicação dos crates — estado e caminho

Para um projeto gerado por `seiva project new` compilar **standalone** (fora da
árvore do framework), os crates `seiva-*`/`delonix-seiva` têm de estar
disponíveis por versão (crates.io) ou por git tag — hoje o scaffold emite deps
`path` com um placeholder (`FRAMEWORK_CRATES`) que só resolve dentro do repo.

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

**BLOQUEADORES RESTANTES (decisão do dono — release pass):**
- `seiva-grpc` é `publish = false`, **mas o módulo gerado depende dele** (para
  `proto()`). Decisão: publicar `seiva-grpc` (se a geração proto é para enviar) ou
  tirar a dependência do módulo gerado (gerar proto só via CLI, não no crate do
  módulo). Até lá, um projeto gerado não resolve `seiva-grpc` de crates.io.
- `seiva-cli` depende dos exemplos (`notes-facade`, `accounts-*`) porque os
  comandos-demo `user create`/`openapi`/`proto` operam sobre eles. Decisão de
  produto: esses comandos são demo-only (extrair para exemplos/testes), feature-
  gated, ou removidos do CLI shippable? **Nota:** os projetos gerados **não**
  dependem do `seiva-cli` (é instalado como binário), por isso isto não bloqueia o
  projeto gerado — só a publicação do próprio CLI em crates.io (instala via
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
