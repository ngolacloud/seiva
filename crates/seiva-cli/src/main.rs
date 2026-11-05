//! The `seiva` binary: reads arguments and environment, then calls the library.
//!
//! The command tree below is the shape fixed by the brief. Only `doctor` and
//! `--version` do real work in this founding build; every other command reports
//! the phase that will implement it rather than faking success. As each command
//! lands, its body becomes a thin call into a public `delonix-seiva` function
//! with a typed result (the "CLI as a library" rule, mirrored from DORM).

use clap::{Parser, Subcommand};
use delonix_seiva::contract::{Facade, FacadeDescriptor};
use delonix_seiva::scaffold::{self, GeneratedFile};
use notes_facade::NotesFacade;
use seiva_grpc::proto_document;
use seiva_openapi::{openapi_document, ApiInfo};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The facades this `seiva` binary can render, discovered through the link-time
/// inventory ([`delonix_seiva::contract::registered_facades`]) rather than a
/// hard-coded list.
///
/// A standalone CLI only links the demonstration facade the framework ships
/// (`notes-facade`'s `NotesFacade`), so that is what `seiva openapi`/`seiva proto`
/// document here — the CLI is not compiled against a user's project crates. The
/// *runtime* binary that mounts a project's modules links their facades, so its
/// `/openapi.json` is the complete, discovered document. Referencing
/// `NotesFacade` below anchors the demo crate into this binary so its
/// `#[seiva::facade]` registration runs.
fn cli_facades() -> Vec<FacadeDescriptor> {
    let _anchor = NotesFacade::DESCRIPTOR;
    delonix_seiva::contract::registered_facades()
}

#[derive(Parser)]
#[command(
    name = "seiva",
    version,
    about = "Delonix Seiva — a Rust backend framework for robust, high-criticality systems."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new Seiva project.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage modules within a project.
    Module {
        #[command(subcommand)]
        action: ModuleAction,
    },
    /// Add or manage optional extensions (e.g. the admin console).
    Ext {
        #[command(subcommand)]
        action: ExtAction,
    },
    /// Data-layer commands (DORM modules only).
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Run the architecture fitness functions locally.
    Lint,
    /// Show what the Seiva macros expand to.
    Expand,
    /// Run the project locally (dev loop).
    ///
    /// Rebuilds and restarts on change when `cargo watch` is available (the
    /// FastAPI `--reload` equivalent). `--watch` requires it (and errors if it is
    /// missing); `--no-watch` forces a plain `cargo run`; with neither, hot-reload
    /// is auto-detected.
    Start {
        /// Force hot-reload; error if `cargo watch` is not installed.
        #[arg(long, conflicts_with = "no_watch")]
        watch: bool,
        /// Disable hot-reload (a plain `cargo run`).
        #[arg(long)]
        no_watch: bool,
    },
    /// Report the environment: framework version and which data layer is linked.
    Doctor,
    /// Print the OpenAPI 3.1 document.
    ///
    /// Inside a project (a `seiva.toml` is present) this documents THAT project's
    /// facades: it builds and runs the composition root with `SEIVA_EMIT=openapi`,
    /// which emits the document from the modules linked into the binary. Outside a
    /// project it falls back to the framework's built-in demonstration facade.
    Openapi {
        /// Pretty-print the JSON (the default when no style flag is given).
        #[arg(long, conflicts_with = "compact")]
        pretty: bool,
        /// Emit compact, single-line JSON instead of the pretty default.
        #[arg(long)]
        compact: bool,
    },
    /// Print the proto3 service document.
    ///
    /// Inside a project (a `seiva.toml` is present) this documents THAT project's
    /// facades via `SEIVA_EMIT=proto` on the composition root; outside a project it
    /// falls back to the framework's built-in demonstration facade.
    Proto,
    /// Manage user credential records.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
}

#[derive(Subcommand)]
enum UserAction {
    /// Create a user credential record: `seiva user create --username <NAME>`.
    ///
    /// The password is read as one line from STDIN (never from an argv flag,
    /// which would leak into shell history); pipe it in to run non-interactively,
    /// e.g. `printf 'secret\n' | seiva user create --username alice`. The record
    /// is printed as JSON to stdout with the Argon2id PHC hash of the password.
    /// A later phase persists the record; this cut proves the hashing path from
    /// the CLI.
    Create {
        /// The username for the new credential record.
        #[arg(long)]
        username: String,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Scaffold a new project: `seiva project new <name>`.
    New { name: String },
}

#[derive(Subcommand)]
enum ModuleAction {
    /// Scaffold a new module: `seiva module new <name> [--data external] [--admin]`.
    New {
        name: String,
        /// Use an external data layer instead of DORM for this module.
        #[arg(long)]
        data: Option<String>,
        /// Register the module with the admin console (ADR 0009): the generated
        /// interface declares an `AdminResource`, so the optional
        /// `seiva-console-admin` extension surfaces it (gated by the module's
        /// permissions). Without this flag the module stays private to the console.
        #[arg(long)]
        admin: bool,
    },
    /// Move a module to `mode = "service"` with its own schema and history.
    Split { name: String },
    /// Fold a service module back into the project schema.
    Merge { name: String },
}

#[derive(Subcommand)]
enum ExtAction {
    /// Integrate an optional extension into the composition root (ADR 0010):
    /// `seiva ext add seiva-console-admin`. Adds the dependency and inserts the
    /// mount at the managed composition point, then you recompile. Explicit and
    /// idempotent — running it twice changes nothing — and it executes no install
    /// script. For a project without the managed anchors, it prints the exact
    /// patch to apply by hand and reports integration as pending.
    Add { name: String },
}

#[derive(Subcommand)]
enum DbAction {
    /// Set up the project's DORM data layer: scaffold a DORM project under `data/`
    /// (schema, migrations, the migrate binaries) that `seiva db dev`/`migrate`
    /// drive. Delegates to `dorm init`; run once per project. See `seiva db dev`.
    Init {
        /// The directory to create the DORM project in (default `data`).
        #[arg(long)]
        dir: Option<String>,
        /// The database backend: `postgres` (default), `mysql` or `sqlite`. Sets
        /// the DORM feature and the schema's `provider`; use a matching DSN.
        #[arg(long, default_value = "postgres")]
        backend: String,
    },
    /// Author the next migration from the schema and apply it — the dev loop
    /// (`prisma migrate dev` / Django `makemigrations` + `migrate`). Delegates to
    /// `dorm migrate dev` inside the project's `data/` DORM project.
    Dev {
        /// A short name for the migration (e.g. `add_price`).
        #[arg(default_value = "migration")]
        name: String,
    },
    /// Apply the recorded migration chain (`prisma migrate deploy`). Delegates to
    /// `dorm migrate` inside the project's `data/` DORM project.
    Migrate,
    /// Introspect an existing database into a schema file (brownfield adoption).
    ///
    /// Delegates to the DORM CLI (`dorm db pull`): it reads the live PostgreSQL
    /// `public` schema and writes approximate `model` blocks you fold into your
    /// modules' `namespace`s. The DSN comes from `--dsn` or the `DELONIX_ORM_DSN`
    /// environment variable. Needs the `dorm` CLI on PATH (or `SEIVA_DORM_BIN`).
    Pull {
        /// Where to write the pulled schema (default `schema/<project>.pulled.schema`).
        #[arg(long)]
        out: Option<String>,
        /// The database DSN to introspect (else `DELONIX_ORM_DSN`).
        #[arg(long)]
        dsn: Option<String>,
    },
    /// Browse and edit the database in your browser — the data admin, Django/Prisma
    /// Studio style. Delegates to `dorm studio` (native: reads the live schema, no
    /// project build). Read-only unless `--write`. Binds 127.0.0.1 only.
    Studio {
        /// TCP port to serve on (default 5555).
        #[arg(long)]
        port: Option<u16>,
        /// Allow writes — create, edit and delete rows (off by default).
        #[arg(long)]
        write: bool,
        /// The database DSN (else `DELONIX_ORM_DSN`).
        #[arg(long)]
        dsn: Option<String>,
    },
}

fn not_yet(what: &str, phase: &str) -> ExitCode {
    eprintln!("seiva: `{what}` is not implemented yet (planned for {phase}).");
    // Exit 64 (EX_USAGE-adjacent): a well-formed command we cannot serve yet,
    // distinct from an unknown command (clap exits 2) and from failure (1).
    ExitCode::from(64)
}

/// Writes generated files under `root`, refusing to overwrite any that exist.
fn write_files(root: &Path, files: &[GeneratedFile]) -> std::io::Result<()> {
    for file in files {
        let full = root.join(&file.path);
        if full.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} already exists", full.display()),
            ));
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full, &file.contents)?;
    }
    Ok(())
}

/// The framework workspace root a generated project's interim `[patch.crates-io]`
/// points at (see [`scaffold::workspace_patch_section`]). `SEIVA_FRAMEWORK_DIR`
/// overrides it; otherwise it is the checkout this `seiva` binary was built from —
/// the CLI crate's manifest dir (`<root>/crates/seiva-cli`) is two levels below
/// the workspace root. Canonicalized when the path exists so the generated
/// manifest carries an absolute path.
fn framework_root() -> String {
    if let Ok(dir) = std::env::var("SEIVA_FRAMEWORK_DIR") {
        return dir;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().and_then(Path::parent).unwrap_or(manifest);
    std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn project_new(name: &str) -> ExitCode {
    let files = match scaffold::project_files(name) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("seiva: {err}");
            return ExitCode::FAILURE;
        }
    };
    let root = PathBuf::from(name);
    if root.exists() {
        eprintln!("seiva: `{name}` already exists");
        return ExitCode::FAILURE;
    }
    if let Err(err) = write_files(&root, &files) {
        eprintln!("seiva: {err}");
        return ExitCode::FAILURE;
    }
    // Interim until the framework crates are published: append a
    // `[patch.crates-io]` redirecting them to this checkout, so `cargo build`
    // resolves the `= "0.1"` dependencies standalone.
    let manifest = root.join("Cargo.toml");
    if let Err(err) = append_patch_section(&manifest, &framework_root()) {
        eprintln!("seiva: writing {}: {err}", manifest.display());
        return ExitCode::FAILURE;
    }
    println!("Created project `{name}`. Next: cd {name} && seiva module new <name>");
    ExitCode::SUCCESS
}

/// Appends the framework `[patch.crates-io]` section to the generated workspace
/// manifest at `manifest`.
fn append_patch_section(manifest: &Path, framework_root: &str) -> std::io::Result<()> {
    let mut cargo = std::fs::read_to_string(manifest)?;
    cargo.push_str(&scaffold::workspace_patch_section(framework_root));
    std::fs::write(manifest, cargo)
}

fn module_new(name: &str, external: bool, admin: bool) -> ExitCode {
    if let Err(err) = scaffold::validate_name(name) {
        eprintln!("seiva: {err}");
        return ExitCode::FAILURE;
    }
    let root = Path::new(".");
    if !root.join("seiva.toml").is_file() {
        eprintln!(
            "seiva: no seiva.toml here — run this inside a project (see `seiva project new`)."
        );
        return ExitCode::FAILURE;
    }
    if external {
        eprintln!(
            "seiva: `--data external` is not implemented yet (planned for the data-layer phase)."
        );
        return ExitCode::from(64);
    }

    let files = match scaffold::module_files(name, admin) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("seiva: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = write_files(root, &files) {
        eprintln!("seiva: {err}");
        return ExitCode::FAILURE;
    }

    // Registration edits: workspace members, seiva.toml, project schema. Each
    // is a pure transform; read, transform, write back.
    if let Err(err) = register_module(root, name) {
        eprintln!("seiva: module files written, but registration failed: {err}");
        return ExitCode::FAILURE;
    }
    let admin_note = if admin {
        " Registered an admin resource (mount `seiva-console-admin` to surface it)."
    } else {
        ""
    };
    println!("Created module `{name}` (facade v1, 5 layers). Registered in the workspace, seiva.toml and schema, and wired into the composition root.{admin_note}");
    ExitCode::SUCCESS
}

/// Applies the three registration edits for a new module.
fn register_module(root: &Path, name: &str) -> Result<(), String> {
    let edit = |path: PathBuf,
                transform: &dyn Fn(&str) -> Result<String, scaffold::ScaffoldError>|
     -> Result<(), String> {
        let current = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let next = transform(&current).map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::write(&path, next).map_err(|e| format!("writing {}: {e}", path.display()))
    };

    edit(root.join("Cargo.toml"), &|s| {
        scaffold::add_members_to_workspace(s, name)
    })?;
    edit(root.join("seiva.toml"), &|s| {
        scaffold::register_module_in_seiva_toml(s, name)
    })?;

    // The schema file is named after the project; read its name from seiva.toml.
    let seiva = std::fs::read_to_string(root.join("seiva.toml"))
        .map_err(|e| format!("reading seiva.toml: {e}"))?;
    let project = project_name(&seiva).ok_or("could not read [project] name from seiva.toml")?;
    edit(root.join(format!("schema/{project}.schema")), &|s| {
        scaffold::append_namespace_block(s, name)
    })?;

    // Auto-wire the module into the composition root: add its interface/
    // infrastructure deps to the app crate, and merge its router in `main.rs`.
    // This is what makes `seiva openapi` document the module (its facade only
    // registers once the binary links it), and saves the hand-wiring step.
    let app_dir = root.join(format!("apps/{project}-api"));
    edit(app_dir.join("Cargo.toml"), &|s| {
        scaffold::add_module_to_app_manifest(s, name)
    })?;
    edit(app_dir.join("src/main.rs"), &|s| {
        scaffold::wire_module_in_main(s, name)
    })?;
    Ok(())
}

/// Extracts the `[project] name = "..."` value from seiva.toml text.
fn project_name(seiva: &str) -> Option<String> {
    seiva
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

/// `seiva ext add <name>`: integrate an optional extension into the composition
/// root (ADR 0010). Explicit and idempotent; executes no install script. On a
/// project without the managed anchors it prints the exact patch and reports the
/// integration as pending instead of guessing.
fn ext_add(name: &str) -> ExitCode {
    let root = Path::new(".");
    if !root.join("seiva.toml").is_file() {
        eprintln!(
            "seiva: no seiva.toml here — run this inside a project (see `seiva project new`)."
        );
        return ExitCode::FAILURE;
    }
    let ext = match scaffold::extension_manifest(name) {
        Some(ext) => ext,
        None => {
            eprintln!(
                "seiva: unknown extension `{name}`. Known: seiva-console-admin.\n       \
                 A third-party extension ships a `seiva-extension.toml`; wire it with the\n       \
                 independent `dx` tool, or add the dependency and its mount by hand."
            );
            return ExitCode::from(64);
        }
    };

    let seiva = match std::fs::read_to_string(root.join("seiva.toml")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("seiva: reading seiva.toml: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(project) = project_name(&seiva) else {
        eprintln!("seiva: could not read [project] name from seiva.toml");
        return ExitCode::FAILURE;
    };
    let app_dir = root.join(format!("apps/{project}-api"));
    let cargo_path = app_dir.join("Cargo.toml");
    let main_path = app_dir.join("src/main.rs");

    let (Ok(cargo), Ok(main)) = (
        std::fs::read_to_string(&cargo_path),
        std::fs::read_to_string(&main_path),
    ) else {
        return ext_add_manual(&ext, "the composition root was not found where expected");
    };

    // Compute BOTH edits before writing either: a missing anchor must leave the
    // project untouched and fall back to a manual patch, never half-applied.
    let new_cargo = scaffold::add_extension_to_app_manifest(&cargo, &ext);
    let new_main = scaffold::mount_extension_in_main(&main, &ext);
    let (Ok(new_cargo), Ok(new_main)) = (new_cargo, new_main) else {
        return ext_add_manual(&ext, "this project has no managed extension anchors");
    };

    let unchanged = new_cargo == cargo && new_main == main;
    if let Err(e) = std::fs::write(&cargo_path, &new_cargo) {
        eprintln!("seiva: writing {}: {e}", cargo_path.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&main_path, &new_main) {
        eprintln!("seiva: writing {}: {e}", main_path.display());
        return ExitCode::FAILURE;
    }

    if unchanged {
        println!("`{name}` is already integrated — nothing to do.");
    } else {
        println!(
            "Added `{name}` to `{project}-api` ({}). Recompile to serve it: `cargo build` (or `seiva start`).",
            ext.summary
        );
    }
    ExitCode::SUCCESS
}

/// Prints the exact manual patch for an extension and reports integration as
/// pending — the fallback for a project the CLI does not recognise as a managed
/// composition root. Returns a distinct exit code so scripts can detect it.
fn ext_add_manual(ext: &scaffold::ExtensionManifest, why: &str) -> ExitCode {
    eprintln!(
        "seiva: {why}; integration is PENDING.\n\n\
         Apply this by hand (the Cargo equivalent of `dx add`):\n\n\
         1. In the composition root's Cargo.toml `[dependencies]`:\n\
         \x20      {crate} = \"{ver}\"\n\n\
         2. In its `main.rs`, after `let app = router_with_modules(...);`:\n\
         \x20      {mount}\n\n\
         3. Recompile (`cargo build`). A declared dependency alone does not mount\n\
         \x20   routes — the mount statement is what wires it, explicitly.",
        crate = ext.crate_name,
        ver = ext.version_req,
        mount = ext.mount_stmt,
    );
    ExitCode::from(65)
}

fn lint() -> ExitCode {
    // CLI stays thin: scan the project and let the library decide. Run from the
    // project root; module manifests live at `modules/<name>/module.toml`.
    let root = std::path::Path::new(".");
    match delonix_seiva::lint::lint_project(root) {
        Ok(report) if report.modules_checked == 0 => {
            println!("seiva lint: no modules found under ./modules (nothing to check).");
            ExitCode::SUCCESS
        }
        Ok(report) if report.is_clean() => {
            println!(
                "seiva lint: contract check passed for {} module(s).",
                report.modules_checked
            );
            ExitCode::SUCCESS
        }
        Ok(report) => {
            eprintln!(
                "seiva lint: {} contract violation(s) across {} module(s):",
                report.violations.len(),
                report.modules_checked
            );
            for violation in &report.violations {
                eprintln!(
                    "  [{}] {}: {}",
                    violation.rule, violation.module, violation.message
                );
            }
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("seiva lint: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Builds the OpenAPI 3.1 document for the built-in facades as a JSON string.
///
/// `compact` chooses the single-line form; otherwise the JSON is pretty-printed.
/// The document's `info.version` is the framework version, so the served
/// contract tracks the installed `seiva`.
fn builtin_openapi_json(compact: bool) -> String {
    let info = ApiInfo {
        title: "Seiva API (built-in facades)".to_string(),
        version: delonix_seiva::VERSION.to_string(),
    };
    let document = openapi_document(info, &cli_facades());
    if compact {
        serde_json::to_string(&document).expect("a serde_json::Value always serialises")
    } else {
        serde_json::to_string_pretty(&document).expect("a serde_json::Value always serialises")
    }
}

/// Renders the proto3 service document(s) for the built-in facades.
///
/// One document per facade, separated by a blank line. Today there is a single
/// built-in facade, so this is `proto_document` of `NotesFacade::DESCRIPTOR`.
fn builtin_proto() -> String {
    cli_facades()
        .iter()
        .map(proto_document)
        .collect::<Vec<_>>()
        .join("\n")
}

/// In-process ports that let the CLI — the application/composition layer — drive
/// the real `accounts` module use case without the DORM data layer.
///
/// Persistence is in-memory and lives only for the command's duration; a
/// DORM-backed repository and the transactional ports arrive through the
/// framework's `dorm` feature. The point today is that `seiva user create` runs
/// the SAME `Accounts::create_user` use case a server would — validation, Argon2id
/// hashing and the `accounts.user_created` event — not an ad-hoc hash.
mod app_ports {
    use async_trait::async_trait;
    use seiva_ports::{EventPublisher, Id, IdGenerator, PortResult, UnitOfWork};
    use std::future::Future;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A time-seeded [`IdGenerator`]: the 16 id bytes are the current time in
    /// nanoseconds, big-endian. Unique enough for a single-shot CLI create; the
    /// DORM gateway supplies a UUID generator for the persistent path.
    pub struct SystemIds;

    impl IdGenerator for SystemIds {
        fn new_id(&self) -> Id {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            Id(nanos.to_be_bytes())
        }
    }

    /// A pass-through [`UnitOfWork`]: it runs the use-case body directly, with no
    /// real transaction. The DORM gateway's unit of work wraps the body in a real
    /// database transaction and the transactional outbox.
    pub struct DirectUnitOfWork;

    impl UnitOfWork for DirectUnitOfWork {
        fn transaction<'a, T, F, Fut>(
            &'a self,
            work: F,
        ) -> std::pin::Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
        where
            F: FnOnce() -> Fut + Send + 'a,
            Fut: Future<Output = PortResult<T>> + Send + 'a,
            T: Send + 'a,
        {
            Box::pin(async move { work().await })
        }
    }

    /// An [`EventPublisher`] that reports the emitted event on stderr so the CLI
    /// run shows the use case fired it. The DORM gateway writes it to the
    /// transactional outbox instead.
    pub struct StderrEventPublisher;

    #[async_trait]
    impl EventPublisher for StderrEventPublisher {
        async fn publish(&self, topic: &str, _payload: &[u8]) -> PortResult<()> {
            eprintln!("seiva: event emitted — {topic}");
            Ok(())
        }
    }
}

/// Runs the `accounts` module's `create_user` use case over an in-memory
/// repository and the in-process [`app_ports`], returning the created user as
/// JSON (`{ "id", "username" }` — never the password or its hash, by the
/// facade's contract).
///
/// # Errors
///
/// Returns the module's error message (e.g. a rejected username or a duplicate)
/// or a runtime-construction failure, as a `String`.
fn create_user_via_module(username: &str, password: &str) -> Result<serde_json::Value, String> {
    use accounts_application::AccountsService;
    use accounts_infrastructure::InMemoryCredentialRepository;
    use accounts_interface::AccountsApi;

    let service = AccountsService {
        repository: InMemoryCredentialRepository::new(),
        ids: app_ports::SystemIds,
        uow: app_ports::DirectUnitOfWork,
        events: app_ports::StderrEventPublisher,
    };
    let api = AccountsApi::new(service);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|err| format!("failed to start a runtime: {err}"))?;
    let dto = runtime.block_on(api.create_user(username.to_string(), password.to_string()))?;

    Ok(serde_json::json!({
        // `id` is a u128; render it as a string so it survives any JSON consumer.
        "id": dto.id.to_string(),
        "username": dto.username,
    }))
}

/// Reads a single line from STDIN, stripping the trailing newline (CR/LF).
fn read_password_line() -> std::io::Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// `seiva openapi`: prints the OpenAPI 3.1 document. Inside a project (a
/// `seiva.toml` is present) it documents **that project's** facades by building
/// and running the composition root with `SEIVA_EMIT=openapi` — which emits the
/// document from the modules linked into the binary, then exits. Outside a
/// project it falls back to the built-in demonstration facade(s).
fn openapi(compact: bool) -> ExitCode {
    match std::fs::read_to_string("seiva.toml") {
        Ok(manifest) => match scaffold::project_name(&manifest) {
            Some(name) => emit_project_openapi(&name, compact),
            None => {
                eprintln!("seiva: seiva.toml has no `[project] name`.");
                ExitCode::FAILURE
            }
        },
        Err(_) => {
            eprintln!(
                "seiva: no project here — documenting the built-in demonstration facade(s). \
                 Run inside a project (see `seiva project new`) to document your own."
            );
            println!("{}", builtin_openapi_json(compact));
            ExitCode::SUCCESS
        }
    }
}

/// Builds and runs `apps/<name>-api` with `SEIVA_EMIT=openapi` to print the
/// project's own OpenAPI document (from its linked modules). Build output goes to
/// stderr (inherited); only the document reaches stdout, so it is pipeable.
fn emit_project_openapi(name: &str, compact: bool) -> ExitCode {
    let app = format!("{name}-api");
    eprintln!("seiva: building `{app}` to document this project's facades ...");
    let output = match std::process::Command::new("cargo")
        .args(["run", "--quiet", "--bin", &app])
        .env("SEIVA_EMIT", "openapi")
        .stderr(std::process::Stdio::inherit())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            eprintln!("seiva: failed to run `{app}`: {error}");
            return ExitCode::FAILURE;
        }
    };
    if !output.status.success() {
        eprintln!("seiva: `{app}` exited with {}", output.status);
        return ExitCode::from(1);
    }
    let json = String::from_utf8_lossy(&output.stdout);
    if compact {
        // Re-serialise compactly; fall back to the raw pretty output on any error.
        match serde_json::from_str::<serde_json::Value>(json.trim()) {
            Ok(value) => println!(
                "{}",
                serde_json::to_string(&value).unwrap_or_else(|_| json.to_string())
            ),
            Err(_) => print!("{json}"),
        }
    } else {
        print!("{json}");
    }
    ExitCode::SUCCESS
}

/// `seiva start`: runs the project's `apps/<name>-api` server in development,
/// with hot-reload when `cargo watch` is available.
///
/// Resolves the project name from `seiva.toml` (so the app crate is
/// `<name>-api`), sets `APP_ENV=development`, and execs cargo. With `cargo watch`
/// installed it rebuilds-and-restarts on change (the FastAPI `--reload`
/// equivalent); without it, a plain `cargo run` plus a hint to install it.
fn start(watch: bool, no_watch: bool) -> ExitCode {
    let manifest = match std::fs::read_to_string("seiva.toml") {
        Ok(contents) => contents,
        Err(_) => {
            eprintln!(
                "seiva: no seiva.toml here — run this inside a project (see `seiva project new`)."
            );
            return ExitCode::FAILURE;
        }
    };
    let name = match scaffold::project_name(&manifest) {
        Some(name) => name,
        None => {
            eprintln!("seiva: seiva.toml has no `[project] name`.");
            return ExitCode::FAILURE;
        }
    };
    let app = format!("{name}-api");

    let hot_reload = match resolve_reload(watch, no_watch, has_cargo_watch()) {
        Ok(reload) => reload,
        Err(message) => {
            eprintln!("seiva: {message}");
            return ExitCode::FAILURE;
        }
    };
    if !hot_reload && !no_watch {
        eprintln!(
            "seiva: `cargo watch` not found — running without hot-reload \
             (install it with `cargo install cargo-watch`, or pass `--no-watch`)."
        );
    }
    let (program, args) = start_plan(&app, hot_reload);
    println!(
        "seiva: starting `{app}` in development{} ...",
        if hot_reload { " with hot-reload" } else { "" }
    );

    match std::process::Command::new(&program)
        .args(&args)
        .env("APP_ENV", "development")
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(error) => {
            eprintln!("seiva: failed to launch `{program}`: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Decides whether `start` runs with hot-reload, given the `--watch`/`--no-watch`
/// flags and whether `cargo watch` is available. Pure, so the policy is
/// unit-tested: `--no-watch` wins (plain run); `--watch` requires `cargo watch`
/// (an error names how to install it); with neither, reload is auto-detected.
fn resolve_reload(
    watch: bool,
    no_watch: bool,
    cargo_watch_available: bool,
) -> Result<bool, String> {
    if no_watch {
        return Ok(false);
    }
    if watch && !cargo_watch_available {
        return Err(
            "`--watch` needs `cargo watch` — install it with `cargo install cargo-watch`, \
             or run without `--watch`."
                .to_owned(),
        );
    }
    Ok(watch || cargo_watch_available)
}

/// The command and arguments `start` runs, split out as a pure function so both
/// branches (hot-reload and plain) are unit-tested without spawning a process.
fn start_plan(app: &str, has_cargo_watch: bool) -> (String, Vec<String>) {
    if has_cargo_watch {
        (
            "cargo".to_owned(),
            vec![
                "watch".to_owned(),
                "-x".to_owned(),
                format!("run --bin {app}"),
            ],
        )
    } else {
        (
            "cargo".to_owned(),
            vec!["run".to_owned(), "--bin".to_owned(), app.to_owned()],
        )
    }
}

/// Whether `cargo watch` is available (so `start` can offer hot-reload).
fn has_cargo_watch() -> bool {
    std::process::Command::new("cargo")
        .args(["watch", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The DORM CLI binary `seiva db` delegates to. `SEIVA_DORM_BIN` overrides it
/// (so a locally built `dorm` can be used without installing); otherwise `dorm`
/// is resolved from `PATH`.
fn dorm_bin() -> String {
    std::env::var("SEIVA_DORM_BIN").unwrap_or_else(|_| "dorm".to_owned())
}

/// The command and arguments `seiva db pull` runs. Pure, so the delegation is
/// unit-tested without spawning a process: a global `--dsn` (when given) precedes
/// the `db pull --out <path>` subcommand, matching the DORM CLI's argument shape.
fn db_pull_plan(dorm: &str, out: &str, dsn: Option<&str>) -> (String, Vec<String>) {
    let mut args: Vec<String> = Vec::new();
    if let Some(dsn) = dsn {
        args.push("--dsn".to_owned());
        args.push(dsn.to_owned());
    }
    args.extend(["db", "pull", "--out", out].map(str::to_owned));
    (dorm.to_owned(), args)
}

/// `seiva db pull`: introspect an existing database into a schema file, by
/// delegating to the DORM CLI (`dorm db pull`). DORM is a separate product with
/// its own CLI, so Seiva orchestrates rather than linking it. Run inside a
/// project; the default output is `schema/<project>.pulled.schema` (a separate
/// file, so the managed `schema/<project>.schema` is never clobbered).
fn db_pull(out: Option<String>, dsn: Option<String>) -> ExitCode {
    let manifest = match std::fs::read_to_string("seiva.toml") {
        Ok(contents) => contents,
        Err(_) => {
            eprintln!(
                "seiva: no seiva.toml here — run this inside a project (see `seiva project new`)."
            );
            return ExitCode::FAILURE;
        }
    };
    let Some(name) = scaffold::project_name(&manifest) else {
        eprintln!("seiva: seiva.toml has no `[project] name`.");
        return ExitCode::FAILURE;
    };
    let out = out.unwrap_or_else(|| format!("schema/{name}.pulled.schema"));

    let dorm = dorm_bin();
    let (program, args) = db_pull_plan(&dorm, &out, dsn.as_deref());
    eprintln!("seiva: introspecting via `{dorm} db pull` → {out} ...");
    match std::process::Command::new(&program).args(&args).status() {
        Ok(status) if status.success() => {
            println!(
                "Pulled the schema into `{out}`. Review it and fold each `model` into \
                 the owning module's `namespace` in schema/{name}.schema."
            );
            ExitCode::SUCCESS
        }
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "seiva: could not run `{program}`: {error}\n\
                 seiva db pull needs the DORM CLI — install it (`cargo install dorm-cli`) \
                 or set SEIVA_DORM_BIN to its path."
            );
            ExitCode::FAILURE
        }
    }
}

/// The DORM CLI arguments for `seiva db studio`. Pure, so it is unit-tested: a
/// global `--dsn` (when given) precedes `studio`, then the optional `--port` and
/// the `--write` flag.
fn dorm_studio_args(port: Option<u16>, write: bool, dsn: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(dsn) = dsn {
        args.push("--dsn".to_owned());
        args.push(dsn.to_owned());
    }
    args.push("studio".to_owned());
    if let Some(port) = port {
        args.push("--port".to_owned());
        args.push(port.to_string());
    }
    if write {
        args.push("--write".to_owned());
    }
    args
}

/// `seiva db studio`: serve the data admin — browse (and, with `--write`, edit)
/// the database in a browser — by delegating to `dorm studio`. Native: it reads
/// the live schema and needs no project build, so it runs anywhere with a DSN
/// (`--dsn` or `DELONIX_ORM_DSN`). Serves on 127.0.0.1 until interrupted.
fn db_studio(port: Option<u16>, write: bool, dsn: Option<String>) -> ExitCode {
    let dorm = dorm_bin();
    let args = dorm_studio_args(port, write, dsn.as_deref());
    eprintln!(
        "seiva: opening the data admin via `{dorm} studio`{} (Ctrl-C to stop) ...",
        if write {
            " (writes enabled)"
        } else {
            " (read-only)"
        }
    );
    match std::process::Command::new(&dorm).args(&args).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "seiva: could not run `{dorm}`: {error}\n\
                 seiva db studio needs the DORM CLI (set SEIVA_DORM_BIN or install dorm-cli)."
            );
            ExitCode::FAILURE
        }
    }
}

/// The directory the project's DORM data layer lives in (the DORM project
/// `seiva db dev`/`migrate` drive). Fixed as `data/` for this first cut.
const DORM_PROJECT_DIR: &str = "data";

/// A local DORM checkout to point the generated DORM project's interim
/// `[patch.crates-io]` at, until DORM publishes: `SEIVA_DORM_DIR` if set, else the
/// sibling `../DORM` beside the framework checkout, when it exists. `None` means
/// "assume DORM is published" (no patch written) — the real-user path.
fn dorm_checkout() -> Option<String> {
    if let Ok(dir) = std::env::var("SEIVA_DORM_DIR") {
        return Some(dir);
    }
    let sibling = Path::new(&framework_root()).parent()?.join("DORM");
    sibling
        .is_dir()
        .then(|| sibling.to_string_lossy().into_owned())
}

/// Reads the `[project] name` from `seiva.toml`, or prints an error and returns
/// `None` when not in a project — the common guard for the `db` subcommands.
fn require_project() -> Option<String> {
    match std::fs::read_to_string("seiva.toml") {
        Ok(manifest) => match scaffold::project_name(&manifest) {
            Some(name) => Some(name),
            None => {
                eprintln!("seiva: seiva.toml has no `[project] name`.");
                None
            }
        },
        Err(_) => {
            eprintln!(
                "seiva: no seiva.toml here — run this inside a project (see `seiva project new`)."
            );
            None
        }
    }
}

/// `seiva db init`: scaffold the project's DORM data layer under `data/` by
/// delegating to `dorm init`, then (until DORM publishes) point its
/// `[patch.crates-io]` at a local DORM checkout so it builds. Idempotent-ish:
/// refuses if the directory already exists (DORM's `init` does).
fn db_init(dir: Option<String>, backend: String) -> ExitCode {
    let Some(project) = require_project() else {
        return ExitCode::FAILURE;
    };
    // Map the backend to the DORM feature to enable and the datasource provider.
    let (feature, provider): (Option<&str>, &str) = match backend.as_str() {
        "postgres" | "postgresql" => (None, "postgresql"),
        "mysql" | "mariadb" => (Some("mysql"), "mysql"),
        "sqlite" => (Some("sqlite"), "sqlite"),
        other => {
            eprintln!("seiva: unknown --backend `{other}` (expected postgres, mysql or sqlite).");
            return ExitCode::FAILURE;
        }
    };
    let dir = dir.unwrap_or_else(|| DORM_PROJECT_DIR.to_owned());
    if Path::new(&dir).exists() {
        eprintln!("seiva: `{dir}` already exists — the DORM data layer looks set up already.");
        return ExitCode::FAILURE;
    }
    let dorm = dorm_bin();
    eprintln!("seiva: scaffolding the DORM data layer in `{dir}/` via `{dorm} init` ...");
    match std::process::Command::new(&dorm)
        .args(["init", &dir])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => return ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "seiva: could not run `{dorm}`: {error}\n\
                 seiva db needs the DORM CLI — install it (`cargo install dorm-cli`) \
                 or set SEIVA_DORM_BIN."
            );
            return ExitCode::FAILURE;
        }
    }
    // The DORM project is its own cargo workspace, not a member of the Seiva
    // project's workspace (it has its own deps and, interim, its own patch). An
    // empty `[workspace]` table makes it a standalone root so the parent workspace
    // ignores it; otherwise cargo refuses the nested package.
    let checkout = dorm_checkout();
    if checkout.is_none() {
        eprintln!(
            "seiva: note — no local DORM checkout found (set SEIVA_DORM_DIR); \
             `{dir}` will resolve DORM from crates.io."
        );
    }
    let manifest = Path::new(&dir).join("Cargo.toml");
    if let Err(error) = finalize_dorm_manifest(&manifest, checkout.as_deref()) {
        eprintln!("seiva: warning — could not finalize `{dir}/Cargo.toml`: {error}");
    }
    // Enable the chosen backend's DORM feature on the data layer's `dorm-core`.
    if let Some(feature) = feature {
        if let Err(error) = enable_dorm_feature(&manifest, feature) {
            eprintln!("seiva: warning — could not enable the `{feature}` feature: {error}");
        }
    }
    // Make the DORM data layer track THIS project's schema from the start: write
    // `data/dorm.schema` from `schema/<project>.schema` (so the module namespaces
    // drive it, not DORM's default template), retarget the provider, regenerate.
    if let Err(error) = sync_dorm_schema(&project, &dir, provider) {
        eprintln!("seiva: warning — could not sync the project schema into `{dir}`: {error}");
    } else if let Err(code) = run_dorm_in(&dir, &["generate".to_owned()]) {
        return code;
    }
    println!(
        "Set up the DORM data layer in `{dir}/` ({backend}) from schema/{project}.schema. \
         Add models with `seiva module new`, then `seiva db dev <name>` to \
         author+apply a migration (needs DELONIX_ORM_DSN for your {backend} database)."
    );
    ExitCode::SUCCESS
}

/// Enables a DORM feature (`mysql`/`sqlite`) on the data layer's `dorm-core`
/// dependency: rewrites `dorm-core = "x"` into `dorm-core = { version = "x",
/// features = ["<feature>"] }`. Idempotent-ish (no-op if a features list already
/// mentions it).
fn enable_dorm_feature(manifest: &Path, feature: &str) -> std::io::Result<()> {
    let cargo = std::fs::read_to_string(manifest)?;
    if cargo.contains(&format!("\"{feature}\"")) {
        return Ok(());
    }
    let re_target = "dorm-core = \"";
    let next = if let Some(start) = cargo.find(re_target) {
        let rest = &cargo[start + re_target.len()..];
        if let Some(end) = rest.find('"') {
            let version = &rest[..end];
            let replaced =
                format!("dorm-core = {{ version = \"{version}\", features = [\"{feature}\"] }}");
            cargo.replacen(&format!("dorm-core = \"{version}\""), &replaced, 1)
        } else {
            cargo
        }
    } else {
        cargo
    };
    std::fs::write(manifest, next)
}

/// Finalizes the generated DORM project's manifest so it builds nested inside a
/// Seiva project: appends an empty `[workspace]` table (making it a standalone
/// workspace root the parent ignores) and, when a local `checkout` is given, an
/// interim `[patch.crates-io]` redirecting every `dorm-* = "x"` dependency it
/// declares to `<checkout>/crates`.
fn finalize_dorm_manifest(manifest: &Path, checkout: Option<&str>) -> std::io::Result<()> {
    let mut out = std::fs::read_to_string(manifest)?;
    out.push_str(
        "\n# A standalone workspace root, so the parent Seiva workspace ignores this\n\
         # DORM data layer (it has its own deps and migrate binaries).\n\
         [workspace]\n",
    );
    if let Some(checkout) = checkout {
        out.push_str(
            "\n# INTERIM (seiva db init) — remove once DORM is published to crates.io.\n\
             [patch.crates-io]\n",
        );
        for line in out.clone().lines() {
            if let Some(name) = line.split('=').next().map(str::trim) {
                if name.starts_with("dorm-") && line.contains("= \"") {
                    out.push_str(&format!(
                        "{name} = {{ path = \"{checkout}/crates/{name}\" }}\n"
                    ));
                }
            }
        }
    }
    std::fs::write(manifest, out)
}

/// Writes the DORM data layer's `<dir>/dorm.schema` from the project's
/// `schema/<project>.schema`, adapted by [`scaffold::dorm_schema_from_project`]
/// (the module `namespace`s drive the data layer; the datasource URL, `@version`
/// and the `provider` are adjusted). Run before a migrate so new modules are
/// picked up.
fn sync_dorm_schema(project: &str, dir: &str, provider: &str) -> std::io::Result<()> {
    let src = std::fs::read_to_string(format!("schema/{project}.schema"))?;
    let dorm_schema = scaffold::dorm_schema_from_project(&src, provider);
    std::fs::write(Path::new(dir).join("dorm.schema"), dorm_schema)
}

/// The datasource `provider` recorded in an existing `<dir>/dorm.schema`, so a
/// re-sync (on `seiva db dev`) keeps the backend chosen at `seiva db init`.
/// Defaults to `postgresql` when absent.
fn dorm_provider_of(dir: &str) -> String {
    std::fs::read_to_string(Path::new(dir).join("dorm.schema"))
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("provider = ")
                    .map(|v| v.trim().trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "postgresql".to_owned())
}

/// Runs the DORM CLI with `args` inside the data-layer directory `dir`. Maps a
/// spawn failure or a non-zero exit to an `ExitCode` for the caller to return.
fn run_dorm_in(dir: &str, args: &[String]) -> Result<(), ExitCode> {
    let dorm = dorm_bin();
    match std::process::Command::new(&dorm)
        .args(args)
        .current_dir(dir)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(ExitCode::from(
            u8::try_from(status.code().unwrap_or(1)).unwrap_or(1),
        )),
        Err(error) => {
            eprintln!(
                "seiva: could not run `{dorm}` in `{dir}`: {error}\n\
                 seiva db needs the DORM CLI (set SEIVA_DORM_BIN or install dorm-cli)."
            );
            Err(ExitCode::FAILURE)
        }
    }
}

/// The DORM migrate arguments for a Seiva `db` action. Pure, so the mapping is
/// unit-tested: `dev` authors+applies the next migration (`migrate dev <name>`);
/// `deploy` applies the recorded chain (`migrate`, whose default is deploy).
fn dorm_migrate_args(action: &str, name: Option<&str>) -> Vec<String> {
    let mut args = vec!["migrate".to_owned()];
    if action == "dev" {
        args.push("dev".to_owned());
        args.push(name.unwrap_or("migration").to_owned());
    }
    args
}

/// Guards that the project's DORM data layer exists, returning the project name
/// and the data dir, or an error `ExitCode` (with a `seiva db init` hint).
fn require_data_layer() -> Result<(String, String), ExitCode> {
    let project = require_project().ok_or(ExitCode::FAILURE)?;
    let dir = DORM_PROJECT_DIR.to_owned();
    if !Path::new(&dir).join("dorm.schema").is_file() {
        eprintln!(
            "seiva: no DORM data layer here — run `seiva db init` first (scaffolds `{dir}/`)."
        );
        return Err(ExitCode::FAILURE);
    }
    Ok((project, dir))
}

/// `seiva db dev`: sync the project schema into the data layer, regenerate the
/// models, then author+apply the next migration — the dev loop. The DSN comes
/// from `DELONIX_ORM_DSN`.
fn db_dev(name: &str) -> ExitCode {
    let (project, dir) = match require_data_layer() {
        Ok(ok) => ok,
        Err(code) => return code,
    };
    // Preserve the backend chosen at `seiva db init` (its provider).
    let provider = dorm_provider_of(&dir);
    if let Err(error) = sync_dorm_schema(&project, &dir, &provider) {
        eprintln!("seiva: could not sync schema/{project}.schema into `{dir}`: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(code) = run_dorm_in(&dir, &["generate".to_owned()]) {
        return code;
    }
    match run_dorm_in(&dir, &dorm_migrate_args("dev", Some(name))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// `seiva db migrate`: apply the recorded migration chain (deploy). No schema
/// sync — it runs exactly the migrations already authored.
fn db_migrate() -> ExitCode {
    let (_project, dir) = match require_data_layer() {
        Ok(ok) => ok,
        Err(code) => return code,
    };
    match run_dorm_in(&dir, &dorm_migrate_args("deploy", None)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// `seiva proto`: prints the proto3 service document. Inside a project it
/// documents THAT project's facades by building and running the composition root
/// with `SEIVA_EMIT=proto` (from the modules linked into the binary); outside a
/// project it falls back to the built-in demonstration facade(s). Mirrors
/// [`openapi`].
fn proto() -> ExitCode {
    match std::fs::read_to_string("seiva.toml") {
        Ok(manifest) => match scaffold::project_name(&manifest) {
            Some(name) => emit_project_proto(&name),
            None => {
                eprintln!("seiva: seiva.toml has no `[project] name`.");
                ExitCode::FAILURE
            }
        },
        Err(_) => {
            eprintln!(
                "seiva: no project here — documenting the built-in demonstration facade(s). \
                 Run inside a project (see `seiva project new`) to document your own."
            );
            println!("{}", builtin_proto());
            ExitCode::SUCCESS
        }
    }
}

/// Builds and runs `apps/<name>-api` with `SEIVA_EMIT=proto` to print the
/// project's own proto3 document (from its linked modules). Build output goes to
/// stderr (inherited); only the document reaches stdout.
fn emit_project_proto(name: &str) -> ExitCode {
    let app = format!("{name}-api");
    eprintln!("seiva: building `{app}` to document this project's facades ...");
    let output = match std::process::Command::new("cargo")
        .args(["run", "--quiet", "--bin", &app])
        .env("SEIVA_EMIT", "proto")
        .stderr(std::process::Stdio::inherit())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            eprintln!("seiva: failed to run `{app}`: {error}");
            return ExitCode::FAILURE;
        }
    };
    if !output.status.success() {
        eprintln!("seiva: `{app}` exited with {}", output.status);
        return ExitCode::from(1);
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    ExitCode::SUCCESS
}

/// `seiva user create`: reads a stdin password and creates the user through the
/// `accounts` module's use case, printing the created user as JSON.
fn user_create(username: &str) -> ExitCode {
    let password = match read_password_line() {
        Ok(password) => password,
        Err(err) => {
            eprintln!("seiva: failed to read password from stdin: {err}");
            return ExitCode::FAILURE;
        }
    };
    match create_user_via_module(username, &password) {
        Ok(json) => {
            eprintln!(
                "seiva: note — created through the accounts module (Argon2id + domain rules); \
                 persistence is in-memory until the `dorm` feature wires a real repository."
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&json).expect("the user JSON always serialises")
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("seiva: {err}");
            ExitCode::FAILURE
        }
    }
}

fn doctor() -> ExitCode {
    println!("Delonix Seiva");
    println!("  framework version : {}", delonix_seiva::VERSION);
    println!(
        "  data layer        : {}",
        if delonix_seiva::DORM_ENABLED {
            "DORM (feature `dorm` on)"
        } else {
            "external (feature `dorm` off)"
        }
    );
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor => doctor(),
        Command::Project { action } => match action {
            ProjectAction::New { name } => project_new(&name),
        },
        Command::Module { action } => match action {
            ModuleAction::New { name, data, admin } => {
                module_new(&name, data.as_deref() == Some("external"), admin)
            }
            ModuleAction::Split { .. } => not_yet("module split", "the module-mode phase"),
            ModuleAction::Merge { .. } => not_yet("module merge", "the module-mode phase"),
        },
        Command::Ext { action } => match action {
            ExtAction::Add { name } => ext_add(&name),
        },
        Command::Db { action } => match action {
            DbAction::Init { dir, backend } => db_init(dir, backend),
            DbAction::Dev { name } => db_dev(&name),
            DbAction::Migrate => db_migrate(),
            DbAction::Pull { out, dsn } => db_pull(out, dsn),
            DbAction::Studio { port, write, dsn } => db_studio(port, write, dsn),
        },
        Command::Lint => lint(),
        Command::Expand => not_yet("expand", "the macros phase"),
        Command::Start { watch, no_watch } => start(watch, no_watch),
        Command::Openapi { compact, .. } => openapi(compact),
        Command::Proto => proto(),
        Command::User { action } => match action {
            UserAction::Create { username } => user_create(&username),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_json_is_valid_and_documents_the_notes_facade() {
        let json = builtin_openapi_json(false);
        let doc: serde_json::Value =
            serde_json::from_str(&json).expect("openapi must be valid JSON");
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["version"], delonix_seiva::VERSION);
        // The built-in Notes facade's `create` operation is present.
        assert!(
            doc["paths"]["/v1/notes"]["post"].is_object(),
            "notes create operation must be documented"
        );
        assert_eq!(
            doc["paths"]["/v1/notes"]["post"]["operationId"],
            "notes_create"
        );
    }

    #[test]
    fn openapi_compact_and_pretty_carry_the_same_document() {
        let compact = builtin_openapi_json(true);
        let pretty = builtin_openapi_json(false);
        assert!(!compact.contains('\n'), "compact JSON must be single-line");
        assert!(pretty.contains('\n'), "pretty JSON must be multi-line");
        let a: serde_json::Value = serde_json::from_str(&compact).unwrap();
        let b: serde_json::Value = serde_json::from_str(&pretty).unwrap();
        assert_eq!(a, b, "the two styles must encode the same document");
    }

    #[test]
    fn proto_document_renders_the_notes_service() {
        let proto = builtin_proto();
        assert!(
            proto.contains("syntax = \"proto3\";"),
            "proto was:\n{proto}"
        );
        assert!(proto.contains("service Notes {"), "proto was:\n{proto}");
        assert!(
            proto.contains("rpc Create(CreateRequest) returns (CreateResponse);"),
            "proto was:\n{proto}"
        );
    }

    #[test]
    fn start_plan_uses_cargo_watch_when_available_else_plain_run() {
        // Hot-reload: `cargo watch -x "run --bin shop-api"`.
        let (program, args) = start_plan("shop-api", true);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["watch", "-x", "run --bin shop-api"]);

        // No cargo-watch: a plain `cargo run --bin shop-api`.
        let (program, args) = start_plan("shop-api", false);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["run", "--bin", "shop-api"]);
    }

    #[test]
    fn db_pull_plan_delegates_to_the_dorm_cli() {
        // No DSN: just `db pull --out <path>`.
        let (program, args) = db_pull_plan("dorm", "schema/shop.pulled.schema", None);
        assert_eq!(program, "dorm");
        assert_eq!(
            args,
            vec!["db", "pull", "--out", "schema/shop.pulled.schema"]
        );
        // With a DSN: the global `--dsn` precedes the subcommand.
        let (_p, args) = db_pull_plan("dorm", "out.schema", Some("postgres://x/y"));
        assert_eq!(
            args,
            vec![
                "--dsn",
                "postgres://x/y",
                "db",
                "pull",
                "--out",
                "out.schema"
            ]
        );
    }

    #[test]
    fn dorm_studio_args_build_the_delegation() {
        // Defaults: just `studio` (read-only, DORM's default port).
        assert_eq!(dorm_studio_args(None, false, None), vec!["studio"]);
        // With a DSN, port and writes: `--dsn X studio --port N --write`.
        assert_eq!(
            dorm_studio_args(Some(5599), true, Some("postgres://x/y")),
            vec![
                "--dsn",
                "postgres://x/y",
                "studio",
                "--port",
                "5599",
                "--write"
            ]
        );
    }

    #[test]
    fn dorm_migrate_args_map_dev_and_deploy() {
        // dev → author + apply the next migration by name.
        assert_eq!(
            dorm_migrate_args("dev", Some("add_price")),
            vec!["migrate", "dev", "add_price"]
        );
        // deploy → apply the chain (DORM's default `migrate` subcommand).
        assert_eq!(dorm_migrate_args("deploy", None), vec!["migrate"]);
    }

    #[test]
    fn resolve_reload_honours_the_flags_and_availability() {
        // Auto-detect with neither flag: follows availability.
        assert_eq!(resolve_reload(false, false, true), Ok(true));
        assert_eq!(resolve_reload(false, false, false), Ok(false));
        // `--no-watch` always disables, even when cargo-watch is present.
        assert_eq!(resolve_reload(false, true, true), Ok(false));
        // `--watch` forces reload when available.
        assert_eq!(resolve_reload(true, false, true), Ok(true));
        // `--watch` without cargo-watch is an actionable error.
        assert!(resolve_reload(true, false, false).is_err());
    }

    #[test]
    fn user_create_goes_through_the_accounts_module_and_hides_the_secret() {
        let user = create_user_via_module("alice", "hunter2").expect("create must succeed");
        assert_eq!(user["username"], "alice");
        assert!(user["id"].is_string(), "the created user carries an id");
        // The facade's DTO carries no secret: neither the password nor its hash
        // ever reaches the CLI output.
        assert!(user.get("password").is_none());
        assert!(user.get("password_hash").is_none());
        let rendered = user.to_string();
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("$argon2id$"),
            "no secret must appear in the output: {rendered}"
        );
    }

    #[test]
    fn user_create_rejects_a_blank_username() {
        // The domain rule runs inside the module's use case, not the CLI.
        let err = create_user_via_module("   ", "hunter2").expect_err("blank username is rejected");
        assert!(!err.is_empty(), "the module returns an error message");
    }
}
