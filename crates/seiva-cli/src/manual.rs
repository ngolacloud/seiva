//! The `seiva` manual: one entry per command — its semantic group, worked
//! examples and cross-references — applied to the clap tree as the long-help
//! tail (`EXAMPLES:` / `SEE ALSO:`), plus the root's `WORKFLOW:` tail.
//!
//! One central table instead of an `after_long_help` on every derive, for the
//! same reasons as the `delonix` binary's manual: the content is reviewable in
//! one place, and the tests below prove that every command has an entry, that
//! every example invokes the command it documents, and that every cross-
//! reference resolves. The group taxonomy here is also what
//! [`help`](crate::help) sections the command listings by — a single source of
//! truth for "where does this command belong".
//!
//! `-h` stays short (clap's `about`); `--help` carries the manual.

/// One command's manual entry.
pub struct Entry {
    /// The command path below `seiva`, space-separated (`"db dev"`).
    pub path: &'static str,
    /// The section the command is listed under in its parent's help.
    pub group: &'static str,
    /// `(what it shows, the command line)` — each line MUST invoke `seiva <path>`.
    pub examples: &'static [(&'static str, &'static str)],
    /// Related command paths (each must exist).
    pub see_also: &'static [&'static str],
}

/// Section order in a listing. Root sections first (the developer's workflow:
/// shape the project, develop it, publish its contract, check it, tool up), then
/// the sections the sub-groups use.
pub const GROUP_ORDER: &[&str] = &[
    // root
    "Project",
    "Development",
    "Contract",
    "Quality",
    "Tooling",
    // sub-groups
    "Scaffold",
    "Topology",
    "Setup",
    "Migration",
    "Data",
    "Integration",
    "Security",
];

pub static ENTRIES: &[Entry] = &[
    // --- Project -------------------------------------------------------------
    Entry {
        path: "project",
        group: "Project",
        examples: &[
            (
                "an HTTP+gRPC API project that hosts modules (the default kind)",
                "seiva project new shop",
            ),
            (
                "a background-jobs project over the worker host",
                "seiva project new jobs --kind worker",
            ),
        ],
        see_also: &["module new", "start", "doctor"],
    },
    Entry {
        path: "project new",
        group: "Scaffold",
        examples: &[
            (
                "an HTTP+gRPC API project that hosts modules (the default kind)",
                "seiva project new shop",
            ),
            (
                "a background-jobs project over the worker host",
                "seiva project new jobs --kind worker",
            ),
            (
                "an HTTP edge/BFF that aggregates upstreams and hosts no modules",
                "seiva project new edge --kind gateway",
            ),
            (
                "an event processor that drains the event bus and reacts",
                "seiva project new reactions --kind event",
            ),
        ],
        see_also: &["module new", "start", "ext add"],
    },
    Entry {
        path: "module",
        group: "Project",
        examples: &[
            (
                "a five-layer module that owns its data (DORM by default)",
                "seiva module new orders",
            ),
            (
                "the same module, also deployed as its own microservice",
                "seiva module split orders",
            ),
        ],
        see_also: &["project new", "db init", "lint"],
    },
    Entry {
        path: "module new",
        group: "Scaffold",
        examples: &[
            (
                "a five-layer module on DORM, the batteries-included default",
                "seiva module new orders",
            ),
            (
                "on the sqlx reference adapter (migrate with sqlx-cli)",
                "seiva module new catalog --data sqlx",
            ),
            (
                "listed in the admin console, gated by the module's permissions",
                "seiva module new products --admin",
            ),
            (
                "you own persistence; the framework ships no migration tooling for it",
                "seiva module new billing --data external",
            ),
        ],
        see_also: &["module split", "db init", "lint", "ext add"],
    },
    Entry {
        path: "module split",
        group: "Topology",
        examples: &[(
            "add an apps/orders-service binary that mounts only `orders`",
            "seiva module split orders",
        )],
        see_also: &["module merge", "module new", "start"],
    },
    Entry {
        path: "module merge",
        group: "Topology",
        examples: &[(
            "fold apps/orders-service back into the monolith (the module is untouched)",
            "seiva module merge orders",
        )],
        see_also: &["module split"],
    },
    Entry {
        path: "ext",
        group: "Project",
        examples: &[(
            "mount the admin console at /admin, behind the framework's auth layers",
            "seiva ext add seiva-console-admin",
        )],
        see_also: &["module new", "start"],
    },
    Entry {
        path: "ext add",
        group: "Integration",
        examples: &[(
            "mount the admin console at /admin, behind the framework's auth layers",
            "seiva ext add seiva-console-admin",
        )],
        see_also: &["module new", "start"],
    },
    // --- Development ---------------------------------------------------------
    Entry {
        path: "start",
        group: "Development",
        examples: &[
            (
                "run the project, hot-reloading when `cargo watch` is installed",
                "seiva start",
            ),
            (
                "require hot-reload (errors if `cargo watch` is missing)",
                "seiva start --watch",
            ),
            ("a plain `cargo run`, no reload", "seiva start --no-watch"),
        ],
        see_also: &["db seed", "openapi", "doctor"],
    },
    Entry {
        path: "db",
        group: "Development",
        examples: &[
            ("set up the data layer once per project", "seiva db init"),
            (
                "turn the schema into the next migration and apply it",
                "seiva db dev add_price",
            ),
            (
                "browse the data in the browser (read-only)",
                "seiva db studio",
            ),
        ],
        see_also: &["module new", "start"],
    },
    Entry {
        path: "db init",
        group: "Setup",
        examples: &[
            (
                "a DORM project under data/ on PostgreSQL (the default backend)",
                "seiva db init",
            ),
            (
                "on SQLite — no server needed for local work",
                "seiva db init --backend sqlite",
            ),
            ("in another directory", "seiva db init --dir database"),
        ],
        see_also: &["db dev", "db migrate", "module new"],
    },
    Entry {
        path: "db dev",
        group: "Migration",
        examples: &[
            (
                "author the next migration from the schema and apply it",
                "seiva db dev",
            ),
            (
                "the same, with a name for the migration",
                "seiva db dev add_price",
            ),
        ],
        see_also: &["db migrate", "db init", "db studio"],
    },
    Entry {
        path: "db migrate",
        group: "Migration",
        examples: &[(
            "apply the recorded migration chain (deploy time)",
            "seiva db migrate",
        )],
        see_also: &["db dev", "db seed"],
    },
    Entry {
        path: "db pull",
        group: "Migration",
        examples: &[
            (
                "introspect an existing database into schema/<project>.pulled.schema",
                "seiva db pull --dsn postgres://app@localhost/legacy",
            ),
            (
                "choose where the pulled schema goes",
                "seiva db pull --out schema/legacy.schema",
            ),
        ],
        see_also: &["db dev", "db init"],
    },
    Entry {
        path: "db seed",
        group: "Data",
        examples: &[
            (
                "load each module's reference data (seed/ CSV, idempotent upsert)",
                "seiva db seed",
            ),
            (
                "also load the demo/ data, as a dev boot would",
                "seiva db seed --demo",
            ),
        ],
        see_also: &["db migrate", "start"],
    },
    Entry {
        path: "db studio",
        group: "Data",
        examples: &[
            (
                "browse the database on 127.0.0.1:5555, read-only",
                "seiva db studio",
            ),
            (
                "on another port, allowing edits",
                "seiva db studio --port 5600 --write",
            ),
        ],
        see_also: &["db dev", "db seed"],
    },
    Entry {
        path: "user",
        group: "Development",
        examples: &[(
            "a credential record with an Argon2id hash (password read from STDIN)",
            "printf 'secret\\n' | seiva user create --username alice",
        )],
        see_also: &["doctor"],
    },
    Entry {
        path: "user create",
        group: "Security",
        examples: &[(
            "a credential record with an Argon2id hash (password read from STDIN)",
            "printf 'secret\\n' | seiva user create --username alice",
        )],
        see_also: &["doctor"],
    },
    // --- Contract ------------------------------------------------------------
    Entry {
        path: "openapi",
        group: "Contract",
        examples: &[
            (
                "this project's OpenAPI 3.1 document, from the facades linked into it",
                "seiva openapi > openapi.json",
            ),
            ("single-line JSON, for piping", "seiva openapi --compact"),
        ],
        see_also: &["proto", "start"],
    },
    Entry {
        path: "proto",
        group: "Contract",
        examples: &[(
            "the proto3 service document, from the same facade contract",
            "seiva proto > api.proto",
        )],
        see_also: &["openapi"],
    },
    Entry {
        path: "expand",
        group: "Contract",
        examples: &[(
            "what the Seiva macros expand to (planned — reports its phase today)",
            "seiva expand",
        )],
        see_also: &["lint", "openapi"],
    },
    // --- Quality -------------------------------------------------------------
    Entry {
        path: "lint",
        group: "Quality",
        examples: &[
            (
                "check the five-layer dependency rule and the other fitness functions",
                "seiva lint",
            ),
            (
                "the same check as a cargo subcommand (the `cargo-seiva` binary)",
                "cargo seiva lint",
            ),
        ],
        see_also: &["module new", "doctor"],
    },
    Entry {
        path: "doctor",
        group: "Quality",
        examples: &[(
            "the framework version and which data layer this binary links",
            "seiva doctor",
        )],
        see_also: &["version", "completion"],
    },
    // --- Tooling -------------------------------------------------------------
    Entry {
        path: "completion",
        group: "Tooling",
        examples: &[
            (
                "bash: enable TAB completion for the current shell",
                "source <(seiva completion bash)",
            ),
            (
                "bash: enable it for every new shell",
                "echo 'source <(seiva completion bash)' >> ~/.bashrc",
            ),
            (
                "zsh: add to ~/.zshrc (after compinit)",
                "source <(seiva completion zsh)",
            ),
            (
                "fish",
                "seiva completion fish > ~/.config/fish/completions/seiva.fish",
            ),
        ],
        see_also: &["version", "doctor"],
    },
    Entry {
        path: "version",
        group: "Tooling",
        examples: &[
            (
                "the build card: commit, build, features, and how to start (= `seiva --version`)",
                "seiva version",
            ),
            (
                "just the version number, for scripts (= `seiva -V`)",
                "seiva version --short",
            ),
        ],
        see_also: &["doctor", "completion"],
    },
];

/// The root's `WORKFLOW:` tail — the order a new project is usually built in.
const WORKFLOW: &[(&str, &str)] = &[
    ("seiva project new <name>", "scaffold the workspace"),
    (
        "seiva module new <name>",
        "a module that owns its data, in five layers",
    ),
    (
        "seiva db init && seiva db dev",
        "the data layer and its first migration",
    ),
    ("seiva start", "run it locally, with hot-reload"),
    ("seiva lint", "prove the architecture rules still hold"),
    (
        "seiva module split <name>",
        "the same module, also as a microservice",
    ),
];

/// The entry for `path`, if any.
pub fn entry(path: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.path == path)
}

/// The group a command path is listed under; `""` when it has no entry (it
/// then lands in a trailing catch-all `Commands:` section instead of vanishing).
pub fn group_of(path: &str) -> &'static str {
    entry(path).map_or("", |e| e.group)
}

/// Display rank of a group; unknown groups sort last.
pub fn group_rank(group: &str) -> usize {
    GROUP_ORDER
        .iter()
        .position(|g| *g == group)
        .unwrap_or(GROUP_ORDER.len())
}

fn tail(entry: &Entry) -> String {
    let mut out = String::from("EXAMPLES:");
    for (what, line) in entry.examples {
        out.push_str(&format!("\n  # {what}\n  {line}\n"));
    }
    if !entry.see_also.is_empty() {
        let refs: Vec<String> = entry
            .see_also
            .iter()
            .map(|p| format!("seiva {p}"))
            .collect();
        out.push_str(&format!("\nSEE ALSO:\n  {}", refs.join(" · ")));
    }
    out
}

fn root_tail() -> String {
    let width = WORKFLOW.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    let mut out = String::from("WORKFLOW:");
    for (i, (cmd, what)) in WORKFLOW.iter().enumerate() {
        out.push_str(&format!("\n  {}. {cmd:<width$}   {what}", i + 1));
    }
    out.push_str(
        "\n\n`seiva <command> --help` shows examples for each command. \
         TAB completion: `seiva completion --help`.\n\
         Docs: https://ngolacloud.github.io/seiva/",
    );
    out
}

/// Attach the manual to the whole tree: the root gets the workflow, every
/// command with an entry gets its examples and cross-references.
pub fn apply(cmd: clap::Command) -> clap::Command {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    let mut cmd = cmd.after_long_help(root_tail());
    for name in names {
        cmd = cmd.mut_subcommand(&name, |s| apply_at(s, name.clone()));
    }
    cmd
}

fn apply_at(mut cmd: clap::Command, path: String) -> clap::Command {
    if let Some(e) = entry(&path) {
        cmd = cmd.after_long_help(tail(e));
    }
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in names {
        let child = format!("{path} {name}");
        cmd = cmd.mut_subcommand(&name, |s| apply_at(s, child));
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command path in the live CLI tree (clap's generated `help`
    /// subcommands excluded).
    fn cli_paths() -> Vec<String> {
        fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
            for s in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
                let path = if prefix.is_empty() {
                    s.get_name().to_string()
                } else {
                    format!("{prefix} {}", s.get_name())
                };
                out.push(path.clone());
                walk(s, &path, out);
            }
        }
        let mut out = Vec::new();
        walk(&crate::cli_command(), "", &mut out);
        out
    }

    #[test]
    fn every_command_has_an_entry_with_examples() {
        for path in cli_paths() {
            let e = entry(&path).unwrap_or_else(|| panic!("`seiva {path}` has no manual entry"));
            assert!(!e.examples.is_empty(), "`seiva {path}` has no examples");
            assert!(
                GROUP_ORDER.contains(&e.group),
                "`seiva {path}` is in unknown group `{}`",
                e.group
            );
        }
    }

    #[test]
    fn every_entry_documents_a_real_command() {
        let paths = cli_paths();
        for e in ENTRIES {
            assert!(
                paths.iter().any(|p| p == e.path),
                "entry `{}` is not a command",
                e.path
            );
        }
    }

    #[test]
    fn every_example_invokes_the_command_it_documents() {
        for e in ENTRIES {
            for (_, line) in e.examples {
                // `seiva lint` is also `cargo seiva lint` — both contain the path.
                assert!(
                    line.contains(&format!("seiva {}", e.path)),
                    "example `{line}` does not invoke `seiva {}`",
                    e.path
                );
            }
        }
    }

    #[test]
    fn every_see_also_resolves() {
        let paths = cli_paths();
        for e in ENTRIES {
            for r in e.see_also {
                assert!(
                    paths.iter().any(|p| p == r),
                    "`{}` refers to unknown `{r}`",
                    e.path
                );
                assert_ne!(*r, e.path, "`{}` refers to itself", e.path);
            }
        }
    }

    #[test]
    fn root_commands_use_root_groups_only() {
        // The root listing is sectioned by the first five groups; a root command
        // filed under a sub-group label would open a stray section at the top.
        let root_groups = &GROUP_ORDER[..5];
        for s in crate::cli_command()
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
        {
            let g = group_of(s.get_name());
            assert!(
                root_groups.contains(&g),
                "root command `{}` is in `{g}`",
                s.get_name()
            );
        }
    }
}
