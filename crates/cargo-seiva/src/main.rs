//! `cargo-seiva` — the architecture fitness-function lint as a cargo subcommand.
//!
//! Invoked as `cargo seiva lint` (cargo injects the bare subcommand name
//! `seiva` as the first argument) or directly as `cargo-seiva lint`. It scans
//! `modules/<name>/module.toml` under the given project root (default `.`) and
//! enforces the Clean-Architecture layer rule, the zero-trust contract check,
//! data-layer neutrality and the tenant-scope policy — the same library the
//! `seiva lint` CLI command calls.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    // When run as `cargo seiva …`, cargo passes the subcommand name `seiva` as
    // the first argument — drop exactly one leading occurrence.
    if args.peek().map(String::as_str) == Some("seiva") {
        args.next();
    }
    // An optional `lint` verb (the only command today) and an optional root.
    if args.peek().map(String::as_str) == Some("lint") {
        args.next();
    }
    let root: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    lint(&root)
}

fn lint(root: &Path) -> ExitCode {
    match cargo_seiva::lint_project(root) {
        Ok(report) if report.modules_checked == 0 && report.is_clean() => {
            println!("cargo seiva: no modules found under ./modules (nothing to check).");
            ExitCode::SUCCESS
        }
        Ok(report) if report.is_clean() => {
            println!(
                "cargo seiva: contract check passed for {} module(s).",
                report.modules_checked
            );
            ExitCode::SUCCESS
        }
        Ok(report) => {
            eprintln!(
                "cargo seiva: {} contract violation(s) across {} module(s):",
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
            eprintln!("cargo seiva: {err}");
            ExitCode::FAILURE
        }
    }
}
