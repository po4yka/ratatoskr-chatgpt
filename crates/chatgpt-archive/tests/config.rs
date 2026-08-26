//! Contract tests for typed configuration loading.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use ratatoskr_chatgpt_archive::config::Config;
use secrecy::ExposeSecret as _;

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
    // Receipt defaults: no staging location, no tenants, the documented cap.
    assert!(config.storage.receipt_staging_root.is_none());
    assert!(config.receipt.tenant_tokens.is_empty());
    assert_eq!(
        config.limits.max_archive_bytes, 17_179_869_184,
        "the documented default cap is 16 GiB"
    );
}

/// The archive size cap parses from the environment.
#[test]
fn max_archive_bytes_parses_with_documented_default() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES", "1024"),
    ]));

    let config = loaded.expect("a positive cap must load");
    assert_eq!(config.limits.max_archive_bytes, 1024);
}

#[test]
fn extraction_caps_are_accepted_as_positive_configuration() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRIES", "17"),
        ("RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRY_BYTES", "1024"),
        ("RATATOSKR__LIMITS__MAX_ARCHIVE_DECOMPRESSED_BYTES", "4096"),
        ("RATATOSKR__LIMITS__MAX_ARCHIVE_COMPRESSION_RATIO", "12"),
    ]));

    assert!(loaded.is_ok(), "positive extraction caps must be accepted");
}

#[test]
fn non_positive_extraction_caps_are_value_free() {
    for key in [
        "RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRIES",
        "RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRY_BYTES",
        "RATATOSKR__LIMITS__MAX_ARCHIVE_DECOMPRESSED_BYTES",
        "RATATOSKR__LIMITS__MAX_ARCHIVE_COMPRESSION_RATIO",
    ] {
        let loaded = Config::from_environment(entries(&[
            ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
            (key, "0"),
        ]));
        let error = loaded.expect_err("zero extraction cap must fail");
        assert!(
            error
                .violations
                .iter()
                .any(|violation| violation.key == key)
        );
        assert!(!format!("{error}").contains('0'));
    }
}

/// A cap that is zero, negative, or not a number refuses startup, naming the
/// key and the rule but never the supplied bytes.
#[test]
fn non_positive_archive_cap_is_reported_value_free() {
    for bad in ["0", "-5", "many"] {
        let loaded = Config::from_environment(entries(&[
            ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
            ("RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES", bad),
        ]));

        let error = loaded.expect_err("a non-positive cap must refuse startup");
        assert!(
            error.violations.iter().any(|violation| {
                violation.key == "RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES"
                    && violation.rule == "must be a positive integer"
            }),
            "the diagnostic must name the key and rule without echoing the value, got: {error}"
        );
        let rendered = format!("{error}");
        assert!(
            !rendered.contains(bad),
            "the refused value must not be echoed: {rendered}"
        );
    }
}

/// A staging root that is not an absolute directory path is refused by name.
#[test]
fn relative_staging_root_is_refused() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        (
            "RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT",
            "staging/relative",
        ),
    ]));

    let error = loaded.expect_err("a relative staging root must refuse startup");
    let violation = error
        .violations
        .iter()
        .find(|violation| violation.key == "RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT")
        .expect("the diagnostic must name the offending key");
    assert_eq!(violation.rule, "must be an absolute directory path");
}

/// An absolute staging root loads.
#[test]
fn absolute_staging_root_loads() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        (
            "RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT",
            "/var/ratatoskr/staging",
        ),
    ]));

    let config = loaded.expect("an absolute staging root must load");
    assert_eq!(
        config.storage.receipt_staging_root,
        Some(PathBuf::from("/var/ratatoskr/staging"))
    );
}

/// Tenant tokens parse into their token-and-account pairs.
#[test]
fn tenant_tokens_parse_into_secret_pairs() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        (
            "RATATOSKR__RECEIPT__TENANT_TOKENS",
            "tok-alpha=acc-one,tok-beta=acc-two",
        ),
    ]));

    let config = loaded.expect("well-formed tenant tokens must load");
    let pairs: Vec<(&str, &str)> = config
        .receipt
        .tenant_tokens
        .iter()
        .map(|pair| (pair.token.expose_secret(), &*pair.account_external_ref))
        .collect();
    assert_eq!(
        pairs,
        vec![("tok-alpha", "acc-one"), ("tok-beta", "acc-two")]
    );
}

/// Every malformed entry is one violation in one report, naming keys and
/// rules but never echoing entry contents.
#[test]
fn malformed_tenant_token_entries_are_each_reported() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        (
            "RATATOSKR__RECEIPT__TENANT_TOKENS",
            "missing-separator,=empty-token,empty-ref=",
        ),
    ]));

    let error = loaded.expect_err("malformed entries must refuse startup");
    let count = error
        .violations
        .iter()
        .filter(|violation| violation.key == "RATATOSKR__RECEIPT__TENANT_TOKENS")
        .count();
    assert!(
        count >= 3,
        "each malformed entry belongs in the report, found {count}: {error}"
    );
}

/// Token material never reaches diagnostics.
#[test]
fn tenant_tokens_render_redacted_in_debug() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        (
            "RATATOSKR__RECEIPT__TENANT_TOKENS",
            "super-secret-token=quiet-account",
        ),
    ]));

    let config = loaded.expect("a well-formed tenant token must load");
    let debug_output = format!("{config:?}");
    assert!(
        debug_output.contains("[REDACTED]"),
        "tokens must render behind placeholders"
    );
    assert!(
        !debug_output.contains("super-secret-token"),
        "token material leaked into Debug output"
    );
    let serialized =
        serde_json::to_string(&config).expect("configuration must serialize for diagnostics");
    assert!(
        !serialized.contains("super-secret-token"),
        "token material leaked into serialization: {serialized}"
    );
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
