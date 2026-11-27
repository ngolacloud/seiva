//! Build metadata for the rich `seiva --version`: the commit, its distance from
//! the newest release tag, the build date and the compiler.
//!
//! No dependencies — shell-outs with fallbacks, so a build without `.git` (a
//! crates.io tarball, a vendored copy) still compiles and simply says `unknown`.
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

fn main() {
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
