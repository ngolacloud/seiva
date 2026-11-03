//! The `seiva` binary: reads arguments and environment, then calls the library.
//!
//! The command tree below is the shape fixed by the brief. Only `doctor` and
//! `--version` do real work in this founding build; every other command reports
//! the phase that will implement it rather than faking success. As each command
//! lands, its body becomes a thin call into a public `delonix-seiva` function
//! with a typed result (the "CLI as a library" rule, mirrored from DORM).

use clap::{Parser, Subcommand};
use delonix_seiva::scaffold::{self, GeneratedFile};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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
        Command::Start => not_yet("start", "the runtime phase"),
    }
}
