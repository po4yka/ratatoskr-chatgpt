//! Contract tests for typed configuration loading.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use ratatoskr_chatgpt_archive::config::Config;

fn entries(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// The one required key plus nothing else yields every documented default.
#[test]
fn minimal_valid_environment_parses() {
    let loaded = Config::from_environment(entries(&[(
        "RATATOSKR__STORAGE__BLOB_ROOT",
        "/tmp/chatgpt-blobs",
    )]));

    let config = loaded.expect("a single required key must be enough");
    assert_eq!(
        config.admin.listen_address,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9084)
    );
    assert_eq!(config.telemetry.log_filter, "info");
    assert_eq!(config.limits.database_connections, 8);
    assert_eq!(config.limits.database_acquire_timeout_ms, 5_000);
    assert_eq!(config.limits.shutdown_timeout_ms, 10_000);
    assert_eq!(
        config.storage.blob_root,
        Some(PathBuf::from("/tmp/chatgpt-blobs"))
    );
    assert!(config.storage.database_url.is_none());
}

/// A key under the prefix that names nothing known is refused by name, and an
/// unprefixed variable is nobody's business.
#[test]
fn unknown_prefixed_key_fails_with_violation() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__SERVICE__NMAE", "typo"),
        ("UNRELATED", "ignored"),
    ]));

    let error = loaded.expect_err("an unknown prefixed key must refuse startup");
    let typo = error
        .violations
        .iter()
        .find(|violation| violation.key == "RATATOSKR__SERVICE__NMAE")
        .expect("the diagnostic must name the offending key");
    assert_eq!(typo.rule, "is not recognized");
}

/// Validation examines every entry before reporting, never stops at the first.
#[test]
fn two_bad_values_report_together() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__LIMITS__DATABASE_CONNECTIONS", "0"),
        ("RATATOSKR__ADMIN__LISTEN_ADDRESS", "not-an-address"),
    ]));

    let error = loaded.expect_err("two invalid values must refuse startup");
    assert!(
        error.violations.len() >= 2,
        "both violations belong in one report, found: {error}"
    );
    assert!(
        error
            .violations
            .iter()
            .any(|violation| violation.key == "RATATOSKR__LIMITS__DATABASE_CONNECTIONS")
    );
    assert!(
        error
            .violations
            .iter()
            .any(|violation| violation.key == "RATATOSKR__ADMIN__LISTEN_ADDRESS")
    );
}

/// A credential-bearing URL survives loading without ever rendering its secret.
#[test]
fn secret_is_redacted_in_debug() {
    let loaded = Config::from_environment(entries(&[
        (
            "RATATOSKR__STORAGE__DATABASE_URL",
            "postgres://owner:hunter2@127.0.0.1:5439/chatgpt",
        ),
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
    ]));

    let config = loaded.expect("a well-formed URL must load");
    let debug_output = format!("{config:?}");
    assert!(
        debug_output.contains("[REDACTED]"),
        "the secret field must render as a placeholder"
    );
    assert!(
        !debug_output.contains("hunter2"),
        "the password leaked into Debug output: {debug_output}"
    );

    let serialized =
        serde_json::to_string(&config).expect("configuration must serialize for diagnostics");
    assert!(
        !serialized.contains("hunter2"),
        "the password leaked into serialization: {serialized}"
    );
}
