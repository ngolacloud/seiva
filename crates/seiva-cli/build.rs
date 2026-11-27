//! Build metadata for the rich `seiva --version`: the commit, its distance from
//! the newest release tag, the build date and the compiler.
//!
//! No dependencies — shell-outs with fallbacks. A build from the published
//! package (`cargo install seiva-cli`) has no `.git`; it reads the commit from
//! the `.cargo_vcs_info.json` that `cargo publish` writes into the package, and
//! never asks git (the package may sit inside some unrelated repository, e.g. a
//! dotfiles home). A tree with neither simply says `unknown`.
//! `SOURCE_DATE_EPOCH` is honoured for reproducible builds. Same approach as the
//! `delonix` binary's `build.rs`.

use std::path::Path;
use std::process::Command;

fn run(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The commit recorded by `cargo publish` in `.cargo_vcs_info.json`
/// (`{"git": {"sha1": "<40 hex>"}, …}`), shortened like `git rev-parse --short=9`.
/// Parsed by hand to keep the build script dependency-free.
fn packaged_commit() -> Option<String> {
    let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let info = std::fs::read_to_string(Path::new(&dir).join(".cargo_vcs_info.json")).ok()?;
    let rest = &info[info.find("\"sha1\"")? + "\"sha1\"".len()..];
    let start = rest.find('"')? + 1;
    let sha: String = rest[start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    (sha.len() >= 9).then(|| sha[..9].to_string())
}

/// Emit the build metadata the `--version` card reads (`env!`): the commit, its
/// distance from the newest tag (`"<n>|<tag>"`, empty at the tag), whether the
/// tree was modified, the build date and the compiler version.
fn emit(hash: &str, since: &str, dirty: bool) {
    let date = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(epoch) => run("date", &["-u", "-d", &format!("@{epoch}"), "+%Y-%m-%d"]),
        Err(_) => run("date", &["-u", "+%Y-%m-%d"]),
    }
    .unwrap_or_else(|| "unknown".into());

    // `rustc 1.99.1 (58cf8f9c3 2026-10-13)` → `1.99.1`.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let rustc_version = run(&rustc, &["--version"])
        .and_then(|v| v.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=SEIVA_GIT_HASH={hash}");
    println!("cargo:rustc-env=SEIVA_GIT_SINCE={since}");
    println!(
        "cargo:rustc-env=SEIVA_GIT_DIRTY={}",
        if dirty { "1" } else { "" }
    );
    println!("cargo:rustc-env=SEIVA_BUILD_DATE={date}");
    println!("cargo:rustc-env=SEIVA_RUSTC_VERSION={rustc_version}");
}

fn main() {
    println!("cargo:rerun-if-changed=.cargo_vcs_info.json");
    if let Some(hash) = packaged_commit() {
        // A published package is exactly its release commit: no distance from
        // the tag, and never "modified".
        emit(&hash, "", false);
        return;
    }

    let hash = run("git", &["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".into());

    // Distance from the newest `v*` tag this commit contains. The workspace
    // version stays equal to that tag between releases, so the number alone
    // cannot tell a release build from one built 40 commits later. Empty at the
    // tag itself and without git.
    let since = run(
        "git",
        &["describe", "--tags", "--long", "--match", "v[0-9]*"],
    )
    .and_then(|d| {
        // `v0.2.0-12-gabcdef123`: the tag may itself contain `-`, so split
        // from the right.
        let mut parts = d.rsplitn(3, '-');
        let _hash = parts.next()?;
        let count = parts.next()?;
        let tag = parts.next()?;
        (count != "0").then(|| format!("{count}|{tag}"))
    })
    .unwrap_or_default();

    // A tree with uncommitted changes is not the commit it names.
    let dirty = run("git", &["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    emit(&hash, &since, dirty);

    // Re-run when HEAD moves or a tag appears — without this the hash fossilizes
    // between commits. Only watched when they exist: a missing path would make
    // cargo re-run this script on every build of a git-less tree.
    // A commit moves the branch ref, not HEAD itself, so the ref HEAD points at
    // is watched too.
    let mut watched = vec![
        "../../.git/HEAD".to_string(),
        "../../.git/refs/tags".to_string(),
        "../../.git/packed-refs".to_string(),
        "../../.git/index".to_string(),
    ];
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD") {
        if let Some(r) = head.trim().strip_prefix("ref: ") {
            watched.push(format!("../../.git/{r}"));
        }
    }
    for p in watched {
        if Path::new(&p).exists() {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
}
