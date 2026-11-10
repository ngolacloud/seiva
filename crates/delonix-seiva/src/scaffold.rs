//! Project and module scaffolding.
//!
//! Generation is pure: [`project_files`] and [`module_files`] return the files
//! to write (path + contents) without touching the filesystem, so they are
//! fully testable. The CLI performs the I/O and the registration edits, which
//! are themselves pure string transforms ([`add_members_to_workspace`],
//! [`register_module_in_seiva_toml`], [`append_namespace_block`]).
//!
//! The generated module is wired to the REAL framework transports, not a
//! hand-rolled controller. Its five layers mirror [`examples/notes`] and follow
//! the dependency rule enforced by the crate graph (the point of "one layer per
//! crate"):
//!
//! - `facade` — a `#[seiva::facade(version = "v1")]` trait plus serde wire DTOs,
//!   the `Validate` impl and the error→`Problem` mapping. The facade's own DTOs
//!   are the wire types, so this layer depends on `delonix-seiva`, `serde` and
//!   `seiva-http`.
//! - `domain` — entities and rules; depends on nothing but `shared/kernel`.
//! - `application` — the service implementing the facade over an outbound
//!   repository port and the `seiva-ports` cross-cutting ports. Transport-free:
//!   it names no `delonix-seiva`, `seiva-http` or `axum` item.
//! - `infrastructure` — an in-memory repository adapter (a DORM-backed one
//!   arrives via the `dorm` feature); depends on `application` + `domain` +
//!   `seiva-ports`.
//! - `interface` — the transport. It holds the `#[seiva::controller]` adapter
//!   that yields `<trait>_router`, and generates the module's OpenAPI document
//!   from the facade descriptor via `seiva-openapi`. (Proto generation is a CLI
//!   tool, not a per-module dependency, so the module crate stays gRPC-free.)
//!
//! The controller is placed in `interface` (not `application`) by deliberate
//! layering choice, so the application layer stays transport-free; the decision
//! is documented in the generated `interface` crate itself.
//!
//! [`examples/notes`]: https://docs.rs/delonix-seiva

/// A file to be written, with a path relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub contents: String,
}

/// A name was not a valid snake_case ASCII identifier, or a registration edit
/// could not find its anchor.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScaffoldError {
    #[error("`{0}` is not a valid name: use lowercase ASCII snake_case (e.g. `order_items`)")]
    InvalidName(String),
    #[error("already present: {0}")]
    AlreadyPresent(String),
    #[error("could not find `{anchor}` in {what}")]
    MissingAnchor { anchor: String, what: String },
}

/// The marker line in the workspace `Cargo.toml` members array before which new
/// module crate members are inserted.
const MEMBERS_ANCHOR: &str = "# seiva:module-members (managed by the seiva CLI)";

/// The marker line in the composition root's `main.rs` before which `seiva module
/// new` inserts each module's router merge (so the facades link into the binary
/// and `seiva openapi` documents them).
const MODULE_MERGES_ANCHOR: &str = "// seiva:module-merges (managed by the seiva CLI)";

/// The marker line in the app crate's `Cargo.toml` `[dependencies]` before which
/// `seiva module new` inserts the module's `interface`/`infrastructure` path deps.
const APP_MODULE_DEPS_ANCHOR: &str = "# seiva:module-deps (managed by the seiva CLI)";

/// The marker line in the app crate's `Cargo.toml` `[dependencies]` before which
/// `seiva ext add` inserts an optional extension's dependency (ADR 0010).
const APP_EXTENSION_DEPS_ANCHOR: &str = "# seiva:extension-deps (managed by the seiva CLI)";

/// The marker line in the composition root's `main.rs` — the managed composition
/// point — before which `seiva ext add` inserts each extension's mount statement
/// (shadowing `app`), after the framework router is built (ADR 0010).
const EXTENSIONS_ANCHOR: &str = "// seiva:extensions (managed by the seiva CLI)";

/// Validates a project or module name: lowercase ASCII, starts with a letter,
/// then letters/digits/underscores, no trailing or doubled underscore.
pub fn validate_name(name: &str) -> Result<(), ScaffoldError> {
    let invalid = || ScaffoldError::InvalidName(name.to_owned());
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Err(invalid()),
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(invalid());
    }
    if name.ends_with('_') || name.contains("__") {
        return Err(invalid());
    }
    Ok(())
}

/// The crate names of a module's five layers, in dependency order.
pub fn layer_crates(module: &str) -> [String; 5] {
    [
        format!("{module}-facade"),
        format!("{module}-domain"),
        format!("{module}-application"),
        format!("{module}-infrastructure"),
        format!("{module}-interface"),
    ]
}

/// The workspace member paths of a module's five layer crates.
pub fn module_member_paths(module: &str) -> Vec<String> {
    [
        "facade",
        "domain",
        "application",
        "infrastructure",
        "interface",
    ]
    .iter()
    .map(|layer| format!("modules/{module}/{layer}"))
    .collect()
}

/// The project name read from a `seiva.toml` manifest's `[project].name`, or
/// `None` when the manifest is unparseable or lacks it. Pure, so the CLI's
/// `start` can resolve the app crate (`<name>-api`) deterministically.
#[must_use]
pub fn project_name(manifest: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(manifest).ok()?;
    value
        .get("project")?
        .get("name")?
        .as_str()
        .map(ToOwned::to_owned)
}

/// The nature of a generated project — it shapes the composition root the
/// scaffold emits (ADR 0024). Every kind is cloud-native (distroless, non-root,
/// 12-factor, graceful shutdown); they differ in the runtime shape, each reusing
/// a framework piece the project already ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectKind {
    /// An HTTP+gRPC API server that hosts the project's modules (the default).
    #[default]
    Api,
    /// A background worker over the worker host (no business HTTP surface).
    Worker,
    /// An HTTP edge/gateway (BFF) shell that aggregates upstreams; hosts no
    /// modules of its own.
    Gateway,
    /// An event-processor: a worker that drains the in-process event bus and
    /// reacts (a choreography consumer).
    Event,
}

impl ProjectKind {
    /// Parses a `--kind` value (case-insensitive; empty or `api` is the default).
    /// Returns a human-readable message listing the valid kinds on error.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "api" => Ok(Self::Api),
            "worker" => Ok(Self::Worker),
            "gateway" => Ok(Self::Gateway),
            "event" => Ok(Self::Event),
            other => Err(format!(
                "unknown kind `{other}` — expected one of: api, worker, gateway, event"
            )),
        }
    }

    /// The manifest token written to `seiva.toml` (`[project] kind = "..."`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Worker => "worker",
            Self::Gateway => "gateway",
            Self::Event => "event",
        }
    }

    /// Whether this kind hosts the project's modules (only `api`): it alone emits
    /// `schema/`, `modules/` and the module/extension composition anchors.
    #[must_use]
    pub fn hosts_modules(self) -> bool {
        matches!(self, Self::Api)
    }

    /// Whether this kind serves an HTTP port (api, gateway): drives the
    /// Delonixfile `EXPOSE`/`SEIVA_HTTP_ADDR` and the compose port mapping.
    #[must_use]
    pub fn serves_http(self) -> bool {
        matches!(self, Self::Api | Self::Gateway)
    }
}

/// Files for a new project `name` of the given [`ProjectKind`]. Paths are relative
/// to the new `name/` dir.
pub fn project_files(name: &str, kind: ProjectKind) -> Result<Vec<GeneratedFile>, ScaffoldError> {
    validate_name(name)?;
    let app = format!("{name}-api");
    let file = |path: &str, contents: String| GeneratedFile {
        path: path.to_owned(),
        contents,
    };

    // The composition root is always `apps/<name>-api` (the application entrypoint,
    // regardless of kind) so the `start`/`openapi`/`db seed`/`module` tooling that
    // resolves `<name>-api` keeps working; the `kind` lives in `seiva.toml` and the
    // `main.rs` shape varies (ADR 0024).
    let mut files = vec![
        file("seiva.toml", seiva_toml(name, kind)),
        file("Cargo.toml", workspace_cargo_toml(&app)),
        file(".gitignore", "/target\n**/*.rs.bk\n.env\n".to_owned()),
        file("README.md", project_readme(name, kind)),
        file(&format!("apps/{app}/Cargo.toml"), bin_cargo_toml(&app, kind)),
        file(&format!("apps/{app}/src/main.rs"), main_rs_for(name, kind)),
        // Cloud-native packaging: a distroless multi-stage image and a compose
        // file that `delonix stack up` (or `docker compose`) runs.
        file("Delonixfile", delonixfile(&app, kind)),
        file("compose.yaml", compose_yaml(name, kind)),
        file(".dockerignore", dockerignore()),
        // shared/kernel: shared value types, no business rules.
        file("shared/kernel/Cargo.toml", lib_cargo_toml("kernel", &[])),
        file(
            "shared/kernel/src/lib.rs",
            "//! Shared value types across modules. No business rules live here.\n\n/// A tenant identifier, shared by every module's scope.\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub struct TenantId(pub u128);\n".to_owned(),
        ),
    ];
    // Only the module-hosting `api` kind ships the project schema and the
    // `modules/` dir; worker/gateway/event do not host business modules.
    if kind.hosts_modules() {
        files.push(file(&format!("schema/{name}.schema"), project_schema()));
        files.push(file("modules/.keep", String::new()));
    }
    Ok(files)
}

/// Dispatches to the composition-root `main.rs` template for the kind.
fn main_rs_for(name: &str, kind: ProjectKind) -> String {
    match kind {
        ProjectKind::Api => app_main_rs(name),
        ProjectKind::Worker => worker_main_rs(name),
        ProjectKind::Gateway => gateway_main_rs(name),
        ProjectKind::Event => event_main_rs(name),
    }
}

/// The project README, which names the kind and its run shape.
fn project_readme(name: &str, kind: ProjectKind) -> String {
    let (nature, next) = match kind {
        ProjectKind::Api => (
            "an HTTP+gRPC API server that hosts your modules",
            "Add a module with `seiva module new <name>`, then run `seiva start`.",
        ),
        ProjectKind::Worker => (
            "a background worker service (no business HTTP surface)",
            "Edit the worker pass in `apps/{name}-api/src/main.rs`, then run `seiva start`.",
        ),
        ProjectKind::Gateway => (
            "an HTTP edge/gateway (BFF) shell that aggregates upstream services",
            "Add your upstream calls where the composition root marks them, then `seiva start`.",
        ),
        ProjectKind::Event => (
            "an event-processor that drains the event bus and reacts (choreography)",
            "Register your subscribers in `apps/{name}-api/src/main.rs`, then run `seiva start`.",
        ),
    };
    let next = next.replace("{name}", name);
    format!(
        "# {name}\n\nA Delonix Seiva project (kind: `{kind}`) — {nature}.\n\n{next}\n\n\
         Build a distroless, non-root image with `delonix stack up` (see the `Delonixfile`\n\
         and `compose.yaml`).\n",
        kind = kind.as_str(),
    )
}

/// Files for a new module `module`, relative to the project root.
///
/// The emitted crates are wired to the real framework transports: the `facade`
/// carries a `#[seiva::facade]` trait, the `interface` a `#[seiva::controller]`
/// adapter that yields `<trait>_router` plus the OpenAPI/proto generation, and
/// the `application` implements the facade over `seiva-ports`. See the module
/// docs for the full shape and the layering decision.
pub fn module_files(module: &str, admin: bool) -> Result<Vec<GeneratedFile>, ScaffoldError> {
    validate_name(module)?;
    // A validated name is already lowercase snake_case, so `snake == module`;
    // we keep the name explicit for the crate-import spellings (`<snake>_facade`).
    let snake = module.replace('-', "_");
    // The facade trait (and its `<Trait>Facade` descriptor, router name and
    // resource path) is named after the module in PascalCase: `order_items`
    // yields the trait `OrderItems` and the builder `order_items_router`.
    let pascal = to_pascal_case(&snake);
    let base = format!("modules/{module}");
    let file = |path: String, contents: String| GeneratedFile { path, contents };

    Ok(vec![
        file(format!("{base}/module.toml"), module_toml(module)),
        // facade: the versioned contract + wire DTOs (serde + Validate + Problem).
        file(
            format!("{base}/facade/Cargo.toml"),
            facade_cargo_toml(module),
        ),
        file(
            format!("{base}/facade/src/lib.rs"),
            facade_lib(module, &pascal),
        ),
        // domain: entities and rules; depends only on shared/kernel.
        file(
            format!("{base}/domain/Cargo.toml"),
            domain_cargo_toml(module),
        ),
        file(format!("{base}/domain/src/lib.rs"), domain_lib()),
        // application: facade impl over the repository port + seiva-ports. No
        // transport, no framework runtime, no DORM.
        file(
            format!("{base}/application/Cargo.toml"),
            application_cargo_toml(module),
        ),
        file(
            format!("{base}/application/src/lib.rs"),
            application_lib(&snake, &pascal),
        ),
        // infrastructure: the in-memory repository adapter (DORM via `dorm`).
        file(
            format!("{base}/infrastructure/Cargo.toml"),
            infrastructure_cargo_toml(module),
        ),
        file(
            format!("{base}/infrastructure/src/lib.rs"),
            infrastructure_lib(&snake),
        ),
        // interface: the transport — the #[seiva::controller] adapter and the
        // OpenAPI/proto generation from the facade descriptor.
        file(
            format!("{base}/interface/Cargo.toml"),
            interface_cargo_toml(module),
        ),
        file(
            format!("{base}/interface/src/lib.rs"),
            interface_lib(&snake, &pascal, admin),
        ),
        // seed/: reference data, loaded on every boot through the data layer with
        // an idempotent upsert (ADR 0022). The first column `id` is a stable
        // textual key hashed to a deterministic id, so a reseed overwrites.
        file(format!("{base}/seed/record.csv"), seed_csv()),
        // demo/: demonstration data, loaded only in dev through the facade (so it
        // exercises the domain invariants). Safe to be rich — it never ships to prod.
        file(format!("{base}/demo/record.csv"), demo_csv()),
    ])
}

/// Inserts a module's layer crates into the workspace `Cargo.toml` members,
/// before the managed anchor. Pure.
pub fn add_members_to_workspace(cargo: &str, module: &str) -> Result<String, ScaffoldError> {
    let anchor_line = cargo
        .lines()
        .find(|line| line.trim() == MEMBERS_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: MEMBERS_ANCHOR.to_owned(),
            what: "Cargo.toml".to_owned(),
        })?;
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    let additions: String = module_member_paths(module)
        .iter()
        .map(|path| format!("{indent}\"{path}\",\n"))
        .collect();
    if cargo.contains(&format!("\"modules/{module}/facade\"")) {
        return Err(ScaffoldError::AlreadyPresent(format!("module `{module}`")));
    }
    Ok(cargo.replacen(anchor_line, &format!("{additions}{anchor_line}"), 1))
}

/// Inserts the module's `interface`/`infrastructure` path dependencies into the
/// app crate's `Cargo.toml`, before the managed anchor — so the composition root
/// links the module (and its facade registers for `seiva openapi`). Pure.
pub fn add_module_to_app_manifest(cargo: &str, module: &str) -> Result<String, ScaffoldError> {
    let anchor_line = cargo
        .lines()
        .find(|line| line.trim() == APP_MODULE_DEPS_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: APP_MODULE_DEPS_ANCHOR.to_owned(),
            what: "apps/<name>-api/Cargo.toml".to_owned(),
        })?;
    if cargo.contains(&format!("{module}-interface = {{ path")) {
        return Err(ScaffoldError::AlreadyPresent(format!("module `{module}`")));
    }
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let additions = format!(
        "{indent}{module}-interface = {{ path = \"../../modules/{module}/interface\" }}\n\
         {indent}{module}-infrastructure = {{ path = \"../../modules/{module}/infrastructure\" }}\n"
    );
    Ok(cargo.replacen(anchor_line, &format!("{additions}{anchor_line}"), 1))
}

/// Inserts the module's router merge into the composition root's `main.rs`, before
/// the managed anchor, picking the in-memory adapters. Shadows `modules` so each
/// merge composes. Pure; the controller/router/adapter names follow the generated
/// module's shape (`<m>_router`, `<Pascal>Controller`, the in-memory adapters).
pub fn wire_module_in_main(main_rs: &str, module: &str) -> Result<String, ScaffoldError> {
    let anchor_line = main_rs
        .lines()
        .find(|line| line.trim() == MODULE_MERGES_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: MODULE_MERGES_ANCHOR.to_owned(),
            what: "apps/<name>-api/src/main.rs".to_owned(),
        })?;
    if main_rs.contains(&format!("{module}_interface::{module}_router(")) {
        return Err(ScaffoldError::AlreadyPresent(format!("module `{module}`")));
    }
    let pascal = to_pascal_case(module);
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    // The dev gate for demo data is declared once, with the first module wired —
    // so a freshly generated project (no modules yet) stays warning-clean (ADR 0022).
    let gate = if main_rs.contains("let load_demo =") {
        String::new()
    } else {
        format!(
            "{indent}// Seed + demo data (ADR 0022): each module loads its `seed/` data on\n\
             {indent}// every boot (idempotent upsert); `demo/` loads only when this gate is\n\
             {indent}// set — dev, or an explicit SEIVA_LOAD_DEMO.\n\
             {indent}let load_demo = config.env.is_development() || std::env::var(\"SEIVA_LOAD_DEMO\").is_ok();\n"
        )
    };
    let merge = format!(
        "{gate}{indent}// `{module}`: build the repo, load its seed (idempotent, always), then\n\
         {indent}// build the controller over it; load demo through the facade in dev.\n\
         {indent}let {module}_repo = {module}_infrastructure::InMemoryRecordRepository::new();\n\
         {indent}let _ = {module}_infrastructure::load_seed(&{module}_repo).await;\n\
         {indent}let {module}_controller = {module}_interface::{pascal}Controller::new(\n\
         {indent}    {module}_repo.clone(),\n\
         {indent}    {module}_infrastructure::CounterIdGenerator::default(),\n\
         {indent}    {module}_infrastructure::DirectUnitOfWork,\n\
         {indent}    {module}_infrastructure::LoggingEventPublisher,\n\
         {indent});\n\
         {indent}if load_demo {{\n\
         {indent}    let _ = {module}_interface::load_demo(&{module}_controller).await;\n\
         {indent}}}\n\
         {indent}let modules = modules.merge({module}_interface::{module}_router({module}_controller));\n"
    );
    Ok(main_rs.replacen(anchor_line, &format!("{merge}{anchor_line}"), 1))
}

// =============================================================================
// Module split / merge — promoting a module to its own microservice and back.
//
// A module is already five independent crates; "splitting" it does not move any
// code, it adds a *second* composition root — a standalone binary that mounts
// only that module — so the same module runs both inside the monolith
// (`apps/<project>-api`) and on its own (`apps/<module>-service`). `module.toml`
// records the chosen mode. Merging removes the service binary and resets the mode.
// This is the deployment knob behind "one codebase, monolith or microservices".
// =============================================================================

/// The workspace member path of a module's standalone service binary.
#[must_use]
pub fn service_member_path(module: &str) -> String {
    format!("apps/{module}-service")
}

/// Files for a module's standalone service binary, relative to the project root.
///
/// Emits `apps/<module>-service/` — a composition root over the runtime serve
/// loop that mounts **only** this module's generated router (the same in-memory
/// adapters the monolith wires by default), plus the framework routes
/// (`/health`, `/openapi.json`, `/docs`, `/redoc`, `/scalar`) and a distroless
/// `Delonixfile`. It links the module by path, so `seiva openapi` run against the
/// service documents exactly this module's contract.
///
/// The service also **owns its own data** (F5 / microservices decentralized data):
/// its `compose.yaml` brings up a database the service alone connects to, with a
/// per-service `DELONIX_ORM_DSN`, and a README spelling out the ownership. The
/// module's `seed/` (ADR 0022) loads into THIS service's store on boot.
pub fn service_files(module: &str) -> Result<Vec<GeneratedFile>, ScaffoldError> {
    validate_name(module)?;
    let app = format!("{module}-service");
    let base = format!("apps/{app}");
    let file = |path: String, contents: String| GeneratedFile { path, contents };
    Ok(vec![
        file(format!("{base}/Cargo.toml"), service_cargo_toml(module)),
        file(format!("{base}/src/main.rs"), service_main_rs(module)),
        // A split service is an HTTP server (it serves the module's router).
        file(
            format!("{base}/Delonixfile"),
            delonixfile(&app, ProjectKind::Api),
        ),
        // Database-per-service: this service owns its data, never the monolith's.
        file(format!("{base}/compose.yaml"), service_compose_yaml(module)),
        file(format!("{base}/README.md"), service_readme(module)),
    ])
}

/// The split service's `compose.yaml`: the service plus the database **it alone
/// owns** (F5 — microservices decentralized data management). The per-service
/// `DELONIX_ORM_DSN` points at that database; no other service connects to it.
fn service_compose_yaml(module: &str) -> String {
    format!(
        "# `{module}` service — run with `delonix stack up` (or `docker compose up`)\n\
         # from this directory. This service OWNS its data: its OWN database, never\n\
         # the monolith's and never another service's (decentralized data).\n\
         services:\n\
         \x20 {module}-service:\n\
         \x20   build:\n\
         \x20     # The service crate depends on the module crates by path, so the\n\
         \x20     # build context is the project root (two levels up).\n\
         \x20     context: ../..\n\
         \x20     dockerfile: apps/{module}-service/Delonixfile\n\
         \x20   image: {module}-service:dev\n\
         \x20   ports:\n\
         \x20     - \"8080:8080\"\n\
         \x20   environment:\n\
         \x20     APP_ENV: production\n\
         \x20     SEIVA_HTTP_ADDR: 0.0.0.0:8080\n\
         \x20     # The service's OWN datasource — a separate database from every\n\
         \x20     # other service. Turn on the `dorm` feature to persist to it; the\n\
         \x20     # default in-memory adapter ignores it.\n\
         \x20     DELONIX_ORM_DSN: postgres://{module}:{module}@{module}-db:5432/{module}\n\
         \x20   depends_on:\n\
         \x20     - {module}-db\n\
         \x20   restart: unless-stopped\n\
         \x20 # The database this service alone owns. No other service connects here.\n\
         \x20 {module}-db:\n\
         \x20   image: postgres:17\n\
         \x20   environment:\n\
         \x20     POSTGRES_USER: {module}\n\
         \x20     POSTGRES_PASSWORD: {module}\n\
         \x20     POSTGRES_DB: {module}\n\
         \x20   restart: unless-stopped\n"
    )
}

/// The split service's README, spelling out that it owns its own data.
fn service_readme(module: &str) -> String {
    format!(
        "# {module} service\n\n\
         A standalone Delonix Seiva service produced by `seiva module split {module}`.\n\
         It mounts ONLY the `{module}` module and **owns its own data** — a separate\n\
         database from the monolith and from every other service (microservices\n\
         decentralized data management). The datasource is `DELONIX_ORM_DSN`, and the\n\
         `compose.yaml` here brings this service up with its own `{module}-db`.\n\n\
         Run with `delonix stack up` from this directory. The default build uses the\n\
         in-memory adapter; turn on the `dorm` feature to persist to this service's\n\
         OWN database. The module's `seed/` reference data loads into THIS service's\n\
         store on boot, and `demo/` in dev (ADR 0022).\n\n\
         `seiva module merge {module}` removes this service and returns the module to\n\
         running inside the monolith.\n"
    )
}

/// The service binary's `Cargo.toml`: the framework crates plus this one module's
/// `interface`/`infrastructure` crates, linked by path. No managed anchors — a
/// single-module service mounts exactly one module, baked in.
fn service_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-service\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         # The service runtime: the serve loop, 12-factor config, structured logging\n\
         # and the shutdown signal — shared with the monolith composition root.\n\
         seiva-server = \"{FRAMEWORK_VERSION}\"\n\
         seiva-http = \"{FRAMEWORK_VERSION}\"\n\
         axum = \"0.8\"\n\
         # Emitting THIS service's contract (`seiva openapi`/`proto`, via SEIVA_EMIT).\n\
         delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
         seiva-openapi = \"{FRAMEWORK_VERSION}\"\n\
         seiva-grpc = \"{FRAMEWORK_VERSION}\"\n\
         serde_json = \"1\"\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         tracing = \"0.1\"\n\
         # The one module this service owns, linked by path:\n\
         {module}-interface = {{ path = \"../../modules/{module}/interface\" }}\n\
         {module}-infrastructure = {{ path = \"../../modules/{module}/infrastructure\" }}\n"
    )
}

/// The service binary's `main.rs`: a real Seiva server that mounts only this
/// module. Mirrors the monolith's composition root but with a single, baked-in
/// module merge instead of the managed anchor.
fn service_main_rs(module: &str) -> String {
    let pascal = to_pascal_case(module);
    let app = format!("{module}-service");
    format!(
        "//! Standalone service for the `{module}` module — its own composition root\n\
         //! and `main`. Produced by `seiva module split {module}`. It mounts only the\n\
         //! `{module}` module; the same module still runs inside the monolith\n\
         //! (`apps/<project>-api`). Keep business logic out of here.\n\n\
         use axum::Router;\n\
         use seiva_http::{{router_with_modules, Health, HttpSecurity}};\n\
         use seiva_server::{{observability, serve_router, shutdown_signal, ServerConfig}};\n\
         use std::process::ExitCode;\n\n\
         #[tokio::main(flavor = \"multi_thread\")]\n\
         async fn main() -> ExitCode {{\n\
         \x20   // `seiva openapi` / `seiva proto` run this binary with SEIVA_EMIT set to\n\
         \x20   // print this service's contract (just the `{module}` facade) and exit.\n\
         \x20   if let Ok(kind) = std::env::var(\"SEIVA_EMIT\") {{\n\
         \x20       return emit_contract(&kind);\n\
         \x20   }}\n\n\
         \x20   observability::init();\n\n\
         \x20   let config = match ServerConfig::from_process_env() {{\n\
         \x20       Ok(config) => config,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"invalid configuration\");\n\
         \x20           return ExitCode::FAILURE;\n\
         \x20       }}\n\
         \x20   }};\n\n\
         \x20   let health = Health::starting();\n\
         \x20   let security = HttpSecurity::default();\n\
         \x20   let auth = None;\n\
         \x20   // Seed + demo (ADR 0022): load `{module}`'s seed (idempotent upsert)\n\
         \x20   // on every boot, and its demo through the facade only in dev.\n\
         \x20   let load_demo = config.env.is_development() || std::env::var(\"SEIVA_LOAD_DEMO\").is_ok();\n\
         \x20   let {module}_repo = {module}_infrastructure::InMemoryRecordRepository::new();\n\
         \x20   let _ = {module}_infrastructure::load_seed(&{module}_repo).await;\n\
         \x20   let {module}_controller = {module}_interface::{pascal}Controller::new(\n\
         \x20       {module}_repo.clone(),\n\
         \x20       {module}_infrastructure::CounterIdGenerator::default(),\n\
         \x20       {module}_infrastructure::DirectUnitOfWork,\n\
         \x20       {module}_infrastructure::LoggingEventPublisher,\n\
         \x20   );\n\
         \x20   if load_demo {{\n\
         \x20       let _ = {module}_interface::load_demo(&{module}_controller).await;\n\
         \x20   }}\n\
         \x20   // This service mounts ONLY the `{module}` module (in-memory adapters).\n\
         \x20   let modules = Router::new().merge({module}_interface::{module}_router({module}_controller));\n\
         \x20   if std::env::var(\"SEIVA_SEED\").is_ok() {{\n\
         \x20       tracing::info!(sources = delonix_seiva::seed::registered_seeds().len(), \"seed/demo applied\");\n\
         \x20       return ExitCode::SUCCESS;\n\
         \x20   }}\n\
         \x20   let app = router_with_modules(health.clone(), &security, auth, modules);\n\n\
         \x20   tracing::info!(env = %config.env, http = %config.http_addr, \"{app} starting\");\n\
         \x20   match serve_router(config.http_addr, app, health, shutdown_signal()).await {{\n\
         \x20       Ok(()) => ExitCode::SUCCESS,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"serve failed\");\n\
         \x20           ExitCode::FAILURE\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n\n\
         /// Emits this service's contract from the `{module}` facade, then exits.\n\
         fn emit_contract(kind: &str) -> ExitCode {{\n\
         \x20   let facades = delonix_seiva::contract::registered_facades();\n\
         \x20   match kind {{\n\
         \x20       \"openapi\" => {{\n\
         \x20           let doc = seiva_openapi::openapi_document(\n\
         \x20               seiva_openapi::ApiInfo {{\n\
         \x20                   title: \"{module} service API\".to_string(),\n\
         \x20                   version: \"0.1.0\".to_string(),\n\
         \x20               }},\n\
         \x20               &facades,\n\
         \x20           );\n\
         \x20           match serde_json::to_string_pretty(&doc) {{\n\
         \x20               Ok(json) => {{\n\
         \x20                   println!(\"{{json}}\");\n\
         \x20                   ExitCode::SUCCESS\n\
         \x20               }}\n\
         \x20               Err(error) => {{\n\
         \x20                   eprintln!(\"seiva: {{error}}\");\n\
         \x20                   ExitCode::FAILURE\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       \"proto\" => {{\n\
         \x20           for facade in &facades {{\n\
         \x20               println!(\"{{}}\", seiva_grpc::proto_document(facade));\n\
         \x20           }}\n\
         \x20           ExitCode::SUCCESS\n\
         \x20       }}\n\
         \x20       other => {{\n\
         \x20           eprintln!(\"seiva: unknown SEIVA_EMIT kind `{{other}}`\");\n\
         \x20           ExitCode::FAILURE\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n"
    )
}

/// Adds a single workspace member `path` before the managed members anchor,
/// idempotently (a no-op if already listed). Pure.
pub fn add_app_member(cargo: &str, path: &str) -> Result<String, ScaffoldError> {
    if cargo.contains(&format!("\"{path}\"")) {
        return Err(ScaffoldError::AlreadyPresent(format!("member `{path}`")));
    }
    let anchor_line = cargo
        .lines()
        .find(|line| line.trim() == MEMBERS_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: MEMBERS_ANCHOR.to_owned(),
            what: "Cargo.toml".to_owned(),
        })?;
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    Ok(cargo.replacen(
        anchor_line,
        &format!("{indent}\"{path}\",\n{anchor_line}"),
        1,
    ))
}

/// Removes a single workspace member line for `path` (the inverse of
/// [`add_app_member`]). Pure; a no-op if the member is absent.
pub fn remove_app_member(cargo: &str, path: &str) -> String {
    cargo
        .lines()
        .filter(|line| line.trim().trim_end_matches(',').trim_matches('"') != path)
        .map(|l| format!("{l}\n"))
        .collect()
}

/// Sets the module's deployment mode in its `module.toml`, inserting a
/// `mode = "<mode>"` line under `[module]` or replacing the existing one. Pure.
pub fn set_module_mode(module_toml: &str, mode: &str) -> Result<String, ScaffoldError> {
    let line = format!("mode = \"{mode}\"");
    if module_toml
        .lines()
        .any(|l| l.trim_start().starts_with("mode ="))
    {
        return Ok(module_toml
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("mode =") {
                    let indent: String = l.chars().take_while(|c| c.is_whitespace()).collect();
                    format!("{indent}{line}\n")
                } else {
                    format!("{l}\n")
                }
            })
            .collect());
    }
    let anchor = module_toml
        .lines()
        .find(|l| l.trim() == "[module]")
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: "[module]".to_owned(),
            what: "module.toml".to_owned(),
        })?;
    Ok(module_toml.replacen(anchor, &format!("{anchor}\n{line}"), 1))
}

/// The declarative integration contract for an optional Seiva extension (ADR
/// 0010): pure data describing how a composition root wires the extension in — a
/// dependency to add and a mount statement to insert at the managed composition
/// point. A tool (`seiva ext add`, or the independent DX) reads this and applies
/// the edits; it is **never** a `build.rs` or an install script, and declaring
/// the dependency alone never mounts routes — the mount is explicit and recompiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionManifest {
    /// The extension's crate name (the `dx add` / `cargo add` argument).
    pub crate_name: &'static str,
    /// The version requirement added to the composition root's `[dependencies]`.
    pub version_req: &'static str,
    /// The statement inserted at the managed composition point, shadowing `app`.
    pub mount_stmt: &'static str,
    /// A one-line human description for the CLI.
    pub summary: &'static str,
}

/// The first-party admin console's integration contract.
pub const CONSOLE_ADMIN_EXTENSION: ExtensionManifest = ExtensionManifest {
    crate_name: "seiva-console-admin",
    version_req: "0.1",
    // Mounts into `modules` BEFORE `router_with_modules` layers them, so /admin is
    // covered by authn/security-headers/rate-limit (see the composition point).
    mount_stmt: "let modules = seiva_console_admin::mount(modules);",
    summary: "The optional admin console at /admin (ADR 0008/0009).",
};

/// The first-party extensions `seiva ext add` knows how to wire, by crate name.
/// Kept tiny and first-party on purpose: `seiva` fetches and executes **nothing**
/// to discover how to install an extension. A third-party extension ships the
/// same shape as a `seiva-extension.toml`, which the independent DX reads.
#[must_use]
pub fn extension_manifest(name: &str) -> Option<ExtensionManifest> {
    match name {
        "seiva-console-admin" => Some(CONSOLE_ADMIN_EXTENSION),
        _ => None,
    }
}

/// Adds `ext`'s dependency to the app crate's `[dependencies]` at the managed
/// extension anchor. **Idempotent**: if the crate is already a dependency the
/// text is returned unchanged. Returns [`ScaffoldError::MissingAnchor`] when the
/// manifest is not a recognised Seiva composition root (so the caller can emit a
/// manual patch instead). Pure.
pub fn add_extension_to_app_manifest(
    cargo: &str,
    ext: &ExtensionManifest,
) -> Result<String, ScaffoldError> {
    let dep_key = format!("{} =", ext.crate_name);
    if cargo.lines().any(|l| l.trim_start().starts_with(&dep_key)) {
        return Ok(cargo.to_owned()); // already declared — idempotent no-op
    }
    let anchor_line = cargo
        .lines()
        .find(|line| line.trim() == APP_EXTENSION_DEPS_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: APP_EXTENSION_DEPS_ANCHOR.to_owned(),
            what: "apps/<name>-api/Cargo.toml".to_owned(),
        })?;
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let addition = format!("{indent}{} = \"{}\"\n", ext.crate_name, ext.version_req);
    Ok(cargo.replacen(anchor_line, &format!("{addition}{anchor_line}"), 1))
}

/// Inserts `ext`'s mount statement at the managed composition point in `main.rs`.
/// **Idempotent**: if the mount is already present the text is returned
/// unchanged. Returns [`ScaffoldError::MissingAnchor`] on an unrecognised project.
/// Pure.
pub fn mount_extension_in_main(
    main_rs: &str,
    ext: &ExtensionManifest,
) -> Result<String, ScaffoldError> {
    if main_rs.contains(ext.mount_stmt) {
        return Ok(main_rs.to_owned()); // already mounted — idempotent no-op
    }
    let anchor_line = main_rs
        .lines()
        .find(|line| line.trim() == EXTENSIONS_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: EXTENSIONS_ANCHOR.to_owned(),
            what: "apps/<name>-api/src/main.rs".to_owned(),
        })?;
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let addition = format!("{indent}{}\n", ext.mount_stmt);
    Ok(main_rs.replacen(anchor_line, &format!("{addition}{anchor_line}"), 1))
}

/// Registers a module in `seiva.toml` by appending a `[modules.<name>]` section.
/// Pure.
pub fn register_module_in_seiva_toml(seiva: &str, module: &str) -> Result<String, ScaffoldError> {
    if seiva.contains(&format!("[modules.{module}]")) {
        return Err(ScaffoldError::AlreadyPresent(format!("module `{module}`")));
    }
    let sep = if seiva.ends_with('\n') { "" } else { "\n" };
    Ok(format!(
        "{seiva}{sep}\n[modules.{module}]\ndata = \"dorm\"\n"
    ))
}

/// Appends a `namespace <module> {{ }}` block to the project schema. Pure.
pub fn append_namespace_block(schema: &str, module: &str) -> Result<String, ScaffoldError> {
    if schema.contains(&format!("namespace {module} "))
        || schema.contains(&format!("namespace {module}{{"))
    {
        return Err(ScaffoldError::AlreadyPresent(format!(
            "namespace `{module}`"
        )));
    }
    let sep = if schema.ends_with('\n') { "" } else { "\n" };
    Ok(format!(
        "{schema}{sep}\nnamespace {module} {{\n  // Models owned by the `{module}` module. Only identifiers cross module\n  // boundaries: no relation to another namespace's models.\n  model Record {{\n    id      Uuid   @id\n    title   String\n    version Int    @version\n  }}\n}}\n"
    ))
}

/// Adapts the Seiva project schema (`schema/<project>.schema`) into a schema the
/// DORM data layer migrates from (`data/dorm.schema`). The two share a DSL — DORM
/// supports `namespace` blocks natively (that is how it imposes the module
/// boundary) — so the module `namespace`s and their models pass through verbatim.
/// Two adjustments make it a valid DORM schema:
///
/// 1. **The datasource URL.** DORM's migrate reads the DSN from the datasource's
///    `url`; the project schema omits it (the DSN is environment-supplied), so a
///    `url = env("DELONIX_ORM_DSN")` is injected when absent.
/// 2. **`@version`.** The scaffold marks the optimistic-concurrency column with
///    `@version`, which this DORM's schema parser does not accept yet; it is
///    dropped so the schema parses. The `version` column itself stays — only the
///    attribute is removed (the concurrency enforcement is a runtime concern).
/// 3. **The datasource provider.** Set to `provider` (`"postgresql"`, `"mysql"`,
///    `"sqlite"`) so DORM compiles the right dialect for the chosen backend.
///
/// Pure, so `seiva db init`/`dev` can write the result and the transform is tested
/// without a database.
#[must_use]
pub fn dorm_schema_from_project(project_schema: &str, provider: &str) -> String {
    // The project schema is authored for `postgresql`; retarget the datasource.
    let mut out = project_schema.replace(
        "provider = \"postgresql\"",
        &format!("provider = \"{provider}\""),
    );
    let anchor = "datasource db {\n";
    if !out.contains("url ") && !out.contains("url=") {
        if let Some(pos) = out.find(anchor) {
            out.insert_str(
                pos + anchor.len(),
                "  url      = env(\"DELONIX_ORM_DSN\")\n",
            );
        }
    }
    out = out.replace("@version", "");
    out
}

// --- templates ---------------------------------------------------------------

fn seiva_toml(name: &str, kind: ProjectKind) -> String {
    format!(
        "# Delonix Seiva project manifest. Managed partly by the seiva CLI.\n\
         [project]\n\
         name = \"{name}\"\n\
         # The project's nature, chosen at `seiva project new --kind` (ADR 0024):\n\
         # api | worker | gateway | event. It shaped the composition root in\n\
         # apps/{name}-api/src/main.rs.\n\
         kind = \"{kind}\"\n\n\
         [data]\n\
         # DORM is the default data layer; set to \"external\" to own it yourself.\n\
         layer = \"dorm\"\n\n\
         # Modules are registered below by `seiva module new`.\n",
        kind = kind.as_str(),
    )
}

fn workspace_cargo_toml(app: &str) -> String {
    format!(
        "# Workspace managed by the seiva CLI. Module crates are added below the\n\
         # managed anchor by `seiva module new`.\n\
         [workspace]\n\
         resolver = \"2\"\n\
         members = [\n\
         \x20   \"apps/{app}\",\n\
         \x20   \"shared/kernel\",\n\
         \x20   {MEMBERS_ANCHOR}\n\
         ]\n"
    )
}

/// The composition-root `Cargo.toml` for the kind. The `api` kind carries the
/// HTTP/contract stack and the managed anchors; the others pull only what their
/// runtime shape needs, so a worker/event image takes on no axum surface.
fn bin_cargo_toml(app: &str, kind: ProjectKind) -> String {
    let header = format!(
        "[package]\nname = \"{app}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
    );
    let body = match kind {
        ProjectKind::Api => format!(
            "# The service runtime: the serve loop, 12-factor config, structured\n\
             # logging and the shutdown signal. The composition root reuses it instead\n\
             # of writing its own `main`.\n\
             seiva-server = \"{FRAMEWORK_VERSION}\"\n\
             # The HTTP application builder (`router_with_modules` + health +\n\
             # /openapi.json + /docs); merge your modules' routers through it below.\n\
             seiva-http = \"{FRAMEWORK_VERSION}\"\n\
             # The composition root holds the merged module routes as an `axum::Router`.\n\
             axum = \"0.8\"\n\
             # Emitting THIS project's contract (`seiva openapi`/`proto`, via SEIVA_EMIT):\n\
             # the facade registry and the OpenAPI/proto generators, driven by the\n\
             # linked modules.\n\
             delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
             seiva-openapi = \"{FRAMEWORK_VERSION}\"\n\
             seiva-grpc = \"{FRAMEWORK_VERSION}\"\n\
             serde_json = \"1\"\n\
             tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
             tracing = \"0.1\"\n\
             # Optional extensions `seiva ext add` wires in (e.g. seiva-console-admin):\n\
             {APP_EXTENSION_DEPS_ANCHOR}\n\
             # Module crates `seiva module new` wires into the composition root:\n\
             {APP_MODULE_DEPS_ANCHOR}\n"
        ),
        ProjectKind::Worker => format!(
            "# A background worker: the worker-host loop + the shutdown signal from\n\
             # the runtime, 12-factor `AppEnv` from the config crate. No HTTP surface.\n\
             delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
             seiva-server = \"{FRAMEWORK_VERSION}\"\n\
             seiva-config = \"{FRAMEWORK_VERSION}\"\n\
             tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
             tracing = \"0.1\"\n"
        ),
        ProjectKind::Event => format!(
            "# An event-processor: the worker-host loop drains the in-process event\n\
             # bus each pass. No HTTP surface. Wire real brokers via the DORM outbox.\n\
             delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
             seiva-server = \"{FRAMEWORK_VERSION}\"\n\
             seiva-config = \"{FRAMEWORK_VERSION}\"\n\
             seiva-events = \"{FRAMEWORK_VERSION}\"\n\
             seiva-messaging = \"{FRAMEWORK_VERSION}\"\n\
             seiva-ports = \"{FRAMEWORK_VERSION}\"\n\
             # DEV-ONLY in-memory consume-once dedup (the inbox). Replace with the\n\
             # DORM-backed durable inbox in production so a restart does not replay.\n\
             seiva-testing = \"{FRAMEWORK_VERSION}\"\n\
             async-trait = \"0.1\"\n\
             serde_json = \"1\"\n\
             tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
             tracing = \"0.1\"\n"
        ),
        ProjectKind::Gateway => format!(
            "# An HTTP edge/gateway shell: the serve loop + the HTTP application\n\
             # builder (health, security headers, authn), with NO modules of its own.\n\
             # Add an HTTP client (reqwest/hyper) yourself to call upstream services.\n\
             seiva-server = \"{FRAMEWORK_VERSION}\"\n\
             seiva-http = \"{FRAMEWORK_VERSION}\"\n\
             axum = \"0.8\"\n\
             tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
             tracing = \"0.1\"\n"
        ),
    };
    format!("{header}{body}")
}

/// The composition root `main.rs`: a real Seiva server over the runtime's serve
/// loop. It reads 12-factor config, installs JSON logging, builds the HTTP
/// application (health, `/openapi.json`, `/docs`) and drains gracefully on
/// SIGTERM. A new module's router is merged into `app` at the marked line.
fn app_main_rs(name: &str) -> String {
    let app = format!("{name}-api");
    format!(
        "//! Composition root for `{name}`: the service's `main`. `seiva module new`\n\
         //! wires each module's router in at the managed anchor below; keep business\n\
         //! logic out.\n\n\
         use axum::Router;\n\
         use seiva_http::{{router_with_modules, Health, HttpSecurity}};\n\
         use seiva_server::{{observability, serve_router, shutdown_signal, ServerConfig}};\n\
         use std::process::ExitCode;\n\n\
         #[tokio::main(flavor = \"multi_thread\")]\n\
         async fn main() -> ExitCode {{\n\
         \x20   // `seiva openapi` / `seiva proto` run this binary with SEIVA_EMIT set to\n\
         \x20   // print THIS project's generated contract — from the facades its modules\n\
         \x20   // register at link time — and exit, instead of serving. Done before\n\
         \x20   // logging so stdout carries only the document.\n\
         \x20   if let Ok(kind) = std::env::var(\"SEIVA_EMIT\") {{\n\
         \x20       return emit_contract(&kind);\n\
         \x20   }}\n\n\
         \x20   // Structured JSON logs to stdout before anything else.\n\
         \x20   observability::init();\n\n\
         \x20   let config = match ServerConfig::from_process_env() {{\n\
         \x20       Ok(config) => config,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"invalid configuration\");\n\
         \x20           return ExitCode::FAILURE;\n\
         \x20       }}\n\
         \x20   }};\n\n\
         \x20   let health = Health::starting();\n\
         \x20   // Transport security posture: secure response headers on, same-origin\n\
         \x20   // only (no CORS) until you add allowed origins.\n\
         \x20   let security = HttpSecurity::default();\n\
         \x20   // No authenticator wired yet, so guarded routes answer 403; supply an\n\
         \x20   // `Arc<dyn seiva_http::Authenticator>` (see `seiva-identity`) to admit\n\
         \x20   // callers. `None` leaves public routes serving.\n\
         \x20   let auth = None;\n\
         \x20   // Module routers are merged here, BEFORE the framework layers wrap them —\n\
         \x20   // `router_with_modules` merges then layers, so authn, security headers\n\
         \x20   // and the rate limit apply to module routes too. `seiva module new`\n\
         \x20   // inserts each module's merge at the anchor below.\n\
         \x20   let modules = Router::new();\n\
         \x20   {MODULE_MERGES_ANCHOR}\n\
         \x20   // Optional extensions merge into `modules` HERE — the managed\n\
         \x20   // composition point — BEFORE `router_with_modules` wraps everything,\n\
         \x20   // so an extension's routes (e.g. the admin console's /admin) get\n\
         \x20   // authn (the `Principal` populated), the security headers and the\n\
         \x20   // rate limit like any module route. `seiva ext add <name>` inserts\n\
         \x20   // each extension's mount at the anchor below — explicit and\n\
         \x20   // recompiled, never a silent hook. (Admin console: ADR 0008/0009/0010.)\n\
         \x20   {EXTENSIONS_ANCHOR}\n\
         \x20   // `seiva db seed` runs this binary with SEIVA_SEED set: the modules above\n\
         \x20   // have already applied their seed (and demo, in dev); report and exit\n\
         \x20   // before serving, so seeding is a one-shot that never starts the server.\n\
         \x20   if std::env::var(\"SEIVA_SEED\").is_ok() {{\n\
         \x20       let sources = delonix_seiva::seed::registered_seeds();\n\
         \x20       tracing::info!(sources = sources.len(), \"seed/demo sources applied\");\n\
         \x20       return ExitCode::SUCCESS;\n\
         \x20   }}\n\
         \x20   let app = router_with_modules(health.clone(), &security, auth, modules);\n\n\
         \x20   tracing::info!(env = %config.env, http = %config.http_addr, \"{app} starting\");\n\
         \x20   match serve_router(config.http_addr, app, health, shutdown_signal()).await {{\n\
         \x20       Ok(()) => ExitCode::SUCCESS,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"serve failed\");\n\
         \x20           ExitCode::FAILURE\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n\n\
         /// Emits this project's generated contract from the facades linked into this\n\
         /// binary (the modules merged above), then exits. Driven by `seiva openapi`.\n\
         fn emit_contract(kind: &str) -> ExitCode {{\n\
         \x20   let facades = delonix_seiva::contract::registered_facades();\n\
         \x20   match kind {{\n\
         \x20       \"openapi\" => {{\n\
         \x20           let doc = seiva_openapi::openapi_document(\n\
         \x20               seiva_openapi::ApiInfo {{\n\
         \x20                   title: \"{name} API\".to_string(),\n\
         \x20                   version: \"0.1.0\".to_string(),\n\
         \x20               }},\n\
         \x20               &facades,\n\
         \x20           );\n\
         \x20           match serde_json::to_string_pretty(&doc) {{\n\
         \x20               Ok(json) => {{\n\
         \x20                   println!(\"{{json}}\");\n\
         \x20                   ExitCode::SUCCESS\n\
         \x20               }}\n\
         \x20               Err(error) => {{\n\
         \x20                   eprintln!(\"seiva: {{error}}\");\n\
         \x20                   ExitCode::FAILURE\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       \"proto\" => {{\n\
         \x20           for facade in &facades {{\n\
         \x20               println!(\"{{}}\", seiva_grpc::proto_document(facade));\n\
         \x20           }}\n\
         \x20           ExitCode::SUCCESS\n\
         \x20       }}\n\
         \x20       other => {{\n\
         \x20           eprintln!(\"seiva: unknown SEIVA_EMIT value `{{other}}` (expected `openapi` or `proto`)\");\n\
         \x20           ExitCode::FAILURE\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n"
    )
}

/// The `worker` composition root: a background service over the framework's
/// worker host (ADR 0024). No HTTP surface — a work loop that drains gracefully
/// on SIGTERM. The `pass` body is a stub the developer replaces with the job.
fn worker_main_rs(name: &str) -> String {
    format!(
        "//! Composition root for `{name}`: a background WORKER (kind `worker`, ADR 0024).\n\
         //! It runs a work loop on the framework's worker host and drains gracefully\n\
         //! on SIGTERM. No HTTP surface — replace the `pass` body with the real job.\n\n\
         use std::process::ExitCode;\n\
         use std::time::Duration;\n\n\
         use delonix_seiva::worker::{{run_worker, WorkerConfig}};\n\
         use seiva_config::AppEnv;\n\
         use seiva_server::{{observability, shutdown_signal}};\n\n\
         #[tokio::main(flavor = \"multi_thread\")]\n\
         async fn main() -> ExitCode {{\n\
         \x20   observability::init();\n\
         \x20   let env = match AppEnv::from_process_env() {{\n\
         \x20       Ok(env) => env,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"invalid configuration\");\n\
         \x20           return ExitCode::FAILURE;\n\
         \x20       }}\n\
         \x20   }};\n\
         \x20   tracing::info!(env = %env.as_str(), \"{name} worker starting\");\n\n\
         \x20   // One pass every 30s. It returns how many items it processed; an Err is\n\
         \x20   // logged and NEVER fatal — the worker outlives a transient failure.\n\
         \x20   let config = WorkerConfig::new(\"{name}\", Duration::from_secs(30));\n\
         \x20   let report = run_worker(config, shutdown_signal(), || async {{\n\
         \x20       // TODO: one unit of background work (poll a queue, run a job, ...).\n\
         \x20       tracing::info!(\"worker pass: nothing to do yet\");\n\
         \x20       Ok::<u64, Box<dyn std::error::Error + Send + Sync + 'static>>(0)\n\
         \x20   }})\n\
         \x20   .await;\n\n\
         \x20   tracing::info!(\n\
         \x20       passes = report.passes,\n\
         \x20       processed = report.processed,\n\
         \x20       errors = report.errors,\n\
         \x20       \"{name} worker stopped\",\n\
         \x20   );\n\
         \x20   ExitCode::SUCCESS\n\
         }}\n"
    )
}

/// The `gateway` composition root: an HTTP edge/BFF shell (ADR 0024). It serves
/// the framework surface (health, security headers, authn) with NO modules of its
/// own; the developer adds upstream-aggregation routes where marked.
fn gateway_main_rs(name: &str) -> String {
    format!(
        "//! Composition root for `{name}`: an HTTP edge/GATEWAY shell (kind `gateway`,\n\
         //! ADR 0024). It serves the framework surface (/health, security headers,\n\
         //! authn) with NO modules of its own. Add your upstream-aggregation routes\n\
         //! where marked (bring an HTTP client such as reqwest/hyper). Keep business\n\
         //! logic in the services behind the gateway, not here.\n\n\
         use axum::Router;\n\
         use seiva_http::{{router_with_modules, Health, HttpSecurity}};\n\
         use seiva_server::{{observability, serve_router, shutdown_signal, ServerConfig}};\n\
         use std::process::ExitCode;\n\n\
         #[tokio::main(flavor = \"multi_thread\")]\n\
         async fn main() -> ExitCode {{\n\
         \x20   observability::init();\n\
         \x20   let config = match ServerConfig::from_process_env() {{\n\
         \x20       Ok(config) => config,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"invalid configuration\");\n\
         \x20           return ExitCode::FAILURE;\n\
         \x20       }}\n\
         \x20   }};\n\
         \x20   let health = Health::starting();\n\
         \x20   let security = HttpSecurity::default();\n\
         \x20   let auth = None;\n\
         \x20   // A gateway hosts NO modules of its own. Build your upstream-aggregation\n\
         \x20   // routes here and merge them into `routes` BEFORE `router_with_modules`\n\
         \x20   // layers authn, security headers and the rate limit over them.\n\
         \x20   let routes = Router::new();\n\
         \x20   let app = router_with_modules(health.clone(), &security, auth, routes);\n\n\
         \x20   tracing::info!(env = %config.env, http = %config.http_addr, \"{name} gateway starting\");\n\
         \x20   match serve_router(config.http_addr, app, health, shutdown_signal()).await {{\n\
         \x20       Ok(()) => ExitCode::SUCCESS,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"serve failed\");\n\
         \x20           ExitCode::FAILURE\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n"
    )
}

/// The `event` composition root: an event-processor (ADR 0024). A worker that
/// drains the in-process event bus each pass and dispatches to registered
/// subscribers (choreography). The in-memory outbox/inbox are the dev default;
/// swap them for the DORM transactional outbox + a durable inbox in production.
fn event_main_rs(name: &str) -> String {
    format!(
        "//! Composition root for `{name}`: an EVENT-PROCESSOR (kind `event`, ADR 0024).\n\
         //! A worker that drains the in-process event bus each pass and dispatches to\n\
         //! the registered subscribers — the same choreography a monolith runs. The\n\
         //! in-memory outbox/inbox are the DEV default; in production swap them for\n\
         //! the DORM transactional outbox and a durable inbox.\n\n\
         use std::process::ExitCode;\n\
         use std::sync::Arc;\n\
         use std::time::Duration;\n\n\
         use async_trait::async_trait;\n\
         use delonix_seiva::worker::{{run_worker, WorkerConfig}};\n\
         use seiva_config::AppEnv;\n\
         use seiva_events::{{drain_in_process, EventSubscriber, InMemoryOutbox, SubscriberRegistry}};\n\
         use seiva_messaging::CloudEvent;\n\
         use seiva_ports::PortResult;\n\
         use seiva_server::{{observability, shutdown_signal}};\n\
         // DEV-ONLY in-memory consume-once dedup. Replace with the DORM-backed inbox\n\
         // (a durable `IdempotencyStore`) in production so a restart does not replay.\n\
         use seiva_testing::InMemoryIdempotencyStore;\n\n\
         /// An example subscriber: it reacts to one event type. Register one per\n\
         /// `(event type, reaction)`; the `subscriber_id` is the stable half of the\n\
         /// consume-once key, so keep it unique and stable across restarts.\n\
         struct ExampleSubscriber;\n\n\
         #[async_trait]\n\
         impl EventSubscriber for ExampleSubscriber {{\n\
         \x20   fn subscriber_id(&self) -> &str {{\n\
         \x20       \"{name}.example\"\n\
         \x20   }}\n\n\
         \x20   async fn handle(&self, event: &CloudEvent) -> PortResult<()> {{\n\
         \x20       // TODO: react to the event (call a use case, update state, ...).\n\
         \x20       tracing::info!(ty = %event.ty, id = %event.id, \"handling event\");\n\
         \x20       Ok(())\n\
         \x20   }}\n\
         }}\n\n\
         #[tokio::main(flavor = \"multi_thread\")]\n\
         async fn main() -> ExitCode {{\n\
         \x20   observability::init();\n\
         \x20   let env = match AppEnv::from_process_env() {{\n\
         \x20       Ok(env) => env,\n\
         \x20       Err(error) => {{\n\
         \x20           tracing::error!(%error, \"invalid configuration\");\n\
         \x20           return ExitCode::FAILURE;\n\
         \x20       }}\n\
         \x20   }};\n\
         \x20   tracing::info!(env = %env.as_str(), \"{name} event-processor starting\");\n\n\
         \x20   // The event bus: an outbox to read from, an inbox to dedup, and the\n\
         \x20   // routing table of subscribers. Your ingestion publishes into `outbox`\n\
         \x20   // (or, with DORM, a use case writes it transactionally); each pass drains.\n\
         \x20   let outbox = Arc::new(InMemoryOutbox::new());\n\
         \x20   let inbox = Arc::new(InMemoryIdempotencyStore::new());\n\
         \x20   let mut registry = SubscriberRegistry::new();\n\
         \x20   registry.register(\"{name}.example\", Arc::new(ExampleSubscriber));\n\
         \x20   let registry = Arc::new(registry);\n\n\
         \x20   // Each pass drains pending events through the subscribers. Cloning the\n\
         \x20   // Arcs per pass keeps the worker's `FnMut` free of borrow conflicts.\n\
         \x20   let config = WorkerConfig::new(\"{name}\", Duration::from_secs(5));\n\
         \x20   let report = run_worker(config, shutdown_signal(), || {{\n\
         \x20       let outbox = Arc::clone(&outbox);\n\
         \x20       let inbox = Arc::clone(&inbox);\n\
         \x20       let registry = Arc::clone(&registry);\n\
         \x20       async move {{\n\
         \x20           let drained = drain_in_process(&*outbox, &registry, &*inbox).await?;\n\
         \x20           Ok::<u64, Box<dyn std::error::Error + Send + Sync + 'static>>(drained as u64)\n\
         \x20       }}\n\
         \x20   }})\n\
         \x20   .await;\n\n\
         \x20   tracing::info!(\n\
         \x20       passes = report.passes,\n\
         \x20       processed = report.processed,\n\
         \x20       errors = report.errors,\n\
         \x20       \"{name} event-processor stopped\",\n\
         \x20   );\n\
         \x20   ExitCode::SUCCESS\n\
         }}\n"
    )
}

/// A distroless, multi-stage build recipe for the app binary. The final image
/// carries only the binary and its runtime libraries — no shell, no package
/// manager — and runs as a non-root user. Dockerfile-compatible, so `delonix`,
/// `docker` or `podman` all build it.
fn delonixfile(app: &str, kind: ProjectKind) -> String {
    // HTTP kinds (api, gateway) bind a port and expose it; worker/event run
    // headless, so they carry no `SEIVA_HTTP_ADDR`/`EXPOSE`.
    let http_env = if kind.serves_http() {
        "# Bind on all interfaces inside the container; the platform maps the port.\n\
         ENV SEIVA_HTTP_ADDR=0.0.0.0:8080\n"
    } else {
        ""
    };
    let expose = if kind.serves_http() {
        "EXPOSE 8080\n\
         # `:nonroot` already runs as uid 65532. Distroless has no shell for a CMD\n\
         # healthcheck; Kubernetes uses an httpGet readiness probe on /health/ready.\n"
    } else {
        "# A headless worker: no port. `:nonroot` already runs as uid 65532;\n\
         # Kubernetes uses a liveness probe (process-based) rather than httpGet.\n"
    };
    format!(
        "# syntax=docker/dockerfile:1\n\
         # Multi-stage build producing a distroless, non-root image for `{app}`.\n\n\
         FROM rust:1-slim AS builder\n\
         WORKDIR /build\n\
         COPY . .\n\
         RUN cargo build --release --bin {app}\n\n\
         # Distroless: only the binary and its runtime libs; no shell, no apt.\n\
         FROM gcr.io/distroless/cc-debian12:nonroot AS runtime\n\
         WORKDIR /app\n\
         COPY --from=builder /build/target/release/{app} /app/{app}\n\
         {http_env}\
         ENV APP_ENV=production\n\
         {expose}\
         USER nonroot\n\
         ENTRYPOINT [\"/app/{app}\"]\n"
    )
}

/// A `compose.yaml` that builds and runs the service with `delonix stack up`
/// (or `docker compose up`). Optional backing services (brokers, identity) live
/// in `compose/*.yaml` and are opted in explicitly, never on by default.
fn compose_yaml(name: &str, kind: ProjectKind) -> String {
    // HTTP kinds map a port and set SEIVA_HTTP_ADDR; worker/event run headless.
    let ports_env = if kind.serves_http() {
        "\x20   ports:\n\
         \x20     - \"8080:8080\"\n\
         \x20   environment:\n\
         \x20     APP_ENV: production\n\
         \x20     SEIVA_HTTP_ADDR: 0.0.0.0:8080\n\
         \x20   # Distroless carries no shell for a CMD healthcheck; readiness is an\n\
         \x20   # httpGet on /health/ready (Kubernetes) — see docs/first-day.md.\n"
    } else {
        "\x20   environment:\n\
         \x20     APP_ENV: production\n\
         \x20   # A headless worker: no port. Liveness is process-based in Kubernetes.\n"
    };
    format!(
        "# Run with `delonix stack up` (or `docker compose up`).\n\
         # Optional backing services live in `compose/*.yaml`; opt in explicitly,\n\
         # e.g. `delonix stack up -f compose.yaml -f compose/postgres.yaml`.\n\
         services:\n\
         \x20 {name}:\n\
         \x20   build:\n\
         \x20     context: .\n\
         \x20     dockerfile: Delonixfile\n\
         \x20   image: {name}:dev\n\
         {ports_env}\
         \x20   restart: unless-stopped\n\
         # Placeholder for opt-in backing services; nothing is enabled by default.\n\
         # To add one: create compose/<service>.yaml and pass it with -f.\n"
    )
}

/// The build-context excludes: never ship build output, VCS or secrets into the
/// image layer.
fn dockerignore() -> String {
    "/target\n**/*.rs.bk\n.git\n.env\n".to_owned()
}

/// A library crate manifest with path dependencies `(package_name, rel_path)`.
fn lib_cargo_toml(pkg: &str, deps: &[&(String, String)]) -> String {
    let mut out = format!(
        "[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
    );
    for (name, path) in deps {
        out.push_str(&format!("{name} = {{ path = \"{path}\" }}\n"));
    }
    out
}

fn module_toml(module: &str) -> String {
    format!(
        "[module]\nname = \"{module}\"\nfacade = \"v1\"\nowners = []\n\
         # Deployment mode: \"library\" (runs inside the monolith) or \"service\"\n\
         # (also runs standalone as apps/{module}-service). Toggled by\n\
         # `seiva module split {module}` / `seiva module merge {module}`.\n\
         mode = \"library\"\n\n\
         # Facades this module calls. Each needs a matching [[allow]] on the provider.\n\
         # [[uses]]\n# module = \"identity\"\n# facade = \"v1\"\n# methods = [\"user_exists\"]\n\n\
         # Callers this module permits. Anything not listed is denied.\n\
         # [[allow]]\n# caller = \"notes\"\n# methods = [\"...\"]\n"
    )
}

/// The crates.io version requirement a generated project uses for the framework
/// crates (`delonix-seiva = "0.1"`, …). Generated manifests depend on the
/// framework by version, so a project compiles standalone once the crates are
/// published; it is not tied to a local framework checkout.
const FRAMEWORK_VERSION: &str = "0.1";

/// The framework crates a generated project names as dependencies
/// (`delonix-seiva = "0.1"`, …). Every other framework crate they pull in is
/// reached transitively, through these crates' own `path`/`workspace`
/// dependencies, so redirecting just these to a local checkout resolves the
/// whole graph (see [`workspace_patch_section`]). `seiva-testing` is a
/// dev-dependency (the module's use-case test asserts against its UnitOfWork/
/// EventPublisher doubles), so it is patched too — `cargo test` must resolve it.
pub const FRAMEWORK_CRATES: [&str; 11] = [
    "delonix-seiva",
    "seiva-contract",
    "seiva-http",
    "seiva-ports",
    "seiva-openapi",
    "seiva-grpc",
    "seiva-server",
    "seiva-testing",
    // Named directly by the worker/event composition roots (ADR 0024): the
    // 12-factor `AppEnv`, the in-process event bus and its CloudEvent envelope.
    // Patched like the rest.
    "seiva-config",
    "seiva-events",
    "seiva-messaging",
];

/// A `[patch.crates-io]` section redirecting every [`FRAMEWORK_CRATES`] entry to
/// its directory under `<framework_root>/crates`, so a generated project compiles
/// against a local framework checkout *before* the crates are published to
/// crates.io. This is the interim that unblocks a standalone `cargo build`: the
/// manifests keep the published contract (`delonix-seiva = "0.1"`), and this one
/// section — deletable in a single edit once the crates are on crates.io —
/// points those versions at the checkout.
///
/// `framework_root` is written verbatim into the manifest; pass an absolute path
/// (forward slashes, which Cargo accepts on every platform) so the project builds
/// regardless of where it is generated. The CLI resolves it; a fitness test
/// resolves its own. Pure: the caller appends the returned text to the generated
/// workspace `Cargo.toml`.
#[must_use]
pub fn workspace_patch_section(framework_root: &str) -> String {
    let root = framework_root.replace('\\', "/");
    let mut out = String::from(
        "\n\
         # INTERIM — remove once the framework crates are published to crates.io.\n\
         # The manifests above depend on the framework by version (`= \"0.1\"`); until\n\
         # those versions exist on crates.io, this redirects them to a local\n\
         # framework checkout so `cargo build` resolves standalone. Regenerate with\n\
         # `SEIVA_FRAMEWORK_DIR=/path/to/seiva` to point elsewhere.\n\
         [patch.crates-io]\n",
    );
    for krate in FRAMEWORK_CRATES {
        out.push_str(&format!(
            "{krate} = {{ path = \"{root}/crates/{krate}\" }}\n"
        ));
    }
    out
}

/// Converts a validated snake_case name to PascalCase for the facade trait and
/// its generated descriptor/router names (`order_items` → `OrderItems`).
fn to_pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// The REST resource segment the controller derives from a facade name, for the
/// route shown in a generated doc comment. Delegates to the SAME authoritative
/// rule the runtime uses ([`crate::contract::resource_segment`]) instead of a
/// second, naive one — so the doc comment matches the real route exactly
/// (`Category` → `categories`, not `categorys`), with one source of truth (F6).
fn resource_of(pascal: &str) -> String {
    crate::contract::resource_segment(pascal)
}

fn facade_lib(module: &str, pascal: &str) -> String {
    let resource = resource_of(pascal);
    format!(
        "//! Facade layer: the module's versioned public contract and wire DTOs.\n\
         //!\n\
         //! Naming note: this is the module's INBOUND PORT / Published Language, not\n\
         //! the GoF Facade pattern — \"facade\" is the project's term for the one\n\
         //! versioned, permission-guarded contract others see.\n\
         //!\n\
         //! This is what other modules and the HTTP/gRPC transports see. The\n\
         //! `#[seiva::facade]` macro (imported here as `facade`) keeps the trait as\n\
         //! the single source of truth and generates the `{pascal}Facade`\n\
         //! descriptor read by the OpenAPI/proto generators and the contract checks.\n\
         //!\n\
         //! The generated controller uses the facade's OWN DTOs as the wire types,\n\
         //! so the write DTO derives `serde` and implements `seiva_contract::Validate`,\n\
         //! the response DTO derives `serde::Serialize`, and the error maps to an\n\
         //! RFC 9457 `seiva_contract::Problem` — all transport-neutral contract types\n\
         //! (ADR 0003), so this layer depends on `delonix-seiva`, `serde` and\n\
         //! `seiva-contract`, and NOT on any transport crate. The HTTP edge renders\n\
         //! the `Problem` as `application/problem+json`; another transport maps it\n\
         //! its own way.\n\
         #![allow(async_fn_in_trait)]\n\n\
         use delonix_seiva::{{facade, WireSchema}};\n\
         use seiva_contract::{{Problem, Validate, ValidationError}};\n\
         use serde::{{Deserialize, Serialize}};\n\n\
         /// Command DTO: the JSON body of `create`, validated by the controller's\n\
         /// `Valid<T>` extractor. Rename `Record`/`title` to the real shape.\n\
         /// `WireSchema` publishes its real fields to the OpenAPI document.\n\
         #[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]\n\
         pub struct CreateRecord {{\n    /// The record's title; must be non-empty after trimming.\n    pub title: String,\n}}\n\n\
         impl Validate for CreateRecord {{\n    fn validate(&self) -> Result<(), ValidationError> {{\n        if self.title.trim().is_empty() {{\n            return Err(ValidationError::new(\"title must not be empty\"));\n        }}\n        Ok(())\n    }}\n}}\n\n\
         /// Response DTO: what `create` returns. Never a domain type — the\n\
         /// application layer maps the domain entity to this at the boundary.\n\
         #[derive(Debug, Clone, PartialEq, Eq, Serialize, WireSchema)]\n\
         pub struct RecordDto {{\n    /// The record's identifier.\n    pub id: u128,\n    /// The record's title.\n    pub title: String,\n}}\n\n\
         /// The facade's error type. The controller maps it to an RFC 9457 problem\n\
         /// via this `Into<Problem>`, so the module owns how failures surface.\n\
         #[derive(Debug, Clone)]\n\
         pub struct RecordError(pub String);\n\n\
         impl From<RecordError> for Problem {{\n    fn from(error: RecordError) -> Self {{\n        Problem::unprocessable_entity(error.0)\n    }}\n}}\n\n\
         /// The `{module}` module's versioned public contract.\n\
         #[facade(version = \"v1\")]\n\
         pub trait {pascal} {{\n    /// Create a record (guarded; `POST /v1/{resource}`).\n    #[requires(\"{module}.create\")]\n    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError>;\n}}\n"
    )
}

fn domain_lib() -> String {
    "//! Domain layer: entities, value objects and rules.\n\
     //!\n\
     //! Depends on nothing but `shared/kernel`. The rule — a record's title must\n\
     //! not be empty — lives here, not in the application or the database. Must\n\
     //! NOT depend on any other layer, the framework or DORM.\n\n\
     /// A record. Construct it through [`Record::new`] so the rule is enforced.\n\
     #[derive(Debug, Clone, PartialEq, Eq)]\n\
     pub struct Record {\n    /// The record's identifier.\n    pub id: u128,\n    /// The record's title.\n    pub title: String,\n}\n\n\
     impl Record {\n    /// Creates a record, rejecting an empty or whitespace-only title.\n    pub fn new(id: u128, title: impl Into<String>) -> Result<Self, DomainError> {\n        let title = title.into();\n        if title.trim().is_empty() {\n            return Err(DomainError::EmptyTitle);\n        }\n        Ok(Self { id, title })\n    }\n}\n\n\
     /// A domain rule was violated.\n\
     #[derive(Debug, Clone, PartialEq, Eq)]\n\
     pub enum DomainError {\n    /// The record's title was empty or whitespace only.\n    EmptyTitle,\n}\n\n\
     impl std::fmt::Display for DomainError {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        match self {\n            DomainError::EmptyTitle => write!(f, \"record title must not be empty\"),\n        }\n    }\n}\n\n\
     impl std::error::Error for DomainError {}\n\n\
     #[cfg(test)]\n\
     mod tests {\n    use super::*;\n\n    #[test]\n    fn rejects_empty_title() {\n        assert_eq!(Record::new(1, \"   \"), Err(DomainError::EmptyTitle));\n        assert!(Record::new(1, \"hello\").is_ok());\n    }\n}\n"
        .to_owned()
}

fn application_lib(snake: &str, pascal: &str) -> String {
    format!(
        "//! Application layer: the service implementing the facade over an outbound\n\
         //! repository port and the framework's cross-cutting `seiva-ports`\n\
         //! (`UnitOfWork`, `IdGenerator`, `EventPublisher`).\n\
         //!\n\
         //! Depends on `domain`, this module's `facade` (DTOs + trait) and\n\
         //! `seiva-ports`. It names NO transport, framework-runtime or DORM item —\n\
         //! the HTTP coupling lives in `interface`, the persistence in\n\
         //! `infrastructure`. This is what keeps the use case independently testable.\n\
         //!\n\
         //! The write use case is ONE transaction (ADR 0021): the save AND the\n\
         //! domain-event publish run inside `UnitOfWork::transaction`, so with the\n\
         //! DORM gateway the event lands in the transactional outbox in the SAME\n\
         //! transaction as the business change — they commit together or not at all.\n\
         #![allow(async_fn_in_trait)]\n\n\
         use std::future::Future;\n\n\
         use {snake}_domain::Record;\n\
         use {snake}_facade::{{CreateRecord, RecordDto, RecordError, {pascal}}};\n\
         use seiva_ports::{{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork}};\n\n\
         /// Outbound port: persistence of records in the domain's language.\n\
         /// Implemented in `infrastructure` (in-memory by default, DORM-backed via\n\
         /// the `dorm` feature). The future is `Send` so the use case can `.await`\n\
         /// it inside a `UnitOfWork::transaction` on a multi-threaded runtime.\n\
         pub trait RecordRepository {{\n    /// Persists a record.\n    fn save(&self, record: &Record) -> impl Future<Output = PortResult<()>> + Send;\n}}\n\n\
         /// The application service: it implements the [`{pascal}`] facade by running\n\
         /// the use case and mapping the domain entity to the DTO. Generic over the\n\
         /// repository port and the cross-cutting ports (`IdGenerator`, `UnitOfWork`,\n\
         /// `EventPublisher`), so it unit-tests with the `seiva-testing` doubles.\n\
         pub struct RecordService<R, G, U, E> {{\n    /// The outbound persistence port.\n    pub repository: R,\n    /// The framework id generator.\n    pub ids: G,\n    /// The unit of work: one transaction per use case.\n    pub uow: U,\n    /// The domain-event publisher (the transactional outbox, with DORM).\n    pub events: E,\n}}\n\n\
         impl<R, G, U, E> RecordService<R, G, U, E>\n\
         where\n    R: RecordRepository + Sync,\n    G: IdGenerator,\n    U: UnitOfWork,\n    E: EventPublisher,\n{{\n    async fn create_record(&self, cmd: CreateRecord) -> PortResult<RecordDto> {{\n        // Validate in the domain first — no point opening a transaction for a\n        // command the rule will reject.\n        let id = u128::from_be_bytes(self.ids.new_id().0);\n        let record = Record::new(id, cmd.title).map_err(BoxError::from)?;\n\n        // The event payload is a JSON envelope — the DORM gateway's outbox\n        // publisher parses the bytes as JSON before persisting them, so a\n        // non-JSON payload would fail against the real data layer.\n        let event = serde_json::to_vec(&serde_json::json!({{\n            \"id\": record.id.to_string(),\n            \"title\": record.title.clone(),\n        }}))\n        .map_err(BoxError::from)?;\n\n        // One transaction per use case (ADR 0021): the save AND the event publish\n        // run inside it, so data and event commit atomically with the DORM gateway.\n        // A module reacts to another's event in the composition root, never by a\n        // direct call — which is what keeps `seiva module split` a composition change.\n        self.uow\n            .transaction(|| async {{\n                self.repository.save(&record).await?;\n                self.events.publish(\"{snake}.record.created\", &event).await\n            }})\n            .await?;\n\n        Ok(RecordDto {{\n            id: record.id,\n            title: record.title,\n        }})\n    }}\n}}\n\n\
         impl<R, G, U, E> {pascal} for RecordService<R, G, U, E>\n\
         where\n    R: RecordRepository + Sync,\n    G: IdGenerator,\n    U: UnitOfWork,\n    E: EventPublisher,\n{{\n    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError> {{\n        // Map the internal error to the contract's error type: no internal\n        // detail leaks across the module boundary beyond a message.\n        self.create_record(cmd)\n            .await\n            .map_err(|err| RecordError(err.to_string()))\n    }}\n}}\n\n\
         #[cfg(test)]\n\
         mod tests {{\n    use super::*;\n    use seiva_testing::{{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator, TxOutcome}};\n    use std::sync::Mutex;\n\n    #[derive(Default)]\n    struct FakeRepo {{\n        saved: Mutex<Vec<Record>>,\n    }}\n\n    impl RecordRepository for FakeRepo {{\n        async fn save(&self, record: &Record) -> PortResult<()> {{\n            self.saved.lock().unwrap().push(record.clone());\n            Ok(())\n        }}\n    }}\n\n    fn service(\n    ) -> RecordService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher> {{\n        RecordService {{\n            repository: FakeRepo::default(),\n            ids: SequenceIdGenerator::new(),\n            uow: InMemoryUnitOfWork::new(),\n            events: RecordingEventPublisher::new(),\n        }}\n    }}\n\n    #[tokio::test]\n    async fn create_persists_commits_and_emits() {{\n        let service = service();\n        let dto = service\n            .create(CreateRecord {{ title: \"hello\".into() }})\n            .await\n            .unwrap();\n        assert_eq!(dto.id, 1);\n        assert_eq!(dto.title, \"hello\");\n        assert_eq!(service.repository.saved.lock().unwrap().len(), 1);\n        // The use case committed exactly one transaction and emitted the event.\n        assert_eq!(service.uow.outcomes(), [TxOutcome::Committed]);\n        assert_eq!(service.events.topics(), [\"{snake}.record.created\"]);\n    }}\n\n    #[tokio::test]\n    async fn empty_title_is_rejected_before_any_transaction() {{\n        let service = service();\n        let result = service.create(CreateRecord {{ title: \"   \".into() }}).await;\n        assert!(result.is_err());\n        // Nothing committed or emitted when the domain rule rejected the command.\n        assert!(service.uow.outcomes().is_empty());\n        assert!(service.events.topics().is_empty());\n    }}\n}}\n"
    )
}

fn infrastructure_lib(snake: &str) -> String {
    format!(
        "//! Infrastructure layer: adapters implementing the application's outbound\n\
         //! ports. Depends on `application` + `domain` + `seiva-ports` (and\n\
         //! `delonix-seiva` for the seed loader); never on `interface`.\n\
         //!\n\
         //! The default adapters are in-memory, so a scaffolded project compiles and\n\
         //! runs with NO database. A DORM-backed repository arrives via the `dorm`\n\
         //! feature (the `seiva-data-dorm` gateway) — swap this type for it in the\n\
         //! composition root when you turn the feature on. The adapters are `Clone`\n\
         //! (state behind an `Arc`) so the transport can hold the service as axum\n\
         //! state.\n\
         //!\n\
         //! The repository is keyed by id, so `save` is an **idempotent upsert** —\n\
         //! which is what makes the seed loader ([`load_seed`]) safe to run on every\n\
         //! boot without duplicating rows (ADR 0022).\n\n\
         use std::collections::HashMap;\n\
         use std::future::Future;\n\
         use std::pin::Pin;\n\
         use std::sync::atomic::{{AtomicU64, Ordering}};\n\
         use std::sync::{{Arc, Mutex}};\n\n\
         use async_trait::async_trait;\n\
         use {snake}_application::RecordRepository;\n\
         use {snake}_domain::Record;\n\
         use seiva_ports::{{EventPublisher, Id, IdGenerator, PortResult, UnitOfWork}};\n\n\
         /// An in-memory [`RecordRepository`], keyed by id so a repeated `save` of\n\
         /// the same id overwrites (upsert). Replace with the DORM-backed adapter\n\
         /// (feature `dorm`) or an external one for production.\n\
         #[derive(Clone, Default)]\n\
         pub struct InMemoryRecordRepository {{\n    by_id: Arc<Mutex<HashMap<u128, Record>>>,\n}}\n\n\
         impl InMemoryRecordRepository {{\n    /// Creates an empty repository.\n    pub fn new() -> Self {{\n        Self::default()\n    }}\n\n    /// Every record saved so far, ordered by id — for composition-level assertions.\n    pub fn saved(&self) -> Vec<Record> {{\n        let guard = self.by_id.lock().expect(\"repository poisoned\");\n        let mut records: Vec<Record> = guard.values().cloned().collect();\n        records.sort_by_key(|record| record.id);\n        records\n    }}\n}}\n\n\
         impl RecordRepository for InMemoryRecordRepository {{\n    async fn save(&self, record: &Record) -> PortResult<()> {{\n        // Upsert by id: idempotent, so seeding the same key twice is a no-op.\n        self.by_id\n            .lock()\n            .map_err(|_| \"repository lock poisoned\")?\n            .insert(record.id, record.clone());\n        Ok(())\n    }}\n}}\n\n\
         /// The `seed/` reference data, embedded at build time (ADR 0022). Declared\n\
         /// in the link-time seed registry so a fitness test and `seiva db seed` can\n\
         /// discover it; applied by [`load_seed`] in the composition root.\n\
         const SEED_CSV: &str = include_str!(\"../../seed/record.csv\");\n\n\
         delonix_seiva::register_seed!(delonix_seiva::seed::SeedSource {{\n    module: \"{snake}\",\n    resource: \"record\",\n    csv: SEED_CSV,\n    demo: false,\n}});\n\n\
         /// Loads the `seed/` reference data into `repo` through an idempotent upsert\n\
         /// (no domain events). The row's `id` column is a stable textual key hashed\n\
         /// to a deterministic entity id, so re-running overwrites rather than\n\
         /// duplicating. Returns the number of rows loaded.\n\
         pub async fn load_seed(repo: &InMemoryRecordRepository) -> usize {{\n    let mut loaded = 0;\n    for row in delonix_seiva::seed::parse_csv(SEED_CSV) {{\n        let (Some(key), Some(title)) = (row.get(\"id\"), row.get(\"title\")) else {{\n            continue;\n        }};\n        let id = delonix_seiva::seed::external_u128(\"{snake}.record\", key);\n        match Record::new(id, title.clone()) {{\n            Ok(record) => {{\n                if repo.save(&record).await.is_ok() {{\n                    loaded += 1;\n                }}\n            }}\n            Err(error) => eprintln!(\"seiva: skipping invalid seed row `{{key}}`: {{error}}\"),\n        }}\n    }}\n    loaded\n}}\n\n\
         /// A process-local monotonic [`IdGenerator`] adapter: it hands out\n\
         /// 1, 2, 3, … as 128-bit ids. Replace with a ULID/UUID generator in\n\
         /// production; this keeps the scaffold dependency-free and deterministic.\n\
         #[derive(Clone, Default)]\n\
         pub struct CounterIdGenerator {{\n    next: Arc<AtomicU64>,\n}}\n\n\
         impl IdGenerator for CounterIdGenerator {{\n    fn new_id(&self) -> Id {{\n        let value = self.next.fetch_add(1, Ordering::Relaxed) + 1;\n        let mut bytes = [0u8; 16];\n        bytes[8..].copy_from_slice(&value.to_be_bytes());\n        Id(bytes)\n    }}\n}}\n\n\
         /// A pass-through [`UnitOfWork`]: it runs the use-case body directly, with no\n\
         /// real transaction. The DORM gateway's unit of work (feature `dorm`) wraps\n\
         /// the body in a real database transaction and the transactional outbox, so\n\
         /// the save and the event commit atomically; this keeps the scaffold runnable\n\
         /// with no database while the use case already codes against the port.\n\
         #[derive(Clone, Default)]\n\
         pub struct DirectUnitOfWork;\n\n\
         impl UnitOfWork for DirectUnitOfWork {{\n    fn transaction<'a, T, F, Fut>(\n        &'a self,\n        work: F,\n    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>\n    where\n        F: FnOnce() -> Fut + Send + 'a,\n        Fut: Future<Output = PortResult<T>> + Send + 'a,\n        T: Send + 'a,\n    {{\n        Box::pin(async move {{ work().await }})\n    }}\n}}\n\n\
         /// An [`EventPublisher`] that logs the emitted event so a dev run shows the\n\
         /// use case fired it. The DORM gateway (feature `dorm`) writes it to the\n\
         /// transactional outbox instead, and the relay worker forwards it to the\n\
         /// broker; module choreography is wired in the composition root, not here.\n\
         #[derive(Clone, Default)]\n\
         pub struct LoggingEventPublisher;\n\n\
         #[async_trait]\n\
         impl EventPublisher for LoggingEventPublisher {{\n    async fn publish(&self, topic: &str, _payload: &[u8]) -> PortResult<()> {{\n        tracing::info!(topic, \"domain event emitted\");\n        Ok(())\n    }}\n}}\n"
    )
}

fn interface_lib(snake: &str, pascal: &str, admin: bool) -> String {
    // When `seiva module new --admin` is used, the interface registers an admin
    // resource (ADR 0009) so the optional console surfaces it. This couples the
    // module only to the framework contract (`delonix-seiva`), NOT to the admin UI
    // crate; with the console unmounted it is inert link-time data. A module
    // scaffolded without `--admin` emits nothing here and stays private to the
    // console.
    let admin_block = if admin {
        format!(
            "use delonix_seiva::admin::{{AdminPermissions, AdminResource}};\n\
             use delonix_seiva::register_admin;\n\n\
             // Make this module visible to the optional admin console (ADR 0009).\n\
             // The resource id matches the module's route segment (`/v1/{snake}/...`)\n\
             // so the console pairs it with the operations in `/openapi.json`, and\n\
             // each permission matches the facade's `#[requires(..)]`. Grant\n\
             // `{snake}.read` to view the resource and `{snake}.create` to use the\n\
             // create form; add `change`/`delete` permissions as you add operations.\n\
             register_admin!(AdminResource {{\n    \
                 namespace: \"{snake}\",\n    \
                 resource: \"{snake}\",\n    \
                 label: \"{pascal}\",\n    \
                 facade: \"{pascal}\",\n    \
                 version: \"v1\",\n    \
                 list_fields: &[\"id\", \"title\"],\n    \
                 search_fields: &[\"title\"],\n    \
                 filters: &[],\n    \
                 ordering: &[\"id\"],\n    \
                 page_size: 25,\n    \
                 permissions: AdminPermissions {{\n        \
                     view: \"{snake}.read\",\n        \
                     create: \"{snake}.create\",\n        \
                     change: \"\",\n        \
                     delete: \"\",\n    \
                 }},\n\
             }});\n\n"
        )
    } else {
        String::new()
    };
    format!(
        "//! Interface layer: the HTTP/gRPC transport. It wires this module's facade\n\
         //! to the real framework transports.\n\
         //!\n\
         //! LAYERING DECISION: the `#[seiva::controller]` lives HERE, not in\n\
         //! `application`, so the application layer stays transport-free. The\n\
         //! controller [`{pascal}Controller<R, G, U, E>`] is GENERIC over the four\n\
         //! ports (repository, id generator, unit of work, event publisher), so this\n\
         //! layer depends only on `application` (the service and the `RecordRepository`\n\
         //! port) and NEVER on `infrastructure` — the layering rule forbids\n\
         //! `interface -> infrastructure`. A concrete self type is still what\n\
         //! `#[seiva::controller]` dispatches through, but `{pascal}Controller<R, G, U, E>`\n\
         //! is concrete once the ports are chosen, and the macro propagates the\n\
         //! generics + their bounds onto the generated router.\n\
         //!\n\
         //! The macro generates a `{snake}_router(service)` builder whose route\n\
         //! version and per-route permission come from the facade descriptor at run\n\
         //! time. The COMPOSITION ROOT in `apps/` picks the concrete adapters and\n\
         //! calls `{snake}_router({pascal}Controller::new(repo, ids, uow, events))`. The same\n\
         //! descriptor drives this module's OpenAPI document ([`openapi`]), so the\n\
         //! wire contract is generated, never hand-maintained.\n\n\
         use std::sync::Arc;\n\n\
         use delonix_seiva::contract::Facade;\n\
         use delonix_seiva::controller;\n\
         use seiva_ports::{{EventPublisher, IdGenerator, UnitOfWork}};\n\
         use {snake}_application::{{RecordRepository, RecordService}};\n\
         use {snake}_facade::{{CreateRecord, RecordDto, RecordError, {pascal}, {pascal}Facade}};\n\n\
         {admin_block}\
         /// Transport adapter: a concrete `{pascal}` service exposed over HTTP,\n\
         /// generic over the repository port `R` and the id-generator port `G`.\n\
         /// Held behind an [`Arc`] so it is `Clone + Send + Sync + 'static` — the\n\
         /// bound the generated `{snake}_router` places on its axum state — without\n\
         /// requiring the service or its ports to be `Clone`.\n\
         pub struct {pascal}Controller<R, G, U, E>(Arc<RecordService<R, G, U, E>>);\n\n\
         // Manual `Clone` (not derived): the controller is `Clone` for ANY port set\n\
         // because it only holds an `Arc`; the derive would wrongly demand each port\n\
         // be `Clone`.\n\
         impl<R, G, U, E> Clone for {pascal}Controller<R, G, U, E> {{\n    fn clone(&self) -> Self {{\n        Self(Arc::clone(&self.0))\n    }}\n}}\n\n\
         impl<R, G, U, E> {pascal}Controller<R, G, U, E> {{\n    /// Builds the controller over the given ports (repository, id generator, unit\n    /// of work, event publisher). The composition root in `apps/` chooses them\n    /// (the in-memory adapters, or DORM-backed ones); the interface never names a\n    /// concrete adapter.\n    pub fn new(repository: R, ids: G, uow: U, events: E) -> Self {{\n        {pascal}Controller(Arc::new(RecordService {{\n            repository,\n            ids,\n            uow,\n            events,\n        }}))\n    }}\n}}\n\n\
         #[controller]\n\
         impl<R, G, U, E> {pascal} for {pascal}Controller<R, G, U, E>\n\
         where\n    R: RecordRepository + Send + Sync + 'static,\n    G: IdGenerator + Send + Sync + 'static,\n    U: UnitOfWork + Send + Sync + 'static,\n    E: EventPublisher + Send + Sync + 'static,\n{{\n    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError> {{\n        self.0.create(cmd).await\n    }}\n}}\n\n\
         /// The module's OpenAPI 3.1 document, generated from the facade descriptor.\n\
         /// (A proto3 document is available from the same descriptor via\n\
         /// `seiva proto`; it is not generated per-module to keep this crate free\n\
         /// of the gRPC generator.)\n\
         pub fn openapi() -> serde_json::Value {{\n    seiva_openapi::openapi_document(\n        seiva_openapi::ApiInfo {{\n            title: \"{snake} API\".to_string(),\n            version: {pascal}Facade::DESCRIPTOR.version.to_string(),\n        }},\n        &[{pascal}Facade::DESCRIPTOR],\n    )\n}}\n\n\
         /// The `demo/` demonstration data, embedded at build time (ADR 0022).\n\
         /// Declared in the seed registry as a DEMO source and applied by\n\
         /// [`load_demo`] — in DEV ONLY — through the facade, so each row runs the\n\
         /// real use case (the domain invariants, and the event where the facade\n\
         /// publishes one). It never ships to production.\n\
         const DEMO_CSV: &str = include_str!(\"../../demo/record.csv\");\n\n\
         delonix_seiva::register_seed!(delonix_seiva::seed::SeedSource {{\n    module: \"{snake}\",\n    resource: \"record\",\n    csv: DEMO_CSV,\n    demo: true,\n}});\n\n\
         /// Loads the `demo/` data through the facade. The composition root calls\n\
         /// this only when `config.env.is_development()`, so demo data never reaches\n\
         /// production. Invalid rows are reported and skipped, never fatal; returns\n\
         /// the number of rows accepted.\n\
         pub async fn load_demo<R, G, U, E>(controller: &{pascal}Controller<R, G, U, E>) -> usize\n\
         where\n    R: RecordRepository + Send + Sync + 'static,\n    G: IdGenerator + Send + Sync + 'static,\n    U: UnitOfWork + Send + Sync + 'static,\n    E: EventPublisher + Send + Sync + 'static,\n{{\n    let mut loaded = 0;\n    for row in delonix_seiva::seed::parse_csv(DEMO_CSV) {{\n        let Some(title) = row.get(\"title\") else {{\n            continue;\n        }};\n        match controller.create(CreateRecord {{ title: title.clone() }}).await {{\n            Ok(_) => loaded += 1,\n            Err(error) => eprintln!(\"seiva: skipping demo row: {{}}\", error.0),\n        }}\n    }}\n    loaded\n}}\n"
    )
}

// --- module crate manifests --------------------------------------------------

fn facade_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-facade\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         # The framework macros (`#[facade]`) and contract descriptor. The project\n\
         # depends on the framework by version (`delonix-seiva = \"0.1\"`); until those\n\
         # crates are published, the workspace root's `[patch.crates-io]` redirects\n\
         # them to a local checkout. The data layer is off here — the facade needs none.\n\
         delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
         # The write DTO's `Validate` AND the RFC 9457 `Problem` the error maps to are\n\
         # transport-neutral contract types, so they come from `seiva-contract` (no\n\
         # axum) — the facade declares its contract WITHOUT naming any transport crate\n\
         # (ADR 0003). The HTTP edge wraps `Problem` in `seiva_http::ProblemResponse`.\n\
         seiva-contract = \"{FRAMEWORK_VERSION}\"\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\n"
    )
}

fn domain_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-domain\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         # The domain depends on nothing but the standard library (and, as the\n\
         # module grows, `shared/kernel`). No framework, no DORM.\n\
         [dependencies]\n"
    )
}

fn application_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-application\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         {module}-domain = {{ path = \"../domain\" }}\n\
         {module}-facade = {{ path = \"../facade\" }}\n\
         # The trait-only cross-cutting ports. The application codes against these,\n\
         # NEVER against `delonix-seiva`, `seiva-http` or DORM — that is what keeps\n\
         # it transport- and data-layer-independent.\n\
         seiva-ports = \"{FRAMEWORK_VERSION}\"\n\
         # The domain event is a JSON envelope the use case serialises inside the\n\
         # transaction (the DORM outbox parses it as JSON).\n\
         serde_json = \"1\"\n\n\
         [dev-dependencies]\n\
         tokio = {{ version = \"1\", features = [\"rt\", \"macros\"] }}\n\
         # The in-memory UnitOfWork/EventPublisher/IdGenerator doubles the use-case\n\
         # test asserts against (commit outcomes + emitted topics), no DB or broker.\n\
         seiva-testing = \"{FRAMEWORK_VERSION}\"\n"
    )
}

fn infrastructure_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-infrastructure\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         {module}-application = {{ path = \"../application\" }}\n\
         {module}-domain = {{ path = \"../domain\" }}\n\
         seiva-ports = \"{FRAMEWORK_VERSION}\"\n\
         # The seed loader: `register_seed!`, `parse_csv` and the deterministic\n\
         # external-id hashing (ADR 0022). The data layer stays off here.\n\
         delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
         # `#[async_trait]` for the in-memory EventPublisher adapter; `tracing` so a\n\
         # dev run shows the emitted event. Both drop out with the DORM gateway.\n\
         async-trait = \"0.1\"\n\
         tracing = \"0.1\"\n\
         # A DORM-backed repository arrives through the `seiva-data-dorm` gateway\n\
         # behind a `dorm` feature; the default in-memory adapter pulls in no ORM.\n"
    )
}

fn interface_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-interface\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         {module}-facade = {{ path = \"../facade\" }}\n\
         # The controller is generic over the repository and id-generator ports, so\n\
         # the interface depends on `application` (the service + the repository\n\
         # port) but NEVER on `infrastructure` — the layering rule forbids\n\
         # `interface -> infrastructure`. The composition root in `apps/` injects\n\
         # the concrete adapters.\n\
         {module}-application = {{ path = \"../application\" }}\n\
         # The controller macro + contract, the HTTP primitives it emits, and the\n\
         # OpenAPI generator — all driven by the facade descriptor.\n\
         delonix-seiva = {{ version = \"{FRAMEWORK_VERSION}\", default-features = false }}\n\
         seiva-http = \"{FRAMEWORK_VERSION}\"\n\
         seiva-openapi = \"{FRAMEWORK_VERSION}\"\n\
         seiva-ports = \"{FRAMEWORK_VERSION}\"\n\
         axum = \"0.8\"\n\
         serde_json = \"1\"\n"
    )
}

fn project_schema() -> String {
    "// Project schema: one `namespace` block per in-process module, added by\n\
     // `seiva module new`. Generated blocks are edited by their module owners only.\n\
     datasource db {\n  provider = \"postgresql\"\n}\n"
        .to_owned()
}

/// The `seed/record.csv` reference-data fixture (ADR 0022). First column `id` is
/// the stable textual key hashed to a deterministic entity id, so the seed is
/// idempotent across reboots. Edit the columns to match the real wire shape.
fn seed_csv() -> String {
    "# Reference data for the `record` resource — loaded on every boot (idempotent).\n\
     # `id` is a stable textual key; same key => same row (upsert, no duplicates).\n\
     id,title\n\
     welcome,Welcome record\n"
        .to_owned()
}

/// The `demo/record.csv` demonstration fixture (ADR 0022). Loaded only in dev,
/// through the facade, so each row exercises the domain invariants.
fn demo_csv() -> String {
    "# Demonstration data — loaded ONLY in dev (APP_ENV=development), via the facade.\n\
     id,title\n\
     demo_first,First demo record\n\
     demo_second,Second demo record\n"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation() {
        assert!(validate_name("notes").is_ok());
        assert!(validate_name("order_items").is_ok());
        assert!(validate_name("Notes").is_err());
        assert!(validate_name("1notes").is_err());
        assert!(validate_name("notes_").is_err());
        assert!(validate_name("order__items").is_err());
        assert!(validate_name("order-items").is_err());
    }

    #[test]
    fn project_files_cover_the_skeleton() {
        let files = project_files("shop", ProjectKind::Api).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"seiva.toml"));
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"schema/shop.schema"));
        assert!(paths.contains(&"apps/shop-api/Cargo.toml"));
        assert!(paths.contains(&"shared/kernel/src/lib.rs"));
        // Cloud-native packaging is part of the skeleton.
        assert!(paths.contains(&"Delonixfile"));
        assert!(paths.contains(&"compose.yaml"));
        assert!(paths.contains(&".dockerignore"));
    }

    /// The generated project is cloud-native-ready: the composition root is a
    /// real server (not a stub), and the image recipe is a distroless,
    /// multi-stage, non-root build. This is the fitness function for Part D.
    #[test]
    fn project_is_cloud_native_ready() {
        let files = project_files("shop", ProjectKind::Api).unwrap();
        let get = |path: &str| {
            files
                .iter()
                .find(|f| f.path == path)
                .unwrap_or_else(|| panic!("missing {path}"))
                .contents
                .as_str()
        };

        // The app is a real Seiva server, not the old println! stub.
        let main = get("apps/shop-api/src/main.rs");
        assert!(main.contains("serve_router"), "app must run the serve loop");
        assert!(
            main.contains("observability::init"),
            "app must log structured"
        );
        assert!(main.contains("Health::starting"), "app must expose health");
        // The old hello-world stub is gone: this is a real serve loop, not a
        // greeting. (`println!` legitimately appears now, in the SEIVA_EMIT path
        // that prints the OpenAPI document.)
        assert!(!main.contains("Hello"), "the hello-world stub must be gone");
        assert!(
            main.contains("serve_router") && main.contains("router_with_modules"),
            "the composition root runs the real serve loop"
        );

        // The composition root merges modules BEFORE the framework layers wrap
        // them, via `router_with_modules` — so authn/security/rate-limit apply to
        // module routes. The `router(health).merge(..)` shape (merge AFTER
        // layering, leaving module routes uncovered) must not appear, not even in
        // a comment that a developer would copy.
        assert!(
            main.contains("router_with_modules(health.clone(), &security, auth, modules)"),
            "app must compose through router_with_modules"
        );
        assert!(
            !main.contains("router(health).merge("),
            "the merge-after-layering shape must be gone"
        );

        // The Delonixfile is distroless, multi-stage and non-root.
        let dockerfile = get("Delonixfile");
        assert!(dockerfile.contains("AS builder"), "multi-stage");
        assert!(
            dockerfile.contains("distroless"),
            "final image must be distroless"
        );
        assert!(dockerfile.contains("USER nonroot"), "must not run as root");
        assert!(dockerfile.contains("cargo build --release --bin shop-api"));

        // Compose builds from the Delonixfile and maps the port.
        let compose = get("compose.yaml");
        assert!(compose.contains("dockerfile: Delonixfile"));
        assert!(compose.contains("8080:8080"));
    }

    /// The framework workspace root, resolved from this crate's manifest dir
    /// (`<root>/crates/delonix-seiva`). The fitness build points the generated
    /// project's `[patch.crates-io]` here.
    fn framework_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("manifest dir is <root>/crates/delonix-seiva")
            .to_path_buf()
    }

    /// Writes `files` (paths relative to `root`), creating parent directories.
    fn write_all(root: &std::path::Path, files: &[GeneratedFile]) {
        for file in files {
            let full = root.join(&file.path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(&full, &file.contents).expect("write generated file");
        }
    }

    /// The fitness function for the standalone-compile blocker: generate a
    /// project AND a module, redirect the framework crates to this checkout with
    /// the same `[patch.crates-io]` the CLI emits, and actually run `cargo build`.
    ///
    /// This COMPILES the generated code — it is not a string assertion on the
    /// manifests — so it catches drift between the templates and the real
    /// framework API (a renamed `router_with_modules`, a changed controller
    /// macro, a moved `seiva-ports` item) that a manifest-text check cannot. It
    /// exercises both the app composition root and all five module layers.
    ///
    /// A persistent `CARGO_TARGET_DIR` keeps reruns fast; the project tree itself
    /// is a fresh temp dir removed on the way in and out.
    #[test]
    fn generated_project_compiles_standalone() {
        let root = framework_root();
        let work =
            std::env::temp_dir().join(format!("seiva-scaffold-fitness-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).expect("create work dir");

        // Project skeleton, then a module, then the workspace registration edit
        // the CLI performs so the module crates are built too.
        let mut project = project_files("demo", ProjectKind::Api).expect("project files");
        let modules = module_files("notes", false).expect("module files");
        let cargo = project
            .iter()
            .find(|f| f.path == "Cargo.toml")
            .expect("workspace manifest")
            .contents
            .clone();
        let registered = add_members_to_workspace(&cargo, "notes").expect("register module");
        // The interim patch redirecting `= "0.1"` to this checkout — exactly what
        // `seiva project new` appends.
        let with_patch = format!(
            "{registered}{}",
            workspace_patch_section(&root.to_string_lossy())
        );
        project
            .iter_mut()
            .find(|f| f.path == "Cargo.toml")
            .expect("workspace manifest")
            .contents = with_patch;

        // Wire the module into the composition root exactly as `seiva module new`
        // does — the app manifest deps and the main.rs seed+merge block — so the
        // boot-time seed/demo path (ADR 0022) is actually compiled, not just the
        // module crates in isolation.
        {
            let app_cargo = project
                .iter_mut()
                .find(|f| f.path == "apps/demo-api/Cargo.toml")
                .expect("app manifest");
            app_cargo.contents =
                add_module_to_app_manifest(&app_cargo.contents, "notes").expect("app dep wire");
        }
        {
            let app_main = project
                .iter_mut()
                .find(|f| f.path == "apps/demo-api/src/main.rs")
                .expect("app main");
            app_main.contents =
                wire_module_in_main(&app_main.contents, "notes").expect("main wire");
        }

        write_all(&work, &project);
        write_all(&work, &modules);

        let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let target_dir = std::env::temp_dir().join("seiva-scaffold-fitness-target");
        // `--tests` compiles the module's unit test too (which asserts the use case
        // commits one transaction and emits the event via the `seiva-testing`
        // doubles), so the test's dev-dependencies and the transaction/event shape
        // are verified to compile, not just the production code.
        let status = std::process::Command::new(cargo_bin)
            .args(["build", "--tests"])
            .current_dir(&work)
            .env("CARGO_TARGET_DIR", &target_dir)
            .status()
            .expect("run cargo build on the generated project");

        let _ = std::fs::remove_dir_all(&work);
        assert!(
            status.success(),
            "the generated project must compile standalone (rc {:?})",
            status.code()
        );
    }

    /// Fitness (ADR 0024): each non-api project kind (`worker`, `gateway`, `event`)
    /// compiles standalone against this checkout — so a renamed worker-host item, a
    /// changed `run_worker`/`drain_in_process` signature or a wrong per-kind
    /// dependency is caught, not just the `api` shape. Bare (no modules), since
    /// those kinds do not host business modules.
    #[test]
    fn generated_project_compiles_for_each_kind() {
        let root = framework_root();
        let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let target_dir = std::env::temp_dir().join("seiva-scaffold-fitness-target");
        for (kind, name) in [
            (ProjectKind::Worker, "wrk"),
            (ProjectKind::Gateway, "gw"),
            (ProjectKind::Event, "evt"),
        ] {
            let work =
                std::env::temp_dir().join(format!("seiva-kind-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&work);
            std::fs::create_dir_all(&work).expect("create work dir");

            let mut project = project_files(name, kind).expect("project files");
            let cargo = project
                .iter()
                .find(|f| f.path == "Cargo.toml")
                .expect("workspace manifest")
                .contents
                .clone();
            let with_patch = format!(
                "{cargo}{}",
                workspace_patch_section(&root.to_string_lossy())
            );
            project
                .iter_mut()
                .find(|f| f.path == "Cargo.toml")
                .expect("workspace manifest")
                .contents = with_patch;
            write_all(&work, &project);

            let status = std::process::Command::new(&cargo_bin)
                .arg("build")
                .current_dir(&work)
                .env("CARGO_TARGET_DIR", &target_dir)
                .status()
                .expect("run cargo build on the generated project");
            let _ = std::fs::remove_dir_all(&work);
            assert!(
                status.success(),
                "the generated `{}` project must compile standalone (rc {:?})",
                kind.as_str(),
                status.code()
            );
        }
    }

    /// Looks up a generated file's contents by its exact path.
    fn contents<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
        files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("missing generated file `{path}`"))
            .contents
            .as_str()
    }

    /// A cheap, context-free well-formedness check: the three bracket kinds each
    /// balance in count. `delonix-seiva` cannot depend on `syn` (that lives in
    /// the proc-macro crate, not a normal dependency of this crate), so the
    /// generated code is NOT parsed; this catches a truncated or unbalanced
    /// template without a parser. Angle brackets are intentionally not counted
    /// (they appear in generics and comparisons, not as delimiters here).
    fn brackets_balance(src: &str) -> bool {
        src.matches('{').count() == src.matches('}').count()
            && src.matches('(').count() == src.matches(')').count()
            && src.matches('[').count() == src.matches(']').count()
    }

    #[test]
    fn module_files_emit_manifest_and_five_layer_crates() {
        let files = module_files("notes", false).unwrap();
        // One module.toml, a Cargo.toml + src/lib.rs per layer (10), plus the
        // seed/ and demo/ CSV fixtures (ADR 0022) = 13.
        assert_eq!(
            files.len(),
            13,
            "expected 1 manifest + 5 crates * 2 files + seed/demo CSV"
        );
        assert!(files
            .iter()
            .any(|f| f.path == "modules/notes/seed/record.csv"));
        assert!(files
            .iter()
            .any(|f| f.path == "modules/notes/demo/record.csv"));
        for layer in [
            "facade",
            "domain",
            "application",
            "infrastructure",
            "interface",
        ] {
            assert!(
                files
                    .iter()
                    .any(|f| f.path == format!("modules/notes/{layer}/src/lib.rs")),
                "missing {layer}/src/lib.rs"
            );
            assert!(
                files
                    .iter()
                    .any(|f| f.path == format!("modules/notes/{layer}/Cargo.toml")),
                "missing {layer}/Cargo.toml"
            );
        }
        assert!(files.iter().any(|f| f.path == "modules/notes/module.toml"));
    }

    /// Fitness: the generated `interface` layer must NOT declare a production
    /// dependency on the module's `infrastructure` crate — the layering rule
    /// (`interface -> infrastructure` forbidden) that `seiva lint` enforces. The
    /// controller is generic over the ports, so `interface` names only
    /// `application`; the composition root injects the concrete adapters.
    #[test]
    fn generated_interface_does_not_depend_on_infrastructure() {
        let files = module_files("notes", false).unwrap();
        let cargo = contents(&files, "modules/notes/interface/Cargo.toml");
        let deps = cargo
            .split("[dev-dependencies]")
            .next()
            .expect("the [dependencies] section precedes any dev-dependencies");
        assert!(
            !deps.contains("notes-infrastructure"),
            "generated interface must not depend on infrastructure in production:\n{deps}"
        );
        assert!(
            deps.contains("notes-application"),
            "generated interface depends on application (allowed):\n{deps}"
        );
    }

    #[test]
    fn every_generated_lib_has_balanced_brackets() {
        let files = module_files("order_items", false).unwrap();
        for layer in [
            "facade",
            "domain",
            "application",
            "infrastructure",
            "interface",
        ] {
            let path = format!("modules/order_items/{layer}/src/lib.rs");
            assert!(
                brackets_balance(contents(&files, &path)),
                "unbalanced brackets in {path}"
            );
        }
    }

    #[test]
    fn generated_facade_is_framework_wired() {
        let files = module_files("notes", false).unwrap();
        let facade = contents(&files, "modules/notes/facade/src/lib.rs");
        // The macro is imported and applied with an explicit version.
        assert!(facade.contains("use delonix_seiva::{facade, WireSchema};"));
        // F1 (ADR 0003, fully closed): `Validate`/`ValidationError` AND the RFC 9457
        // `Problem` the error maps to are transport-neutral contract types from
        // `seiva-contract`. The facade names NO transport crate at all.
        assert!(facade.contains("use seiva_contract::{Problem, Validate, ValidationError};"));
        assert!(
            !facade.contains("seiva_http") && !facade.contains("seiva-http"),
            "the facade must not name the HTTP transport (F1):\n{facade}"
        );
        assert!(facade.contains("#[facade(version = \"v1\")]"));
        // At least one method carries a `#[requires(..)]`, keyed to the module.
        assert!(facade.contains("#[requires(\"notes.create\")]"));
        // The trait is named after the module in PascalCase.
        assert!(facade.contains("pub trait Notes {"));
        assert!(facade.contains(
            "async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError>;"
        ));
        // Wire DTOs: serde + Validate on the write DTO, Serialize on the response,
        // WireSchema so the OpenAPI document carries the real fields, error →
        // Problem mapping. This is the controller-test shape.
        assert!(facade.contains("#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]"));
        assert!(facade.contains("impl Validate for CreateRecord"));
        assert!(facade.contains("impl From<RecordError> for Problem"));
    }

    /// F6: the generated route doc uses the SAME pluralisation as the runtime
    /// route (`crate::contract::resource_segment`), not a second naive rule — so
    /// `Category` → `categories`, never `categorys`.
    #[test]
    fn generated_facade_route_uses_the_shared_pluralisation() {
        assert_eq!(resource_of("Category"), "categories");
        assert_eq!(resource_of("Inventory"), "inventories");
        let files = module_files("category", false).unwrap();
        let facade = contents(&files, "modules/category/facade/src/lib.rs");
        assert!(
            facade.contains("POST /v1/categories"),
            "route doc must match the real pluralised route:\n{facade}"
        );
        assert!(
            !facade.contains("categorys"),
            "the naive plural must be gone"
        );
    }

    #[test]
    fn generated_application_implements_the_facade_over_ports() {
        let files = module_files("notes", false).unwrap();
        let app = contents(&files, "modules/notes/application/src/lib.rs");
        assert!(app.contains(
            "use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};"
        ));
        assert!(app.contains("pub trait RecordRepository {"));
        assert!(app.contains("impl<R, G, U, E> Notes for RecordService<R, G, U, E>"));
        // F2 (ADR 0021): the write use case is ONE transaction and publishes a
        // domain event INSIDE it — the save and the publish both run within
        // `UnitOfWork::transaction`. The generated seed must teach the correct shape.
        assert!(
            app.contains("self.uow\n            .transaction(")
                || app.contains(".transaction(|| async {"),
            "the use case must open a UnitOfWork transaction:\n{app}"
        );
        assert!(
            app.contains("self.events.publish(\"notes.record.created\""),
            "the use case must publish a domain event inside the transaction:\n{app}"
        );
        // The test asserts commit + emit via the seiva-testing doubles.
        assert!(app.contains("TxOutcome::Committed") && app.contains(".topics()"));
        // The application layer is transport-free: no HTTP, axum or framework
        // runtime item leaks into it (ADR 0003).
        assert!(
            !app.contains("seiva_http"),
            "application must be transport-free"
        );
        assert!(!app.contains("axum"), "application must be transport-free");
        assert!(
            !app.contains("delonix_seiva"),
            "application must not name a framework-runtime item"
        );
    }

    #[test]
    fn generated_infrastructure_is_in_memory_and_dorm_free() {
        let files = module_files("notes", false).unwrap();
        let infra = contents(&files, "modules/notes/infrastructure/src/lib.rs");
        assert!(infra.contains("pub struct InMemoryRecordRepository"));
        assert!(infra.contains("impl RecordRepository for InMemoryRecordRepository"));
        assert!(infra.contains("PortResult"));
        // DORM is mentioned only as the future feature-gated option, in a comment.
        assert!(
            infra.contains("`dorm`") && infra.contains("feature"),
            "the DORM-via-feature note must be present"
        );
    }

    /// Fitness (ADR 0022): a generated module ships `seed/` + `demo/` CSV, the repo
    /// is idempotent (keyed by id), and both sources register in the seed inventory.
    #[test]
    fn generated_module_has_seed_and_demo() {
        let files = module_files("notes", false).unwrap();

        // The CSV fixtures exist and carry the external-id `id` column.
        let seed = contents(&files, "modules/notes/seed/record.csv");
        let demo = contents(&files, "modules/notes/demo/record.csv");
        assert!(seed.contains("id,title"), "seed CSV has an `id` column");
        assert!(demo.contains("id,title"), "demo CSV has an `id` column");

        // Infrastructure: idempotent upsert repo + embedded seed + registration +
        // the data-layer loader (no events).
        let infra = contents(&files, "modules/notes/infrastructure/src/lib.rs");
        assert!(
            infra.contains("HashMap<u128, Record>") && infra.contains(".insert(record.id"),
            "the repo must be keyed by id for an idempotent upsert:\n{infra}"
        );
        assert!(infra.contains("include_str!(\"../../seed/record.csv\")"));
        assert!(infra.contains("delonix_seiva::register_seed!"));
        assert!(infra.contains("pub async fn load_seed("));
        assert!(
            infra.contains("external_u128("),
            "seed derives a deterministic id from the row key"
        );

        // Interface: embedded demo + registration (demo: true) + facade loader.
        let interface = contents(&files, "modules/notes/interface/src/lib.rs");
        assert!(interface.contains("include_str!(\"../../demo/record.csv\")"));
        assert!(interface.contains("demo: true"));
        assert!(
            interface.contains("pub async fn load_demo<R, G, U, E>("),
            "demo loads through the facade (generic over the four ports)"
        );
        assert!(
            interface.contains("controller.create(CreateRecord"),
            "demo goes through the facade create, so invariants run:\n{interface}"
        );
        // Still no infrastructure dependency (the layering rule holds).
        assert!(!interface.contains("notes_infrastructure"));
    }

    /// Fitness (ADR 0022): wiring a module into the composition root inserts the
    /// boot-time seed step — seed always, demo gated on dev.
    #[test]
    fn composition_root_wires_seed_on_boot() {
        let main = app_main_rs("shop");
        // A freshly generated project (no modules yet) is warning-clean: the dev
        // gate is NOT present until the first module is wired.
        assert!(!main.contains("let load_demo ="));
        // The one-shot `seiva db seed` exit path is present from the start.
        assert!(main.contains("if std::env::var(\"SEIVA_SEED\").is_ok()"));

        // Wiring the first module inserts the gate exactly once.
        let wired = wire_module_in_main(&main, "notes").unwrap();
        assert!(wired.contains("let load_demo = config.env.is_development()"));
        assert!(
            wired.contains("notes_infrastructure::load_seed(&notes_repo)"),
            "the module's seed loads on boot:\n{wired}"
        );
        assert!(
            wired.contains("if load_demo {") && wired.contains("notes_interface::load_demo("),
            "demo loads only when the dev gate is set"
        );
        // The router merge still happens (and the idempotency guard still matches).
        assert!(wired.contains("notes_interface::notes_router(notes_controller)"));

        // A SECOND module does not re-declare the gate (no shadow/duplicate).
        let wired2 = wire_module_in_main(&wired, "polls").unwrap();
        assert_eq!(
            wired2.matches("let load_demo =").count(),
            1,
            "the dev gate is declared exactly once"
        );
    }

    #[test]
    fn generated_interface_holds_controller_and_transport_generation() {
        let files = module_files("notes", false).unwrap();
        let interface = contents(&files, "modules/notes/interface/src/lib.rs");
        // The controller macro is imported and applied to a GENERIC adapter impl:
        // generic over the repository/id-generator ports, so the interface never
        // names the infrastructure crate (the layering rule).
        assert!(interface.contains("use delonix_seiva::controller;"));
        assert!(interface.contains("#[controller]"));
        assert!(interface
            .contains("pub struct NotesController<R, G, U, E>(Arc<RecordService<R, G, U, E>>);"));
        assert!(interface.contains("impl<R, G, U, E> Notes for NotesController<R, G, U, E>"));
        // It depends on application (allowed), NOT infrastructure.
        assert!(interface.contains("use notes_application::{RecordRepository, RecordService};"));
        assert!(!interface.contains("notes_infrastructure"));
        // The four ports are injected via `new(repository, ids, uow, events)`; there
        // is no no-arg `router()` default that would need to name a concrete adapter.
        assert!(
            interface.contains("pub fn new(repository: R, ids: G, uow: U, events: E) -> Self {")
        );
        assert!(!interface.contains("pub fn router()"));
        // OpenAPI generation from the facade descriptor. Proto generation is NOT
        // per-module (it would pull the gRPC generator into every module crate);
        // it is available from the same descriptor via the `seiva proto` CLI.
        assert!(interface.contains("seiva_openapi::openapi_document("));
        assert!(!interface.contains("seiva_grpc"));
        assert!(interface.contains("NotesFacade::DESCRIPTOR"));
        // The generated interface must NOT silence `result_large_err`: `Problem`
        // is small enough (see seiva-http) that handlers returning it by value
        // never trip the lint, so scaffolded projects keep the gate intact.
        assert!(!interface.contains("result_large_err"));
        // Without `--admin` the module registers NOTHING: it is private to the
        // console (ADR 0009).
        assert!(!interface.contains("register_admin!"));
        assert!(!interface.contains("AdminResource"));
    }

    #[test]
    fn module_new_admin_scaffolds_a_registration() {
        let files = module_files("notes", true).unwrap();
        let interface = contents(&files, "modules/notes/interface/src/lib.rs");
        // The `--admin` flag emits a registration coupling only to the framework
        // contract — not to the admin UI crate.
        assert!(interface.contains("use delonix_seiva::register_admin;"));
        assert!(interface.contains("register_admin!(AdminResource {"));
        assert!(!interface.contains("seiva_console_admin"));
        // The resource id matches the module's route segment, the permissions the
        // facade's `#[requires(..)]`.
        assert!(interface.contains("namespace: \"notes\""));
        assert!(interface.contains("resource: \"notes\""));
        assert!(interface.contains("view: \"notes.read\""));
        assert!(interface.contains("create: \"notes.create\""));
        // It still compiles as valid-looking Rust: the controller follows the
        // registration (the block was inserted, not appended past EOF).
        assert!(interface.contains("pub struct NotesController"));
    }

    #[test]
    fn scaffolded_app_has_the_extension_composition_points() {
        let files = project_files("shop", ProjectKind::Api).unwrap();
        let cargo = contents(&files, "apps/shop-api/Cargo.toml");
        let main = contents(&files, "apps/shop-api/src/main.rs");
        assert!(cargo.contains(APP_EXTENSION_DEPS_ANCHOR));
        assert!(main.contains(EXTENSIONS_ANCHOR));
    }

    #[test]
    fn ext_add_is_explicit_and_idempotent() {
        let files = project_files("shop", ProjectKind::Api).unwrap();
        let cargo0 = contents(&files, "apps/shop-api/Cargo.toml");
        let main0 = contents(&files, "apps/shop-api/src/main.rs");
        let ext = CONSOLE_ADMIN_EXTENSION;

        // The freshly scaffolded root must NOT already contain the mount — else
        // the idempotency `contains` check would wrongly treat an example in a
        // comment as "already mounted" and never insert the real statement.
        assert!(
            !main0.contains(ext.mount_stmt),
            "the template must not pre-contain the mount statement"
        );
        assert!(!cargo0.contains("seiva-console-admin ="));

        // First application wires both the dependency and the mount.
        let cargo1 = add_extension_to_app_manifest(cargo0, &ext).unwrap();
        let main1 = mount_extension_in_main(main0, &ext).unwrap();
        assert!(cargo1.contains("seiva-console-admin = \"0.1\""));
        assert_eq!(
            main1.matches(ext.mount_stmt).count(),
            1,
            "exactly one mount statement after a single application"
        );

        // Second application is a no-op: nothing duplicated.
        let cargo2 = add_extension_to_app_manifest(&cargo1, &ext).unwrap();
        let main2 = mount_extension_in_main(&main1, &ext).unwrap();
        assert_eq!(cargo1, cargo2, "the dependency must not be added twice");
        assert_eq!(main1, main2, "the mount must not be inserted twice");
        assert_eq!(main2.matches("seiva_console_admin::mount").count(), 1);
    }

    #[test]
    fn ext_add_on_an_unrecognised_project_is_a_missing_anchor() {
        let ext = CONSOLE_ADMIN_EXTENSION;
        // A hand-written manifest/main with no managed anchors.
        let err = add_extension_to_app_manifest("[dependencies]\naxum = \"0.8\"\n", &ext)
            .expect_err("no extension anchor");
        assert!(matches!(err, ScaffoldError::MissingAnchor { .. }));
        let err = mount_extension_in_main("fn main() {}\n", &ext).expect_err("no mount anchor");
        assert!(matches!(err, ScaffoldError::MissingAnchor { .. }));
    }

    #[test]
    fn extension_manifest_resolves_only_known_extensions() {
        assert!(extension_manifest("seiva-console-admin").is_some());
        assert!(extension_manifest("not-an-extension").is_none());
    }

    #[test]
    fn module_manifests_depend_on_the_right_framework_crates() {
        let files = module_files("notes", false).unwrap();

        // The framework crates are depended on by crates.io VERSION (so a
        // generated project compiles standalone once they are published); the
        // intra-project layer crates stay `path`.
        let facade = contents(&files, "modules/notes/facade/Cargo.toml");
        assert!(facade.contains("delonix-seiva = { version = \"0.1\", default-features = false }"));
        // F1 (ADR 0003): the facade depends on the transport-neutral `seiva-contract`
        // (Validate + the RFC 9457 Problem), NOT on the HTTP transport.
        assert!(facade.contains("seiva-contract = \"0.1\""));
        assert!(
            !facade.contains("seiva-http"),
            "the facade must not depend on the HTTP transport (F1)"
        );
        assert!(facade.contains("serde = { version = \"1\", features = [\"derive\"] }"));

        // The application depends on the ports, never on the transport/framework.
        let app = contents(&files, "modules/notes/application/Cargo.toml");
        assert!(app.contains("notes-domain = { path = \"../domain\" }"));
        assert!(app.contains("notes-facade = { path = \"../facade\" }"));
        assert!(app.contains("seiva-ports = \"0.1\""));
        // No transport/framework DEPENDENCY entries (the prose may name them).
        assert!(
            !app.contains("seiva-http = {"),
            "application stays transport-free"
        );
        assert!(
            !app.contains("delonix-seiva = {"),
            "application stays framework-free"
        );

        let infra = contents(&files, "modules/notes/infrastructure/Cargo.toml");
        assert!(infra.contains("notes-application = { path = \"../application\" }"));
        assert!(infra.contains("seiva-ports = \"0.1\""));

        // The interface carries the transport crates + `application` (allowed),
        // but NOT `infrastructure` (the layering rule forbids it).
        let interface = contents(&files, "modules/notes/interface/Cargo.toml");
        assert!(interface.contains("notes-facade = { path = \"../facade\" }"));
        assert!(interface.contains("notes-application = { path = \"../application\" }"));
        assert!(
            !interface.contains("notes-infrastructure = {"),
            "interface must not depend on infrastructure (layering rule)"
        );
        assert!(interface.contains("seiva-ports = \"0.1\""));
        assert!(interface.contains("delonix-seiva = { version = \"0.1\""));
        assert!(interface.contains("seiva-http = \"0.1\""));
        // No seiva-grpc: proto generation is not per-module (it is a CLI tool).
        assert!(!interface.contains("seiva-grpc"));
        assert!(interface.contains("seiva-openapi = \"0.1\""));
        assert!(interface.contains("axum = \"0.8\""));
        assert!(interface.contains("serde_json = \"1\""));
    }

    #[test]
    fn no_generated_file_pulls_in_dorm() {
        let files = module_files("notes", false).unwrap();
        for file in &files {
            assert!(
                !file.contents.contains("seiva-data-dorm = "),
                "`{}` must not depend on the DORM gateway",
                file.path
            );
        }
    }

    #[test]
    fn pascal_case_conversion_handles_snake_names() {
        assert_eq!(to_pascal_case("notes"), "Notes");
        assert_eq!(to_pascal_case("order_items"), "OrderItems");
        assert_eq!(to_pascal_case("a"), "A");
    }

    #[test]
    fn workspace_registration_inserts_members() {
        let cargo = workspace_cargo_toml("shop-api");
        let out = add_members_to_workspace(&cargo, "notes").unwrap();
        assert!(out.contains("\"modules/notes/facade\","));
        assert!(out.contains("\"modules/notes/interface\","));
        // Idempotence guard: a second insert of the same module is refused.
        assert!(add_members_to_workspace(&out, "notes").is_err());
    }

    #[test]
    fn app_manifest_wiring_inserts_module_deps() {
        let cargo = bin_cargo_toml("shop-api", ProjectKind::Api);
        let out = add_module_to_app_manifest(&cargo, "notes").unwrap();
        assert!(out.contains("notes-interface = { path = \"../../modules/notes/interface\" }"));
        assert!(out
            .contains("notes-infrastructure = { path = \"../../modules/notes/infrastructure\" }"));
        // The deps land before the managed anchor, which survives for the next module.
        assert!(out.contains(APP_MODULE_DEPS_ANCHOR));
        // Idempotence: a second insert of the same module is refused.
        assert!(add_module_to_app_manifest(&out, "notes").is_err());
    }

    #[test]
    fn main_wiring_inserts_router_merge() {
        let main = app_main_rs("shop");
        let out = wire_module_in_main(&main, "order_items").unwrap();
        // The merge uses the module's generated router + controller + in-memory
        // adapters, and the PascalCase controller name.
        assert!(out.contains("order_items_interface::order_items_router("));
        assert!(out.contains("order_items_interface::OrderItemsController::new("));
        assert!(out.contains("order_items_infrastructure::InMemoryRecordRepository::new()"));
        // The anchor remains for the next module, and `modules` is shadowed (merged).
        assert!(out.contains(MODULE_MERGES_ANCHOR));
        assert!(out.contains("let modules = modules.merge("));
        assert!(wire_module_in_main(&out, "order_items").is_err());
    }

    #[test]
    fn split_scaffolds_a_single_module_service_binary() {
        let files = service_files("catalog").expect("service files");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"apps/catalog-service/Cargo.toml"));
        assert!(paths.contains(&"apps/catalog-service/src/main.rs"));
        assert!(paths.contains(&"apps/catalog-service/Delonixfile"));
        // F5 — the split service owns its own data: a compose with a dedicated
        // database and a per-service DSN, plus a README spelling out the ownership.
        assert!(paths.contains(&"apps/catalog-service/compose.yaml"));
        assert!(paths.contains(&"apps/catalog-service/README.md"));
        let compose = contents(&files, "apps/catalog-service/compose.yaml");
        assert!(
            compose.contains("catalog-db:") && compose.contains("image: postgres"),
            "the service must bring up its OWN database:\n{compose}"
        );
        assert!(
            compose.contains("DELONIX_ORM_DSN: postgres://catalog:catalog@catalog-db:5432/catalog"),
            "the service must point at its own database, not the monolith's:\n{compose}"
        );
        let readme = contents(&files, "apps/catalog-service/README.md");
        assert!(readme.contains("owns its own data"));

        let main = &files
            .iter()
            .find(|f| f.path.ends_with("src/main.rs"))
            .unwrap()
            .contents;
        // Mounts exactly this module, with the generated router/controller/adapters.
        assert!(main.contains("catalog_interface::catalog_router("));
        assert!(main.contains("catalog_interface::CatalogController::new("));
        assert!(main.contains("router_with_modules("));

        let cargo = &files
            .iter()
            .find(|f| f.path.ends_with("Cargo.toml"))
            .unwrap()
            .contents;
        assert!(cargo.contains("name = \"catalog-service\""));
        assert!(
            cargo.contains("catalog-interface = { path = \"../../modules/catalog/interface\" }")
        );
    }

    #[test]
    fn workspace_member_add_and_remove_round_trips() {
        let cargo = workspace_cargo_toml("shop-api");
        let added = add_app_member(&cargo, "apps/catalog-service").expect("add member");
        assert!(added.contains("\"apps/catalog-service\","));
        // Idempotence guard: a second add is refused.
        assert!(add_app_member(&added, "apps/catalog-service").is_err());
        // Removing it returns the member list to its prior shape.
        let removed = remove_app_member(&added, "apps/catalog-service");
        assert!(!removed.contains("apps/catalog-service"));
        assert!(removed.contains("\"apps/shop-api\","));
    }

    #[test]
    fn module_mode_is_inserted_then_replaced() {
        // A fresh module.toml carries mode = "library"; split flips it to service.
        let toml = module_toml("catalog");
        assert!(toml.contains("mode = \"library\""));
        let as_service = set_module_mode(&toml, "service").expect("set service");
        assert!(as_service.contains("mode = \"service\""));
        assert!(!as_service.contains("mode = \"library\""));
        // A legacy module.toml without a mode key gets one inserted under [module].
        let legacy = "[module]\nname = \"catalog\"\nfacade = \"v1\"\n";
        let fixed = set_module_mode(legacy, "service").expect("insert mode");
        assert!(fixed.contains("[module]"));
        assert!(fixed.contains("mode = \"service\""));
    }

    /// Fitness for P0-3 (runtime facade discovery): a scaffolded project, with a
    /// module auto-wired into its composition root exactly as `seiva module new`
    /// does, emits **its own** OpenAPI document — the module's route, resolved
    /// from the facade the binary links at run time (`SEIVA_EMIT=openapi`), not
    /// the CLI's built-in demo. Proves the whole discovery path by running it.
    #[test]
    fn generated_project_emits_its_own_openapi() {
        let root = framework_root();
        let work =
            std::env::temp_dir().join(format!("seiva-scaffold-openapi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).expect("create work dir");

        // A distinctly named module so its route could only come from the project.
        let module = "widgets";
        let mut project = project_files("shop", ProjectKind::Api).expect("project files");
        let modules = module_files(module, false).expect("module files");

        // Apply every edit `seiva module new` performs: workspace members + the
        // interim patch, the app crate's module deps, and the main.rs merge.
        let edit = |files: &mut Vec<GeneratedFile>, path: &str, f: &dyn Fn(&str) -> String| {
            let file = files.iter_mut().find(|g| g.path == path).expect(path);
            file.contents = f(&file.contents);
        };
        edit(&mut project, "Cargo.toml", &|s| {
            let members = add_members_to_workspace(s, module).expect("register module");
            format!(
                "{members}{}",
                workspace_patch_section(&root.to_string_lossy())
            )
        });
        edit(&mut project, "apps/shop-api/Cargo.toml", &|s| {
            add_module_to_app_manifest(s, module).expect("app manifest wiring")
        });
        edit(&mut project, "apps/shop-api/src/main.rs", &|s| {
            wire_module_in_main(s, module).expect("main wiring")
        });

        write_all(&work, &project);
        write_all(&work, &modules);

        let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let target_dir = std::env::temp_dir().join("seiva-scaffold-fitness-target");
        let output = std::process::Command::new(cargo_bin)
            .args(["run", "--quiet", "--bin", "shop-api"])
            .current_dir(&work)
            .env("CARGO_TARGET_DIR", &target_dir)
            .env("SEIVA_EMIT", "openapi")
            .output()
            .expect("run the generated binary with SEIVA_EMIT=openapi");

        assert!(
            output.status.success(),
            "the generated binary must emit its OpenAPI (rc {:?}); stderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        let doc: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("the emitted document is JSON");
        assert!(
            doc["paths"]["/v1/widgets"].is_object(),
            "the project's own module route must be documented; got paths: {:?}",
            doc["paths"]
                .as_object()
                .map(|m| m.keys().collect::<Vec<_>>())
        );

        // The same binary also emits the project's proto3 (SEIVA_EMIT=proto),
        // from the same linked facade — the generated-contracts symmetry (P2-12).
        let proto = std::process::Command::new(
            std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()),
        )
        .args(["run", "--quiet", "--bin", "shop-api"])
        .current_dir(&work)
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("SEIVA_EMIT", "proto")
        .output()
        .expect("run the generated binary with SEIVA_EMIT=proto");

        let _ = std::fs::remove_dir_all(&work);
        assert!(
            proto.status.success(),
            "the generated binary must emit its proto (rc {:?}); stderr:\n{}",
            proto.status.code(),
            String::from_utf8_lossy(&proto.stderr)
        );
        let proto = String::from_utf8_lossy(&proto.stdout);
        assert!(
            proto.contains("service Widgets {") && proto.contains("syntax = \"proto3\";"),
            "the project's own module must be a proto service; got:\n{proto}"
        );
    }

    #[test]
    fn seiva_toml_registration_appends_section() {
        let seiva = seiva_toml("shop", ProjectKind::Api);
        let out = register_module_in_seiva_toml(&seiva, "notes").unwrap();
        assert!(out.contains("[modules.notes]"));
        assert!(register_module_in_seiva_toml(&out, "notes").is_err());
    }

    #[test]
    fn project_name_reads_the_manifest() {
        assert_eq!(
            project_name(&seiva_toml("shop", ProjectKind::Api)).as_deref(),
            Some("shop")
        );
        assert_eq!(project_name("not = valid").as_deref(), None);
        assert_eq!(project_name("[other]\nname = \"x\"").as_deref(), None);
    }

    #[test]
    fn namespace_block_is_appended_once() {
        let schema = project_schema();
        let out = append_namespace_block(&schema, "notes").unwrap();
        assert!(out.contains("namespace notes {"));
        assert!(append_namespace_block(&out, "notes").is_err());
    }

    #[test]
    fn dorm_schema_adapts_the_project_schema() {
        // A project schema with a module namespace (and the scaffold's `@version`).
        let project = append_namespace_block(&project_schema(), "notes").unwrap();
        let dorm = dorm_schema_from_project(&project, "postgresql");
        // The datasource gains the env-driven URL DORM's migrate needs.
        assert!(dorm.contains("url      = env(\"DELONIX_ORM_DSN\")"));
        // The module namespace and its model pass through (shared DSL).
        assert!(dorm.contains("namespace notes {"));
        assert!(dorm.contains("model Record {"));
        assert!(dorm.contains("id      Uuid   @id"));
        // `@version` is dropped (DORM's parser rejects it); the column remains.
        assert!(!dorm.contains("@version"));
        assert!(dorm.contains("version Int"));
        // Idempotent on the URL: running it again does not add a second one.
        let twice = dorm_schema_from_project(&dorm, "postgresql");
        assert_eq!(twice.matches("DELONIX_ORM_DSN").count(), 1);

        // A non-PG backend retargets the datasource provider.
        let my = dorm_schema_from_project(&project, "mysql");
        assert!(my.contains("provider = \"mysql\""));
        assert!(!my.contains("provider = \"postgresql\""));
    }
}
