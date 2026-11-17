//! CI gate (ADR 0030, Fatia 3): `seiva lint` FAILS when a pure layer names an ORM
//! or adapter crate. This drives the real file-scanning path — [`lint_project`]
//! over a module tree on disk — so the fitness function is proven end to end, not
//! just over in-memory fixtures. A clean module in the same shape must pass, so a
//! failure here is the rule firing, not the scaffold being malformed.

use std::fs;
use std::path::{Path, PathBuf};

use delonix_seiva::lint::lint_project;

/// A throwaway project root under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("seiva-neutrality-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch root");
        Scratch(dir)
    }
    fn write(&self, rel: &str, contents: &str) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).expect("create parent dirs");
        fs::write(path, contents).expect("write file");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes a minimal module `name` whose `application` depends on `app_dep`. The
/// manifest and the five-layer shape mirror what `seiva module new` scaffolds.
fn write_module(scratch: &Scratch, name: &str, app_dep: &str) {
    scratch.write(
        &format!("modules/{name}/module.toml"),
        &format!("[module]\nname = \"{name}\"\nfacade = \"v1\"\nowners = []\n"),
    );
    let cargo = |layer: &str, deps: &str| {
        format!("[package]\nname = \"{name}-{layer}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}")
    };
    scratch.write(
        &format!("modules/{name}/domain/Cargo.toml"),
        &cargo("domain", ""),
    );
    scratch.write(
        &format!("modules/{name}/application/Cargo.toml"),
        &cargo("application", &format!("{app_dep} = \"1\"\n")),
    );
}

#[test]
fn lint_fails_when_application_names_an_orm() {
    let scratch = Scratch::new("orm");
    // The infractor: its application layer names `sqlx` directly.
    write_module(&scratch, "orders", "sqlx");

    let report = lint_project(Path::new(&scratch.0)).expect("lint scan must not error");
    assert_eq!(report.modules_checked, 1, "the module must be discovered");
    assert!(
        !report.is_clean(),
        "an ORM in `application` must make the lint fail (rc != 0)"
    );
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "data-layer-neutrality" && v.module == "orders::application"),
        "expected a data-layer-neutrality violation, got: {:#?}",
        report.violations
    );
}

#[test]
fn lint_fails_when_facade_names_an_orm() {
    let scratch = Scratch::new("facade-orm");
    // The published contract (facade) is the single source of OpenAPI + proto +
    // routes; it must stay ORM-free too (ADR 0030, defence-in-depth). Write a module
    // whose `facade` — not its application — names `sqlx`.
    scratch.write(
        "modules/billing/module.toml",
        "[module]\nname = \"billing\"\nfacade = \"v1\"\nowners = []\n",
    );
    let cargo = |layer: &str, deps: &str| {
        format!("[package]\nname = \"billing-{layer}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}")
    };
    scratch.write("modules/billing/domain/Cargo.toml", &cargo("domain", ""));
    scratch.write(
        "modules/billing/application/Cargo.toml",
        &cargo("application", "seiva-ports = \"1\"\n"),
    );
    scratch.write(
        "modules/billing/facade/Cargo.toml",
        &cargo("facade", "sqlx = \"0.9\"\n"),
    );

    let report = lint_project(Path::new(&scratch.0)).expect("lint scan must not error");
    assert!(
        !report.is_clean()
            && report
                .violations
                .iter()
                .any(|v| v.rule == "data-layer-neutrality" && v.module == "billing::facade"),
        "an ORM in `facade` (the published contract) must fail the lint: {:#?}",
        report.violations
    );
}

#[test]
fn lint_fails_when_application_names_a_seiva_data_adapter() {
    let scratch = Scratch::new("adapter");
    // Naming the adapter crate directly is just as forbidden as naming the ORM.
    write_module(&scratch, "catalog", "seiva-data-sqlx");

    let report = lint_project(Path::new(&scratch.0)).expect("lint scan must not error");
    assert!(
        !report.is_clean()
            && report
                .violations
                .iter()
                .any(|v| v.rule == "data-layer-neutrality"),
        "naming `seiva-data-sqlx` in `application` must fail the lint: {:#?}",
        report.violations
    );
}

#[test]
fn lint_passes_for_a_pure_module_in_the_same_shape() {
    let scratch = Scratch::new("pure");
    // Same shape, but the application names only the ports crate — clean.
    write_module(&scratch, "notes", "seiva-ports");

    let report = lint_project(Path::new(&scratch.0)).expect("lint scan must not error");
    assert_eq!(report.modules_checked, 1);
    assert!(
        report
            .violations
            .iter()
            .all(|v| v.rule != "data-layer-neutrality"),
        "a pure module must raise no neutrality violation: {:#?}",
        report.violations
    );
}
