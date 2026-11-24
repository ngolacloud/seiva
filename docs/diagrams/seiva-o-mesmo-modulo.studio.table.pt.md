# Seiva — o mesmo módulo: tabela de confirmação

Desenho do Studio de arquitectura do dxtutor (`seiva-o-mesmo-modulo.studio.arch.json`). Os dois
painéis são *strict*: cada caixa, fronteira e seta tem a fonte no código, e o Studio valida-o com
as regras do backend (0 achados). Linhas confirmadas no commit `26592bd` do repositório `seiva`.

Os dois painéis são o que os geradores do `seiva` produzem (`crates/seiva/src/scaffold.rs`): à
esquerda o monólito de `seiva project new` (`apps/<projecto>-api`), à direita o serviço de
`seiva module split <módulo>` (`apps/<módulo>-service`). Os cinco crates do módulo são os mesmos
nos dois — o split não move código.

As setas cheias e cinzentas são **dependências** entre crates, tiradas dos `Cargo.toml` gerados
(`a → b` = `a` depende de `b`); as tracejadas são o que acontece em execução. A legenda do desenho
distingue-as.

| Painel | Tipo | Elemento | Confirmado em |
|---|---|---|---|
| Monólito | Fronteira | Monólito modular · um processo | crates/seiva/src/scaffold.rs:283 project_files (apps/<projecto>-api); :1107 app_main_rs |
| Monólito | Fronteira | o mesmo módulo: modules/<módulo> | crates/seiva/src/scaffold.rs:369 module_files (os cinco crates) |
| Serviço | Fronteira | seiva module split · um serviço | crates/seiva/src/scaffold.rs:568 service_files (apps/<módulo>-service, monta só este módulo); crates/seiva-cli/src/main.rs:495 module_split |
| Serviço | Fronteira | o mesmo módulo: modules/<módulo> | crates/seiva/src/scaffold.rs:664-665 o mesmo módulo (interface + infrastructure), ligado por path (../../modules/<módulo>) |
| Monólito | Caixa | seiva-server (`serve_router`) | crates/seiva/src/scaffold.rs:1165 router_with_modules; :1167 serve_router (app_main_rs) |
| Monólito | Caixa | outros módulos o mesmo padrão | crates/seiva/src/scaffold.rs:491 wire_module_in_main (cada módulo junta o seu router em :533) |
| Monólito | Caixa | interface (`#[seiva::controller]`) | crates/seiva/src/scaffold.rs:1939 interface_cargo_toml; :1846 #[controller] |
| Monólito | Caixa | application | crates/seiva/src/scaffold.rs:1899 application_cargo_toml (portas de seiva-ports) |
| Monólito | Caixa | facade (`#[seiva::facade]`) | crates/seiva/src/scaffold.rs:1872 facade_cargo_toml; :1620 #[facade(version = "v1")] |
| Monólito | Caixa | domain | crates/seiva/src/scaffold.rs:1890 domain_cargo_toml (sem dependências) |
| Monólito | Caixa | infrastructure | crates/seiva/src/scaffold.rs:1920 infrastructure_cargo_toml (em memória por omissão) |
| Serviço | Caixa | seiva-server (`serve_router`) | crates/seiva/src/scaffold.rs:722 router_with_modules; :724 serve_router (service_main_rs) |
| Serviço | Caixa | interface (`#[seiva::controller]`) | crates/seiva/src/scaffold.rs:1939 interface_cargo_toml; :1846 #[controller] |
| Serviço | Caixa | application | crates/seiva/src/scaffold.rs:1899 application_cargo_toml (portas de seiva-ports) |
| Serviço | Caixa | facade (`#[seiva::facade]`) | crates/seiva/src/scaffold.rs:1872 facade_cargo_toml; :1620 #[facade(version = "v1")] |
| Serviço | Caixa | domain | crates/seiva/src/scaffold.rs:1890 domain_cargo_toml (sem dependências) |
| Serviço | Caixa | infrastructure | crates/seiva/src/scaffold.rs:1920 infrastructure_cargo_toml (em memória por omissão) |
| Serviço | Caixa | <módulo>-db (`postgres:17`) | crates/seiva/src/scaffold.rs:616 serviço <módulo>-db; :617 image: postgres:17 (service_compose_yaml) |
| Monólito | Seta | seiva-server → interface «monta o router» | crates/seiva/src/scaffold.rs:533 modules.merge(<módulo>_router(...)) |
| Monólito | Seta | seiva-server → infrastructure «injecta» | crates/seiva/src/scaffold.rs:522-528 InMemoryRecordRepository, DirectUnitOfWork, LoggingEventPublisher |
| Monólito | Seta | seiva-server → outros módulos o mesmo padrão | crates/seiva/src/scaffold.rs:533 (o mesmo merge por cada módulo) |
| Monólito | Dependência | interface → facade | crates/seiva/src/scaffold.rs:1943 {module}-facade |
| Monólito | Dependência | interface → application | crates/seiva/src/scaffold.rs:1949 {module}-application (nunca infrastructure) |
| Monólito | Dependência | application → facade | crates/seiva/src/scaffold.rs:1904 {module}-facade |
| Monólito | Dependência | application → domain | crates/seiva/src/scaffold.rs:1903 {module}-domain |
| Monólito | Dependência | infrastructure → application | crates/seiva/src/scaffold.rs:1924 {module}-application |
| Monólito | Dependência | infrastructure → domain | crates/seiva/src/scaffold.rs:1925 {module}-domain |
| Serviço | Seta | seiva-server → interface «só este router» | crates/seiva/src/scaffold.rs:717 Router::new().merge(<módulo>_router(...)) — só este módulo |
| Serviço | Seta | seiva-server → infrastructure «injecta» | crates/seiva/src/scaffold.rs:705-711 os mesmos adaptadores |
| Serviço | Dependência | interface → facade | crates/seiva/src/scaffold.rs:1943 {module}-facade |
| Serviço | Dependência | interface → application | crates/seiva/src/scaffold.rs:1949 {module}-application (nunca infrastructure) |
| Serviço | Dependência | application → facade | crates/seiva/src/scaffold.rs:1904 {module}-facade |
| Serviço | Dependência | application → domain | crates/seiva/src/scaffold.rs:1903 {module}-domain |
| Serviço | Dependência | infrastructure → application | crates/seiva/src/scaffold.rs:1924 {module}-application |
| Serviço | Dependência | infrastructure → domain | crates/seiva/src/scaffold.rs:1925 {module}-domain |
| Serviço | Seta | seiva-server → <módulo>-db «DELONIX_ORM_DSN» | crates/seiva/src/scaffold.rs:611 DELONIX_ORM_DSN; :612 depends_on (compose); usado com a feature dorm (o código gerado começa em memória, :705) |

## Divergências encontradas ao desenhar

Corrigidas no mesmo ramo, no commit antes do que traz este desenho:

1. `docs/architecture.md` e `docs/architecture.pt.md` desenhavam `interface → infrastructure` e
   diziam que a `interface` depende «apenas da fachada». O gerador e o ADR 0003 dizem `interface →
   application, facade` e nunca `infrastructure` (`scaffold.rs:1944-1947`; ADR 0003, §3). Diagrama
   ASCII e texto corrigidos nas duas línguas.
2. Os mesmos documentos e o `CLAUDE.md` diziam que a porta de repositório é definida pela
   `application`; desde a revisão do ADR 0003 vive no `domain` e a `application` reexporta-a
   (`scaffold.rs:1656` e `:1683`). Corrigido.
3. O `CLAUDE.md` descrevia o fluxo `orders.placed → inventory.reserved → payments.captured →
   notifications.sent` como ligado pela raiz de composição. Cada módulo publica o seu evento, mas em
   `apps/shop-api` só o primeiro salto está encaminhado (`orders.placed` → `InventorySubscriber`,
   `apps/shop-api/src/lib.rs:229-233`). Corrigido para dizer isso.
4. A mensagem de `seiva module split` dizia que o serviço é dono dos seus dados sem dizer que o código
   gerado começa em memória (o README gerado já o dizia, `scaffold.rs:635-637`). A mensagem passa a
   dizê-lo (`crates/seiva-cli/src/main.rs:553`).
