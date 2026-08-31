//! Process configuration read from `RATATOSKR__`-prefixed environment
//! variables.
//!
//! The key set is closed: every entry under the prefix must name a known
//! key and carry a valid value, and nothing is silently ignored. All entries
//! are examined so one load reports every violation found, never only the
//! first, and the report names keys and rules but never renders supplied
//! values — a value that reached an error message is a value that reached a
//! log aggregator.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use secrecy::SecretString;
use serde::Serialize;
use sqlx::postgres::PgConnectOptions;

const ENV_PREFIX: &str = "RATATOSKR__";

/// Process configuration with finite built-in limits.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    /// Operator listener configuration.
    pub admin: AdminConfig,
    /// Owned durable storage configuration.
    pub storage: StorageConfig,
    /// Telemetry pipeline configuration.
    pub telemetry: TelemetryConfig,
    /// Archive receipt configuration.
    pub receipt: ReceiptConfig,
    /// Resource and shutdown limits.
    pub limits: Limits,
}

/// One configured tenant credential: a bearer token bound to exactly one
/// archive account reference.
#[derive(Clone)]
pub struct TenantToken {
    /// The bearer credential. Never rendered.
    pub token: SecretString,
    /// The account external reference the token authenticates to.
    pub account_external_ref: String,
}

impl core::fmt::Debug for TenantToken {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TenantToken")
            .field("token", &"[REDACTED]")
            .field("account_external_ref", &self.account_external_ref)
            .finish()
    }
}

/// Archive receipt configuration.
#[derive(Clone, Serialize)]
pub struct ReceiptConfig {
    /// Bearer tokens that may authenticate a receipt, one per tenant.
    #[serde(skip_serializing)]
    pub tenant_tokens: Vec<TenantToken>,
    /// Explicit Platform user to local account mappings for loopback archive receipts.
    pub platform_accounts: Vec<(uuid::Uuid, String)>,
    /// Optional NATS endpoint used to publish the durable receipt outbox.
    #[serde(skip_serializing)]
    pub event_bus_url: Option<SecretString>,
    /// Absolute `NATS` `NKey` seed path used by the `ChatGPT` service identity.
    #[serde(skip_serializing)]
    pub event_bus_nkey_seed_path: Option<PathBuf>,
}

impl core::fmt::Debug for ReceiptConfig {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReceiptConfig")
            .field("tenant_tokens", &self.tenant_tokens)
            .field("platform_accounts", &self.platform_accounts)
            .field("event_bus_url", &"[REDACTED]")
            .field("event_bus_nkey_seed_path", &"[REDACTED]")
            .finish()
    }
}

/// Loopback operator listener configuration.
#[derive(Debug, Clone, Serialize)]
pub struct AdminConfig {
    /// Socket address for health, metrics, and version routes.
    pub listen_address: SocketAddr,
}

/// Durable storage locations owned by this service.
#[derive(Clone, Serialize)]
pub struct StorageConfig {
    /// Directory holding this service's content-addressed blobs. Required:
    /// archive bytes must land somewhere deliberate, and no default path in
    /// source is ever deliberate.
    pub blob_root: Option<PathBuf>,
    /// Archive `PostgreSQL` connection URL. Absent until configured; there is
    /// deliberately no default that is not either wrong or a secret in the
    /// source tree.
    #[serde(skip_serializing)]
    pub database_url: Option<SecretString>,
    /// Directory holding receipt staging files while an upload streams in.
    /// Absent until configured; without it the receipt surface does not mount.
    pub receipt_staging_root: Option<PathBuf>,
}

impl std::fmt::Debug for StorageConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorageConfig")
            .field("blob_root", &self.blob_root)
            .field("receipt_staging_root", &self.receipt_staging_root)
            .field("database_url", &"[REDACTED]")
            .finish()
    }
}

/// Telemetry pipeline configuration.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetryConfig {
    /// Structured log filter expression.
    pub log_filter: String,
}

/// Finite limits used by the process foundation.
#[derive(Debug, Clone, Serialize)]
pub struct Limits {
    /// Maximum database connections.
    pub database_connections: u32,
    /// Maximum wait for a database connection.
    pub database_acquire_timeout_ms: u64,
    /// Maximum graceful shutdown duration.
    pub shutdown_timeout_ms: u64,
    /// Maximum accepted archive size in bytes.
    pub max_archive_bytes: u64,
    /// Maximum central-directory entries in one archive.
    pub max_archive_entries: u32,
    /// Maximum decompressed bytes in a single archive entry.
    pub max_archive_entry_bytes: u64,
    /// Maximum aggregate decompressed bytes in one archive.
    pub max_archive_decompressed_bytes: u64,
    /// Maximum permitted per-entry decompression ratio.
    pub max_archive_compression_ratio: u64,
}

/// One configuration violation. The offending key and the rule it broke, and
/// never the supplied value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The environment variable key.
    pub key: String,
    /// The rule the value violated.
    pub rule: &'static str,
}

/// Configuration loading failure carrying every violation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// Every violation found, in first-seen order.
    pub violations: Vec<Violation>,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "configuration is invalid")?;
        for violation in &self.violations {
            write!(formatter, "\n  {} {}", violation.key, violation.rule)?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Loads the current process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] carrying every violation found.
    pub fn load() -> Result<Self, ConfigError> {
        let mut entries = Vec::new();
        for (key, value) in std::env::vars_os() {
            let Some(key) = key.into_string().ok() else {
                continue;
            };
            if !key.starts_with(ENV_PREFIX) {
                continue;
            }
            let Ok(value) = value.into_string() else {
                return Err(ConfigError {
                    violations: vec![Violation {
                        key,
                        rule: "must contain Unicode text",
                    }],
                });
            };
            entries.push((key, value));
        }

        Self::from_environment(entries)
    }

    /// Loads configuration from prefixed environment entries.
    ///
    /// Every entry under [`ENV_PREFIX`] must name a known key and carry a
    /// valid value; nothing is silently ignored. All entries are examined so
    /// one load reports every violation found, never only the first.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] carrying every violation found.
    pub fn from_environment<I, K, V>(entries: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut violations = Vec::new();
        let mut config = Self::default();
        for (key, value) in entries {
            let key = key.as_ref();
            if !key.starts_with(ENV_PREFIX) {
                continue;
            }
            apply_entry(&mut config, key, value.as_ref(), &mut violations);
        }

        // A missing required key is one more entry in the same report.
        if config.storage.blob_root.is_none()
            && !violations
                .iter()
                .any(|violation| violation.key == KEY_BLOB_ROOT)
        {
            violations.push(Violation {
                key: KEY_BLOB_ROOT.to_owned(),
                rule: "is required",
            });
        }
        if !config.receipt.platform_accounts.is_empty()
            && config.receipt.event_bus_url.is_none()
            && !violations
                .iter()
                .any(|violation| violation.key == KEY_EVENT_BUS_URL)
        {
            violations.push(Violation {
                key: KEY_EVENT_BUS_URL.to_owned(),
                rule: "is required when Platform account mappings are configured",
            });
        }
        if (config.receipt.event_bus_url.is_some() || !config.receipt.platform_accounts.is_empty())
            && config.receipt.event_bus_nkey_seed_path.is_none()
            && !violations
                .iter()
                .any(|violation| violation.key == KEY_EVENT_BUS_NKEY_SEED_PATH)
        {
            violations.push(Violation {
                key: KEY_EVENT_BUS_NKEY_SEED_PATH.to_owned(),
                rule: "is required when the event bus URL is configured",
            });
        }

        if violations.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError { violations })
        }
    }
}

const KEY_LISTEN_ADDRESS: &str = "RATATOSKR__ADMIN__LISTEN_ADDRESS";
const KEY_BLOB_ROOT: &str = "RATATOSKR__STORAGE__BLOB_ROOT";
const KEY_DATABASE_URL: &str = "RATATOSKR__STORAGE__DATABASE_URL";
const KEY_LOG_FILTER: &str = "RATATOSKR__TELEMETRY__LOG_FILTER";
const KEY_DATABASE_CONNECTIONS: &str = "RATATOSKR__LIMITS__DATABASE_CONNECTIONS";
const KEY_DATABASE_ACQUIRE_TIMEOUT_MS: &str = "RATATOSKR__LIMITS__DATABASE_ACQUIRE_TIMEOUT_MS";
const KEY_SHUTDOWN_TIMEOUT_MS: &str = "RATATOSKR__LIMITS__SHUTDOWN_TIMEOUT_MS";
const KEY_MAX_ARCHIVE_BYTES: &str = "RATATOSKR__LIMITS__MAX_ARCHIVE_BYTES";
const KEY_MAX_ARCHIVE_ENTRIES: &str = "RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRIES";
const KEY_MAX_ARCHIVE_ENTRY_BYTES: &str = "RATATOSKR__LIMITS__MAX_ARCHIVE_ENTRY_BYTES";
const KEY_MAX_ARCHIVE_DECOMPRESSED_BYTES: &str =
    "RATATOSKR__LIMITS__MAX_ARCHIVE_DECOMPRESSED_BYTES";
const KEY_MAX_ARCHIVE_COMPRESSION_RATIO: &str = "RATATOSKR__LIMITS__MAX_ARCHIVE_COMPRESSION_RATIO";
const KEY_RECEIPT_STAGING_ROOT: &str = "RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT";
const KEY_TENANT_TOKENS: &str = "RATATOSKR__RECEIPT__TENANT_TOKENS";
const KEY_PLATFORM_ACCOUNTS: &str = "RATATOSKR__RECEIPT__PLATFORM_ACCOUNTS";
const KEY_EVENT_BUS_URL: &str = "RATATOSKR__RECEIPT__EVENT_BUS_URL";
const KEY_EVENT_BUS_NKEY_SEED_PATH: &str = "RATATOSKR__RECEIPT__EVENT_BUS_NKEY_SEED_PATH";

#[allow(
    clippy::too_many_lines,
    reason = "closed-key dispatch keeps validation in one exhaustive match"
)]
fn apply_entry(config: &mut Config, key: &str, value: &str, violations: &mut Vec<Violation>) {
    let refused = |rule: &'static str| Violation {
        key: key.to_owned(),
        rule,
    };
    match key {
        KEY_LISTEN_ADDRESS => match value.parse::<SocketAddr>() {
            Ok(address) if address.ip().is_loopback() && address.port() != 0 => {
                config.admin.listen_address = address;
            }
            Ok(_) => violations.push(refused("must be a loopback address with a port")),
            Err(_) => violations.push(refused("must be a socket address")),
        },
        KEY_BLOB_ROOT => match parse_absolute_path(value) {
            Ok(path) => config.storage.blob_root = Some(path),
            Err(()) => violations.push(refused("must be an absolute directory path")),
        },
        KEY_DATABASE_URL => {
            if value.parse::<PgConnectOptions>().is_ok() {
                config.storage.database_url = Some(SecretString::from(value.to_owned()));
            } else {
                violations.push(refused(
                    "must be a PostgreSQL connection URL naming user, password, host, and database",
                ));
            }
        }
        KEY_LOG_FILTER => {
            if value.trim().is_empty() {
                violations.push(refused("must be a non-empty tracing filter expression"));
            } else {
                value.clone_into(&mut config.telemetry.log_filter);
            }
        }
        KEY_DATABASE_CONNECTIONS => match parse_positive::<u32>(value) {
            Ok(parsed) => config.limits.database_connections = parsed,
            Err(rule) => violations.push(refused(rule)),
        },
        KEY_DATABASE_ACQUIRE_TIMEOUT_MS | KEY_SHUTDOWN_TIMEOUT_MS => {
            match parse_positive::<u64>(value) {
                Ok(parsed) if key == KEY_DATABASE_ACQUIRE_TIMEOUT_MS => {
                    config.limits.database_acquire_timeout_ms = parsed;
                }
                Ok(parsed) => config.limits.shutdown_timeout_ms = parsed,
                Err(rule) => violations.push(refused(rule)),
            }
        }
        KEY_MAX_ARCHIVE_BYTES => match parse_positive::<u64>(value) {
            Ok(parsed) => config.limits.max_archive_bytes = parsed,
            Err(rule) => violations.push(refused(rule)),
        },
        KEY_MAX_ARCHIVE_ENTRIES => match parse_positive::<u32>(value) {
            Ok(parsed) => config.limits.max_archive_entries = parsed,
            Err(rule) => violations.push(refused(rule)),
        },
        KEY_MAX_ARCHIVE_ENTRY_BYTES => match parse_positive::<u64>(value) {
            Ok(parsed) => config.limits.max_archive_entry_bytes = parsed,
            Err(rule) => violations.push(refused(rule)),
        },
        KEY_MAX_ARCHIVE_DECOMPRESSED_BYTES => match parse_positive::<u64>(value) {
            Ok(parsed) => config.limits.max_archive_decompressed_bytes = parsed,
            Err(rule) => violations.push(refused(rule)),
        },
        KEY_MAX_ARCHIVE_COMPRESSION_RATIO => match parse_positive::<u64>(value) {
            Ok(parsed) => config.limits.max_archive_compression_ratio = parsed,
            Err(rule) => violations.push(refused(rule)),
        },
        KEY_RECEIPT_STAGING_ROOT => match parse_absolute_path(value) {
            Ok(path) => config.storage.receipt_staging_root = Some(path),
            Err(()) => violations.push(refused("must be an absolute directory path")),
        },
        KEY_TENANT_TOKENS => {
            let refused_token = |rule: &'static str| Violation {
                key: key.to_owned(),
                rule,
            };
            if value.is_empty() {
                violations.push(refused_token(
                    "must be a comma-separated list of <token>=<account-ref> pairs",
                ));
            } else {
                for entry in value.split(',') {
                    let Some((token, reference)) = entry.split_once('=') else {
                        violations.push(refused_token(
                            "must be a comma-separated list of <token>=<account-ref> pairs",
                        ));
                        continue;
                    };
                    if token.is_empty() || reference.is_empty() {
                        violations.push(refused_token(
                            "every pair needs a non-empty token and a non-empty account reference",
                        ));
                        continue;
                    }
                    config.receipt.tenant_tokens.push(TenantToken {
                        token: SecretString::from(token.to_owned()),
                        account_external_ref: reference.to_owned(),
                    });
                }
            }
        }
        KEY_PLATFORM_ACCOUNTS => {
            if value.is_empty() {
                violations.push(refused(
                    "must be a comma-separated list of <platform-user-uuid>=<account-ref> pairs",
                ));
            } else {
                for entry in value.split(',') {
                    let Some((user_id, account)) = entry.split_once('=') else {
                        violations.push(refused("must be a comma-separated list of <platform-user-uuid>=<account-ref> pairs"));
                        continue;
                    };
                    let Ok(user_id) = user_id.parse::<uuid::Uuid>() else {
                        violations.push(refused(
                            "every mapping needs a canonical platform user UUID",
                        ));
                        continue;
                    };
                    if account.is_empty() {
                        violations
                            .push(refused("every mapping needs a non-empty account reference"));
                        continue;
                    }
                    config
                        .receipt
                        .platform_accounts
                        .push((user_id, account.to_owned()));
                }
            }
        }
        KEY_EVENT_BUS_URL => {
            if value.starts_with("nats://") || value.starts_with("tls://") {
                config.receipt.event_bus_url = Some(SecretString::from(value.to_owned()));
            } else {
                violations.push(refused("must be a nats:// or tls:// endpoint"));
            }
        }
        KEY_EVENT_BUS_NKEY_SEED_PATH => match parse_absolute_path(value) {
            Ok(path) => config.receipt.event_bus_nkey_seed_path = Some(path),
            Err(()) => violations.push(refused("must be an absolute file path")),
        },
        _ => violations.push(refused("is not recognized")),
    }
}

/// An absolute path is one the process can anchor to regardless of where it
/// was started; a relative blob root would silently follow the service's
/// working directory.
fn parse_absolute_path(value: &str) -> Result<PathBuf, ()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(());
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(())
    }
}

fn parse_positive<T>(value: &str) -> Result<T, &'static str>
where
    T: std::str::FromStr + Default + PartialOrd,
{
    let parsed = value
        .parse::<T>()
        .map_err(|_| "must be a positive integer")?;
    if parsed <= T::default() {
        return Err("must be a positive integer");
    }
    Ok(parsed)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            admin: AdminConfig {
                listen_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9084),
            },
            storage: StorageConfig {
                blob_root: None,
                database_url: None,
                receipt_staging_root: None,
            },
            telemetry: TelemetryConfig {
                log_filter: "info".to_owned(),
            },
            receipt: ReceiptConfig {
                tenant_tokens: Vec::new(),
                platform_accounts: Vec::new(),
                event_bus_url: None,
                event_bus_nkey_seed_path: None,
            },
            limits: Limits {
                database_connections: 8,
                database_acquire_timeout_ms: 5_000,
                shutdown_timeout_ms: 10_000,
                // Generous default for full account exports with assets:
                // 16 GiB, overridable per deployment through limits.
                max_archive_bytes: 17_179_869_184,
                max_archive_entries: 10_000,
                max_archive_entry_bytes: 2_147_483_648,
                max_archive_decompressed_bytes: 34_359_738_368,
                max_archive_compression_ratio: 100,
            },
        }
    }
}
