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
use std::path::Path;

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
    for path in paths {
        manifests.push(ModuleManifest::from_path(&path)?);
    }

    Ok(check_contracts(&manifests))
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
