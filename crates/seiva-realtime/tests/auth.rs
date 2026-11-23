//! Authenticate-before-upgrade: the deny-by-default gate and the `Origin`
//! allowlist (CSWSH) that every transport must run before completing an upgrade.

use seiva_identity::Principal;
use seiva_ports::Scope;
use seiva_realtime::{
    authorize_upgrade, Authenticated, OriginAllowlist, RealtimeError, StaticRealtimeAuth,
};

fn dev_auth() -> StaticRealtimeAuth {
    StaticRealtimeAuth::new().with_token(
        "conn-token-abc",
        Authenticated::new(
            Principal::in_tenant("alice", "acme", ["realtime.connect"]),
            Scope::tenant("acme").unwrap(),
        ),
    )
}

#[tokio::test]
async fn valid_token_and_allowed_origin_upgrades() {
    let auth = dev_auth();
    let allow = OriginAllowlist::new().with("https://app.example.com");

    let ok = authorize_upgrade(
        &auth,
        Some("conn-token-abc"),
        Some("https://app.example.com"),
        &allow,
    )
    .await
    .expect("should authorize");
    assert_eq!(ok.principal.subject, "alice");
    assert!(ok.principal.owns("acme"));
    assert_eq!(
        ok.scope.tenant_id().map(|t| t.to_string()).as_deref(),
        Some("acme")
    );
}

#[tokio::test]
async fn missing_empty_and_unknown_tokens_are_denied() {
    let auth = dev_auth();
    let allow = OriginAllowlist::new().with("https://app.example.com");
    let origin = Some("https://app.example.com");

    assert_eq!(
        authorize_upgrade(&auth, None, origin, &allow).await,
        Err(RealtimeError::Unauthenticated),
        "a missing token never passes — deny by default"
    );
    assert_eq!(
        authorize_upgrade(&auth, Some(""), origin, &allow).await,
        Err(RealtimeError::Unauthenticated),
    );
    assert_eq!(
        authorize_upgrade(&auth, Some("not-a-real-token"), origin, &allow).await,
        Err(RealtimeError::Unauthenticated),
    );
}

#[tokio::test]
async fn origin_is_checked_before_the_token() {
    let auth = dev_auth();
    let allow = OriginAllowlist::new().with("https://app.example.com");

    // A cross-site origin is refused even with an otherwise valid token (CSWSH).
    assert_eq!(
        authorize_upgrade(
            &auth,
            Some("conn-token-abc"),
            Some("https://evil.example.net"),
            &allow,
        )
        .await,
        Err(RealtimeError::ForbiddenOrigin),
    );

    // A missing Origin is denied too, unless the deployment opts out explicitly.
    assert_eq!(
        authorize_upgrade(&auth, Some("conn-token-abc"), None, &allow).await,
        Err(RealtimeError::ForbiddenOrigin),
    );
    let any = OriginAllowlist::any();
    assert!(
        authorize_upgrade(&auth, Some("conn-token-abc"), None, &any)
            .await
            .is_ok(),
        "any() is the explicit, visible opt-out of the CSWSH guard"
    );
}

#[tokio::test]
async fn empty_authenticator_denies_everything() {
    let empty = StaticRealtimeAuth::new();
    assert!(empty.is_empty());
    let allow = OriginAllowlist::any();
    assert_eq!(
        authorize_upgrade(&empty, Some("anything"), Some("https://x"), &allow).await,
        Err(RealtimeError::Unauthenticated),
        "zero trust out of the box",
    );
}
