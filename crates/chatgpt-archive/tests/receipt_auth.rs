//! Contract tests for tenant authentication at the receipt surface.

use ratatoskr_chatgpt_archive::receipt::auth::{ConfigTenantAuthenticator, TenantAuthenticator};

fn authenticator() -> ConfigTenantAuthenticator {
    ConfigTenantAuthenticator::new(vec![
        ("tok-alpha".to_owned(), "acc-one".to_owned()),
        ("tok-beta".to_owned(), "acc-two".to_owned()),
    ])
}

/// A configured bearer credential resolves to exactly its tenant account.
#[test]
fn configured_token_resolves_its_tenant_account() {
    let principal = authenticator()
        .authenticate(Some("Bearer tok-alpha"))
        .expect("a configured token must authenticate");
    assert_eq!(principal.account_external_ref, "acc-one");

    let principal = authenticator()
        .authenticate(Some("bearer tok-beta"))
        .expect("the scheme name is case-insensitive");
    assert_eq!(principal.account_external_ref, "acc-two");
}

/// A request carrying no credential is refused.
#[test]
fn missing_credential_is_unauthenticated() {
    let outcome = authenticator().authenticate(None);
    assert!(
        outcome.is_err(),
        "an absent credential must not authenticate"
    );
}

/// Unknown credentials are indistinguishable from missing ones: the refusal
/// carries nothing that would tell an attacker how close it came.
#[test]
fn unknown_token_is_indistinguishable_from_missing() {
    let unknown = authenticator()
        .authenticate(Some("Bearer not-a-token"))
        .expect_err("an unknown token must not authenticate");
    let missing = authenticator()
        .authenticate(None)
        .expect_err("a missing credential must not authenticate");
    // One refusal shape for every failure reason.
    assert_eq!(unknown.to_string(), missing.to_string());
}

/// Malformed authorization values are refused without disclosure.
#[test]
fn malformed_authorization_header_is_unauthenticated() {
    for malformed in [
        "Token tok-alpha",
        "Bearer",
        "Bearer ",
        "Bearer tok-alpha extra",
        "",
    ] {
        let outcome = authenticator().authenticate(Some(malformed));
        assert!(
            outcome.is_err(),
            "malformed credential {malformed:?} must not authenticate"
        );
    }
}
