//! Project and module scaffolding.
//!
//! Generation is pure: [`project_files`] and [`module_files`] return the files
//! to write (path + contents) without touching the filesystem, so they are
//! fully testable. The CLI performs the I/O and the registration edits, which
//! are themselves pure string transforms ([`add_members_to_workspace`],
//! [`register_module_in_seiva_toml`], [`append_namespace_block`]).
//!
//! This first cut lays down the five-layer structure with the dependency rule
//! enforced by the crate graph (the point of "one layer per crate"). The
//! generated crates compile standalone; wiring them to the framework runtime
//! (the facade macro, HTTP/gRPC, DI) lands in a later phase and is flagged in
//! the generated files.

/// A file to be written, with a path relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub contents: String,
}

/// A name was not a valid snake_case ASCII identifier, or a registration edit
/// could not find its anchor.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScaffoldError {
    #[error("`{0}` is not a valid name: use lowercase ASCII snake_case (e.g. `order_items`)")]
    InvalidName(String),
    #[error("already present: {0}")]
    AlreadyPresent(String),
    #[error("could not find `{anchor}` in {what}")]
    MissingAnchor { anchor: String, what: String },
}

/// The marker line in the workspace `Cargo.toml` members array before which new
/// module crate members are inserted.
const MEMBERS_ANCHOR: &str = "# seiva:module-members (managed by the seiva CLI)";

/// Validates a project or module name: lowercase ASCII, starts with a letter,
/// then letters/digits/underscores, no trailing or doubled underscore.
pub fn validate_name(name: &str) -> Result<(), ScaffoldError> {
    let invalid = || ScaffoldError::InvalidName(name.to_owned());
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Err(invalid()),
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(invalid());
    }
    if name.ends_with('_') || name.contains("__") {
        return Err(invalid());
    }
    Ok(())
}

/// The crate names of a module's five layers, in dependency order.
pub fn layer_crates(module: &str) -> [String; 5] {
    [
        format!("{module}-facade"),
        format!("{module}-domain"),
        format!("{module}-application"),
        format!("{module}-infrastructure"),
        format!("{module}-interface"),
    ]
}

/// The workspace member paths of a module's five layer crates.
pub fn module_member_paths(module: &str) -> Vec<String> {
    [
        "facade",
        "domain",
        "application",
        "infrastructure",
        "interface",
    ]
    .iter()
    .map(|layer| format!("modules/{module}/{layer}"))
    .collect()
}

/// Files for a new project `name`. Paths are relative to the new `name/` dir.
pub fn project_files(name: &str) -> Result<Vec<GeneratedFile>, ScaffoldError> {
    validate_name(name)?;
    let app = format!("{name}-api");
    let file = |path: &str, contents: String| GeneratedFile {
        path: path.to_owned(),
        contents,
    };

    Ok(vec![
        file("seiva.toml", seiva_toml(name)),
        file("Cargo.toml", workspace_cargo_toml(&app)),
        file(".gitignore", "/target\n**/*.rs.bk\n.env\n".to_owned()),
        file("README.md", format!("# {name}\n\nA Delonix Seiva project. Add modules with `seiva module new <name>`.\n")),
        file(&format!("schema/{name}.schema"), project_schema()),
        file("modules/.keep", String::new()),
        // apps/<name>-api: the composition root — wires modules, holds no logic.
        file(&format!("apps/{app}/Cargo.toml"), bin_cargo_toml(&app)),
        file(
            &format!("apps/{app}/src/main.rs"),
            format!(
                "//! Composition root for `{name}`: wire modules here, no business logic.\n\nfn main() {{\n    println!(\"{name} api — composition root (modules are wired here).\");\n}}\n"
            ),
        ),
        // shared/kernel: shared value types, no business rules.
        file("shared/kernel/Cargo.toml", lib_cargo_toml("kernel", &[])),
        file(
            "shared/kernel/src/lib.rs",
            "//! Shared value types across modules. No business rules live here.\n\n/// A tenant identifier, shared by every module's scope.\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub struct TenantId(pub u128);\n".to_owned(),
        ),
    ])
}

/// Files for a new module `module`, relative to the project root.
pub fn module_files(module: &str) -> Result<Vec<GeneratedFile>, ScaffoldError> {
    validate_name(module)?;
    let snake = module.replace('-', "_");
    let base = format!("modules/{module}");
    let file = |path: String, contents: String| GeneratedFile { path, contents };

    let dep = |layer: &str| (format!("{module}-{layer}"), format!("../{layer}"));
    let domain_dep = dep("domain");
    let application_dep = dep("application");
    let facade_dep = dep("facade");

    Ok(vec![
        file(format!("{base}/module.toml"), module_toml(module)),
        // facade
        file(
            format!("{base}/facade/Cargo.toml"),
            lib_cargo_toml(&format!("{module}-facade"), &[]),
        ),
        file(format!("{base}/facade/src/lib.rs"), facade_lib()),
        // domain
        file(
            format!("{base}/domain/Cargo.toml"),
            lib_cargo_toml(&format!("{module}-domain"), &[]),
        ),
        file(format!("{base}/domain/src/lib.rs"), domain_lib()),
        // application (depends on domain)
        file(
            format!("{base}/application/Cargo.toml"),
            lib_cargo_toml(&format!("{module}-application"), &[&domain_dep]),
        ),
        file(
            format!("{base}/application/src/lib.rs"),
            application_lib(&snake),
        ),
        // infrastructure (depends on application + domain)
        file(
            format!("{base}/infrastructure/Cargo.toml"),
            lib_cargo_toml(
                &format!("{module}-infrastructure"),
                &[&application_dep, &domain_dep],
            ),
        ),
        file(
            format!("{base}/infrastructure/src/lib.rs"),
            infrastructure_lib(&snake),
        ),
        // interface (depends on application + facade)
        file(
            format!("{base}/interface/Cargo.toml"),
            lib_cargo_toml(
                &format!("{module}-interface"),
                &[&application_dep, &facade_dep],
            ),
        ),
        file(
            format!("{base}/interface/src/lib.rs"),
            interface_lib(&snake),
        ),
    ])
}

/// Inserts a module's layer crates into the workspace `Cargo.toml` members,
/// before the managed anchor. Pure.
pub fn add_members_to_workspace(cargo: &str, module: &str) -> Result<String, ScaffoldError> {
    let anchor_line = cargo
        .lines()
        .find(|line| line.trim() == MEMBERS_ANCHOR)
        .ok_or_else(|| ScaffoldError::MissingAnchor {
            anchor: MEMBERS_ANCHOR.to_owned(),
            what: "Cargo.toml".to_owned(),
        })?;
    let indent: String = anchor_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    let additions: String = module_member_paths(module)
        .iter()
        .map(|path| format!("{indent}\"{path}\",\n"))
        .collect();
    if cargo.contains(&format!("\"modules/{module}/facade\"")) {
        return Err(ScaffoldError::AlreadyPresent(format!("module `{module}`")));
    }
    Ok(cargo.replacen(anchor_line, &format!("{additions}{anchor_line}"), 1))
}

/// Registers a module in `seiva.toml` by appending a `[modules.<name>]` section.
/// Pure.
pub fn register_module_in_seiva_toml(seiva: &str, module: &str) -> Result<String, ScaffoldError> {
    if seiva.contains(&format!("[modules.{module}]")) {
        return Err(ScaffoldError::AlreadyPresent(format!("module `{module}`")));
    }
    let sep = if seiva.ends_with('\n') { "" } else { "\n" };
    Ok(format!(
        "{seiva}{sep}\n[modules.{module}]\ndata = \"dorm\"\n"
    ))
}

/// Appends a `namespace <module> {{ }}` block to the project schema. Pure.
pub fn append_namespace_block(schema: &str, module: &str) -> Result<String, ScaffoldError> {
    if schema.contains(&format!("namespace {module} "))
        || schema.contains(&format!("namespace {module}{{"))
    {
        return Err(ScaffoldError::AlreadyPresent(format!(
            "namespace `{module}`"
        )));
    }
    let sep = if schema.ends_with('\n') { "" } else { "\n" };
    Ok(format!(
        "{schema}{sep}\nnamespace {module} {{\n  // Models owned by the `{module}` module. Only identifiers cross module\n  // boundaries: no relation to another namespace's models.\n  model Record {{\n    id      Uuid   @id\n    title   String\n    version Int    @version\n  }}\n}}\n"
    ))
}

// --- templates ---------------------------------------------------------------

fn seiva_toml(name: &str) -> String {
    format!(
        "# Delonix Seiva project manifest. Managed partly by the seiva CLI.\n\
         [project]\n\
         name = \"{name}\"\n\n\
         [data]\n\
         # DORM is the default data layer; set to \"external\" to own it yourself.\n\
         layer = \"dorm\"\n\n\
         # Modules are registered below by `seiva module new`.\n"
    )
}

fn workspace_cargo_toml(app: &str) -> String {
    format!(
        "# Workspace managed by the seiva CLI. Module crates are added below the\n\
         # managed anchor by `seiva module new`.\n\
         [workspace]\n\
         resolver = \"2\"\n\
         members = [\n\
         \x20   \"apps/{app}\",\n\
         \x20   \"shared/kernel\",\n\
         \x20   {MEMBERS_ANCHOR}\n\
         ]\n"
    )
}

fn bin_cargo_toml(app: &str) -> String {
    format!(
        "[package]\nname = \"{app}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
    )
}

/// A library crate manifest with path dependencies `(package_name, rel_path)`.
fn lib_cargo_toml(pkg: &str, deps: &[&(String, String)]) -> String {
    let mut out = format!(
        "[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n"
    );
    for (name, path) in deps {
        out.push_str(&format!("{name} = {{ path = \"{path}\" }}\n"));
    }
    out
}

fn module_toml(module: &str) -> String {
    format!(
        "[module]\nname = \"{module}\"\nfacade = \"v1\"\nowners = []\n\n\
         # Facades this module calls. Each needs a matching [[allow]] on the provider.\n\
         # [[uses]]\n# module = \"identity\"\n# facade = \"v1\"\n# methods = [\"user_exists\"]\n\n\
         # Callers this module permits. Anything not listed is denied.\n\
         # [[allow]]\n# caller = \"notes\"\n# methods = [\"...\"]\n"
    )
}

fn facade_lib() -> String {
    "//! Facade layer: the module's versioned public contract and DTOs.\n\
     //! May depend only on `shared/kernel` and the Seiva macros. Must NOT depend\n\
     //! on `domain`, `application` or `infrastructure`.\n\
     //!\n\
     //! Later phase: annotate the trait with `#[seiva::facade(version = \"v1\")]`\n\
     //! once the framework dependency is wired. Until then it is a plain trait.\n\
     #![allow(async_fn_in_trait)]\n\n\
     /// What other modules see of this module. Keep DTOs here, not domain types.\n\
     pub trait Contract {\n    async fn ping(&self) -> Result<(), String>;\n}\n"
        .to_owned()
}

fn domain_lib() -> String {
    "//! Domain layer: entities, value objects, domain events and rules.\n\
     //! May depend only on `shared/kernel`. Must NOT depend on any other layer,\n\
     //! the framework or DORM — the crate graph enforces this.\n\n\
     /// An example aggregate. Replace with the module's real domain model.\n\
     #[derive(Debug, Clone, PartialEq, Eq)]\n\
     pub struct Record {\n    pub id: u64,\n    pub title: String,\n}\n\n\
     impl Record {\n    pub fn new(id: u64, title: impl Into<String>) -> Self {\n        Self { id, title: title.into() }\n    }\n}\n"
        .to_owned()
}

fn application_lib(snake: &str) -> String {
    format!(
        "//! Application layer: use cases and outbound ports. Depends on `domain`\n\
         //! (and, once wired, `seiva-ports` and this module's facade DTOs). Must\n\
         //! NOT depend on infrastructure, interface, the framework or DORM.\n\
         #![allow(async_fn_in_trait)]\n\n\
         use {snake}_domain::Record;\n\n\
         /// Outbound port the use case needs; implemented in `infrastructure`.\n\
         pub trait RecordRepository {{\n    async fn save(&self, record: &Record) -> Result<(), String>;\n}}\n\n\
         /// Example use case (command). Generic over the port so it tests with a fake.\n\
         pub struct CreateRecord<R: RecordRepository> {{\n    pub repository: R,\n}}\n\n\
         impl<R: RecordRepository> CreateRecord<R> {{\n    pub async fn execute(&self, id: u64, title: &str) -> Result<Record, String> {{\n        let record = Record::new(id, title);\n        self.repository.save(&record).await?;\n        Ok(record)\n    }}\n}}\n"
    )
}

fn infrastructure_lib(snake: &str) -> String {
    format!(
        "//! Infrastructure layer: adapters implementing the application's ports —\n\
         //! repositories (over DORM or an external library), messaging, clients to\n\
         //! other modules' facades. Depends on `application` + `domain`. Must NOT\n\
         //! depend on `interface`.\n\n\
         use std::sync::Mutex;\n\
         use {snake}_application::RecordRepository;\n\
         use {snake}_domain::Record;\n\n\
         /// A simple in-memory adapter. Replace with a DORM-backed or external one.\n\
         #[derive(Default)]\n\
         pub struct InMemoryRecordRepository {{\n    saved: Mutex<Vec<Record>>,\n}}\n\n\
         impl RecordRepository for InMemoryRecordRepository {{\n    async fn save(&self, record: &Record) -> Result<(), String> {{\n        self.saved.lock().map_err(|_| \"lock poisoned\".to_string())?.push(record.clone());\n        Ok(())\n    }}\n}}\n"
    )
}

fn interface_lib(snake: &str) -> String {
    format!(
        "//! Interface layer: REST, gRPC and event consumers. Depends on\n\
         //! `application` + `facade` + the framework. Must NOT depend on\n\
         //! `infrastructure` or `domain`.\n\
         //!\n\
         //! Later phase: the framework generates route/gRPC wiring here.\n\n\
         use {snake}_application::{{CreateRecord, RecordRepository}};\n\n\
         /// Example controller shell. The framework will attach routes here.\n\
         pub struct Controller<R: RecordRepository> {{\n    pub create_record: CreateRecord<R>,\n}}\n"
    )
}

fn project_schema() -> String {
    "// Project schema: one `namespace` block per in-process module, added by\n\
     // `seiva module new`. Generated blocks are edited by their module owners only.\n\
     datasource db {\n  provider = \"postgresql\"\n}\n"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation() {
        assert!(validate_name("notes").is_ok());
        assert!(validate_name("order_items").is_ok());
        assert!(validate_name("Notes").is_err());
        assert!(validate_name("1notes").is_err());
        assert!(validate_name("notes_").is_err());
        assert!(validate_name("order__items").is_err());
        assert!(validate_name("order-items").is_err());
    }

    #[test]
    fn project_files_cover_the_skeleton() {
        let files = project_files("shop").unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"seiva.toml"));
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"schema/shop.schema"));
        assert!(paths.contains(&"apps/shop-api/Cargo.toml"));
        assert!(paths.contains(&"shared/kernel/src/lib.rs"));
    }

    #[test]
    fn module_files_have_five_layers_plus_manifest() {
        let files = module_files("notes").unwrap();
        for layer in [
            "facade",
            "domain",
            "application",
            "infrastructure",
            "interface",
        ] {
            assert!(
                files
                    .iter()
                    .any(|f| f.path == format!("modules/notes/{layer}/src/lib.rs")),
                "missing {layer}"
            );
        }
        assert!(files.iter().any(|f| f.path == "modules/notes/module.toml"));
    }

    #[test]
    fn workspace_registration_inserts_members() {
        let cargo = workspace_cargo_toml("shop-api");
        let out = add_members_to_workspace(&cargo, "notes").unwrap();
        assert!(out.contains("\"modules/notes/facade\","));
        assert!(out.contains("\"modules/notes/interface\","));
        // Idempotence guard: a second insert of the same module is refused.
        assert!(add_members_to_workspace(&out, "notes").is_err());
    }

    #[test]
    fn seiva_toml_registration_appends_section() {
        let seiva = seiva_toml("shop");
        let out = register_module_in_seiva_toml(&seiva, "notes").unwrap();
        assert!(out.contains("[modules.notes]"));
        assert!(register_module_in_seiva_toml(&out, "notes").is_err());
    }

    #[test]
    fn namespace_block_is_appended_once() {
        let schema = project_schema();
        let out = append_namespace_block(&schema, "notes").unwrap();
        assert!(out.contains("namespace notes {"));
        assert!(append_namespace_block(&out, "notes").is_err());
    }
}
