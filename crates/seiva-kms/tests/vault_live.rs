//! Against a real Vault (`vault server -dev`): transit wrapping, a rotation of
//! the transit key and the re-wrap of the ring, a data-key rotation with
//! re-sealing and retirement, AppRole login (and a new login when the token
//! stops working), and refusals.
//!
//! Runs only when `SEIVA_KMS_TEST_VAULT_ADDR` and `SEIVA_KMS_TEST_VAULT_TOKEN`
//! (a root token) are set; otherwise each test says so and passes. When they are
//! set but Vault does not answer, the tests skip too, unless
//! `SEIVA_KMS_TEST_VAULT_REQUIRED=1` (CI), which makes that a failure.
//!
//! ```sh
//! docker run -d --rm --name seiva-vault --cap-add IPC_LOCK -p 18200:8200 \
//!   -e VAULT_DEV_ROOT_TOKEN_ID=seiva-test hashicorp/vault:1.18
//! SEIVA_KMS_TEST_VAULT_ADDR=http://127.0.0.1:18200 SEIVA_KMS_TEST_VAULT_TOKEN=seiva-test \
//!   cargo test -p seiva-kms --features vault --test vault_live
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use seiva_kms::{
    wrap_version, Aad, KeyRing, KeyWrapper, KmsError, MemoryRingStore, Reseal, RingKeeper,
    RingStore, VaultAuth, VaultConfig, VaultTransit,
};
use serde_json::{json, Value};

struct Vault {
    addr: String,
    token: String,
    http: reqwest::Client,
}

impl Vault {
    /// The test Vault, or `None` (with the reason printed) to skip.
    async fn get(test: &str) -> Option<Vault> {
        let (Ok(addr), Ok(token)) = (
            std::env::var("SEIVA_KMS_TEST_VAULT_ADDR"),
            std::env::var("SEIVA_KMS_TEST_VAULT_TOKEN"),
        ) else {
            eprintln!(
                "SKIP {test}: set SEIVA_KMS_TEST_VAULT_ADDR and SEIVA_KMS_TEST_VAULT_TOKEN to run against a real Vault"
            );
            return None;
        };
        let v = Vault {
            addr: addr.trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::new(),
        };
        let up = v
            .http
            .get(format!("{}/v1/sys/health", v.addr))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if !up {
            if std::env::var("SEIVA_KMS_TEST_VAULT_REQUIRED").as_deref() == Ok("1") {
                panic!("{test}: Vault at {} is not available", v.addr);
            }
            eprintln!("SKIP {test}: Vault at {} is not available", v.addr);
            return None;
        }
        // Idempotent set-up: "path is already in use" is fine.
        v.call("POST", "sys/mounts/transit", json!({ "type": "transit" }))
            .await;
        Some(v)
    }

    /// An admin call with the root token: (status, body).
    async fn call(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let r = self
            .http
            .request(m, format!("{}/v1/{path}", self.addr))
            .header("X-Vault-Token", &self.token)
            .json(&body)
            .send()
            .await
            .expect("Vault answers");
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (s, v) = self.call(method, path, body).await;
        assert!((200..300).contains(&s), "{method} {path}: {s} {v}");
        v
    }

    /// A fresh transit key, unique to this test run.
    async fn transit_key(&self, test: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("seiva-{test}-{}-{nanos}", std::process::id());
        self.ok("POST", &format!("transit/keys/{name}"), json!({}))
            .await;
        name
    }

    fn wrapper(&self, key: &str, auth: VaultAuth) -> VaultTransit {
        VaultTransit::new(VaultConfig::new(&self.addr, key, auth)).unwrap()
    }

    fn root(&self, key: &str) -> VaultTransit {
        self.wrapper(key, VaultAuth::Token(self.token.clone()))
    }
}

fn aad(record: &str) -> Aad {
    Aad::new("acme", "identity.totp", record).unwrap()
}

#[tokio::test]
async fn transit_wraps_unwraps_and_refuses() {
    let Some(v) = Vault::get("transit_wraps_unwraps_and_refuses").await else {
        return;
    };
    let key = v.transit_key("wrap").await;
    let t = v.root(&key);
    t.wait_ready(Duration::from_secs(10)).await.unwrap();
    assert_eq!(t.provider(), "vault");
    assert_eq!(t.kek_id(), key);

    let g = t.generate().await.unwrap();
    assert_eq!(wrap_version(&g.wrapped), Some(1), "{}", g.wrapped);
    assert_eq!(t.unwrap(&key, &g.wrapped).await.unwrap(), g.plain);
    assert_eq!(t.rewrap(&key, &g.wrapped, &g.plain).await.unwrap(), None);
    let g2 = t.generate().await.unwrap();
    assert_ne!(g2.plain, g.plain);

    // A bad token: refused, not "unavailable" (start-up must not keep retrying).
    let bad = v.wrapper(&key, VaultAuth::Token("hvs.not-a-token".into()));
    let e = bad.generate().await.unwrap_err();
    assert!(matches!(e, KmsError::Refused(_)), "{e}");
    assert!(matches!(bad.health().await, Err(KmsError::Refused(_))));
    let e = bad.wait_ready(Duration::from_secs(10)).await.unwrap_err();
    assert!(!e.is_transient());

    // Another transit key does not open this one's data key.
    let other = v.transit_key("other").await;
    assert!(t.unwrap(&other, &g.wrapped).await.is_err());
    // A key that does not exist is refused.
    assert!(matches!(
        v.root("seiva-missing-key").generate().await,
        Err(KmsError::Refused(_))
    ));
}

#[tokio::test]
async fn transit_key_rotation_rewraps_the_ring() {
    let Some(v) = Vault::get("transit_key_rotation_rewraps_the_ring").await else {
        return;
    };
    let key = v.transit_key("rot").await;
    let t: Arc<VaultTransit> = Arc::new(v.root(&key));
    let store = Arc::new(MemoryRingStore::new());
    let keeper = RingKeeper::new(t.clone(), store.clone());

    let ring = keeper.boot("org", Duration::from_secs(10)).await.unwrap();
    let sealed = ring.seal(&aad("u1"), b"JBSWY3DPEHPK3PXP").unwrap();
    let before = store.load("org").await.unwrap().unwrap();
    assert!(before.ring.current.wrapped.starts_with("vault:v1:"));
    assert_eq!(before.ring.current.provider, "vault");
    assert_eq!(before.ring.current.kek, key);

    // The KEK rotates in Vault.
    v.ok("POST", &format!("transit/keys/{key}/rotate"), json!({}))
        .await;
    let plain = t.unwrap(&key, &before.ring.current.wrapped).await.unwrap();
    let re = t
        .rewrap(&key, &before.ring.current.wrapped, &plain)
        .await
        .unwrap()
        .expect("re-wrapped with v2");
    assert!(re.starts_with("vault:v2:"), "{re}");
    assert_eq!(t.rewrap(&key, &re, &plain).await.unwrap(), None);

    // The next load re-wraps the stored ring with v2; the data key is the same.
    let ring2 = keeper.load("org").await.unwrap();
    assert_eq!(ring2.current_id(), ring.current_id());
    let after = store.load("org").await.unwrap().unwrap();
    assert!(after.ring.current.wrapped.starts_with("vault:v2:"));
    assert!(after.version > before.version);
    assert_eq!(
        &ring2.open(&aad("u1"), &sealed).unwrap()[..],
        b"JBSWY3DPEHPK3PXP"
    );

    // v1 can now be retired in Vault: the stored ring no longer needs it.
    v.ok(
        "POST",
        &format!("transit/keys/{key}/config"),
        json!({ "min_decryption_version": 2 }),
    )
    .await;
    assert!(t.unwrap(&key, &before.ring.current.wrapped).await.is_err());
    let ring3 = keeper.load("org").await.unwrap();
    assert_eq!(
        &ring3.open(&aad("u1"), &sealed).unwrap()[..],
        b"JBSWY3DPEHPK3PXP"
    );
}

#[derive(Default)]
struct Secrets(Mutex<Vec<(String, String)>>);

#[async_trait]
impl Reseal for Secrets {
    async fn reseal(&self, _scope: &str, ring: &KeyRing) -> Result<usize, KmsError> {
        let mut n = 0;
        for (record, blob) in self.0.lock().unwrap().iter_mut() {
            if ring.needs_reseal(blob) {
                let plain = ring.open(&aad(record), blob)?;
                *blob = ring.seal(&aad(record), &plain)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

#[tokio::test]
async fn data_key_rotation_reseals_and_retires() {
    let Some(v) = Vault::get("data_key_rotation_reseals_and_retires").await else {
        return;
    };
    let key = v.transit_key("dek").await;
    let keeper = RingKeeper::new(Arc::new(v.root(&key)), Arc::new(MemoryRingStore::new()));
    let r1 = keeper.load("org").await.unwrap();
    let secrets = Secrets::default();
    for (rec, p) in [("u1", "one"), ("u2", "two")] {
        let s = r1.seal(&aad(rec), p.as_bytes()).unwrap();
        secrets.0.lock().unwrap().push((rec.into(), s));
    }

    let rot = keeper.rotate("org", &[&secrets]).await.unwrap();
    assert!(rot.reseal_errors.is_empty());
    assert_eq!(rot.resealed, 2);
    assert_eq!(rot.ring.previous_ids(), [r1.current_id()]);
    assert!(rot.ring.current_id().starts_with("vault-"));

    let done = keeper
        .retire("org", Duration::ZERO, &[&secrets])
        .await
        .unwrap()
        .expect("due");
    assert_eq!(done.resealed, 0, "already re-sealed at the rotation");
    let r3 = keeper.load("org").await.unwrap();
    assert!(r3.previous_ids().is_empty());
    for (rec, blob) in secrets.0.lock().unwrap().iter() {
        let want = if rec == "u1" { "one" } else { "two" };
        assert_eq!(&r3.open(&aad(rec), blob).unwrap()[..], want.as_bytes());
    }
}

#[tokio::test]
async fn approle_login_and_a_new_login_when_the_token_is_used_up() {
    let Some(v) = Vault::get("approle_login").await else {
        return;
    };
    let key = v.transit_key("approle").await;
    v.call("POST", "sys/auth/approle", json!({ "type": "approle" }))
        .await;
    let policy = format!(
        r#"path "transit/datakey/plaintext/{key}" {{ capabilities = ["update"] }}
path "transit/decrypt/{key}" {{ capabilities = ["update"] }}
path "transit/rewrap/{key}" {{ capabilities = ["update"] }}"#
    );
    v.ok(
        "PUT",
        &format!("sys/policies/acl/{key}"),
        json!({ "policy": policy }),
    )
    .await;
    // Each login token works for ONE call: every second call needs a new login.
    v.ok(
        "POST",
        &format!("auth/approle/role/{key}"),
        json!({ "token_policies": [key], "token_num_uses": 1, "token_ttl": "10m" }),
    )
    .await;
    let role_id = v
        .ok(
            "GET",
            &format!("auth/approle/role/{key}/role-id"),
            json!({}),
        )
        .await["data"]["role_id"]
        .as_str()
        .unwrap()
        .to_string();
    let dir = std::env::temp_dir().join(format!("seiva-kms-approle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_file = dir.join("secret-id");
    let mint_secret = || async {
        v.ok(
            "POST",
            &format!("auth/approle/role/{key}/secret-id"),
            json!({}),
        )
        .await["data"]["secret_id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    std::fs::write(&secret_file, mint_secret().await).unwrap();

    let t = v.wrapper(&key, VaultAuth::approle(&role_id, &secret_file));
    t.wait_ready(Duration::from_secs(10)).await.unwrap();
    let g = t.generate().await.unwrap();
    assert_eq!(t.unwrap(&key, &g.wrapped).await.unwrap(), g.plain);
    assert_eq!(t.unwrap(&key, &g.wrapped).await.unwrap(), g.plain);

    // The policy is scoped to this key: another transit key is refused.
    let other = v.transit_key("approle-other").await;
    let o = v.root(&other).generate().await.unwrap();
    assert!(matches!(
        t.unwrap(&other, &o.wrapped).await,
        Err(KmsError::Refused(_))
    ));

    // A wrong secret id: the login is refused.
    std::fs::write(&secret_file, "00000000-0000-0000-0000-000000000000").unwrap();
    let fresh = v.wrapper(&key, VaultAuth::approle(&role_id, &secret_file));
    assert!(matches!(fresh.generate().await, Err(KmsError::Refused(_))));
    std::fs::remove_dir_all(dir).unwrap();
}
