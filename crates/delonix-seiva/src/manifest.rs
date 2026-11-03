//! The `module.toml` manifest: a module's identity and its zero-trust contract.
//!
//! Each module declares, in `module.toml`, the facade version it exposes, who
//! owns it, the facades it **uses** (`[[uses]]`) and who it **allows** to call
//! it (`[[allow]]`). The [`crate::lint`] contract check reads these and enforces
//! that every `uses` has a matching `allow` — everything is denied by default.

use serde::Deserialize;

/// A parsed `module.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModuleManifest {
    pub module: ModuleSection,
    /// Facades this module calls. Each needs a matching `[[allow]]` on the
    /// provider, or the contract check fails.
    #[serde(default)]
    pub uses: Vec<Uses>,
    /// Callers this module permits, and the methods each may call. Anything not
    /// listed is denied.
    #[serde(default)]
    pub allow: Vec<Allow>,
}

/// The `[module]` section: the module's identity.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModuleSection {
    /// The module name (snake_case ASCII), unique within the project.
    pub name: String,
    /// The version of the facade this module exposes, e.g. `"v1"`.
    pub facade: String,
    /// Owners who must approve changes to this module's schema block
    /// (`seiva lint --owners`). Optional.
    #[serde(default)]
    pub owners: Vec<String>,
}

/// A `[[uses]]` entry: a facade this module calls.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Uses {
    /// The provider module's name.
    pub module: String,
    /// The provider facade version this module codes against.
    pub facade: String,
    /// The methods this module calls on that facade.
    #[serde(default)]
    pub methods: Vec<String>,
}

/// An `[[allow]]` entry: a caller this module permits, and the methods allowed.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Allow {
    /// The calling module's name.
    pub caller: String,
    /// The methods that caller may call. Anything not listed is denied.
    #[serde(default)]
    pub methods: Vec<String>,
}

/// Failed to read or parse a `module.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}

impl ModuleManifest {
    /// Parses a manifest from TOML text, tagging errors with `path` for context.
    pub fn from_toml(text: &str, path: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|source| ManifestError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    /// Reads and parses a manifest from a file.
    pub fn from_path(path: &std::path::Path) -> Result<Self, ManifestError> {
        let display = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: display.clone(),
            source,
        })?;
        Self::from_toml(&text, &display)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_manifest() {
        let text = r#"
            [module]
            name = "notes"
            facade = "v1"
            owners = ["team-notes"]

            [[uses]]
            module = "identity"
            facade = "v1"
            methods = ["user_exists"]
        "#;
        let m = ModuleManifest::from_toml(text, "notes/module.toml").unwrap();
        assert_eq!(m.module.name, "notes");
        assert_eq!(m.module.facade, "v1");
        assert_eq!(m.module.owners, ["team-notes"]);
        assert_eq!(m.uses.len(), 1);
        assert_eq!(m.uses[0].module, "identity");
        assert_eq!(m.uses[0].methods, ["user_exists"]);
        assert!(m.allow.is_empty());
    }

    #[test]
    fn uses_and_allow_default_to_empty() {
        let text = r#"
            [module]
            name = "identity"
            facade = "v1"
        "#;
        let m = ModuleManifest::from_toml(text, "identity/module.toml").unwrap();
        assert!(m.uses.is_empty());
        assert!(m.allow.is_empty());
        assert!(m.module.owners.is_empty());
    }

    #[test]
    fn a_parse_error_carries_the_path() {
        let err = ModuleManifest::from_toml("not = valid = toml", "bad/module.toml").unwrap_err();
        assert!(matches!(err, ManifestError::Parse { .. }));
        assert!(err.to_string().contains("bad/module.toml"));
    }
}
