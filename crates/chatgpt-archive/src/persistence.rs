//! The owned database: pool construction and schema application.
//!
//! No migration tooling exists by development status: `schema.sql` at the
//! repository root is the one definition, embedded here, applied inside one
//! advisory-locked transaction so concurrent starters cannot interleave, and
//! edited in place as the schema grows.

use std::time::Duration;

use secrecy::ExposeSecret as _;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::config::{Limits, StorageConfig};

/// The advisory-lock key for schema application. One fixed fleet-style
/// constant: two processes applying concurrently must serialize, and the key
/// must never collide with another service's lock in a shared cluster.
const SCHEMA_LOCK_KEY: i64 = 0x7261_7461_736b_7201; // "rataskr" family, archive

/// Schema application failure.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    /// The pool could not be created.
    #[error("the database pool could not be created")]
    Pool(#[source] sqlx::Error),
    /// The schema transaction failed.
    #[error("applying the chatgpt_archive schema failed")]
    ApplySchema(#[source] sqlx::Error),
}

/// The process's connection to its `PostgreSQL` instance.
#[derive(Debug, Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connects with the configured URL and limits.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError`] when no connection can be established.
    pub async fn connect(
        storage: &StorageConfig,
        limits: &Limits,
    ) -> Result<Self, PersistenceError> {
        let Some(url) = storage.database_url.as_ref() else {
            return Err(PersistenceError::Pool(sqlx::Error::Configuration(
                "no database URL is configured".into(),
            )));
        };
        let pool = PgPoolOptions::new()
            .max_connections(limits.database_connections)
            .acquire_timeout(Duration::from_millis(limits.database_acquire_timeout_ms))
            .connect(url.expose_secret())
            .await
            .map_err(PersistenceError::Pool)?;
        Ok(Self { pool })
    }

    /// True when a trivial query round-trips; the readiness check body.
    ///
    /// # Errors
    ///
    /// Returns the underlying error so readiness can log it once.
    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    /// The pool, for the integration tests that probe applied relations.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies the first-version schema. Repeatable by construction.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError`] when the transaction fails.
    pub async fn apply_schema(&self) -> Result<(), PersistenceError> {
        const SCHEMA_SQL: &str = include_str!("../../../schema.sql");
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PersistenceError::ApplySchema)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(SCHEMA_LOCK_KEY)
            .execute(&mut *transaction)
            .await
            .map_err(PersistenceError::ApplySchema)?;
        sqlx::raw_sql(SCHEMA_SQL)
            .execute(&mut *transaction)
            .await
            .map_err(PersistenceError::ApplySchema)?;
        transaction
            .commit()
            .await
            .map_err(PersistenceError::ApplySchema)?;
        Ok(())
    }
}
