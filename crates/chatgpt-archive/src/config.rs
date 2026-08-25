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
    /// Resource and shutdown limits.
    pub limits: Limits,
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
}

impl std::fmt::Debug for StorageConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorageConfig")
            .field("blob_root", &self.blob_root)
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
            },
            telemetry: TelemetryConfig {
                log_filter: "info".to_owned(),
            },
            limits: Limits {
                database_connections: 8,
                database_acquire_timeout_ms: 5_000,
                shutdown_timeout_ms: 10_000,
            },
        }
    }
}
