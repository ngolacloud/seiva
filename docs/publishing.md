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

**Não publicáveis por intenção:** `seiva-grpc` (`publish = false`); os crates
acoplados ao DORM privado (`seiva-data-dorm`, `seiva-outbox-relay`,
`seiva-rag-worker`, `seiva-mcp`), excluídos do workspace.

**BLOQUEADOS — dependem de crates de exemplo (`publish = false`):**
- `seiva-http` → `notes-facade` (o demo `notes_router` vive dentro do crate que
  devia enviar).
- `seiva-server` → `accounts-interface`, `accounts-infrastructure` (monta a demo
  `accounts` de forma fixa).
- `seiva-cli` → `notes-facade`, `accounts-*` (o `user create`/`openapi`/`proto`
  usam as demos).

Um crate publicável **não pode** depender de um `publish = false`. Logo estes
três precisam de extrair a demo antes de publicar — é a dívida "demo bleed"
registada em S3.

## O que falta (campanha dedicada, recomendada)

1. **Extrair as demos dos crates shippable:**
   - `seiva-http`: remover `notes_router`/`notes-facade`; o demo passa a viver em
     `examples/notes` (ou num `examples/*-server`). `seiva-http` fica só com
     problem/valid/guard/health/security/rate_limit/docs + o builder de router.
   - `seiva-server`: tornar-se demo-free (já tem `serve_router` genérico); a
     montagem do `accounts` move-se para um binário de exemplo
     (`examples/accounts-server`). Alternativa limpa: dividir em `seiva-runtime`
     (lib demo-free) + um exemplo que compõe `accounts`.
   - `seiva-cli`: o caminho `user create`/`openapi`/`proto` deixa de depender dos
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
