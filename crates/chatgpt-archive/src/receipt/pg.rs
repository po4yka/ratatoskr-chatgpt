//! `PostgreSQL` implementation of the receipt persistence seam.
//!
//! Every stage change is a guarded compare-and-set update (`WHERE
//! state = $expected`); losing the race surfaces as
//! [`RepositoryError::Conflict`] rather than blocking. The export insert and
//! its run transition share one transaction, so a crash never leaves an
//! export without its run anchor.

use sqlx::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

use super::AcquisitionMode;
use super::report::raw_stored_partial;
use super::repository::{
    PublishRequest, PublishedExport, ReceiptRepository, RepoFuture, RepositoryError, RunRecord,
};
use super::state::ImportState;

/// The receipt seam backed by the owned `chatgpt_archive` schema.
#[derive(Debug, Clone)]
pub struct PostgresReceiptRepository {
    pool: PgPool,
}

impl PostgresReceiptRepository {
    /// Creates a repository over an established pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn backend(error: sqlx::Error) -> RepositoryError {
    RepositoryError::backend(error)
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn parse_state(spelling: &str) -> Result<ImportState, RepositoryError> {
    ImportState::parse(spelling)
        .ok_or_else(|| RepositoryError::backend(std::io::Error::other("undeclared state spelling")))
}

/// One `import_runs` row as the database hands it over.
struct RunRow {
    id: Uuid,
    account_ref: Option<String>,
    acquisition_mode: Option<String>,
    media_type: Option<String>,
    #[allow(dead_code, reason = "carried for future parser-version reporting")]
    parser_version: Option<String>,
    state: String,
    sha256_hex: Option<String>,
    byte_length: Option<i64>,
    export_id: Option<Uuid>,
}

impl<'row> FromRow<'row, sqlx::postgres::PgRow> for RunRow {
    fn from_row(row: &'row sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row as _;
        Ok(Self {
            id: row.try_get("id")?,
            account_ref: row.try_get("account_ref")?,
            acquisition_mode: row.try_get("acquisition_mode")?,
            media_type: row.try_get("media_type")?,
            parser_version: row.try_get("parser_version")?,
            state: row.try_get("state")?,
            sha256_hex: row.try_get("sha256_hex")?,
            byte_length: row.try_get("byte_length")?,
            export_id: row.try_get("export_id")?,
        })
    }
}

impl RunRow {
    /// Converts into the seam's record, refusing undeclared state spellings.
    fn into_record(self) -> Result<RunRecord, RepositoryError> {
        Ok(RunRecord {
            id: self.id,
            account_external_ref: self.account_ref.unwrap_or_default(),
            acquisition_mode: self
                .acquisition_mode
                .unwrap_or_else(|| AcquisitionMode::ConsumerExport.as_str().to_owned()),
            media_type: self
                .media_type
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            state: parse_state(&self.state)?,
            sha256_hex: self.sha256_hex,
            byte_length: self.byte_length,
            export_id: self.export_id,
        })
    }
}

const TERMINAL_STATES: [&str; 5] = ["completed", "partial", "failed", "duplicate", "quarantined"];

impl ReceiptRepository for PostgresReceiptRepository {
    fn create_run(
        &self,
        account_external_ref: &str,
        mode: &AcquisitionMode,
        media_type: &str,
    ) -> RepoFuture<Result<Uuid, RepositoryError>> {
        let pool = self.pool.clone();
        let account = account_external_ref.to_owned();
        let mode_spelling = mode.as_str().to_owned();
        let media_type = media_type.to_owned();
        Box::pin(async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO chatgpt_archive.import_runs (id, account_ref, acquisition_mode, media_type, state) VALUES ($1, $2, $3, $4, 'received')",
            )
            .bind(id)
            .bind(&account)
            .bind(&mode_spelling)
            .bind(&media_type)
            .execute(&pool)
            .await
            .map_err(backend)?;
            Ok(id)
        })
    }

    fn record_hash(
        &self,
        run_id: Uuid,
        sha256_hex: String,
        byte_length: u64,
    ) -> RepoFuture<Result<(), RepositoryError>> {
        let pool = self.pool.clone();
        Box::pin(async move {
            let outcome = sqlx::query(
                "UPDATE chatgpt_archive.import_runs SET state = 'hashed', sha256_hex = $2, byte_length = $3 WHERE id = $1 AND state = 'received'",
            )
            .bind(run_id)
            .bind(&sha256_hex)
            .bind(to_i64(byte_length))
            .execute(&pool)
            .await
            .map_err(backend)?;
            if outcome.rows_affected() == 0 {
                return Err(RepositoryError::Conflict);
            }
            Ok(())
        })
    }

    fn mark_run(
        &self,
        run_id: Uuid,
        expected: &ImportState,
        target: ImportState,
    ) -> RepoFuture<Result<(), RepositoryError>> {
        let pool = self.pool.clone();
        let expected = expected.as_str().to_owned();
        let target_spelling = target.as_str().to_owned();
        let terminal = TERMINAL_STATES.contains(&target.as_str());
        Box::pin(async move {
            let outcome = if terminal {
                sqlx::query(
                    "UPDATE chatgpt_archive.import_runs SET state = $2, finished_at = now() WHERE id = $1 AND state = $3",
                )
                .bind(run_id)
                .bind(&target_spelling)
                .bind(&expected)
                .execute(&pool)
                .await
                .map_err(backend)?
            } else {
                sqlx::query(
                    "UPDATE chatgpt_archive.import_runs SET state = $2 WHERE id = $1 AND state = $3",
                )
                .bind(run_id)
                .bind(&target_spelling)
                .bind(&expected)
                .execute(&pool)
                .await
                .map_err(backend)?
            };
            if outcome.rows_affected() == 0 {
                return Err(RepositoryError::Conflict);
            }
            Ok(())
        })
    }

    fn load_run(&self, run_id: Uuid) -> RepoFuture<Result<Option<RunRecord>, RepositoryError>> {
        let pool = self.pool.clone();
        Box::pin(async move {
            let row = sqlx::query_as::<_, RunRow>(
                "SELECT id, account_ref, acquisition_mode, media_type, parser_version, state, sha256_hex, byte_length, export_id FROM chatgpt_archive.import_runs WHERE id = $1",
            )
            .bind(run_id)
            .fetch_optional(&pool)
            .await
            .map_err(backend)?;
            row.map(RunRow::into_record).transpose()
        })
    }

    fn list_resumable(&self) -> RepoFuture<Result<Vec<RunRecord>, RepositoryError>> {
        let pool = self.pool.clone();
        Box::pin(async move {
            let rows = sqlx::query_as::<_, RunRow>(
                "SELECT id, account_ref, acquisition_mode, media_type, parser_version, state, sha256_hex, byte_length, export_id FROM chatgpt_archive.import_runs WHERE state IN ('received', 'hashed') ORDER BY started_at",
            )
            .fetch_all(&pool)
            .await
            .map_err(backend)?;
            rows.into_iter().map(RunRow::into_record).collect()
        })
    }

    fn find_export_by_digest(
        &self,
        account_external_ref: &str,
        sha256_hex: &str,
    ) -> RepoFuture<Result<Option<Uuid>, RepositoryError>> {
        let pool = self.pool.clone();
        let account = account_external_ref.to_owned();
        let digest = sha256_hex.to_owned();
        Box::pin(async move {
            let existing: Option<(Uuid,)> = sqlx::query_as(
                "SELECT e.id FROM chatgpt_archive.exports e JOIN chatgpt_archive.accounts a ON a.id = e.account_id WHERE a.external_ref = $1 AND e.sha256_hex = $2 LIMIT 1",
            )
            .bind(&account)
            .bind(&digest)
            .fetch_optional(&pool)
            .await
            .map_err(backend)?;
            Ok(existing.map(|(id,)| id))
        })
    }

    fn publish_export(
        &self,
        request: PublishRequest,
    ) -> RepoFuture<Result<PublishedExport, RepositoryError>> {
        let pool = self.pool.clone();
        let account = request.account_external_ref;
        let mode_spelling = request.mode.as_str().to_owned();
        let run_id = request.run_id;
        let blob_ref_json = request.blob_ref_json;
        let sha256_hex = request.sha256_hex;
        let byte_length = request.byte_length;
        let platform_operation = request.platform_operation;
        Box::pin(async move {
            let mut transaction = pool.begin().await.map_err(backend)?;

            // The tenant's owning account exists before its first receipt.
            let owner: (Uuid,) = sqlx::query_as(
                "INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2) ON CONFLICT (external_kind, external_ref) DO UPDATE SET last_seen_at = now() RETURNING id",
            )
            .bind(Uuid::now_v7())
            .bind(&account)
            .fetch_one(&mut *transaction)
            .await
            .map_err(backend)?;

            if let Some((existing,)) = sqlx::query_as::<_, (Uuid,)>(
                "SELECT id FROM chatgpt_archive.exports WHERE account_id = $1 AND sha256_hex = $2 LIMIT 1",
            )
            .bind(owner.0)
            .bind(&sha256_hex)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(backend)?
            {
                return Err(RepositoryError::DuplicateExisting {
                    existing_export_id: existing,
                });
            }

            let export_id = Uuid::now_v7();
            let ai_archive_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length, import_started_at) VALUES ($1, $2, $3, $4, $5, $6, $7, now())",
            )
            .bind(export_id)
            .bind(ai_archive_id)
            .bind(owner.0)
            .bind(&mode_spelling)
            .bind(&blob_ref_json)
            .bind(&sha256_hex)
            .bind(to_i64(byte_length))
            .execute(&mut *transaction)
            .await
            .map_err(backend)?;

            let advanced = sqlx::query(
                "UPDATE chatgpt_archive.import_runs SET state = 'stored', export_id = $2 WHERE id = $1 AND state = 'hashed'",
            )
            .bind(run_id)
            .bind(export_id)
            .execute(&mut *transaction)
            .await
            .map_err(backend)?;
            if advanced.rows_affected() == 0 {
                return Err(RepositoryError::Conflict);
            }

            if let Some(operation) = platform_operation {
                let report = raw_stored_partial(operation, ai_archive_id)?;
                sqlx::query(
                    "INSERT INTO chatgpt_archive.outbox_events (event_type, aggregate_id, payload) VALUES ('platform.operation.reported.v1', $1, $2)",
                )
                .bind(operation.operation_id)
                .bind(report)
                .execute(&mut *transaction)
                .await
                .map_err(backend)?;
            }

            transaction.commit().await.map_err(backend)?;
            Ok(PublishedExport {
                export_id,
                ai_archive_id,
            })
        })
    }
}

impl RunRecord {
    /// The recorded byte length widened to the seam's unsigned contract.
    #[must_use]
    pub fn length_u64(&self) -> u64 {
        from_i64(self.byte_length.unwrap_or_default())
    }
}
