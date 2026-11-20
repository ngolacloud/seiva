//! The built-in demonstration facade and user-create, behind the `demo` feature.
//!
//! A standalone, publishable `seiva` binary cannot link the `examples/` crates
//! (`notes-*`, `accounts-*`): they are `publish = false`, so a `path` dependency
//! on them — even an optional one — blocks `cargo publish`. So the handful of
//! commands that need something to *show* outside a project carry their own tiny,
//! self-contained demonstration here, built only from crates that are published
//! to crates.io (`seiva`, `seiva-openapi`, `seiva-auth`).
//!
//! This is a demonstration, not the canonical module shape — `examples/notes`
//! and `examples/accounts` remain the five-layer references a real project copies.
//! The point of keeping it compilable is that `seiva openapi --features demo` and
//! `seiva user create --features demo` exercise the real framework primitives
//! (the `#[facade]` descriptor, the OpenAPI renderer, Argon2id hashing).

#![allow(async_fn_in_trait)]
// The facade trait and its DTOs exist to be read by the `#[facade]`-generated
// descriptor (what `seiva openapi` renders), not to be called from this binary —
// so the dead-code analysis cannot see them used. In the `examples/notes` crate
// they are public exports and so never flagged; inlined into this bin crate they
// would be. The contract is the point; silence the false positive here.
#![allow(dead_code)]

use seiva::{facade, WireSchema};

/// Command DTO: the input to the demo facade's `create`.
#[derive(Debug, Clone, WireSchema)]
pub struct CreateNote {
    /// The note's title.
    pub title: String,
}

/// Response DTO: what `create` returns. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct NoteDto {
    /// The note's stable identifier.
    pub id: u128,
    /// The note's title.
    pub title: String,
}

/// The built-in demonstration contract, mirroring `examples/notes`'s `Notes`
/// facade. The `#[facade]` macro keeps the trait as the single source of truth
/// and generates the `NotesFacade` descriptor the CLI renders.
#[facade(version = "v1")]
pub trait Notes {
    /// Creates a note.
    #[requires("notes.create")]
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
}

/// The maximum length, in bytes, of a username — the same bound the `accounts`
/// domain enforces (`examples/accounts/domain`).
const MAX_USERNAME_LEN: usize = 64;

/// Creates a user the way the `accounts` module's use case would, but inline and
/// without persistence: it enforces the same domain rules (a non-empty, bounded
/// username) and hashes the password with the **same** Argon2id primitive a
/// production module uses (`seiva-auth`), then returns the created user as JSON.
///
/// The returned value carries only `{ "id", "username" }` — never the password
/// or its hash, matching the `accounts` facade's contract. Persistence is nil:
/// the created user lives only for the command's duration (a real repository
/// arrives through the framework's `dorm` feature in a generated project).
///
/// # Errors
///
/// Returns the violated domain rule (empty or over-long username) or a hashing
/// failure, as a `String`.
pub fn create_user(username: &str, password: &str) -> Result<serde_json::Value, String> {
    // Domain rules, enforced here exactly as `User::new` enforces them.
    if username.trim().is_empty() {
        return Err("username must not be empty".to_string());
    }
    if username.len() > MAX_USERNAME_LEN {
        return Err(format!(
            "username must be at most {MAX_USERNAME_LEN} bytes (got {})",
            username.len()
        ));
    }

    // The real Argon2id hashing a module uses — a PHC string, never the plaintext.
    let password_hash = seiva_auth::hash_password(password).map_err(|err| err.to_string())?;
    // Prove the credential was hashed without ever revealing it: report only the
    // algorithm id embedded in the PHC string (`$argon2id$...`).
    let algorithm = password_hash.split('$').nth(1).unwrap_or("argon2id");
    eprintln!("seiva: credential hashed with {algorithm}; not stored (in-memory demo)");
    eprintln!("seiva: event emitted — accounts.user_created");

    // A time-seeded id (nanoseconds, as a string): unique enough for a single-shot
    // create. The DORM gateway supplies a real id generator for the persistent path.
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();

    Ok(serde_json::json!({
        // `id` is a u128; render it as a string so it survives any JSON consumer.
        "id": id.to_string(),
        "username": username,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_user_hides_the_secret_and_carries_an_id() {
        let user = create_user("alice", "hunter2").expect("create must succeed");
        assert_eq!(user["username"], "alice");
        assert!(user["id"].is_string(), "the created user carries an id");
        // The DTO carries no secret: neither the password nor its hash is present.
        assert!(user.get("password").is_none());
        assert!(user.get("password_hash").is_none());
        let rendered = user.to_string();
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("$argon2id$"),
            "no secret must appear in the output: {rendered}"
        );
    }

    #[test]
    fn create_user_rejects_a_blank_username() {
        let err = create_user("   ", "hunter2").expect_err("blank username is rejected");
        assert!(!err.is_empty(), "the demo returns an error message");
    }

    #[test]
    fn create_user_rejects_an_overlong_username() {
        let long = "a".repeat(MAX_USERNAME_LEN + 1);
        let err = create_user(&long, "hunter2").expect_err("over-long username is rejected");
        assert!(err.contains("at most"), "error names the bound: {err}");
    }
}
