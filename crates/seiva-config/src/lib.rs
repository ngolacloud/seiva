//! Typed, 12-factor configuration for the Delonix Seiva framework.
//!
//! The twelve-factor "config" principle is that configuration lives in the
//! *environment*, not in the code or in a checked-in file. This crate reads
//! configuration only from process environment variables, decodes it into
//! typed values, and — crucially — **validates it at startup**. A service that
//! is missing a required variable or was handed an unparseable value should
//! refuse to boot, not surface the problem on the first request that happens to
//! touch the bad value.
//!
//! # The three pieces
//!
//! - [`AppEnv`] — the runtime environment ([`Development`](AppEnv::Development),
//!   [`Staging`](AppEnv::Staging), [`Production`](AppEnv::Production)), read
//!   from `APP_ENV` (defaulting to development). It carries the
//!   [production-safety guard](AppEnv::reject_dev_shortcut).
//! - [`EnvReader`] — a small typed reader with [`require_var`](EnvReader::require_var),
//!   [`optional_var`](EnvReader::optional_var) and
//!   [`var_parsed`](EnvReader::var_parsed). It **collects every problem** it
//!   meets instead of stopping at the first, so a misconfigured deployment
//!   learns about all its mistakes in one boot.
//! - [`ConfigError`] — a compact error (a `Vec<String>` of named problems behind
//!   one small struct) that reports all collected problems together.
//!
//! # Ergonomics
//!
//! A config struct implements [`FromEnv`] by reading *every* field up front
//! (so each missing or bad variable is recorded) and only then assembling
//! itself. [`load`] drives the whole thing and turns the accumulated problems
//! into a single [`ConfigError`].
//!
//! ```
//! use seiva_config::{AppEnv, ConfigError, EnvReader, FromEnv, load_from};
//! use std::collections::HashMap;
//!
//! #[derive(Debug)]
//! struct ServerConfig {
//!     env: AppEnv,
//!     bind_port: u16,
//!     database_url: String,
//!     allow_insecure_tls: bool,
//! }
//!
//! impl FromEnv for ServerConfig {
//!     fn from_env(r: &mut EnvReader) -> Option<Self> {
//!         // Read every field FIRST so each problem is collected, not just the
//!         // first one. The `?`s below only decide whether to assemble.
//!         let env = r.app_env();
//!         let bind_port = r.var_parsed::<u16>("BIND_PORT");
//!         let database_url = r.require_var("DATABASE_URL");
//!         let allow_insecure_tls = r.optional_bool("ALLOW_INSECURE_TLS", false);
//!
//!         let config = Self {
//!             env,
//!             bind_port: bind_port?,
//!             database_url: database_url?,
//!             allow_insecure_tls: allow_insecure_tls?,
//!         };
//!
//!         // Production-safety guard: a dev-only shortcut left on in production
//!         // is a startup error, not a silent risk.
//!         r.guard(config.env.reject_dev_shortcut(
//!             "ALLOW_INSECURE_TLS",
//!             config.allow_insecure_tls,
//!         ));
//!
//!         Some(config)
//!     }
//! }
//!
//! // `load_from` takes an explicit map (tests use it; `load` reads the process
//! // environment). A valid development configuration boots:
//! let vars = HashMap::from([
//!     ("APP_ENV".to_string(), "development".to_string()),
//!     ("BIND_PORT".to_string(), "8080".to_string()),
//!     ("DATABASE_URL".to_string(), "postgres://localhost/app".to_string()),
//! ]);
//! let config: ServerConfig = load_from(vars).expect("valid config");
//! assert_eq!(config.bind_port, 8080);
//! assert!(config.env.is_development());
//!
//! // A missing required variable AND a bad port are reported together:
//! let bad = HashMap::from([("BIND_PORT".to_string(), "not-a-port".to_string())]);
//! let err: ConfigError = load_from::<ServerConfig>(bad).unwrap_err();
//! assert_eq!(err.problems().len(), 2);
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// The runtime environment a service runs in, read from the `APP_ENV`
/// environment variable.
///
/// Parsing is case-insensitive and accepts the common short aliases a
/// deployment platform might set: `dev`, `stage`/`stg`, `prod`. An unset
/// `APP_ENV` means [`Development`](AppEnv::Development) — the safe default for
/// a workstation, and the one that *permits* the development shortcuts the
/// [production guard](AppEnv::reject_dev_shortcut) later refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppEnv {
    /// A developer workstation or a local test. The default when `APP_ENV` is
    /// unset. Development shortcuts are permitted here.
    #[default]
    Development,
    /// A pre-production environment that mirrors production. Shortcuts are
    /// permitted (staging is not the guarded environment) but should be rare.
    Staging,
    /// Real production traffic. The [guard](AppEnv::reject_dev_shortcut)
    /// refuses development shortcuts here.
    Production,
}

impl AppEnv {
    /// The environment variable this type is read from.
    pub const VAR: &'static str = "APP_ENV";

    /// Reads [`AppEnv`] from the process environment's `APP_ENV`, defaulting to
    /// [`Development`](AppEnv::Development) when unset.
    ///
    /// Returns a [`ConfigError`] naming `APP_ENV` when the value is set but
    /// unrecognised, so a typo (`APP_ENV=prod_uction`) fails at startup rather
    /// than silently downgrading to development.
    pub fn from_process_env() -> Result<Self, ConfigError> {
        match std::env::var(Self::VAR) {
            Ok(raw) => raw
                .parse()
                .map_err(|e: ParseAppEnvError| ConfigError::single(e.into_problem(Self::VAR))),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::single(format!(
                "environment variable `{}` is not valid UTF-8",
                Self::VAR
            ))),
        }
    }

    /// True for [`Development`](AppEnv::Development).
    #[must_use]
    pub fn is_development(self) -> bool {
        matches!(self, Self::Development)
    }

    /// True for [`Staging`](AppEnv::Staging).
    #[must_use]
    pub fn is_staging(self) -> bool {
        matches!(self, Self::Staging)
    }

    /// True for [`Production`](AppEnv::Production).
    #[must_use]
    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    /// The canonical lowercase name of this environment (`"development"`,
    /// `"staging"`, `"production"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }

    /// The production-safety guard: refuses a development shortcut when running
    /// in production.
    ///
    /// Call this for any flag that is convenient in development but dangerous
    /// in production — allowing insecure TLS, seeding test data, exposing
    /// internal error details to clients, disabling auth. When
    /// `self.is_production() && enabled`, it returns a [`ConfigError`] naming
    /// the offending flag; otherwise it is `Ok`.
    ///
    /// ```
    /// use seiva_config::AppEnv;
    ///
    /// // A dev-only flag left on in production is rejected...
    /// let prod = AppEnv::Production;
    /// assert!(prod.reject_dev_shortcut("ALLOW_INSECURE_TLS", true).is_err());
    /// assert!(prod.reject_dev_shortcut("ALLOW_INSECURE_TLS", false).is_ok());
    ///
    /// // ...but the same flag is fine in development.
    /// let dev = AppEnv::Development;
    /// assert!(dev.reject_dev_shortcut("ALLOW_INSECURE_TLS", true).is_ok());
    /// ```
    pub fn reject_dev_shortcut(self, name: &str, enabled: bool) -> Result<(), ConfigError> {
        if self.is_production() && enabled {
            Err(ConfigError::single(format!(
                "development-only shortcut `{name}` must not be enabled in production \
                 (APP_ENV=production); unset it or set it to a non-truthy value"
            )))
        } else {
            Ok(())
        }
    }
}

impl fmt::Display for AppEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when `APP_ENV` holds an unrecognised value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseAppEnvError {
    raw: String,
}

impl ParseAppEnvError {
    fn into_problem(self, var: &str) -> String {
        format!(
            "environment variable `{}` has unrecognised value `{}` \
             (expected one of: development, staging, production)",
            var, self.raw
        )
    }
}

impl fmt::Display for ParseAppEnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unrecognised APP_ENV value `{}` (expected development, staging or production)",
            self.raw
        )
    }
}

impl std::error::Error for ParseAppEnvError {}

impl FromStr for AppEnv {
    type Err = ParseAppEnvError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "development" | "dev" | "develop" | "local" => Ok(Self::Development),
            "staging" | "stage" | "stg" => Ok(Self::Staging),
            "production" | "prod" | "prd" | "live" => Ok(Self::Production),
            _ => Err(ParseAppEnvError { raw: s.to_string() }),
        }
    }
}

/// A compact startup-configuration error.
///
/// It holds a `Vec<String>` of human-readable problems, each naming the
/// variable at fault, so the whole struct is pointer-sized and cheap to return
/// by value (it does not trip clippy's `result_large_err`). Multiple problems
/// discovered during a single [`load`] are reported together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    problems: Vec<String>,
}

impl ConfigError {
    /// Builds an error from a single problem.
    #[must_use]
    pub fn single(problem: impl Into<String>) -> Self {
        Self {
            problems: vec![problem.into()],
        }
    }

    /// Builds an error from a collection of problems.
    ///
    /// Returns `None` when `problems` is empty — there is no error to report.
    #[must_use]
    pub fn from_problems(problems: Vec<String>) -> Option<Self> {
        if problems.is_empty() {
            None
        } else {
            Some(Self { problems })
        }
    }

    /// The collected problems, each a message naming the offending variable.
    #[must_use]
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.problems.len() == 1 {
            write!(f, "configuration error: {}", self.problems[0])
        } else {
            write!(
                f,
                "configuration invalid ({} problems):",
                self.problems.len()
            )?;
            for problem in &self.problems {
                write!(f, "\n  - {problem}")?;
            }
            Ok(())
        }
    }
}

impl std::error::Error for ConfigError {}

/// Where an [`EnvReader`] reads variables from.
enum EnvSource {
    /// The live process environment (`std::env::var`). The production path.
    Process,
    /// An explicit map. Used by tests so reads never touch — and never race on
    /// — the process-global environment.
    Explicit(HashMap<String, String>),
}

/// A typed reader over a set of environment variables that **accumulates
/// problems** rather than stopping at the first.
///
/// Construct one with [`EnvReader::from_process`] (the real 12-factor path) or
/// [`EnvReader::from_map`] (deterministic, race-free tests). Read each field
/// with [`require_var`](Self::require_var), [`optional_var`](Self::optional_var),
/// [`var_parsed`](Self::var_parsed) or [`optional_bool`](Self::optional_bool);
/// a missing required variable or an unparseable value records a problem and
/// yields `None`, but reading *continues*. Feed cross-field checks (such as the
/// [production guard](AppEnv::reject_dev_shortcut)) back in with
/// [`guard`](Self::guard). Finish with [`into_result`](Self::into_result), or
/// let [`load`]/[`load_from`] do it for you.
pub struct EnvReader {
    source: EnvSource,
    problems: Vec<String>,
}

impl EnvReader {
    /// A reader over the live process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            source: EnvSource::Process,
            problems: Vec::new(),
        }
    }

    /// A reader over an explicit variable map. Nothing it does touches the
    /// process environment, so tests using it are immune to env races.
    #[must_use]
    pub fn from_map(vars: HashMap<String, String>) -> Self {
        Self {
            source: EnvSource::Explicit(vars),
            problems: Vec::new(),
        }
    }

    fn lookup(&self, name: &str) -> Option<String> {
        match &self.source {
            EnvSource::Process => std::env::var(name).ok(),
            EnvSource::Explicit(map) => map.get(name).cloned(),
        }
    }

    /// Reads a required string variable. Records a "missing" problem and returns
    /// `None` when it is absent (or empty).
    pub fn require_var(&mut self, name: &str) -> Option<String> {
        match self.lookup(name) {
            Some(value) if !value.is_empty() => Some(value),
            _ => {
                self.problems
                    .push(format!("required environment variable `{name}` is not set"));
                None
            }
        }
    }

    /// Reads an optional string variable. Never records a problem; returns
    /// `None` when absent or empty.
    #[must_use]
    pub fn optional_var(&self, name: &str) -> Option<String> {
        self.lookup(name).filter(|v| !v.is_empty())
    }

    /// Reads a required variable and parses it into `T`. Records a problem (and
    /// returns `None`) when the variable is missing or fails to parse, naming
    /// the variable and the parse error either way.
    pub fn var_parsed<T>(&mut self, name: &str) -> Option<T>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let raw = self.require_var(name)?;
        match raw.parse::<T>() {
            Ok(value) => Some(value),
            Err(e) => {
                self.problems.push(format!(
                    "environment variable `{name}` has invalid value `{raw}`: {e}"
                ));
                None
            }
        }
    }

    /// Reads an optional variable, parsing it into `T` when present and falling
    /// back to `default` when absent. A present-but-unparseable value is a
    /// problem (and yields `None`): a typo must not silently become the default.
    pub fn optional_parsed<T>(&mut self, name: &str, default: T) -> Option<T>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        match self.optional_var(name) {
            None => Some(default),
            Some(raw) => match raw.parse::<T>() {
                Ok(value) => Some(value),
                Err(e) => {
                    self.problems.push(format!(
                        "environment variable `{name}` has invalid value `{raw}`: {e}"
                    ));
                    None
                }
            },
        }
    }

    /// Reads an optional boolean flag with common truthy/falsy spellings
    /// (`true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`), falling back to
    /// `default` when absent. An unrecognised value is a problem.
    pub fn optional_bool(&mut self, name: &str, default: bool) -> Option<bool> {
        match self.optional_var(name) {
            None => Some(default),
            Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "y" | "on" => Some(true),
                "false" | "0" | "no" | "n" | "off" => Some(false),
                _ => {
                    self.problems.push(format!(
                        "environment variable `{name}` has invalid boolean value `{raw}` \
                         (expected true/false, 1/0, yes/no, on/off)"
                    ));
                    None
                }
            },
        }
    }

    /// Reads [`AppEnv`] from this reader's source, recording a problem for an
    /// unrecognised value and defaulting to [`Development`](AppEnv::Development)
    /// when unset.
    pub fn app_env(&mut self) -> AppEnv {
        match self.optional_var(AppEnv::VAR) {
            None => AppEnv::default(),
            Some(raw) => match raw.parse::<AppEnv>() {
                Ok(env) => env,
                Err(e) => {
                    self.problems.push(e.into_problem(AppEnv::VAR));
                    AppEnv::default()
                }
            },
        }
    }

    /// Folds the result of a cross-field check (typically
    /// [`AppEnv::reject_dev_shortcut`]) into the accumulated problems, so it is
    /// reported alongside any missing or invalid variables.
    pub fn guard(&mut self, result: Result<(), ConfigError>) {
        if let Err(e) = result {
            self.problems.extend(e.problems);
        }
    }

    /// True once at least one problem has been recorded.
    #[must_use]
    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }

    /// Consumes the reader, turning accumulated problems into a single
    /// [`ConfigError`].
    ///
    /// `value` is the assembled configuration (`None` when a required field was
    /// missing). When there are no problems the value is returned; otherwise the
    /// collected problems are. A `None` value with no recorded problems is
    /// itself reported as a problem, since a correct [`FromEnv`] impl only
    /// returns `None` after recording why.
    pub fn into_result<T>(self, value: Option<T>) -> Result<T, ConfigError> {
        match ConfigError::from_problems(self.problems) {
            Some(err) => Err(err),
            None => value.ok_or_else(|| {
                ConfigError::single(
                    "configuration could not be assembled but no problem was recorded \
                     (bug in a FromEnv implementation)",
                )
            }),
        }
    }
}

/// A configuration type that can be built from environment variables.
///
/// Implementations read **every** field up front — each read records any
/// problem into the [`EnvReader`] — and only then assemble `Self`, so a single
/// [`load`] reports all problems at once instead of stopping at the first. See
/// the [crate-level example](crate#ergonomics) for the shape.
pub trait FromEnv: Sized {
    /// Reads and validates this type from the given reader, returning `None`
    /// (after recording problems) when it cannot be assembled.
    fn from_env(reader: &mut EnvReader) -> Option<Self>;
}

/// Loads a [`FromEnv`] configuration from the live process environment,
/// validating it at startup.
///
/// This is the entrypoint a `main` calls: on success it has a fully typed,
/// fully validated configuration; on failure it has a [`ConfigError`] listing
/// every problem, suitable for logging and a non-zero exit.
///
/// # Errors
///
/// Returns a [`ConfigError`] when any required variable is missing, any value
/// fails to parse, or any [guard](AppEnv::reject_dev_shortcut) fails.
pub fn load<T: FromEnv>() -> Result<T, ConfigError> {
    let mut reader = EnvReader::from_process();
    let value = T::from_env(&mut reader);
    reader.into_result(value)
}

/// Loads a [`FromEnv`] configuration from an explicit variable map.
///
/// Identical to [`load`] but reads from `vars` instead of the process
/// environment, so tests are deterministic and never race on the process-global
/// environment.
///
/// # Errors
///
/// Returns a [`ConfigError`] on the same conditions as [`load`].
pub fn load_from<T: FromEnv>(vars: HashMap<String, String>) -> Result<T, ConfigError> {
    let mut reader = EnvReader::from_map(vars);
    let value = T::from_env(&mut reader);
    reader.into_result(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // A single serializing guard for the FEW tests that must touch the
    // process-global `APP_ENV`. Map-based tests below need no lock at all.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // --- AppEnv parsing -----------------------------------------------------

    #[test]
    fn app_env_parses_canonical_and_aliases() {
        assert_eq!("development".parse(), Ok(AppEnv::Development));
        assert_eq!("DEV".parse(), Ok(AppEnv::Development));
        assert_eq!("  Staging ".parse(), Ok(AppEnv::Staging));
        assert_eq!("prod".parse(), Ok(AppEnv::Production));
        assert_eq!("PRODUCTION".parse(), Ok(AppEnv::Production));
    }

    #[test]
    fn app_env_default_is_development() {
        assert_eq!(AppEnv::default(), AppEnv::Development);
        assert!(AppEnv::default().is_development());
    }

    #[test]
    fn app_env_rejects_unknown_value() {
        let err = "prod_uction".parse::<AppEnv>().unwrap_err();
        assert!(err.to_string().contains("prod_uction"));
    }

    #[test]
    fn app_env_from_process_env_reads_the_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY of correctness, not memory: serialized by ENV_LOCK so no other
        // test reads/writes APP_ENV concurrently.
        std::env::set_var(AppEnv::VAR, "production");
        assert_eq!(AppEnv::from_process_env().unwrap(), AppEnv::Production);
        std::env::remove_var(AppEnv::VAR);
        assert_eq!(AppEnv::from_process_env().unwrap(), AppEnv::Development);
    }

    #[test]
    fn app_env_from_process_env_rejects_bad_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(AppEnv::VAR, "banana");
        let err = AppEnv::from_process_env().unwrap_err();
        std::env::remove_var(AppEnv::VAR);
        assert_eq!(err.problems().len(), 1);
        assert!(err.problems()[0].contains("APP_ENV"));
        assert!(err.problems()[0].contains("banana"));
    }

    // --- Example config, loaded from a map (no env races) -------------------

    #[derive(Debug)]
    struct ServerConfig {
        env: AppEnv,
        bind_port: u16,
        database_url: String,
        allow_insecure_tls: bool,
    }

    impl FromEnv for ServerConfig {
        fn from_env(r: &mut EnvReader) -> Option<Self> {
            let env = r.app_env();
            let bind_port = r.var_parsed::<u16>("BIND_PORT");
            let database_url = r.require_var("DATABASE_URL");
            let allow_insecure_tls = r.optional_bool("ALLOW_INSECURE_TLS", false);

            let config = Self {
                env,
                bind_port: bind_port?,
                database_url: database_url?,
                allow_insecure_tls: allow_insecure_tls?,
            };
            r.guard(
                config
                    .env
                    .reject_dev_shortcut("ALLOW_INSECURE_TLS", config.allow_insecure_tls),
            );
            Some(config)
        }
    }

    #[test]
    fn loads_valid_config_from_map() {
        let cfg: ServerConfig = load_from(map(&[
            ("APP_ENV", "staging"),
            ("BIND_PORT", "8080"),
            ("DATABASE_URL", "postgres://localhost/app"),
        ]))
        .expect("valid config");
        assert_eq!(cfg.env, AppEnv::Staging);
        assert_eq!(cfg.bind_port, 8080);
        assert_eq!(cfg.database_url, "postgres://localhost/app");
        assert!(!cfg.allow_insecure_tls);
    }

    #[test]
    fn missing_required_var_is_a_startup_error_naming_the_var() {
        let err = load_from::<ServerConfig>(map(&[("BIND_PORT", "8080")])).unwrap_err();
        assert_eq!(err.problems().len(), 1);
        assert!(err.problems()[0].contains("DATABASE_URL"));
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn multiple_problems_are_reported_together() {
        // DATABASE_URL missing AND BIND_PORT unparseable: both must surface.
        let err = load_from::<ServerConfig>(map(&[("BIND_PORT", "not-a-number")])).unwrap_err();
        assert_eq!(err.problems().len(), 2);
        let joined = err.problems().join("\n");
        assert!(joined.contains("BIND_PORT"));
        assert!(joined.contains("DATABASE_URL"));
    }

    #[test]
    fn production_guard_rejects_dev_shortcut() {
        let err = load_from::<ServerConfig>(map(&[
            ("APP_ENV", "production"),
            ("BIND_PORT", "443"),
            ("DATABASE_URL", "postgres://db/app"),
            ("ALLOW_INSECURE_TLS", "true"),
        ]))
        .unwrap_err();
        assert_eq!(err.problems().len(), 1);
        assert!(err.problems()[0].contains("ALLOW_INSECURE_TLS"));
        assert!(err.problems()[0].contains("production"));
    }

    #[test]
    fn development_allows_the_same_dev_shortcut() {
        let cfg: ServerConfig = load_from(map(&[
            ("APP_ENV", "development"),
            ("BIND_PORT", "8080"),
            ("DATABASE_URL", "postgres://localhost/app"),
            ("ALLOW_INSECURE_TLS", "true"),
        ]))
        .expect("dev permits the shortcut");
        assert!(cfg.allow_insecure_tls);
        assert!(cfg.env.is_development());
    }

    #[test]
    fn invalid_bool_is_reported() {
        let err = load_from::<ServerConfig>(map(&[
            ("BIND_PORT", "8080"),
            ("DATABASE_URL", "postgres://localhost/app"),
            ("ALLOW_INSECURE_TLS", "maybe"),
        ]))
        .unwrap_err();
        assert!(err.problems()[0].contains("ALLOW_INSECURE_TLS"));
    }

    #[test]
    fn optional_parsed_falls_back_and_catches_typos() {
        let mut r = EnvReader::from_map(map(&[("A", "7")]));
        assert_eq!(r.optional_parsed::<u32>("A", 1), Some(7));
        assert_eq!(r.optional_parsed::<u32>("MISSING", 42), Some(42));
        assert_eq!(r.optional_parsed::<u32>("MISSING", 42), Some(42));
        assert!(!r.has_problems());
        let mut r2 = EnvReader::from_map(map(&[("A", "x")]));
        assert_eq!(r2.optional_parsed::<u32>("A", 1), None);
        assert!(r2.has_problems());
    }
}
