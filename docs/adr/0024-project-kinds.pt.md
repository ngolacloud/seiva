# ADR 0024 — Perfis de projecto: um `seiva project new --kind` aconselhador

- Estado: Aceite
- Data: 2026-11-09
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0003, ADR 0012, ADR 0021

## Contexto

O `seiva project new <nome>` gerava um único esqueleto — um servidor HTTP+gRPC a
hospedar módulos — fosse qual fosse o que o programador estava a construir. A
revisão de arquitectura (achado F3) assinalou-o: o bootstrap não se adapta à
natureza do projecto nem aconselha. Nem todo o serviço é uma API pedido/resposta:
uns são workers de fundo, outros são event-processors a reagir ao barramento de
eventos (ADR 0021), outros são bordas HTTP/BFF a agregar upstreams. Gerar a forma
errada ensina o ponto de partida errado — e o esqueleto gerado é a documentação
mais lida.

## Decisão

1. **Quatro perfis de projecto, escolhidos com `--kind`:** `seiva project new
   <nome> [--kind api|worker|gateway|event]`. Cada um gera a composição-raiz
   cloud-native adequada a essa natureza; `api` é o default.
   - **api** — servidor HTTP+gRPC que hospeda os módulos do projecto (a forma
     actual): `serve_router` + `router_with_modules` + `/health` + `/openapi.json`
     + `/docs`, o caminho de contrato `SEIVA_EMIT`, o passo de seed/demo (ADR 0022)
     e os anchors geridos de módulo/extensão.
   - **worker** — serviço de fundo sobre o worker host do framework
     (`delonix_seiva::worker::{run_worker, WorkerConfig}` + `shutdown_signal`), sem
     superfície HTTP. Um `pass` stub que o programador substitui pelo trabalho.
   - **gateway** — shell de borda HTTP/BFF: a superfície do framework (health,
     cabeçalhos de segurança, authn) **sem módulos próprios** e um sítio marcado
     para acrescentar rotas de agregação de upstreams (o programador traz o cliente
     HTTP).
   - **event** — event-processor: um worker cujo passo drena o barramento de eventos
     in-process (`seiva_events::drain_in_process`) e despacha para os subscritores
     registados, a mesma coreografia que um monólito corre (ADR 0021). Outbox/inbox
     in-memory por omissão; o outbox transaccional DORM + um inbox durável em
     produção.

2. **Aconselhador, não interactivo.** Sem `--kind` a CLI assume `api` **e imprime
   orientação** a listar os quatro perfis e para que serve cada um. Sem prompt
   interactivo — a CLI mantém-se não-interactiva e reproduzível em CI.

3. **O crate da composição-raiz é `apps/<nome>-api` em todos os perfis.** `-api`
   lê-se como «o entrypoint da aplicação», não «HTTP». O `kind` grava-se em
   `seiva.toml` (`[project] kind = "..."`); só o `main.rs`, o Delonixfile/compose
   (portas vs headless), o README e o aconselhamento variam. Isto mantém todo o
   toolchain `start`/`openapi`/`db seed`/`module new`/`module split`/`ext add` — que
   resolve `<nome>-api` — a funcionar sem alterações.

4. **Só o `api` hospeda módulos.** Só ele emite `schema/`, `modules/` e os anchors
   de composição de módulo/extensão. `seiva module new` num projecto
   worker/gateway/event falha com o erro `MissingAnchor` já existente (esses perfis
   não hospedam módulos de negócio).

5. **Todos os perfis são cloud-native** (ADR 0012): distroless, non-root, config
   12-factor, graceful shutdown. As imagens worker/event não têm porta HTTP.

## Consequências

- O bootstrap passa a guiar o programador para a forma certa conforme a natureza do
  projecto sem abrir mão dos defaults cloud-native — fechando o F3.
- Cada perfil reaproveita uma peça do framework já existente (serve loop, worker
  host, event bus), logo sem runtime novo e sem dependência externa nova — o
  `cargo deny` fica verde. O inbox in-memory do perfil `event` é o double do
  `seiva-testing`, marcado como dev-only (substituir pelo inbox DORM em produção).
- Uma fitness function (`generated_project_compiles_for_each_kind`) compila um
  projecto `worker`/`gateway`/`event` contra o checkout, logo uma assinatura de
  worker-host ou de event-bus que derive é apanhada tal como a forma `api` é pelo
  `generated_project_compiles_standalone`.
- O `[patch.crates-io]` interino lista todos os crates do framework; um projecto
  não-api não os usa a todos, logo o `cargo build` imprime avisos benignos de
  «patch not used» até os crates serem publicados e o patch removido.

## Alternativas consideradas

- **Nomear o crate `apps/<nome>-<kind>`.** Rejeitada: obrigaria o toolchain de
  module/split/ext/start/openapi a resolver o nome específico do kind,
  desestabilizando uma teia funcional por um ganho marginal de nome. O `kind` no
  `seiva.toml` regista a natureza sem mover a pasta.
- **Prompt interactivo quando `--kind` é omitido.** Rejeitada: quebra CI/automação e
  exige TTY, contra o estilo não-interactivo da CLI. Um default + orientação
  impressa dá a mesma ajuda.
- **Um cliente real de agregação de upstreams no `gateway`.** Diferida: puxaria um
  cliente HTTP (reqwest/hyper) e um padrão de routing para o scaffold; o gateway
  entrega um shell documentado e o programador escolhe o cliente.
