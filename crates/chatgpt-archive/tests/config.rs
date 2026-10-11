//! Contract tests for typed configuration loading.

#![allow(
    clippy::expect_used,
    reason = "the helper that reads the shipped example fails the test through expect"
)]

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
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9085)
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

/// Enabling operation publication without a service credential must fail
/// before the receipt runtime can claim readiness.
#[test]
fn event_bus_url_requires_nkey_seed_path() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__RECEIPT__EVENT_BUS_URL", "nats://127.0.0.1:4222"),
    ]));

    let error = loaded.expect_err("a report endpoint without credentials must fail closed");
    assert!(error.violations.iter().any(|violation| {
        violation.key == "RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH"
            && violation.rule == "is required when the event bus URL is configured"
    }));
    assert!(!format!("{error}").contains("127.0.0.1:4222"));
}

#[test]
fn platform_receipt_mapping_requires_report_bus_and_nkey() {
    let loaded = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        (
            "RATATOSKR__RECEIPT__PLATFORM_ACCOUNTS",
            "018f0000-0000-7000-8000-000000000001=owner",
        ),
    ]));

    let error =
        loaded.expect_err("Platform receipt must not start without its terminal report bus");
    assert!(
        error
            .violations
            .iter()
            .any(|violation| violation.key == "RATATOSKR__RECEIPT__EVENT_BUS_URL")
    );
    assert!(
        error
            .violations
            .iter()
            .any(|violation| { violation.key == "RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH" })
    );
}

#[test]
fn event_bus_url_and_nkey_seed_path_load_without_rendering_values() {
    let config = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__RECEIPT__EVENT_BUS_URL", "nats://127.0.0.1:4222"),
        (
            "RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH",
            "/run/credentials/chatgpt.nkey",
        ),
    ]))
    .expect("a credentialed NATS publisher must load");

    assert_eq!(
        config.receipt.event_bus_nkey_seed_path,
        Some(PathBuf::from("/run/credentials/chatgpt.nkey"))
    );
    let rendered = format!("{:?}", config.receipt);
    assert!(!rendered.contains("127.0.0.1:4222"));
    assert!(!rendered.contains("chatgpt.nkey"));
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

/// Platform identities are an explicit local routing map, never inferred
/// from a caller's arbitrary UUID at receipt time.
#[test]
fn platform_account_mappings_parse_as_uuid_to_account_pairs() {
    let config = Config::from_environment(entries(&[
        ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs"),
        ("RATATOSKR__RECEIPT__EVENT_BUS_URL", "nats://127.0.0.1:4222"),
        (
            "RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH",
            "/run/credentials/chatgpt.nkey",
        ),
        (
            "RATATOSKR__RECEIPT__PLATFORM_ACCOUNTS",
            "00000000-0000-0000-0000-000000000011=acc-one",
        ),
    ]))
    .expect("one explicit Platform account mapping must load");

    assert_eq!(
        config.receipt.platform_accounts,
        vec![(
            "00000000-0000-0000-0000-000000000011"
                .parse()
                .expect("uuid"),
            "acc-one".to_owned()
        )]
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

/// XR-021 CONTRACTS.md R2-01: the receipt has its own loopback listener, on 8096 by default, and the
/// operator plane is never served on it.
#[test]
fn receipt_listener_defaults_and_validation() {
    let blob = ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs");
    let config = Config::from_environment(entries(&[blob])).expect("defaults must load");
    assert_eq!(
        config.receipt.listen_address,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8096),
        "the receipt default is the port Edge probes"
    );
    assert_eq!(
        config.admin.listen_address,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9085),
        "the operator default is unchanged"
    );

    let config = Config::from_environment(entries(&[
        blob,
        ("RATATOSKR__RECEIPT__LISTEN_ADDRESS", "127.0.0.1:18096"),
    ]))
    .expect("a loopback receipt address must load");
    assert_eq!(
        config.receipt.listen_address,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 18096)
    );

    for refused in [
        "0.0.0.0:8096",
        "192.0.2.10:8096",
        "127.0.0.1:0",
        "not-an-address",
    ] {
        let error = Config::from_environment(entries(&[
            blob,
            ("RATATOSKR__RECEIPT__LISTEN_ADDRESS", refused),
        ]))
        .expect_err("a non-loopback, portless or malformed address must be refused");
        assert!(
            error
                .violations
                .iter()
                .any(|violation| violation.key == "RATATOSKR__RECEIPT__LISTEN_ADDRESS"),
            "the violation names the key: {error}"
        );
        assert!(
            !error.to_string().contains(refused),
            "the supplied value is never rendered"
        );
    }

    let error = Config::from_environment(entries(&[
        blob,
        ("RATATOSKR__ADMIN__LISTEN_ADDRESS", "127.0.0.1:18096"),
        ("RATATOSKR__RECEIPT__LISTEN_ADDRESS", "127.0.0.1:18096"),
    ]))
    .expect_err("the receipt address must differ from the operator address");
    assert!(
        error.violations.iter().any(|violation| {
            violation.key == "RATATOSKR__RECEIPT__LISTEN_ADDRESS"
                && violation.rule.contains("RATATOSKR__ADMIN__LISTEN_ADDRESS")
        }),
        "the violation names both keys: {error}"
    );
}

/// XR-021 CONTRACTS.md R2-04 rule 4: the enqueue-time size guard defaults to 8000000 bytes and
/// accepts 65536 through 8388608, the broker ceiling the platform configuration sets.
#[test]
fn max_event_bytes_defaults_and_is_bounded() {
    let blob = ("RATATOSKR__STORAGE__BLOB_ROOT", "/tmp/chatgpt-blobs");
    let config = Config::from_environment(entries(&[blob])).expect("defaults must load");
    assert_eq!(config.receipt.max_event_bytes, 8_000_000);

    for (accepted, expected) in [
        ("65536", 65_536),
        ("8388608", 8_388_608),
        ("1000000", 1_000_000),
    ] {
        let config = Config::from_environment(entries(&[
            blob,
            ("RATATOSKR__RECEIPT__MAX_EVENT_BYTES", accepted),
        ]))
        .expect("a value inside the range must load");
        assert_eq!(config.receipt.max_event_bytes, expected);
    }

    // "0" is left out of the value-free check below: the rule text names 8388608.
    for refused in ["65535", "8388609", "-1", "many", "0"] {
        let error = Config::from_environment(entries(&[
            blob,
            ("RATATOSKR__RECEIPT__MAX_EVENT_BYTES", refused),
        ]))
        .expect_err("a value outside the range must be refused");
        assert!(
            error
                .violations
                .iter()
                .any(|violation| violation.key == "RATATOSKR__RECEIPT__MAX_EVENT_BYTES"),
            "the violation names the key: {error}"
        );
        assert!(
            refused == "0" || !error.to_string().contains(refused),
            "the supplied value is never rendered"
        );
    }
}

// ---- XR-021 CONTRACTS.md R3-11: the operator example names a provisioned role ----

/// The database URL the operator example ships, split into `(role, host_port, database)`.
fn example_database() -> (String, String, String) {
    let example = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/systemd/chatgpt.conf.example"
    ))
    .expect("the operator example is shipped");
    let url = example
        .lines()
        .find_map(|line| line.strip_prefix("RATATOSKR__STORAGE__DATABASE_URL="))
        .expect("the example names a database URL");
    let rest = url
        .strip_prefix("postgres://")
        .expect("the URL is a postgres URL");
    let (credentials, location) = rest.split_once('@').expect("the URL carries credentials");
    let (host_port, database) = location.split_once('/').expect("the URL names a database");
    let (role, _) = credentials
        .split_once(':')
        .expect("the credentials name a role");
    (role.to_owned(), host_port.to_owned(), database.to_owned())
}

/// The role, database and port the example names are the ones `deploy/postgres/role.sql`
/// provisions, so a deployment that follows the files can start the service.
#[test]
fn the_example_role_file_exists() {
    let (role, host_port, database) = example_database();
    assert_eq!(role, "ratatoskr_chatgpt");
    assert_eq!(database, "ratatoskr");
    assert_eq!(
        host_port, "127.0.0.1:5432",
        "the target runs PostgreSQL on 5432"
    );

    let provisioning = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/postgres/role.sql"
    ))
    .expect("deploy/postgres/role.sql is shipped beside the operator example");
    assert!(
        provisioning.contains(&format!("create role {role} login")),
        "role.sql creates the role the example connects as"
    );
    assert!(
        provisioning.contains(&format!(
            "grant connect, create on database {database} to {role}"
        )),
        "role.sql grants that role the database the example names"
    );
    assert!(
        provisioning.contains(&format!("\\connect {database}")),
        "role.sql revokes the public schema inside that database"
    );
    assert!(
        provisioning.contains(&format!("revoke all on schema public from {role}")),
        "the role owns only its own schema"
    );
    assert!(
        provisioning.contains("nosuperuser nocreatedb nocreaterole noreplication nobypassrls"),
        "the role has no privilege beyond login"
    );
    assert!(
        provisioning
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .all(|line| !line.to_ascii_lowercase().contains("password")),
        "no credential belongs in the file; the password is set separately"
    );
}
