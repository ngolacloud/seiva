//! Architecture fitness functions. First rule: the zero-trust contract check.
//!
//! The brief's rule, verbatim: a consumer declares `[[uses]]` and a provider
//! declares `[[allow]]`; the two must coincide or the build is refused, and
//! everything is denied by default. This module turns that into a check over a
//! set of [`ModuleManifest`]s so `seiva lint` can enforce it before anything
//! else. The check is pure (no I/O), so it is fully testable with fixtures; the
//! file scanning lives in [`lint_project`].

use crate::manifest::{ManifestError, ModuleManifest};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A single contract violation found by the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// A stable kebab-case rule id, for suppression and tooling.
    pub rule: &'static str,
    /// The module the violation is attributed to (the consumer).
    pub module: String,
    /// A human-readable explanation.
    pub message: String,
}

/// The outcome of a contract check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// How many modules were checked.
    pub modules_checked: usize,
    /// Every violation found, in a stable order.
    pub violations: Vec<Violation>,
}

impl Report {
    /// Whether the check passed (no violations).
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Checks that every `[[uses]]` is matched by a provider's `[[allow]]`, that the
/// facade versions agree, and that module names are unique. Pure over the given
/// manifests; deterministic order out.
pub fn check_contracts(manifests: &[ModuleManifest]) -> Report {
    let mut violations = Vec::new();

    // Index by module name, reporting duplicates rather than silently dropping.
    let mut by_name: BTreeMap<&str, &ModuleManifest> = BTreeMap::new();
    for manifest in manifests {
        let name = manifest.module.name.as_str();
        if by_name.insert(name, manifest).is_some() {
            violations.push(Violation {
                rule: "duplicate-module-name",
                module: name.to_owned(),
                message: format!("module name `{name}` is declared more than once"),
            });
        }
    }

    for consumer in manifests {
        let consumer_name = consumer.module.name.as_str();
        for used in &consumer.uses {
            let Some(provider) = by_name.get(used.module.as_str()) else {
                violations.push(Violation {
                    rule: "uses-unknown-module",
                    module: consumer_name.to_owned(),
                    message: format!(
                        "uses module `{}`, which is not declared in the project",
                        used.module
                    ),
                });
                continue;
            };

            if provider.module.facade != used.facade {
                violations.push(Violation {
                    rule: "facade-version-mismatch",
                    module: consumer_name.to_owned(),
                    message: format!(
                        "uses `{}` facade `{}`, but `{}` exposes facade `{}`",
                        used.module, used.facade, used.module, provider.module.facade
                    ),
                });
            }

            // Everything is denied by default: the provider must have an
            // `[[allow]]` naming this consumer, and that entry must list every
            // method the consumer calls.
            let allowed: Vec<&str> = provider
                .allow
                .iter()
                .filter(|a| a.caller == consumer_name)
                .flat_map(|a| a.methods.iter().map(String::as_str))
                .collect();

            let has_allow_entry = provider.allow.iter().any(|a| a.caller == consumer_name);
            if !has_allow_entry {
                violations.push(Violation {
                    rule: "uses-without-allow",
                    module: consumer_name.to_owned(),
                    message: format!(
                        "uses `{}`, but `{}` does not `[[allow]]` caller `{}`",
                        used.module, used.module, consumer_name
                    ),
                });
                continue;
            }

            for method in &used.methods {
                if !allowed.contains(&method.as_str()) {
                    violations.push(Violation {
                        rule: "method-not-allowed",
                        module: consumer_name.to_owned(),
                        message: format!(
                            "calls `{}::{}`, which `{}` does not allow `{}` to call",
                            used.module, method, used.module, consumer_name
                        ),
                    });
                }
            }
        }
    }

    Report {
        modules_checked: manifests.len(),
        violations,
    }
}

// --- fitness function 2: the layer dependency rule -------------------------

/// The five layer crate suffixes, outermost-last. A module's crates are named
/// `<module>-<layer>`, and the dependency rule between them is enforced by the
/// crate graph (the compiler); this fitness function catches a `Cargo.toml` that
/// declares a forbidden dependency before the compiler would even see the import.
pub const LAYERS: [&str; 5] = [
    "facade",
    "domain",
    "application",
    "infrastructure",
    "interface",
];

/// One module's layer crates and the dependency crate names each declares. Kept
/// separate from I/O so [`check_layers`] is testable with fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleLayers {
    pub module: String,
    /// Layer name (`"domain"`, …) → the dependency crate names in its Cargo.toml.
    pub layers: BTreeMap<String, Vec<String>>,
}

/// The layer `<module>-<layer>` a dependency crate belongs to, if any.
fn layer_suffix(dep: &str) -> Option<&'static str> {
    LAYERS.iter().copied().find(|l| {
        dep.len() > l.len() + 1
            && dep.ends_with(l)
            && dep.as_bytes()[dep.len() - l.len() - 1] == b'-'
    })
}

/// Whether a dependency is a DORM crate (the data layer).
fn is_dorm(dep: &str) -> bool {
    dep == "dorm" || dep.starts_with("dorm-")
}

/// Whether a dependency is a Seiva data-adapter crate (`seiva-data-dorm`,
/// `seiva-data-sqlx`, `seiva-data-seaorm`, …). These bind the ports to a concrete
/// backend and belong in `infrastructure`, never in `application`/`domain`.
fn is_data_adapter(dep: &str) -> bool {
    dep.starts_with("seiva-data-")
}

/// Whether a dependency is a known ORM or raw SQL-driver crate — a data-layer
/// concern the pure layers must never name (ADR 0030). Matched by crate name so it
/// works even when the adapter crate is excluded from the workspace (the reference
/// adapters are): the lint reads dependency *names*, not the resolved graph.
fn is_orm_crate(dep: &str) -> bool {
    // Exact ORM/driver crates plus their sub-crates (`sqlx-postgres`,
    // `sea-orm-migration`, `diesel_migrations`, …) via a prefix on the family.
    const ORM_FAMILIES: [&str; 11] = [
        "sqlx",
        "sea-orm",
        "sea_orm",
        "diesel",
        "tokio-postgres",
        "postgres",
        "rusqlite",
        "mysql",
        "mysql_async",
        "rbatis",
        "rbdc",
    ];
    ORM_FAMILIES.iter().any(|fam| {
        dep == *fam || dep.starts_with(&format!("{fam}-")) || dep.starts_with(&format!("{fam}_"))
    })
}

/// Whether a dependency is any data-layer crate the pure layers must not name:
/// DORM, a Seiva data adapter, or a third-party ORM/driver.
fn is_data_layer_dep(dep: &str) -> bool {
    is_dorm(dep) || is_data_adapter(dep) || is_orm_crate(dep)
}

/// The reason, if any, that `layer` of `module` may not depend on `dep`
/// (Clean-Architecture dependency rule; ADR 0003 of the Seiva repo).
fn forbidden_layer_dep(module: &str, layer: &str, dep: &str) -> Option<String> {
    let suffix = layer_suffix(dep);
    match layer {
        // Depends only on shared kernel types.
        "domain" => {
            (dep == "delonix-seiva" || dep == "seiva-ports" || is_dorm(dep) || suffix.is_some())
                .then(|| format!("`domain` depends only on shared kernel types, not `{dep}`"))
        }
        // The public contract: shared kernel + the Seiva macros only.
        "facade" => (is_dorm(dep)
            || matches!(
                suffix,
                Some("domain" | "application" | "infrastructure" | "interface")
            ))
        .then(|| format!("`facade` is the public contract and must not depend on `{dep}`")),
        // Use cases + ports: its own domain/facade and `seiva-ports`; never the
        // framework runtime, the data layer, lower layers, or another module.
        "application" => {
            if dep == "delonix-seiva" || is_dorm(dep) {
                Some(format!(
                    "`application` stays free of the framework runtime and the data layer, not `{dep}`"
                ))
            } else if matches!(suffix, Some("infrastructure" | "interface")) {
                Some(format!(
                    "`application` must not depend on the lower layer `{dep}`"
                ))
            } else if matches!(suffix, Some("domain" | "facade"))
                && dep != format!("{module}-domain")
                && dep != format!("{module}-facade")
            {
                Some(format!(
                    "`application` may use only its own domain and facade, not another module's `{dep}`"
                ))
            } else {
                None
            }
        }
        // Adapters: may reach application/domain, the data layer and other
        // modules' facades, but never the interface layer.
        "infrastructure" => matches!(suffix, Some("interface")).then(|| {
            format!("`infrastructure` must not depend on the `interface` layer (`{dep}`)")
        }),
        // Transport: application + facade; never infrastructure or domain.
        "interface" => matches!(suffix, Some("infrastructure" | "domain")).then(|| {
            format!(
                "`interface` must not depend on `{dep}` (the {} layer)",
                suffix.unwrap()
            )
        }),
        _ => None,
    }
}

/// Checks the layer dependency rule over a set of modules. Pure; deterministic.
pub fn check_layers(modules: &[ModuleLayers]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for m in modules {
        for (layer, deps) in &m.layers {
            for dep in deps {
                if let Some(message) = forbidden_layer_dep(&m.module, layer, dep) {
                    violations.push(Violation {
                        rule: "layer-dependency",
                        module: format!("{}::{layer}", m.module),
                        message,
                    });
                }
            }
        }
    }
    violations
}

/// Checks the data-layer neutrality rule (ADR 0030): an `application` or `domain`
/// crate must name no data-layer crate — not a Seiva data adapter (`seiva-data-*`)
/// and not an ORM/driver (`sqlx`, `sea-orm`, `diesel`, `tokio-postgres`, …). The
/// pure layers code against the ports in `seiva-ports` and their own repository
/// port, never a concrete backend. Pure; deterministic. The DORM-specific edge is
/// also caught by [`check_layers`]; this check is the broader neutrality gate that
/// turns "don't name an ORM" from a convention into a fitness function.
pub fn check_data_layer_neutrality(modules: &[ModuleLayers]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for m in modules {
        for layer in ["application", "domain"] {
            let Some(deps) = m.layers.get(layer) else {
                continue;
            };
            for dep in deps {
                if is_data_layer_dep(dep) {
                    violations.push(Violation {
                        rule: "data-layer-neutrality",
                        module: format!("{}::{layer}", m.module),
                        message: format!(
                            "`{layer}` must stay ORM-free and name no data-layer crate, not `{dep}` \
                             (ADR 0030): code against `seiva-ports` and the module's repository port, \
                             and bind the backend in `infrastructure`"
                        ),
                    });
                }
            }
        }
    }
    violations
}

/// Failure while scanning a project for module manifests.
#[derive(Debug, thiserror::Error)]
pub enum LintError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

/// Loads every `modules/<name>/module.toml` under `root` and runs the contract
/// check. A project with no `modules/` directory, or no manifests in it, yields
/// an empty clean report (0 modules checked) rather than an error.
pub fn lint_project(root: &Path) -> Result<Report, LintError> {
    let modules_dir = root.join("modules");
    if !modules_dir.is_dir() {
        return Ok(Report::default());
    }

    let mut manifests = Vec::new();
    let entries = std::fs::read_dir(&modules_dir).map_err(|source| LintError::Io {
        path: modules_dir.display().to_string(),
        source,
    })?;
    // Collect and sort for deterministic order regardless of filesystem order.
    let mut paths: Vec<_> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LintError::Io {
            path: modules_dir.display().to_string(),
            source,
        })?;
        let manifest_path = entry.path().join("module.toml");
        if manifest_path.is_file() {
            paths.push(manifest_path);
        }
    }
    paths.sort();
    let mut module_dirs = Vec::new();
    for path in paths {
        manifests.push(ModuleManifest::from_path(&path)?);
        // The manifest sits at modules/<dir>/module.toml; keep <dir> for the
        // layer scan, which reads modules/<dir>/<layer>/Cargo.toml.
        if let Some(dir) = path.parent().map(Path::to_path_buf) {
            module_dirs.push(dir);
        }
    }

    let mut report = check_contracts(&manifests);

    // Fitness function 2: the layer dependency rule, read from each module's
    // five layer-crate Cargo.toml files.
    let mut module_layers = Vec::new();
    for dir in &module_dirs {
        if let Some(scanned) = scan_one_module(dir)? {
            module_layers.push(scanned);
        }
    }
    report.violations.extend(check_layers(&module_layers));
    // Fitness function 3: data-layer neutrality (ADR 0030) — no ORM/adapter crate
    // named by `application`/`domain`.
    report
        .violations
        .extend(check_data_layer_neutrality(&module_layers));

    Ok(report)
}

/// Scans a directory whose immediate children are modules — each laid out as
/// `<module>/<layer>/Cargo.toml` — and returns the [`ModuleLayers`] for the layer
/// dependency rule, WITHOUT requiring a `module.toml` contract manifest.
///
/// This is what lets a CI gate run the layer rule over `examples/**` (SG2): the
/// canonical reference modules live there, not in a project's `modules/`, so
/// [`lint_project`] — which keys off `module.toml` — never saw them, and a layer
/// regression in an example passed green. A child directory with no layer crate
/// is skipped (e.g. a `shop/` grouping dir, scanned one level down by the caller).
///
/// # Errors
/// Returns [`LintError::Io`] if `modules_root` or a layer `Cargo.toml` cannot be read.
pub fn scan_module_layers(modules_root: &Path) -> Result<Vec<ModuleLayers>, LintError> {
    let read_err = |path: &Path| {
        let path = path.display().to_string();
        move |source| LintError::Io { path, source }
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(modules_root).map_err(read_err(modules_root))? {
        let entry = entry.map_err(read_err(modules_root))?;
        if entry.path().is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort(); // deterministic regardless of filesystem order

    let mut out = Vec::new();
    for dir in &dirs {
        if let Some(scanned) = scan_one_module(dir)? {
            out.push(scanned);
        }
    }
    Ok(out)
}

/// Scans one module directory (`<dir>/<layer>/Cargo.toml` for the five layers) into
/// a [`ModuleLayers`], or `None` when the directory holds no layer crate (a
/// grouping dir like `shop/`, scanned one level down by the caller).
///
/// The module's identity is taken from its crate PACKAGE NAMES, not the directory:
/// a grouped module in `examples/shop/orders/` has crates `shop-orders-<layer>`, so
/// keying off the directory (`orders`) would make the layer rule read its own
/// `shop-orders-domain` as "another module's" and false-positive. The identity is
/// the shared `<prefix>` of `<prefix>-<layer>`; the directory name is the fallback.
fn scan_one_module(dir: &Path) -> Result<Option<ModuleLayers>, LintError> {
    let read_err = |path: &Path| {
        let path = path.display().to_string();
        move |source| LintError::Io { path, source }
    };
    let mut layers = BTreeMap::new();
    let mut identity: Option<String> = None;
    for layer in LAYERS {
        let cargo = dir.join(layer).join("Cargo.toml");
        if cargo.is_file() {
            let text = std::fs::read_to_string(&cargo).map_err(read_err(&cargo))?;
            // The first layer that names itself `<prefix>-<layer>` fixes the module
            // identity for all five (they share the prefix).
            if identity.is_none() {
                identity = crate_name_prefix(&text, layer);
            }
            layers.insert(layer.to_owned(), cargo_dependency_names(&text));
        }
    }
    if layers.is_empty() {
        return Ok(None);
    }
    let module = identity.unwrap_or_else(|| {
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned()
    });
    Ok(Some(ModuleLayers { module, layers }))
}

/// The `<prefix>` of a layer crate whose `package.name` is `<prefix>-<layer>` (e.g.
/// `shop-orders-domain` at layer `domain` → `shop-orders`). `None` if the manifest
/// is unparseable, has no `package.name`, or the name does not end in `-<layer>`.
fn crate_name_prefix(cargo_text: &str, layer: &str) -> Option<String> {
    let name = toml::from_str::<toml::Table>(cargo_text)
        .ok()?
        .get("package")?
        .get("name")?
        .as_str()?
        .to_owned();
    name.strip_suffix(&format!("-{layer}"))
        .map(ToOwned::to_owned)
}

/// The dependency crate names declared in a `Cargo.toml`'s `[dependencies]`
/// table (dev-/build-dependencies are not part of the layer rule). Best-effort:
/// an unparseable manifest yields no names rather than failing the whole lint —
/// the build would reject it anyway.
fn cargo_dependency_names(text: &str) -> Vec<String> {
    // Parse with serde (`toml::from_str`), NOT `str::parse::<toml::Value>()`:
    // in toml 0.9 `Value`'s `FromStr` no longer parses a document, so `.parse()`
    // silently yielded `Err` here and this scan returned no names at all — making
    // the layer rule and the data-layer-neutrality gate vacuous no-ops over real
    // files (the toml-0.9 trap recorded for `check_layers`). `from_str` parses.
    let Ok(root) = toml::from_str::<toml::Table>(text) else {
        return Vec::new();
    };
    match root.get("dependencies") {
        Some(toml::Value::Table(deps)) => deps.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Allow, ModuleSection, Uses};

    fn module(name: &str, facade: &str) -> ModuleManifest {
        ModuleManifest {
            module: ModuleSection {
                name: name.to_owned(),
                facade: facade.to_owned(),
                owners: vec![],
            },
            uses: vec![],
            allow: vec![],
        }
    }

    fn layers(module: &str, pairs: &[(&str, &[&str])]) -> ModuleLayers {
        ModuleLayers {
            module: module.to_owned(),
            layers: pairs
                .iter()
                .map(|(l, deps)| (l.to_string(), deps.iter().map(|d| d.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn a_well_layered_module_passes() {
        let m = layers(
            "notes",
            &[
                ("facade", &["delonix-seiva"]),
                ("domain", &["kernel"]),
                (
                    "application",
                    &["notes-domain", "notes-facade", "seiva-ports"],
                ),
                (
                    "infrastructure",
                    &[
                        "notes-application",
                        "notes-domain",
                        "dorm-core",
                        "identity-facade",
                    ],
                ),
                (
                    "interface",
                    &["notes-application", "notes-facade", "delonix-seiva"],
                ),
            ],
        );
        assert!(check_layers(&[m]).is_empty());
    }

    #[test]
    fn neutrality_passes_for_a_pure_module() {
        // A well-layered module names only ports + its own repository port in the
        // pure layers; the ORM lives in infrastructure. No neutrality violation.
        let m = layers(
            "notes",
            &[
                ("domain", &["kernel"]),
                (
                    "application",
                    &["notes-domain", "notes-facade", "seiva-ports"],
                ),
                (
                    "infrastructure",
                    &["notes-application", "sqlx", "seiva-data-sqlx"],
                ),
            ],
        );
        assert!(
            check_data_layer_neutrality(&[m]).is_empty(),
            "an ORM in `infrastructure` is allowed"
        );
    }

    #[test]
    fn neutrality_flags_an_orm_or_adapter_in_the_pure_layers() {
        // The infractor: application names sqlx AND a Seiva adapter; domain names
        // tokio-postgres. Each is a data-layer crate forbidden in a pure layer.
        let m = layers(
            "orders",
            &[
                ("domain", &["kernel", "tokio-postgres"]),
                (
                    "application",
                    &[
                        "orders-domain",
                        "orders-facade",
                        "sqlx",
                        "seiva-data-seaorm",
                    ],
                ),
            ],
        );
        let v = check_data_layer_neutrality(&[m]);
        assert_eq!(v.len(), 3, "{v:?}");
        assert!(v.iter().all(|x| x.rule == "data-layer-neutrality"));
        assert!(v.iter().any(|x| x.module == "orders::domain"));
        assert!(v.iter().any(|x| x.module == "orders::application"));
    }

    #[test]
    fn is_orm_crate_matches_families_and_subcrates_only() {
        assert!(super::is_orm_crate("sqlx"));
        assert!(super::is_orm_crate("sqlx-postgres"));
        assert!(super::is_orm_crate("sea-orm"));
        assert!(super::is_orm_crate("sea-orm-migration"));
        assert!(super::is_orm_crate("diesel_migrations"));
        assert!(super::is_data_adapter("seiva-data-dorm"));
        // Not ORMs: a lookalike name that is not part of a known family.
        assert!(!super::is_orm_crate("my-sql-builder")); // not the `mysql` family
        assert!(!super::is_orm_crate("seiva-ports"));
        assert!(!super::is_orm_crate("serde"));
    }

    #[test]
    fn domain_may_not_depend_on_the_framework_or_the_data_layer() {
        let m = layers(
            "notes",
            &[(
                "domain",
                &["delonix-seiva", "dorm-core", "notes-application"],
            )],
        );
        let v = check_layers(&[m]);
        assert_eq!(v.len(), 3, "{v:?}");
        assert!(v
            .iter()
            .all(|x| x.rule == "layer-dependency" && x.module == "notes::domain"));
    }

    #[test]
    fn application_may_not_reach_a_lower_layer_or_another_modules_internals() {
        let m = layers(
            "notes",
            &[(
                "application",
                &[
                    "notes-infrastructure", // lower layer
                    "identity-domain",      // another module's domain
                    "delonix-seiva",        // the framework runtime
                    "notes-domain",         // OK: own domain
                    "seiva-ports",          // OK
                ],
            )],
        );
        let v = check_layers(&[m]);
        assert_eq!(v.len(), 3, "{v:?}");
    }

    #[test]
    fn interface_may_not_depend_on_infrastructure_or_domain() {
        let m = layers(
            "notes",
            &[(
                "interface",
                &[
                    "notes-infrastructure",
                    "notes-domain",
                    "notes-application",
                    "notes-facade",
                ],
            )],
        );
        let v = check_layers(&[m]);
        assert_eq!(
            v.len(),
            2,
            "only the infrastructure and domain deps are refused: {v:?}"
        );
    }

    #[test]
    fn matching_uses_and_allow_pass() {
        let mut notes = module("notes", "v1");
        notes.uses.push(Uses {
            module: "identity".into(),
            facade: "v1".into(),
            methods: vec!["user_exists".into()],
        });
        let mut identity = module("identity", "v1");
        identity.allow.push(Allow {
            caller: "notes".into(),
            methods: vec!["user_exists".into()],
        });

        let report = check_contracts(&[notes, identity]);
        assert!(report.is_clean(), "{:?}", report.violations);
        assert_eq!(report.modules_checked, 2);
    }

    #[test]
    fn uses_without_allow_is_denied_by_default() {
        let mut notes = module("notes", "v1");
        notes.uses.push(Uses {
            module: "identity".into(),
            facade: "v1".into(),
            methods: vec!["user_exists".into()],
        });
        let identity = module("identity", "v1"); // no [[allow]]

        let report = check_contracts(&[notes, identity]);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].rule, "uses-without-allow");
        assert_eq!(report.violations[0].module, "notes");
    }

    #[test]
    fn calling_an_unallowed_method_fails() {
        let mut notes = module("notes", "v1");
        notes.uses.push(Uses {
            module: "identity".into(),
            facade: "v1".into(),
            methods: vec!["user_exists".into(), "delete_user".into()],
        });
        let mut identity = module("identity", "v1");
        identity.allow.push(Allow {
            caller: "notes".into(),
            methods: vec!["user_exists".into()], // delete_user NOT allowed
        });

        let report = check_contracts(&[notes, identity]);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].rule, "method-not-allowed");
        assert!(report.violations[0].message.contains("delete_user"));
    }

    #[test]
    fn using_an_unknown_module_fails() {
        let mut notes = module("notes", "v1");
        notes.uses.push(Uses {
            module: "ghost".into(),
            facade: "v1".into(),
            methods: vec![],
        });
        let report = check_contracts(&[notes]);
        assert_eq!(report.violations[0].rule, "uses-unknown-module");
    }

    #[test]
    fn facade_version_must_match() {
        let mut notes = module("notes", "v1");
        notes.uses.push(Uses {
            module: "identity".into(),
            facade: "v2".into(), // identity exposes v1
            methods: vec![],
        });
        let mut identity = module("identity", "v1");
        identity.allow.push(Allow {
            caller: "notes".into(),
            methods: vec![],
        });
        let report = check_contracts(&[notes, identity]);
        assert!(report
            .violations
            .iter()
            .any(|v| v.rule == "facade-version-mismatch"));
    }

    #[test]
    fn duplicate_module_names_are_reported() {
        let report = check_contracts(&[module("notes", "v1"), module("notes", "v1")]);
        assert!(report
            .violations
            .iter()
            .any(|v| v.rule == "duplicate-module-name"));
    }
}
