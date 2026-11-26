//! TAB completion candidates for argument VALUES.
//!
//! `clap_complete`'s dynamic engine already completes commands, subcommands and
//! flags from the CLI definition; these functions add the values: the project
//! kinds, the data layers and backends, the known extensions, and the modules of
//! the project in the current directory (read from its `seiva.toml`, the same
//! file `seiva module …` edits).
//!
//! Rule: a completer FAILS SILENTLY. It runs in the middle of the user's command
//! line on every TAB, so it never prints and never panics — a missing or
//! unreadable `seiva.toml` simply means "no suggestions".

use clap_complete::engine::CompletionCandidate;
use std::path::Path;

fn cand(value: &str, help: &str) -> CompletionCandidate {
    CompletionCandidate::new(value).help(Some(help.to_string().into()))
}

/// `--kind` of `seiva project new` (ADR 0024).
pub fn kinds() -> Vec<CompletionCandidate> {
    vec![
        cand("api", "HTTP+gRPC server that hosts modules (default)"),
        cand("worker", "background jobs over the worker host"),
        cand("gateway", "HTTP edge/BFF that aggregates upstreams"),
        cand("event", "event processor that drains the event bus"),
    ]
}

/// `--data` of `seiva module new` (ADR 0030).
pub fn data_layers() -> Vec<CompletionCandidate> {
    vec![
        cand("dorm", "the Delonix ORM, batteries included (default)"),
        cand("sqlx", "the sqlx reference adapter"),
        cand("seaorm", "the SeaORM reference adapter"),
        cand("external", "you own persistence"),
    ]
}

/// `--backend` of `seiva db init`.
pub fn backends() -> Vec<CompletionCandidate> {
    vec![
        cand("postgres", "PostgreSQL (default)"),
        cand("mysql", "MySQL"),
        cand("sqlite", "SQLite, no server needed"),
    ]
}

/// The extensions `seiva ext add` knows, from the scaffold's own list.
pub fn extensions() -> Vec<CompletionCandidate> {
    seiva::scaffold::EXTENSIONS
        .iter()
        .map(|e| cand(e.crate_name, e.summary))
        .collect()
}

/// Module names registered in a `seiva.toml` text (`[modules.<name>]`).
pub fn modules_in(seiva_toml: &str) -> Vec<String> {
    seiva_toml
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("[modules.")
                .and_then(|r| r.strip_suffix(']'))
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn project_modules(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("seiva.toml"))
        .map(|s| modules_in(&s))
        .unwrap_or_default()
}

fn is_split(root: &Path, module: &str) -> bool {
    root.join(format!("apps/{module}-service")).is_dir()
}

/// Modules of this project that are not yet a microservice — `module split`.
pub fn splittable_modules() -> Vec<CompletionCandidate> {
    let root = Path::new(".");
    project_modules(root)
        .into_iter()
        .filter(|m| !is_split(root, m))
        .map(CompletionCandidate::new)
        .collect()
}

/// Modules of this project that have a service to fold back — `module merge`.
pub fn split_modules() -> Vec<CompletionCandidate> {
    let root = Path::new(".");
    project_modules(root)
        .into_iter()
        .filter(|m| is_split(root, m))
        .map(CompletionCandidate::new)
        .collect()
}

/// Shells `seiva completion` can register for.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

/// Print the script that registers `seiva`'s dynamic completion with `shell`.
/// The script calls back into `seiva` (with `COMPLETE=<shell>` set) on every
/// TAB, so the candidates always match the installed binary.
pub fn print_registration(shell: Shell) -> std::io::Result<()> {
    use clap_complete::env::{Bash, Elvish, EnvCompleter, Fish, Powershell, Zsh};
    use std::io::Write;
    let completer: &dyn EnvCompleter = match shell {
        Shell::Bash => &Bash,
        Shell::Zsh => &Zsh,
        Shell::Fish => &Fish,
        Shell::Elvish => &Elvish,
        Shell::Powershell => &Powershell,
    };
    let mut buf = Vec::new();
    completer.write_registration("COMPLETE", "seiva", "seiva", "seiva", &mut buf)?;
    std::io::stdout().write_all(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// Complete `line` with the cursor at its last word, through the same engine
    /// entry point `CompleteEnv` uses.
    fn complete(line: &str) -> Vec<String> {
        let mut args: Vec<OsString> = line.split(' ').map(OsString::from).collect();
        if line.ends_with(' ') {
            args.pop();
            args.push(OsString::new());
        }
        let index = args.len() - 1;
        clap_complete::engine::complete(&mut crate::cli_command(), args, index, None)
            .unwrap()
            .into_iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            // A positional also offers `--help`; these tests are about values.
            .filter(|v| !v.starts_with('-'))
            .collect()
    }

    #[test]
    fn subcommands_complete() {
        let got = complete("seiva mo");
        assert_eq!(got, vec!["module"]);
        let got = complete("seiva db ");
        for want in ["init", "dev", "migrate", "pull", "studio", "seed"] {
            assert!(
                got.iter().any(|g| g == want),
                "`{want}` missing from {got:?}"
            );
        }
    }

    #[test]
    fn flag_values_complete() {
        assert_eq!(
            complete("seiva project new shop --kind "),
            ["api", "worker", "gateway", "event"]
        );
        assert_eq!(
            complete("seiva module new orders --data s"),
            ["sqlx", "seaorm"]
        );
        assert_eq!(
            complete("seiva db init --backend "),
            ["postgres", "mysql", "sqlite"]
        );
        assert_eq!(complete("seiva ext add "), ["seiva-console-admin"]);
        assert_eq!(complete("seiva completion f"), ["fish"]);
    }

    #[test]
    fn module_names_come_from_seiva_toml() {
        let toml = "[project]\nname = \"shop\"\n\n[modules.orders]\ndata = \"dorm\"\n\n[modules.inventory]\ndata = \"sqlx\"\n";
        assert_eq!(modules_in(toml), ["orders", "inventory"]);
        assert!(modules_in("[project]\nname = \"x\"\n").is_empty());
    }

    #[test]
    fn registration_script_calls_back_into_seiva() {
        use clap_complete::env::{Bash, EnvCompleter};
        let mut buf = Vec::new();
        Bash.write_registration("COMPLETE", "seiva", "seiva", "seiva", &mut buf)
            .unwrap();
        let script = String::from_utf8(buf).unwrap();
        assert!(script.contains("COMPLETE"), "{script}");
        assert!(script.contains("seiva"), "{script}");
    }
}
