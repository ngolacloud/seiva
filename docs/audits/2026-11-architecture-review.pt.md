# Revisão de arquitectura — backlog de achados (2026-11)

Painel: **Martin Fowler** + **Robert C. Martin (Uncle Bob)**, convidado **Eric
Evans** (skill `seiva-architecture-reviewers`). A revisão achou a espinha
arquitectural sólida — cinco camadas com a regra de dependências imposta pelo
grafo de crates **e** pelo `seiva lint`, o contrato como fonte única,
monólito↔microserviço por composição — e deixou os achados abaixo. Um achado só
está **fechado** quando a sua **fitness function de fecho** está verde, não quando
o código «parece» certo.

Como usar este ficheiro: o agent `architecture-reviewer` reconcilia cada achado
novo com esta lista; as skills `seiva-architecture-reviewers` e
`seiva-pattern-fitness` referenciam-na. Actualiza a coluna **Estado** à medida que
cada item fecha.

## Achados

| ID | Sev | Achado | Âncora | Estado | Fitness function de fecho |
|----|-----|--------|--------|--------|---------------------------|
| **F1** | P0 | A camada de contrato depende do transporte HTTP: a `facade` depende de `seiva-http` (`Problem` = RFC 9457, `Validate`). O contrato mais interno conhece um mecanismo de entrega; num framework dual-transporte a semântica de erro fica moldada ao HTTP. | `scaffold.rs` `facade_cargo_toml`/`facade_lib`; `docs/adr/0003` | **Fechado** (esta sessão) | ✅ `Validate`/`ValidationError` **e** o `Problem` RFC 9457 vivem agora no neutro `seiva-contract` (sem axum; `http::StatusCode`). O `seiva-http` re-exporta o `Problem` e render-o via um novo wrapper `ProblemResponse` (`IntoResponse` axum); o braço de erro do macro `#[controller]` passa por ele; os extractors/middleware internos passaram a `ProblemResponse`. A facade (scaffold + 6 exemplos) já **não** nomeia nenhum crate de transporte; `generated_facade_is_framework_wired` + `module_manifests_depend_on_the_right_framework_crates` asserem-no, e `cargo test -p seiva-http` prova que as respostas 422/413/403/429 continuam a sair `problem+json`. O único tipo de transporte que uma facade ainda pode nomear é o `seiva_http::Ctx` (contexto de pedido) — acoplamento separado e mais estreito, documentado na ADR 0003. |
| **F2** | P0 | O caso de uso gerado não tem fronteira transaccional nem evento de domínio, contra o invariante «um caso de uso = uma transacção, os módulos cooperam por eventos» (ADR 0021). O programador copia a forma demasiado simples. | `scaffold.rs` `application_lib` | **Fechado** (esta sessão) | ✅ `generated_application_implements_the_facade_over_ports` assere agora que o caso de uso abre `UnitOfWork::transaction` e chama `events.publish("notes.record.created", …)` lá dentro; o teste gerado assere `TxOutcome::Committed` + tópicos emitidos pelos doubles `seiva-testing`, e `generated_project_compiles_standalone` compila-o (`cargo build --tests`). |
| **F3** | P1 | O bootstrap não se adapta à natureza do projecto: `project new` é um esqueleto fixo, sem perfil/kind (API / worker / gateway / event-processor), sem diálogo aconselhador. | `seiva-cli` `project new` | **Fechado** (esta sessão) | ✅ `seiva project new <nome> --kind api\|worker\|gateway\|event` (ADR 0024): cada um gera a composição-raiz cloud-native adequada (worker host / drain do event-bus / borda HTTP / API que hospeda módulos). Sem `--kind` assume `api` e imprime orientação. Provado por `generated_project_compiles_for_each_kind` (worker/gateway/event compilam standalone) + arranque ao vivo de cada um. |
| **F4** | P2 | Escolha DDD vs Clean Arch deixada implícita: o porto `Repository` vive na `application` (convenção Clean Arch), não no `domain` (DDD clássico). Defensável, mas devia ser uma decisão explícita e documentada. | `docs/adr/0003` §4; `lint::forbidden_layer_dep` | **Parcialmente documentado** (a ADR 0003 di-lo) → fecha nomeando-a como escolha deliberada na doc de arquitectura + na skill de revisão (feito). | A ADR/skill declaram a escolha explicitamente; sem mudança de código. |
| **F5** | P1 | O `module split` não trata a propriedade de dados: o serviço gerado monta adaptadores in-memory, logo falta a metade «dados descentralizados» dos microserviços. | `scaffold.rs` `service_files`/`service_main_rs` | **Fechado** (esta sessão) | ✅ O `module split` scaffolda agora o serviço como dono dos seus dados: `apps/<nome>-service/compose.yaml` sobe uma base de dados a que só este serviço se liga (`<nome>-db`) com um `DELONIX_ORM_DSN` por-serviço, mais um README a explicitar a propriedade; o `seed/` do módulo carrega no store DESTE serviço (ADR 0022). Provado por `split_scaffolds_a_single_module_service_binary` + ao vivo. (Ligar a feature `dorm` persiste na BD própria do serviço; o adaptador in-memory é o default de dev.) |
| **F6** | P2 | Vocabulário impreciso e uma regra empírica: «facade» designa o inbound port / Published Language (não o Facade do GoF); `resource_of` pluraliza ingenuamente (`category`→`categorys`). | `scaffold.rs` `resource_of`; o nome `facade` | **Fechado** (esta sessão) | ✅ `resource_of` delega agora na regra autoritária `crate::contract::resource_segment` (uma só regra; `Category`→`categories`), provado por `generated_facade_route_uses_the_shared_pluralisation`. ✅ Nota de vocabulário no doc do macro `#[facade]` e no cabeçalho da facade gerada: é o inbound port / Published Language do módulo, não o Facade do GoF. |

## Fechado / entregue nesta sessão

- **ADR 0022 — dados de seed e demo por módulo em CSV** (inspirado no Odoo, carga
  híbrida). Entregue ponta-a-ponta: núcleo `seed.rs` (external-id, `parse_csv`,
  registo), o scaffold (repo com chave idempotente, CSV `seed/`+`demo/` embebidos,
  `load_seed`/`load_demo`, passo de arranque), `seiva db seed [--demo]`. Provado
  pelas fitness tests (`generated_module_has_seed_and_demo`,
  `composition_root_wires_seed_on_boot`, `generated_project_compiles_standalone`
  agora a ligar o módulo ao `main`) e por um arranque ao vivo
  (`APP_ENV=development SEIVA_SEED=1 SEIVA_LOAD_DEMO=1` → "seed/demo sources
  applied", saída 0).
- **Tooling de revisão de arquitectura**: a skill `seiva-architecture-reviewers`
  (Fowler + Uncle Bob + Evans) e o agent `architecture-reviewer`, para os achados
  fecharem de forma não-empírica.

## Diferido (rastreado, não é um achado)

- **Enriquecimento dos exemplos**: dar ao módulo `examples/notes` um `seed/`+`demo/`
  de referência (teste de seed-idempotente + demo-pela-facade) e provar a
  coreografia demo-pela-facade no `examples/shop` (demo → `drain_in_process` →
  `orders.placed` → `inventory.reserved`). A feature está provada no scaffold (o que
  os módulos novos recebem); isto acrescenta a referência escrita à mão de onde os
  programadores copiam.
