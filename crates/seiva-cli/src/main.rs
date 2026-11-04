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
    Start,
    /// Report the environment: framework version and which data layer is linked.
    Doctor,
    /// Print the OpenAPI 3.1 document for the built-in demonstration facade(s).
    ///
    /// LIMITATION: this first cut cannot introspect a user's own crate at
    /// runtime, so it documents only the framework's built-in demonstration
    /// facade (`NotesFacade`). A later phase will discover a project's own
    /// facades. A short note to this effect is also printed to stderr.
    Openapi {
        /// Pretty-print the JSON (the default when no style flag is given).
        #[arg(long, conflicts_with = "compact")]
        pretty: bool,
        /// Emit compact, single-line JSON instead of the pretty default.
        #[arg(long)]
        compact: bool,
    },
    /// Print the proto3 service document for the built-in demonstration facade(s).
    ///
    /// LIMITATION: like `openapi`, this first cut documents only the framework's
    /// built-in demonstration facade (`NotesFacade`); a later phase will
    /// discover a project's own facades. A short note is also printed to stderr.
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
    /// Scaffold a new module: `seiva module new <name> [--data external]`.
    New {
        name: String,
        /// Use an external data layer instead of DORM for this module.
        #[arg(long)]
        data: Option<String>,
    },
    /// Move a module to `mode = "service"` with its own schema and history.
    Split { name: String },
    /// Fold a service module back into the project schema.
    Merge { name: String },
}

#[derive(Subcommand)]
enum DbAction {
    /// Generate models, create and apply a migration locally for one module.
    Dev { module: String },
    /// Apply every DORM module's migrations in dependency order.
    Migrate,
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
    println!("Created project `{name}`. Next: cd {name} && seiva module new <name>");
    ExitCode::SUCCESS
}

fn module_new(name: &str, external: bool) -> ExitCode {
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

    let files = match scaffold::module_files(name) {
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
    println!("Created module `{name}` (facade v1, 5 layers). Registered in the workspace, seiva.toml and schema.");
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
    Ok(())
}

/// Extracts the `[project] name = "..."` value from seiva.toml text.
fn project_name(seiva: &str) -> Option<String> {
    seiva
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .map(|v| v.trim().trim_matches('"').to_owned())
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

/// `seiva openapi`: prints the OpenAPI 3.1 document for the built-in facades.
fn openapi(compact: bool) -> ExitCode {
    eprintln!(
        "seiva: note — this documents the built-in demonstration facade(s) only; \
         runtime introspection of a project's own facades is a later phase."
    );
    println!("{}", builtin_openapi_json(compact));
    ExitCode::SUCCESS
}

/// `seiva start`: runs the project's `apps/<name>-api` server in development,
/// with hot-reload when `cargo watch` is available.
///
/// Resolves the project name from `seiva.toml` (so the app crate is
/// `<name>-api`), sets `APP_ENV=development`, and execs cargo. With `cargo watch`
/// installed it rebuilds-and-restarts on change (the FastAPI `--reload`
/// equivalent); without it, a plain `cargo run` plus a hint to install it.
fn start() -> ExitCode {
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

    let hot_reload = has_cargo_watch();
    if !hot_reload {
        eprintln!(
            "seiva: `cargo watch` not found — running without hot-reload \
             (install it with `cargo install cargo-watch`)."
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

/// `seiva proto`: prints the proto3 service document for the built-in facades.
fn proto() -> ExitCode {
    eprintln!(
        "seiva: note — this documents the built-in demonstration facade(s) only; \
         runtime introspection of a project's own facades is a later phase."
    );
    println!("{}", builtin_proto());
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
            ModuleAction::New { name, data } => {
                module_new(&name, data.as_deref() == Some("external"))
            }
            ModuleAction::Split { .. } => not_yet("module split", "the module-mode phase"),
            ModuleAction::Merge { .. } => not_yet("module merge", "the module-mode phase"),
        },
        Command::Db { action } => match action {
            DbAction::Dev { .. } => not_yet("db dev", "the data-layer phase"),
            DbAction::Migrate => not_yet("db migrate", "the data-layer phase"),
        },
        Command::Lint => lint(),
        Command::Expand => not_yet("expand", "the macros phase"),
        Command::Start => start(),
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
