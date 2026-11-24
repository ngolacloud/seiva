# Seiva — the same module: confirmation table

A drawing from the dxtutor architecture Studio (`seiva-o-mesmo-modulo.studio.arch.json`). Both
panels are *strict*: every box, boundary and arrow has its source in the code, and the Studio checks
it with the backend rules (0 findings). Lines confirmed at commit `26592bd` of the `seiva`
repository.

The two panels are what the `seiva` generators produce (`crates/seiva/src/scaffold.rs`): on the
left the monolith from `seiva project new` (`apps/<project>-api`), on the right the service from
`seiva module split <module>` (`apps/<module>-service`). The module's five crates are the same in
both — the split moves no code.

The solid grey arrows are **dependencies** between crates, taken from the generated `Cargo.toml`
files (`a → b` = `a` depends on `b`); the dashed ones are what happens at run time. The drawing's
legend tells them apart. The drawing's labels are in Portuguese; they are quoted here with an
English gloss.

| Panel | Type | Element | Confirmed in |
|---|---|---|---|
| Monolith | Boundary | «Monólito modular · um processo» (modular monolith, one process) | crates/seiva/src/scaffold.rs:283 project_files (apps/<project>-api); :1107 app_main_rs |
| Monolith | Boundary | «o mesmo módulo: modules/<módulo>» (the same module) | crates/seiva/src/scaffold.rs:369 module_files (the five crates) |
| Service | Boundary | «seiva module split · um serviço» (one service) | crates/seiva/src/scaffold.rs:568 service_files (apps/<module>-service, mounts only this module); crates/seiva-cli/src/main.rs:495 module_split |
| Service | Boundary | «o mesmo módulo: modules/<módulo>» (the same module) | crates/seiva/src/scaffold.rs:664-665 the same module (interface + infrastructure), linked by path (../../modules/<module>) |
| Monolith | Box | seiva-server (`serve_router`) | crates/seiva/src/scaffold.rs:1165 router_with_modules; :1167 serve_router (app_main_rs) |
| Monolith | Box | «outros módulos o mesmo padrão» (other modules, same pattern) | crates/seiva/src/scaffold.rs:491 wire_module_in_main (each module merges its router at :533) |
| Monolith | Box | interface (`#[seiva::controller]`) | crates/seiva/src/scaffold.rs:1939 interface_cargo_toml; :1846 #[controller] |
| Monolith | Box | application | crates/seiva/src/scaffold.rs:1899 application_cargo_toml (ports from seiva-ports) |
| Monolith | Box | facade (`#[seiva::facade]`) | crates/seiva/src/scaffold.rs:1872 facade_cargo_toml; :1620 #[facade(version = "v1")] |
| Monolith | Box | domain | crates/seiva/src/scaffold.rs:1890 domain_cargo_toml (no dependencies) |
| Monolith | Box | infrastructure | crates/seiva/src/scaffold.rs:1920 infrastructure_cargo_toml (in-memory by default) |
| Service | Box | seiva-server (`serve_router`) | crates/seiva/src/scaffold.rs:722 router_with_modules; :724 serve_router (service_main_rs) |
| Service | Box | interface (`#[seiva::controller]`) | crates/seiva/src/scaffold.rs:1939 interface_cargo_toml; :1846 #[controller] |
| Service | Box | application | crates/seiva/src/scaffold.rs:1899 application_cargo_toml (ports from seiva-ports) |
| Service | Box | facade (`#[seiva::facade]`) | crates/seiva/src/scaffold.rs:1872 facade_cargo_toml; :1620 #[facade(version = "v1")] |
| Service | Box | domain | crates/seiva/src/scaffold.rs:1890 domain_cargo_toml (no dependencies) |
| Service | Box | infrastructure | crates/seiva/src/scaffold.rs:1920 infrastructure_cargo_toml (in-memory by default) |
| Service | Box | <module>-db (`postgres:17`) | crates/seiva/src/scaffold.rs:616 service <module>-db; :617 image: postgres:17 (service_compose_yaml) |
| Monolith | Arrow | seiva-server → interface «monta o router» (mounts the router) | crates/seiva/src/scaffold.rs:533 modules.merge(<module>_router(...)) |
| Monolith | Arrow | seiva-server → infrastructure «injecta» (injects) | crates/seiva/src/scaffold.rs:522-528 InMemoryRecordRepository, DirectUnitOfWork, LoggingEventPublisher |
| Monolith | Arrow | seiva-server → other modules | crates/seiva/src/scaffold.rs:533 (the same merge for each module) |
| Monolith | Dependency | interface → facade | crates/seiva/src/scaffold.rs:1943 {module}-facade |
| Monolith | Dependency | interface → application | crates/seiva/src/scaffold.rs:1949 {module}-application (never infrastructure) |
| Monolith | Dependency | application → facade | crates/seiva/src/scaffold.rs:1904 {module}-facade |
| Monolith | Dependency | application → domain | crates/seiva/src/scaffold.rs:1903 {module}-domain |
| Monolith | Dependency | infrastructure → application | crates/seiva/src/scaffold.rs:1924 {module}-application |
| Monolith | Dependency | infrastructure → domain | crates/seiva/src/scaffold.rs:1925 {module}-domain |
| Service | Arrow | seiva-server → interface «só este router» (only this router) | crates/seiva/src/scaffold.rs:717 Router::new().merge(<module>_router(...)) — only this module |
| Service | Arrow | seiva-server → infrastructure «injecta» (injects) | crates/seiva/src/scaffold.rs:705-711 the same adapters |
| Service | Dependency | interface → facade | crates/seiva/src/scaffold.rs:1943 {module}-facade |
| Service | Dependency | interface → application | crates/seiva/src/scaffold.rs:1949 {module}-application (never infrastructure) |
| Service | Dependency | application → facade | crates/seiva/src/scaffold.rs:1904 {module}-facade |
| Service | Dependency | application → domain | crates/seiva/src/scaffold.rs:1903 {module}-domain |
| Service | Dependency | infrastructure → application | crates/seiva/src/scaffold.rs:1924 {module}-application |
| Service | Dependency | infrastructure → domain | crates/seiva/src/scaffold.rs:1925 {module}-domain |
| Service | Arrow | seiva-server → <module>-db «DELONIX_ORM_DSN» | crates/seiva/src/scaffold.rs:611 DELONIX_ORM_DSN; :612 depends_on (compose); used with the dorm feature (the generated code starts in memory, :705) |

## Divergences found while drawing

Fixed on the same branch, in the commit before the one that adds this drawing:

1. `docs/architecture.md` and `docs/architecture.pt.md` drew `interface → infrastructure` and said
   that `interface` depends «only on the facade». The generator and ADR 0003 say `interface →
   application, facade` and never `infrastructure` (`scaffold.rs:1944-1947`; ADR 0003, §3). ASCII
   diagram and text fixed in both languages.
2. The same documents and `CLAUDE.md` said the repository port is defined by `application`; since
   the ADR 0003 revision it lives in `domain` and `application` re-exports it (`scaffold.rs:1656` and
   `:1683`). Fixed.
3. `CLAUDE.md` described the `orders.placed → inventory.reserved → payments.captured →
   notifications.sent` flow as wired by the composition root. Each module publishes its own event,
   but in `apps/shop-api` only the first hop is routed (`orders.placed` → `InventorySubscriber`,
   `apps/shop-api/src/lib.rs:229-233`). Fixed to say so.
4. The `seiva module split` message said the service owns its data without saying that the generated
   code starts in memory (the generated README already said so, `scaffold.rs:635-637`). The message
   now says it (`crates/seiva-cli/src/main.rs:553`).
