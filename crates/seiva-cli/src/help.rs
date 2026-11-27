//! Grouped, semantic help for `seiva` — the same mechanism as the `delonix`
//! binary's `help.rs`.
//!
//! clap 4 does not group subcommands (`help_heading` exists only for
//! arguments), so its help lists every subcommand of a node in one flat
//! `Commands:` column. This module renders the SAME help clap would, then moves
//! that flat block's lines under semantic section headings taken from
//! [`crate::manual::group_of`] — one taxonomy for the root and
//! for every group (`db`, `module`, …). It reuses clap's rendered lines
//! verbatim (about text and wrapping included) and keeps everything else — the
//! about paragraph, `Usage:`, `Options:` and the manual tail — byte-for-byte.
//!
//! A leaf (`seiva start --help`) has no subcommand list and is left to clap.

use std::collections::BTreeMap;

use crate::manual;

/// True when the argv asks for the TOP-LEVEL help: `seiva` alone, `seiva help`,
/// `seiva --help` or `seiva -h`.
pub fn wants_top_help(argv: &[String]) -> bool {
    let toks: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    matches!(toks.as_slice(), [] | ["--help"] | ["-h"] | ["help"])
}

/// The path of a GROUP whose help is requested — `seiva <path…> --help|-h` or
/// `seiva help <path…>` — when that path resolves to a command WITH
/// subcommands. `None` for a leaf (clap renders it) or an unknown path (clap
/// errors).
pub fn group_help_path(argv: &[String], root: &clap::Command) -> Option<Vec<String>> {
    let toks: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    let path: &[&str] = if toks.first() == Some(&"help") {
        &toks[1..]
    } else if matches!(toks.last(), Some(&"--help") | Some(&"-h")) {
        &toks[..toks.len() - 1]
    } else {
        return None;
    };
    if path.is_empty() {
        return None;
    }
    let mut cur = root;
    for seg in path {
        cur = cur.find_subcommand(seg)?;
    }
    cur.get_subcommands()
        .any(|s| s.get_name() != "help")
        .then(|| path.iter().map(|s| (*s).to_string()).collect())
}

/// The grouped TOP-LEVEL help of `cmd` (the fully-dressed root command).
pub fn render(mut cmd: clap::Command) -> String {
    let full = cmd.render_long_help().to_string();
    regroup(&full, |name| name.to_string())
}

/// The grouped help of the GROUP at `path`, from the help clap rendered for it.
pub fn render_group(native_help: &str, path: &[String]) -> String {
    let prefix = path.join(" ");
    regroup(native_help, |name| format!("{prefix} {name}"))
}

/// Replace clap's single flat `Commands:` block with sections; keep everything
/// else byte-for-byte. `child_path(name)` maps a listed command to its full
/// path, whose manual group names its section.
fn regroup(full: &str, child_path: impl Fn(&str) -> String) -> String {
    const CMDS: &str = "\nCommands:\n";
    const OPTS: &str = "\nOptions:";
    let Some(cmds_at) = full.find(CMDS) else {
        return full.to_string();
    };
    let block_start = cmds_at + CMDS.len();
    let Some(opts_rel) = full[block_start..].find(OPTS) else {
        return full.to_string();
    };
    let opts_at = block_start + opts_rel;

    let head = full[..cmds_at].trim_end(); // about paragraph + `Usage:` line
    let block = &full[block_start..opts_at]; // the flat command lines
    let after_options = &full[opts_at + OPTS.len()..]; // `-h`/`-V` + manual tail

    // clap lists each command at column 2 and indents a wrapped about line past
    // the name column: a line whose third byte is non-space starts a command,
    // anything else rides with the one above it.
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        let starts_command = line.starts_with("  ")
            && line
                .as_bytes()
                .get(2)
                .is_some_and(|b| !b.is_ascii_whitespace());
        if starts_command {
            let name = line[2..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            entries.push((name, line.to_string()));
        } else if let Some(last) = entries.last_mut() {
            last.1.push('\n');
            last.1.push_str(line);
        }
    }
    // clap's generated `help` subcommand is not a verb of this CLI; `seiva help
    // <cmd>` keeps working, it just is not listed.
    entries.retain(|(name, _)| name != "help");

    // Sections sort by group rank; commands keep the order they are declared in
    // (the derive order is the workflow order).
    let mut secs: BTreeMap<(usize, String), Vec<String>> = BTreeMap::new();
    for (name, rendered) in entries {
        let group = manual::group_of(&child_path(&name));
        let heading = if group.is_empty() {
            "Commands:".to_string()
        } else {
            format!("{group} Commands:")
        };
        secs.entry((manual::group_rank(group), heading))
            .or_default()
            .push(rendered);
    }

    let mut grouped = String::new();
    for ((_, heading), items) in secs {
        if !grouped.is_empty() {
            grouped.push_str("\n\n");
        }
        grouped.push_str(&heading);
        for rendered in items {
            grouped.push('\n');
            grouped.push_str(&rendered);
        }
    }

    let mut out = format!("{head}\n\n{grouped}\n\nOptions:{after_options}");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        std::iter::once("seiva")
            .chain(s.split_whitespace())
            .map(String::from)
            .collect()
    }

    #[test]
    fn top_help_is_detected_for_every_spelling() {
        for a in ["", "--help", "-h", "help"] {
            assert!(wants_top_help(&argv(a)), "`seiva {a}` must be top help");
        }
        assert!(!wants_top_help(&argv("db --help")));
        assert!(!wants_top_help(&argv("start")));
    }

    #[test]
    fn group_help_path_resolves_groups_only() {
        let root = crate::cli_command();
        assert_eq!(
            group_help_path(&argv("db --help"), &root),
            Some(vec!["db".into()])
        );
        assert_eq!(
            group_help_path(&argv("help module"), &root),
            Some(vec!["module".into()])
        );
        // A leaf is clap's to render, an unknown path is clap's to reject.
        assert_eq!(group_help_path(&argv("start --help"), &root), None);
        assert_eq!(group_help_path(&argv("nope --help"), &root), None);
    }

    #[test]
    fn root_help_lists_every_command_once_under_its_section() {
        let out = render(crate::cli_command());
        for heading in [
            "Project Commands:",
            "Development Commands:",
            "Contract Commands:",
            "Quality Commands:",
            "Tooling Commands:",
        ] {
            assert!(out.contains(heading), "missing `{heading}` in:\n{out}");
        }
        // Sections appear in workflow order.
        let at = |h: &str| out.find(h).unwrap();
        assert!(at("Project Commands:") < at("Development Commands:"));
        assert!(at("Development Commands:") < at("Contract Commands:"));
        assert!(at("Quality Commands:") < at("Tooling Commands:"));
        // No catch-all section and no listed `help` verb.
        assert!(
            !out.contains("\nCommands:\n"),
            "a command fell into the catch-all:\n{out}"
        );
        assert!(
            !out.contains("\n  help "),
            "`help` must not be listed:\n{out}"
        );
        for s in crate::cli_command()
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
        {
            let line = format!("\n  {} ", s.get_name());
            assert_eq!(
                out.matches(&line).count(),
                1,
                "`{}` listed != once:\n{out}",
                s.get_name()
            );
        }
        assert!(out.contains("WORKFLOW:"));
        assert!(out.contains("Options:"));
    }

    #[test]
    fn group_help_is_sectioned_by_the_same_taxonomy() {
        let mut root = crate::cli_command();
        let err = root
            .try_get_matches_from_mut(["seiva", "db", "--help"])
            .expect_err("--help is reported as an error kind");
        let out = render_group(&err.render().to_string(), &["db".to_string()]);
        let at = |h: &str| {
            out.find(h)
                .unwrap_or_else(|| panic!("missing `{h}`:\n{out}"))
        };
        assert!(at("Setup Commands:") < at("Migration Commands:"));
        assert!(at("Migration Commands:") < at("Data Commands:"));
        assert!(
            out.contains("EXAMPLES:"),
            "the db manual tail must survive:\n{out}"
        );
    }
}
