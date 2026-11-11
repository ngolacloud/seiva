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
        let module = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let mut layers = BTreeMap::new();
        for layer in LAYERS {
            let cargo = dir.join(layer).join("Cargo.toml");
            if cargo.is_file() {
                let text = std::fs::read_to_string(&cargo).map_err(|source| LintError::Io {
                    path: cargo.display().to_string(),
                    source,
                })?;
                layers.insert(layer.to_owned(), cargo_dependency_names(&text));
            }
        }
        if !layers.is_empty() {
            module_layers.push(ModuleLayers { module, layers });
        }
    }
    report.violations.extend(check_layers(&module_layers));

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
        let module = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let mut layers = BTreeMap::new();
        for layer in LAYERS {
            let cargo = dir.join(layer).join("Cargo.toml");
            if cargo.is_file() {
                let text = std::fs::read_to_string(&cargo).map_err(read_err(&cargo))?;
                layers.insert(layer.to_owned(), cargo_dependency_names(&text));
            }
        }
        if !layers.is_empty() {
            out.push(ModuleLayers { module, layers });
        }
    }
    Ok(out)
}

/// The dependency crate names declared in a `Cargo.toml`'s `[dependencies]`
/// table (dev-/build-dependencies are not part of the layer rule). Best-effort:
/// an unparseable manifest yields no names rather than failing the whole lint —
/// the build would reject it anyway.
fn cargo_dependency_names(text: &str) -> Vec<String> {
    let Ok(toml::Value::Table(root)) = text.parse::<toml::Value>() else {
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
