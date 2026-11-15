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
| **F1** | P0 | A camada de contrato depende do transporte HTTP: a `facade` depende de `seiva-http` (`Problem` = RFC 9457, `Validate`). O contrato mais interno conhece um mecanismo de entrega; num framework dual-transporte a semântica de erro fica moldada ao HTTP. | `scaffold.rs` `facade_cargo_toml`/`facade_lib`; `docs/adr/0003` | **Fechado (scaffold)** — falta a regra de lint permanente (A1) + a semântica HTTP (C) | ✅ `Validate`/`ValidationError` **e** o `Problem` RFC 9457 vivem agora no neutro `seiva-contract` (sem axum; `http::StatusCode`). O `seiva-http` re-exporta o `Problem` e render-o via um novo wrapper `ProblemResponse` (`IntoResponse` axum); o braço de erro do macro `#[controller]` passa por ele; os extractors/middleware internos passaram a `ProblemResponse`. A facade (scaffold + 6 exemplos) já **não** nomeia nenhum crate de transporte; `generated_facade_is_framework_wired` + `module_manifests_depend_on_the_right_framework_crates` asserem-no, e `cargo test -p seiva-http` prova que as respostas 422/413/403/429 continuam a sair `problem+json`. O único tipo de transporte que uma facade ainda pode nomear é o `seiva_http::Ctx` (contexto de pedido) — acoplamento separado e mais estreito, documentado na ADR 0003. |
| **F2** | P0 | O caso de uso gerado não tem fronteira transaccional nem evento de domínio, contra o invariante «um caso de uso = uma transacção, os módulos cooperam por eventos» (ADR 0021). O programador copia a forma demasiado simples. | `scaffold.rs` `application_lib` | **Fechado** (esta sessão) | ✅ `generated_application_implements_the_facade_over_ports` assere agora que o caso de uso abre `UnitOfWork::transaction` e chama `events.publish("notes.record.created", …)` lá dentro; o teste gerado assere `TxOutcome::Committed` + tópicos emitidos pelos doubles `seiva-testing`, e `generated_project_compiles_standalone` compila-o (`cargo build --tests`). |
| **F3** | P1 | O bootstrap não se adapta à natureza do projecto: `project new` é um esqueleto fixo, sem perfil/kind (API / worker / gateway / event-processor), sem diálogo aconselhador. | `seiva-cli` `project new` | **Fechado** (esta sessão) | ✅ `seiva project new <nome> --kind api\|worker\|gateway\|event` (ADR 0024): cada um gera a composição-raiz cloud-native adequada (worker host / drain do event-bus / borda HTTP / API que hospeda módulos). Sem `--kind` assume `api` e imprime orientação. Provado por `generated_project_compiles_for_each_kind` (worker/gateway/event compilam standalone) + arranque ao vivo de cada um. |
| **F4** | P2 | Escolha DDD vs Clean Arch: o porto `Repository` vivia na `application` (Clean Arch), não no `domain` (DDD clássico/Evans). | `docs/adr/0003` §4; `scaffold.rs` `domain_lib`/`application_lib` | **Fechado** (esta sessão) | ✅ Movido para DDD clássico: o porto de repositório é agora **definido no `domain`**, num `RepositoryError` de domínio; a `application` **re-exporta-o** (logo o `interface` continua a importar da `application` — regra de camadas + pureza do domínio inalteradas, sem mudar o `lint`) e adapta o erro para `PortResult`. As portas técnicas do framework (UoW/EventPublisher/…) ficam na `application`. Scaffold + o exemplo canónico `examples/notes` migrados; `generated_application_implements_the_facade_over_ports` assere que o porto vive no `domain` e `generated_project_compiles_standalone` compila-o. ADR 0003 §4 actualizada (revertida). **Follow-up:** os outros 7 módulos-exemplo (accounts, polls, shop/*) ainda usam o layout anterior — migração mecânica rastreada. |
| **F5** | P1 | O `module split` não trata a propriedade de dados: o serviço gerado monta adaptadores in-memory, logo falta a metade «dados descentralizados» dos microserviços. | `scaffold.rs` `service_files`/`service_main_rs` | **Fechado** (esta sessão) | ✅ O `module split` scaffolda agora o serviço como dono dos seus dados: `apps/<nome>-service/compose.yaml` sobe uma base de dados a que só este serviço se liga (`<nome>-db`) com um `DELONIX_ORM_DSN` por-serviço, mais um README a explicitar a propriedade; o `seed/` do módulo carrega no store DESTE serviço (ADR 0022). Provado por `split_scaffolds_a_single_module_service_binary` + ao vivo. (Ligar a feature `dorm` persiste na BD própria do serviço; o adaptador in-memory é o default de dev.) |
| **F6** | P2 | Vocabulário impreciso e uma regra empírica: «facade» designa o inbound port / Published Language (não o Facade do GoF); `resource_of` pluraliza ingenuamente (`category`→`categorys`). | `scaffold.rs` `resource_of`; o nome `facade` | **Fechado** (esta sessão) | ✅ `resource_of` delega agora na regra autoritária `crate::contract::resource_segment` (uma só regra; `Category`→`categories`), provado por `generated_facade_route_uses_the_shared_pluralisation`. ✅ Nota de vocabulário no doc do macro `#[facade]` e no cabeçalho da facade gerada: é o inbound port / Published Language do módulo, não o Facade do GoF. |

## Achados novos — re-revisão do scaffold (2026-11-15)

Re-revisão do painel sobre o scaffold tal como está, adversarial quanto aos
«Fechado» de F1–F6. A espinha aguenta (F2 abre a transacção + publica lá dentro;
F4 pôs o porto de repositório no `domain`). Estes são **novos**, não estão na
tabela acima. Um achado só fecha quando a sua **fitness function** fica verde.

| ID | Sev | Revisor / lente | Achado | Âncora | Fitness que fecha |
|----|-----|-----------------|--------|--------|-------------------|
| **A1** | Alto | Uncle Bob — regra de dependências | O `seiva lint` **não tem regra** para crates de transporte na `facade`: `forbidden_layer_dep("facade", …)` só recusa DORM + crates de camada-irmã, logo uma facade escrita à mão que dependa de `seiva-http` passa verde. O invariante «transport-neutral» de F1 é imposto **só** por um teste-snapshot do scaffold (`module_manifests_depend_on_the_right_framework_crates`), não pela fitness permanente. A ADR 0003 ainda *permite* `facade → seiva_http::Ctx`, logo a posição real é «neutro por omissão, permitido de facto». | `lint.rs:178-183`; `scaffold.rs:2582` | Uma regra em `seiva lint` que recusa crates de transporte (`seiva-http` excepto o `Ctx` nomeado, `axum`, `seiva-grpc`) na `facade`, com fixture a provar que uma facade que depende de `axum` falha o lint. |
| **A2** | Alto | Evans + Fowler — Published Language / modelo de domínio | O evento inter-módulo é **sem esquema**: o scaffold publica `serde_json::json!({…})` sob um tópico-string `"{snake}.record.created"`, consumido como `CloudEvent` com `event.ty` string. O canal que o framework declara como acoplamento principal é o único sem contrato — enquanto os DTOs síncronos da facade têm `WireSchema` e chegam ao OpenAPI. Um consumidor não tem contra que compilar. | `scaffold.rs:1617` (publica); `scaffold.rs:1251` (consome) | Um tipo de evento tipado, com `WireSchema`, tópico versionado, registado em link-time como as facades; uma fitness a provar que o payload gerado corresponde a um esquema registado. Exige **ADR 0027** primeiro. |
| **B** | Médio | Fowler — fitness functions / distribuição | «Compila standalone» assenta num `[patch.crates-io]` de caminho local: a versão `0.1` do framework não está em crates.io, logo cada projecto gerado embebe um **caminho absoluto para o checkout do gerador**, e `generated_project_compiles_standalone` «resolve o seu próprio» root — verde por construção, provando uma portabilidade que o artefacto não tem (só compila onde esse caminho resolva). | `scaffold.rs:1445-1476` (`workspace_patch_section`); `FRAMEWORK_VERSION` = `"0.1"` | Publicar os crates do framework e apagar a secção `[patch]` (o comentário diz que é «deletable in a single edit»); a fitness compila o projecto **sem** o patch. Entretanto, renomear o teste `..._against_local_checkout` e dizê-lo no README gerado. |
| **C** | Médio | Uncle Bob + Fowler — acoplamento honesto | F1 moveu `Problem`/`Validate` de `seiva-http` para `seiva-contract`, mas `Problem` **é** RFC 9457 e carrega `http::StatusCode`; a facade gerada mapeia o erro com `Problem::unprocessable_entity` (422). Num framework dual-transporte o contrato mais interno continua a falar códigos de estado HTTP — o acoplamento mudou de nome-de-crate para semântica, menos visível, não removido. *Contestado:* a ADR 0003 argumenta que `StatusCode` é «the protocol-level status HTTP and gRPC share» (plausível). | `scaffold.rs:1543` (`Problem::unprocessable_entity`); ADR 0003 | Ou um enum de erro do domínio-do-framework que cada transporte mapeia para o seu código (HTTP 422 / gRPC `INVALID_ARGUMENT`), ou uma nota na ADR de que a neutralidade de F1 é de nome-de-crate, não de semântica. |
| **D** | Baixo | Fowler — arquitectura evolutiva | `project new --kind gateway\|event\|worker` oferece topologias distribuídas ao dia zero, uma tensão leve com MonolithFirst (o developer escolhe topologia antes de ter um monólito para extrair). Mitigado: o default é `api` com conselho impresso, e são raízes de composição, não microserviços forçados. | `scaffold.rs:239` (`main_rs_for`); ADR 0024 | Texto aconselhador nos cabeçalhos `gateway`/`event`: «usar só quando já existem serviços a jusante; de outro modo começa em `api`». |

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
