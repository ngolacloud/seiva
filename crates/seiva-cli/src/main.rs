//! The `seiva` binary: reads arguments and environment, then calls the library.
//!
//! The command tree below is the shape fixed by the brief. Only `doctor` and
//! `--version` do real work in this founding build; every other command reports
//! the phase that will implement it rather than faking success. As each command
//! lands, its body becomes a thin call into a public `delonix-seiva` function
//! with a typed result (the "CLI as a library" rule, mirrored from DORM).

use clap::{Parser, Subcommand};
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
            ProjectAction::New { .. } => not_yet("project new", "the scaffolding phase"),
        },
        Command::Module { action } => match action {
            ModuleAction::New { .. } => not_yet("module new", "the scaffolding phase"),
            ModuleAction::Split { .. } => not_yet("module split", "the module-mode phase"),
            ModuleAction::Merge { .. } => not_yet("module merge", "the module-mode phase"),
        },
        Command::Db { action } => match action {
            DbAction::Dev { .. } => not_yet("db dev", "the data-layer phase"),
            DbAction::Migrate => not_yet("db migrate", "the data-layer phase"),
        },
        Command::Lint => not_yet("lint", "the fitness-functions phase"),
        Command::Expand => not_yet("expand", "the macros phase"),
        Command::Start => not_yet("start", "the runtime phase"),
    }
}
