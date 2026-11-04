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

/// Files for a new project `name`. Paths are relative to the new `name/` dir.
pub fn project_files(name: &str) -> Result<Vec<GeneratedFile>, ScaffoldError> {
    validate_name(name)?;
    let app = format!("{name}-api");
    let file = |path: &str, contents: String| GeneratedFile {
        path: path.to_owned(),
        contents,
    };

    Ok(vec![
        file("seiva.toml", seiva_toml(name)),
        file("Cargo.toml", workspace_cargo_toml(&app)),
        file(".gitignore", "/target\n**/*.rs.bk\n.env\n".to_owned()),
        file("README.md", format!("# {name}\n\nA Delonix Seiva project. Add modules with `seiva module new <name>`.\n\nRun locally with `seiva start`; build a distroless image with `delonix stack up` (see the `Delonixfile` and `compose.yaml`).\n")),
        file(&format!("schema/{name}.schema"), project_schema()),
        file("modules/.keep", String::new()),
        // apps/<name>-api: the composition root — wires modules, holds no logic.
        file(&format!("apps/{app}/Cargo.toml"), bin_cargo_toml(&app)),
        file(&format!("apps/{app}/src/main.rs"), app_main_rs(name)),
        // Cloud-native packaging: a distroless multi-stage image and a compose
        // file that `delonix stack up` (or `docker compose`) runs.
        file("Delonixfile", delonixfile(&app)),
        file("compose.yaml", compose_yaml(name)),
        file(".dockerignore", dockerignore()),
        // shared/kernel: shared value types, no business rules.
        file("shared/kernel/Cargo.toml", lib_cargo_toml("kernel", &[])),
        file(
            "shared/kernel/src/lib.rs",
            "//! Shared value types across modules. No business rules live here.\n\n/// A tenant identifier, shared by every module's scope.\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub struct TenantId(pub u128);\n".to_owned(),
        ),
    ])
}

/// Files for a new module `module`, relative to the project root.
///
/// The emitted crates are wired to the real framework transports: the `facade`
/// carries a `#[seiva::facade]` trait, the `interface` a `#[seiva::controller]`
/// adapter that yields `<trait>_router` plus the OpenAPI/proto generation, and
/// the `application` implements the facade over `seiva-ports`. See the module
/// docs for the full shape and the layering decision.
pub fn module_files(module: &str) -> Result<Vec<GeneratedFile>, ScaffoldError> {
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
            interface_lib(&snake, &pascal),
        ),
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

// --- templates ---------------------------------------------------------------

fn seiva_toml(name: &str) -> String {
    format!(
        "# Delonix Seiva project manifest. Managed partly by the seiva CLI.\n\
         [project]\n\
         name = \"{name}\"\n\n\
         [data]\n\
         # DORM is the default data layer; set to \"external\" to own it yourself.\n\
         layer = \"dorm\"\n\n\
         # Modules are registered below by `seiva module new`.\n"
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

fn bin_cargo_toml(app: &str) -> String {
    format!(
        "[package]\nname = \"{app}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         # The service runtime: the serve loop, 12-factor config, structured\n\
         # logging and the shutdown signal. The composition root reuses it instead\n\
         # of writing its own `main`.\n\
         seiva-server = \"{FRAMEWORK_VERSION}\"\n\
         # The HTTP application builder (`router_with_modules` + health +\n\
         # /openapi.json + /docs); merge your modules' routers through it below.\n\
         seiva-http = \"{FRAMEWORK_VERSION}\"\n\
         # The composition root holds the merged module routes as an `axum::Router`.\n\
         axum = \"0.8\"\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         tracing = \"0.1\"\n"
    )
}

/// The composition root `main.rs`: a real Seiva server over the runtime's serve
/// loop. It reads 12-factor config, installs JSON logging, builds the HTTP
/// application (health, `/openapi.json`, `/docs`) and drains gracefully on
/// SIGTERM. A new module's router is merged into `app` at the marked line.
fn app_main_rs(name: &str) -> String {
    let app = format!("{name}-api");
    format!(
        "//! Composition root for `{name}`: the service's `main`. Wire your\n\
         //! modules' routers into `modules` below; keep business logic out.\n\n\
         use axum::Router;\n\
         use seiva_http::{{router_with_modules, Health, HttpSecurity}};\n\
         use seiva_server::{{observability, serve_router, shutdown_signal, ServerConfig}};\n\
         use std::process::ExitCode;\n\n\
         #[tokio::main(flavor = \"multi_thread\")]\n\
         async fn main() -> ExitCode {{\n\
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
         \x20   // Merge each module's generated router here, BEFORE the framework\n\
         \x20   // layers wrap them — `router_with_modules` merges then layers, so\n\
         \x20   // authn, security headers and the rate limit apply to module routes\n\
         \x20   // too (a plain `.merge()` after layering would leave them uncovered).\n\
         \x20   // For each module you add, pick its adapters and merge its router:\n\
         \x20   //   let modules = Router::new().merge(notes_interface::notes_router(\n\
         \x20   //       notes_interface::NotesController::new(\n\
         \x20   //           notes_infrastructure::InMemoryRecordRepository::new(),\n\
         \x20   //           notes_infrastructure::CounterIdGenerator::default(),\n\
         \x20   //       ),\n\
         \x20   //   ));\n\
         \x20   let modules = Router::new();\n\
         \x20   // The framework application (/health, /openapi.json, /docs) plus the\n\
         \x20   // modules, all under the framework layers.\n\
         \x20   let app = router_with_modules(health.clone(), &security, auth, modules);\n\n\
         \x20   tracing::info!(env = %config.env, http = %config.http_addr, \"{app} starting\");\n\
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

/// A distroless, multi-stage build recipe for the app binary. The final image
/// carries only the binary and its runtime libraries — no shell, no package
/// manager — and runs as a non-root user. Dockerfile-compatible, so `delonix`,
/// `docker` or `podman` all build it.
fn delonixfile(app: &str) -> String {
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
         # Bind on all interfaces inside the container; the platform maps the port.\n\
         ENV SEIVA_HTTP_ADDR=0.0.0.0:8080\n\
         ENV APP_ENV=production\n\
         EXPOSE 8080\n\
         # `:nonroot` already runs as uid 65532. Distroless has no shell for a CMD\n\
         # healthcheck; Kubernetes uses an httpGet readiness probe on /health/ready.\n\
         USER nonroot\n\
         ENTRYPOINT [\"/app/{app}\"]\n"
    )
}

/// A `compose.yaml` that builds and runs the service with `delonix stack up`
/// (or `docker compose up`). Optional backing services (brokers, identity) live
/// in `compose/*.yaml` and are opted in explicitly, never on by default.
fn compose_yaml(name: &str) -> String {
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
         \x20   ports:\n\
         \x20     - \"8080:8080\"\n\
         \x20   environment:\n\
         \x20     APP_ENV: production\n\
         \x20     SEIVA_HTTP_ADDR: 0.0.0.0:8080\n\
         \x20   # Distroless carries no shell for a CMD healthcheck; readiness is an\n\
         \x20   # httpGet on /health/ready (Kubernetes) — see docs/first-day.md.\n\
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
        "[module]\nname = \"{module}\"\nfacade = \"v1\"\nowners = []\n\n\
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

/// The framework crates a generated project names as crates.io dependencies
/// (`delonix-seiva = "0.1"`, …). Every other framework crate they pull in is
/// reached transitively, through these crates' own `path`/`workspace`
/// dependencies, so redirecting just these to a local checkout resolves the
/// whole graph (see [`workspace_patch_section`]).
pub const FRAMEWORK_CRATES: [&str; 5] = [
    "delonix-seiva",
    "seiva-http",
    "seiva-ports",
    "seiva-openapi",
    "seiva-server",
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

/// The REST resource segment the controller derives from a facade name:
/// lowercased, then naively pluralised (append `s` unless it already ends in
/// one). Used only to render the route in a generated doc comment.
fn resource_of(pascal: &str) -> String {
    let lower = pascal.to_ascii_lowercase();
    if lower.ends_with('s') {
        lower
    } else {
        format!("{lower}s")
    }
}

fn facade_lib(module: &str, pascal: &str) -> String {
    let resource = resource_of(pascal);
    format!(
        "//! Facade layer: the module's versioned public contract and wire DTOs.\n\
         //!\n\
         //! This is what other modules and the HTTP/gRPC transports see. The\n\
         //! `#[seiva::facade]` macro (imported here as `facade`) keeps the trait as\n\
         //! the single source of truth and generates the `{pascal}Facade`\n\
         //! descriptor read by the OpenAPI/proto generators and the contract checks.\n\
         //!\n\
         //! The generated controller uses the facade's OWN DTOs as the wire types,\n\
         //! so the write DTO derives `serde` and implements `seiva_http::Validate`,\n\
         //! the response DTO derives `serde::Serialize`, and the error maps to an\n\
         //! RFC 9457 `Problem` here — the module owns its wire contract. Hence this\n\
         //! layer depends on `delonix-seiva`, `serde` and `seiva-http`.\n\
         #![allow(async_fn_in_trait)]\n\n\
         use delonix_seiva::facade;\n\
         use seiva_http::{{Problem, Validate, ValidationError}};\n\
         use serde::{{Deserialize, Serialize}};\n\n\
         /// Command DTO: the JSON body of `create`, validated by the controller's\n\
         /// `Valid<T>` extractor. Rename `Record`/`title` to the real shape.\n\
         #[derive(Debug, Clone, Deserialize, Serialize)]\n\
         pub struct CreateRecord {{\n    /// The record's title; must be non-empty after trimming.\n    pub title: String,\n}}\n\n\
         impl Validate for CreateRecord {{\n    fn validate(&self) -> Result<(), ValidationError> {{\n        if self.title.trim().is_empty() {{\n            return Err(ValidationError::new(\"title must not be empty\"));\n        }}\n        Ok(())\n    }}\n}}\n\n\
         /// Response DTO: what `create` returns. Never a domain type — the\n\
         /// application layer maps the domain entity to this at the boundary.\n\
         #[derive(Debug, Clone, PartialEq, Eq, Serialize)]\n\
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
         //! repository port and the framework's cross-cutting `seiva-ports`.\n\
         //!\n\
         //! Depends on `domain`, this module's `facade` (DTOs + trait) and\n\
         //! `seiva-ports`. It names NO transport, framework-runtime or DORM item —\n\
         //! the HTTP coupling lives in `interface`, the persistence in\n\
         //! `infrastructure`. This is what keeps the use case independently testable.\n\
         #![allow(async_fn_in_trait)]\n\n\
         use std::future::Future;\n\n\
         use {snake}_domain::Record;\n\
         use {snake}_facade::{{CreateRecord, RecordDto, RecordError, {pascal}}};\n\
         use seiva_ports::{{BoxError, IdGenerator, PortResult}};\n\n\
         /// Outbound port: persistence of records in the domain's language.\n\
         /// Implemented in `infrastructure` (in-memory by default, DORM-backed via\n\
         /// the `dorm` feature). The future is `Send` so a use case can `.await` it\n\
         /// on a multi-threaded runtime.\n\
         pub trait RecordRepository {{\n    /// Persists a record.\n    fn save(&self, record: &Record) -> impl Future<Output = PortResult<()>> + Send;\n}}\n\n\
         /// The application service: it implements the [`{pascal}`] facade by running\n\
         /// the use case and mapping the domain entity to the DTO. Generic over the\n\
         /// repository port and an [`IdGenerator`] so it unit-tests with fakes. It is\n\
         /// `Clone` (when its ports are) so the transport layer can hold it as state.\n\
         #[derive(Clone)]\n\
         pub struct RecordService<R, G> {{\n    /// The outbound persistence port.\n    pub repository: R,\n    /// The framework id generator.\n    pub ids: G,\n}}\n\n\
         impl<R, G> RecordService<R, G>\n\
         where\n    R: RecordRepository + Sync,\n    G: IdGenerator,\n{{\n    async fn create_record(&self, cmd: CreateRecord) -> PortResult<RecordDto> {{\n        let id = u128::from_be_bytes(self.ids.new_id().0);\n        let record = Record::new(id, cmd.title).map_err(BoxError::from)?;\n        self.repository.save(&record).await?;\n        Ok(RecordDto {{\n            id: record.id,\n            title: record.title,\n        }})\n    }}\n}}\n\n\
         impl<R, G> {pascal} for RecordService<R, G>\n\
         where\n    R: RecordRepository + Sync,\n    G: IdGenerator,\n{{\n    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError> {{\n        // Map the internal error to the contract's error type: no internal\n        // detail leaks across the module boundary beyond a message.\n        self.create_record(cmd)\n            .await\n            .map_err(|err| RecordError(err.to_string()))\n    }}\n}}\n\n\
         #[cfg(test)]\n\
         mod tests {{\n    use super::*;\n    use seiva_ports::Id;\n    use std::sync::Mutex;\n\n    #[derive(Default)]\n    struct FakeRepo {{\n        saved: Mutex<Vec<Record>>,\n    }}\n\n    impl RecordRepository for FakeRepo {{\n        async fn save(&self, record: &Record) -> PortResult<()> {{\n            self.saved.lock().unwrap().push(record.clone());\n            Ok(())\n        }}\n    }}\n\n    struct FixedIds;\n    impl IdGenerator for FixedIds {{\n        fn new_id(&self) -> Id {{\n            Id([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])\n        }}\n    }}\n\n    #[tokio::test]\n    async fn create_persists_and_returns_the_dto() {{\n        let service = RecordService {{\n            repository: FakeRepo::default(),\n            ids: FixedIds,\n        }};\n        let dto = service\n            .create(CreateRecord {{ title: \"hello\".into() }})\n            .await\n            .unwrap();\n        assert_eq!(dto.id, 1);\n        assert_eq!(dto.title, \"hello\");\n        assert_eq!(service.repository.saved.lock().unwrap().len(), 1);\n    }}\n}}\n"
    )
}

fn infrastructure_lib(snake: &str) -> String {
    format!(
        "//! Infrastructure layer: adapters implementing the application's outbound\n\
         //! ports. Depends on `application` + `domain` + `seiva-ports`; never on\n\
         //! `interface`.\n\
         //!\n\
         //! The default adapters are in-memory, so a scaffolded project compiles and\n\
         //! runs with NO database. A DORM-backed repository arrives via the `dorm`\n\
         //! feature (the `seiva-data-dorm` gateway) — swap this type for it in the\n\
         //! composition root when you turn the feature on. The adapters are `Clone`\n\
         //! (state behind an `Arc`) so the transport can hold the service as axum\n\
         //! state.\n\n\
         use std::sync::atomic::{{AtomicU64, Ordering}};\n\
         use std::sync::{{Arc, Mutex}};\n\n\
         use {snake}_application::RecordRepository;\n\
         use {snake}_domain::Record;\n\
         use seiva_ports::{{Id, IdGenerator, PortResult}};\n\n\
         /// An in-memory [`RecordRepository`]. Replace with the DORM-backed adapter\n\
         /// (feature `dorm`) or an external one for production.\n\
         #[derive(Clone, Default)]\n\
         pub struct InMemoryRecordRepository {{\n    saved: Arc<Mutex<Vec<Record>>>,\n}}\n\n\
         impl InMemoryRecordRepository {{\n    /// Creates an empty repository.\n    pub fn new() -> Self {{\n        Self::default()\n    }}\n\n    /// Every record saved so far — for composition-level assertions.\n    pub fn saved(&self) -> Vec<Record> {{\n        self.saved.lock().expect(\"repository poisoned\").clone()\n    }}\n}}\n\n\
         impl RecordRepository for InMemoryRecordRepository {{\n    async fn save(&self, record: &Record) -> PortResult<()> {{\n        self.saved\n            .lock()\n            .map_err(|_| \"repository lock poisoned\")?\n            .push(record.clone());\n        Ok(())\n    }}\n}}\n\n\
         /// A process-local monotonic [`IdGenerator`] adapter: it hands out\n\
         /// 1, 2, 3, … as 128-bit ids. Replace with a ULID/UUID generator in\n\
         /// production; this keeps the scaffold dependency-free and deterministic.\n\
         #[derive(Clone, Default)]\n\
         pub struct CounterIdGenerator {{\n    next: Arc<AtomicU64>,\n}}\n\n\
         impl IdGenerator for CounterIdGenerator {{\n    fn new_id(&self) -> Id {{\n        let value = self.next.fetch_add(1, Ordering::Relaxed) + 1;\n        let mut bytes = [0u8; 16];\n        bytes[8..].copy_from_slice(&value.to_be_bytes());\n        Id(bytes)\n    }}\n}}\n"
    )
}

fn interface_lib(snake: &str, pascal: &str) -> String {
    format!(
        "//! Interface layer: the HTTP/gRPC transport. It wires this module's facade\n\
         //! to the real framework transports.\n\
         //!\n\
         //! LAYERING DECISION: the `#[seiva::controller]` lives HERE, not in\n\
         //! `application`, so the application layer stays transport-free. The\n\
         //! controller [`{pascal}Controller<R, G>`] is GENERIC over the repository\n\
         //! and id-generator ports, so this layer depends only on `application` (the\n\
         //! service and the `RecordRepository` port) and NEVER on `infrastructure` —\n\
         //! the layering rule forbids `interface -> infrastructure`. A concrete self\n\
         //! type is still what `#[seiva::controller]` dispatches through, but\n\
         //! `{pascal}Controller<R, G>` is concrete once the ports are chosen, and the\n\
         //! macro propagates the generics + their bounds onto the generated router.\n\
         //!\n\
         //! The macro generates a `{snake}_router(service)` builder whose route\n\
         //! version and per-route permission come from the facade descriptor at run\n\
         //! time. The COMPOSITION ROOT in `apps/` picks the concrete adapters and\n\
         //! calls `{snake}_router({pascal}Controller::new(repo, ids))`. The same\n\
         //! descriptor drives this module's OpenAPI document ([`openapi`]), so the\n\
         //! wire contract is generated, never hand-maintained.\n\
         //!\n\
         //! The generated handlers return `Result<_, seiva_http::Problem>`, a large\n\
         //! value, so the large-`Err` lint is allowed crate-wide (as `seiva-http`\n\
         //! itself does).\n\
         #![allow(clippy::result_large_err)]\n\n\
         use std::sync::Arc;\n\n\
         use delonix_seiva::contract::Facade;\n\
         use delonix_seiva::controller;\n\
         use seiva_ports::IdGenerator;\n\
         use {snake}_application::{{RecordRepository, RecordService}};\n\
         use {snake}_facade::{{CreateRecord, RecordDto, RecordError, {pascal}, {pascal}Facade}};\n\n\
         /// Transport adapter: a concrete `{pascal}` service exposed over HTTP,\n\
         /// generic over the repository port `R` and the id-generator port `G`.\n\
         /// Held behind an [`Arc`] so it is `Clone + Send + Sync + 'static` — the\n\
         /// bound the generated `{snake}_router` places on its axum state — without\n\
         /// requiring the service or its ports to be `Clone`.\n\
         pub struct {pascal}Controller<R, G>(Arc<RecordService<R, G>>);\n\n\
         // Manual `Clone` (not derived): the controller is `Clone` for ANY `R`/`G`\n\
         // because it only holds an `Arc`; the derive would wrongly demand both be\n\
         // `Clone`.\n\
         impl<R, G> Clone for {pascal}Controller<R, G> {{\n    fn clone(&self) -> Self {{\n        Self(Arc::clone(&self.0))\n    }}\n}}\n\n\
         impl<R: RecordRepository, G: IdGenerator> {pascal}Controller<R, G> {{\n    /// Builds the controller over the given ports. The composition root in\n    /// `apps/` chooses them (the in-memory adapters, or DORM-backed ones); the\n    /// interface never names a concrete adapter.\n    pub fn new(repository: R, ids: G) -> Self {{\n        {pascal}Controller(Arc::new(RecordService {{ repository, ids }}))\n    }}\n}}\n\n\
         #[controller]\n\
         impl<R, G> {pascal} for {pascal}Controller<R, G>\n\
         where\n    R: RecordRepository + Send + Sync + 'static,\n    G: IdGenerator + Send + Sync + 'static,\n{{\n    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError> {{\n        self.0.create(cmd).await\n    }}\n}}\n\n\
         /// The module's OpenAPI 3.1 document, generated from the facade descriptor.\n\
         /// (A proto3 document is available from the same descriptor via\n\
         /// `seiva proto`; it is not generated per-module to keep this crate free\n\
         /// of the gRPC generator.)\n\
         pub fn openapi() -> serde_json::Value {{\n    seiva_openapi::openapi_document(\n        seiva_openapi::ApiInfo {{\n            title: \"{snake} API\".to_string(),\n            version: {pascal}Facade::DESCRIPTOR.version.to_string(),\n        }},\n        &[{pascal}Facade::DESCRIPTOR],\n    )\n}}\n"
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
         # The controller uses the facade's own DTOs as wire types, so the write DTO\n\
         # implements `seiva_http::Validate` and the error maps to `seiva_http::Problem`.\n\
         seiva-http = \"{FRAMEWORK_VERSION}\"\n\
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
         seiva-ports = \"{FRAMEWORK_VERSION}\"\n\n\
         [dev-dependencies]\n\
         tokio = {{ version = \"1\", features = [\"rt\", \"macros\"] }}\n"
    )
}

fn infrastructure_cargo_toml(module: &str) -> String {
    format!(
        "[package]\nname = \"{module}-infrastructure\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         {module}-application = {{ path = \"../application\" }}\n\
         {module}-domain = {{ path = \"../domain\" }}\n\
         seiva-ports = \"{FRAMEWORK_VERSION}\"\n\
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
        let files = project_files("shop").unwrap();
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
        let files = project_files("shop").unwrap();
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
        assert!(!main.contains("println!"), "the stub must be gone");

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
        let work = std::env::temp_dir().join(format!(
            "seiva-scaffold-fitness-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).expect("create work dir");

        // Project skeleton, then a module, then the workspace registration edit
        // the CLI performs so the module crates are built too.
        let mut project = project_files("demo").expect("project files");
        let modules = module_files("notes").expect("module files");
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

        write_all(&work, &project);
        write_all(&work, &modules);

        let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let target_dir = std::env::temp_dir().join("seiva-scaffold-fitness-target");
        let status = std::process::Command::new(cargo_bin)
            .arg("build")
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
        let files = module_files("notes").unwrap();
        // One module.toml plus a Cargo.toml and a src/lib.rs per layer = 11.
        assert_eq!(files.len(), 11, "expected 1 manifest + 5 crates * 2 files");
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
        let files = module_files("notes").unwrap();
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
        let files = module_files("order_items").unwrap();
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
        let files = module_files("notes").unwrap();
        let facade = contents(&files, "modules/notes/facade/src/lib.rs");
        // The macro is imported and applied with an explicit version.
        assert!(facade.contains("use delonix_seiva::facade;"));
        assert!(facade.contains("#[facade(version = \"v1\")]"));
        // At least one method carries a `#[requires(..)]`, keyed to the module.
        assert!(facade.contains("#[requires(\"notes.create\")]"));
        // The trait is named after the module in PascalCase.
        assert!(facade.contains("pub trait Notes {"));
        assert!(facade.contains(
            "async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError>;"
        ));
        // Wire DTOs: serde + Validate on the write DTO, Serialize on the response,
        // error → Problem mapping. This is the controller-test shape.
        assert!(facade.contains("#[derive(Debug, Clone, Deserialize, Serialize)]"));
        assert!(facade.contains("impl Validate for CreateRecord"));
        assert!(facade.contains("impl From<RecordError> for Problem"));
    }

    #[test]
    fn generated_application_implements_the_facade_over_ports() {
        let files = module_files("notes").unwrap();
        let app = contents(&files, "modules/notes/application/src/lib.rs");
        assert!(app.contains("use seiva_ports::{BoxError, IdGenerator, PortResult};"));
        assert!(app.contains("pub trait RecordRepository {"));
        assert!(app.contains("impl<R, G> Notes for RecordService<R, G>"));
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
        let files = module_files("notes").unwrap();
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

    #[test]
    fn generated_interface_holds_controller_and_transport_generation() {
        let files = module_files("notes").unwrap();
        let interface = contents(&files, "modules/notes/interface/src/lib.rs");
        // The controller macro is imported and applied to a GENERIC adapter impl:
        // generic over the repository/id-generator ports, so the interface never
        // names the infrastructure crate (the layering rule).
        assert!(interface.contains("use delonix_seiva::controller;"));
        assert!(interface.contains("#[controller]"));
        assert!(interface.contains("pub struct NotesController<R, G>(Arc<RecordService<R, G>>);"));
        assert!(interface.contains("impl<R, G> Notes for NotesController<R, G>"));
        // It depends on application (allowed), NOT infrastructure.
        assert!(interface.contains("use notes_application::{RecordRepository, RecordService};"));
        assert!(!interface.contains("notes_infrastructure"));
        // The ports are injected via `new(repository, ids)`; there is no no-arg
        // `router()` default that would need to name a concrete adapter.
        assert!(interface.contains("pub fn new(repository: R, ids: G) -> Self {"));
        assert!(!interface.contains("pub fn router()"));
        // OpenAPI generation from the facade descriptor. Proto generation is NOT
        // per-module (it would pull the gRPC generator into every module crate);
        // it is available from the same descriptor via the `seiva proto` CLI.
        assert!(interface.contains("seiva_openapi::openapi_document("));
        assert!(!interface.contains("seiva_grpc"));
        assert!(interface.contains("NotesFacade::DESCRIPTOR"));
        // The large-Err lint allowance the generated handlers need.
        assert!(interface.contains("#![allow(clippy::result_large_err)]"));
    }

    #[test]
    fn module_manifests_depend_on_the_right_framework_crates() {
        let files = module_files("notes").unwrap();

        // The framework crates are depended on by crates.io VERSION (so a
        // generated project compiles standalone once they are published); the
        // intra-project layer crates stay `path`.
        let facade = contents(&files, "modules/notes/facade/Cargo.toml");
        assert!(facade.contains("delonix-seiva = { version = \"0.1\", default-features = false }"));
        assert!(facade.contains("seiva-http = \"0.1\""));
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
        let files = module_files("notes").unwrap();
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
    fn seiva_toml_registration_appends_section() {
        let seiva = seiva_toml("shop");
        let out = register_module_in_seiva_toml(&seiva, "notes").unwrap();
        assert!(out.contains("[modules.notes]"));
        assert!(register_module_in_seiva_toml(&out, "notes").is_err());
    }

    #[test]
    fn project_name_reads_the_manifest() {
        assert_eq!(project_name(&seiva_toml("shop")).as_deref(), Some("shop"));
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
}
