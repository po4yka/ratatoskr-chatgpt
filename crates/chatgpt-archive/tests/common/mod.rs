//! Helpers shared by the integration tests that need a live `PostgreSQL` or `NATS`.
//!
//! A missing server is a failure to fix, not a reason to skip: every helper here
//! panics with the name of the variable that is not set. Each test that mutates
//! shared state creates its own disposable database, so tests never observe each
//! other's rows and the import worker, which scans every pending run, only ever sees
//! its own.

#![allow(
    dead_code,
    clippy::expect_used,
    clippy::panic,
    reason = "shared test helpers; each test binary uses a different subset and failures report through panics"
)]

use std::io::Write as _;

use ratatoskr_chatgpt_archive::Database;
use ratatoskr_chatgpt_archive::config::{Limits, StorageConfig};
use secrecy::SecretString;
use uuid::Uuid;

pub(crate) const MEDIA_TYPE: &str = "application/zip";

/// The administrative database URL the suite is configured with.
pub(crate) fn database_url() -> String {
    #[allow(
        clippy::disallowed_methods,
        reason = "the integration harness reads the PostgreSQL URL the runner exports"
    )]
    let url = std::env::var("CHATGPT_TEST_DATABASE_URL").ok();
    url.filter(|url| !url.trim().is_empty())
        .expect("CHATGPT_TEST_DATABASE_URL must point at a PostgreSQL 17 server")
}

/// The `NATS` server with `JetStream` enabled the suite is configured with.
pub(crate) fn nats_url() -> String {
    #[allow(
        clippy::disallowed_methods,
        reason = "the integration harness reads the NATS URL the runner exports"
    )]
    let url = std::env::var("CHATGPT_TEST_NATS_URL").ok();
    url.filter(|url| !url.trim().is_empty())
        .expect("CHATGPT_TEST_NATS_URL must point at a NATS server with JetStream")
}

pub(crate) fn limits() -> Limits {
    Limits {
        database_connections: 4,
        database_acquire_timeout_ms: 5_000,
        shutdown_timeout_ms: 5_000,
        max_archive_bytes: 17_179_869_184,
        max_archive_entries: 10_000,
        max_archive_entry_bytes: 2_147_483_648,
        max_archive_decompressed_bytes: 34_359_738_368,
        max_archive_compression_ratio: 100,
    }
}

pub(crate) fn archive_limits() -> ratatoskr_chatgpt_archive::ArchiveLimits {
    ratatoskr_chatgpt_archive::ArchiveLimits {
        max_entries: 32,
        max_compressed_bytes: 1_048_576,
        max_entry_bytes: 1_048_576,
        max_decompressed_bytes: 2_097_152,
        max_compression_ratio: 100,
    }
}

fn storage(url: &str) -> StorageConfig {
    StorageConfig {
        blob_root: None,
        database_url: Some(SecretString::from(url.to_owned())),
        receipt_staging_root: None,
    }
}

/// A database that exists only for one test, created from the current `schema.sql`.
pub(crate) struct DisposableDatabase {
    pub(crate) database: Database,
    admin: sqlx::PgPool,
    name: String,
}

impl DisposableDatabase {
    /// Creates a fresh database on the configured server and applies the schema.
    pub(crate) async fn create() -> Self {
        let admin_url = database_url();
        let admin = sqlx::PgPool::connect(&admin_url)
            .await
            .expect("the administrative database accepts connections");
        let name = format!("xr021_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .expect("a disposable database can be created");
        let (server, _) = admin_url
            .rsplit_once('/')
            .expect("the database URL names a database");
        let database = Database::connect(&storage(&format!("{server}/{name}")), &limits())
            .await
            .expect("the disposable database accepts connections");
        database
            .apply_schema()
            .await
            .expect("the current schema applies");
        Self {
            database,
            admin,
            name,
        }
    }

    pub(crate) fn pool(&self) -> &sqlx::PgPool {
        self.database.pool()
    }

    /// Drops the database. Tests call this last; a failed test leaves its database
    /// behind for inspection, which is what a failing test should do.
    pub(crate) async fn discard(self) {
        self.database.pool().close().await;
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        ))
        .execute(&self.admin)
        .await
        .expect("the disposable database can be dropped");
    }
}

pub(crate) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("a synthetic entry starts");
        writer.write_all(bytes).expect("a synthetic entry writes");
    }
    writer
        .finish()
        .expect("the synthetic zip closes")
        .into_inner()
}

/// A synthetic export with two conversations and no projects.
pub(crate) fn conversations_only_zip() -> Vec<u8> {
    zip_of(&[(
        "conversations.json",
        include_bytes!("../fixtures/synthetic_conversations.json"),
    )])
}

/// A synthetic export with one project and the conversation it lists.
pub(crate) fn project_zip() -> Vec<u8> {
    zip_of(&[
        (
            "conversations.json",
            include_bytes!("../fixtures/synthetic_archive_conversations.json"),
        ),
        (
            "projects.json",
            include_bytes!("../fixtures/synthetic_archive_projects.json"),
        ),
    ])
}

/// A synthetic export whose single conversation carries `text` as its only message.
pub(crate) fn single_message_zip(conversation_id: &str, text: &str) -> Vec<u8> {
    let document = serde_json::json!([{
        "id": conversation_id,
        "title": "Synthetic",
        "mapping": {
            "m1": {
                "id": "m1",
                "parent": null,
                "message": {
                    "id": "m1",
                    "author": { "role": "user" },
                    "content": { "parts": [text] }
                }
            }
        }
    }]);
    zip_of(&[("conversations.json", document.to_string().as_bytes())])
}

/// What `receive_export` stored.
pub(crate) struct Received {
    pub(crate) export: Uuid,
    pub(crate) archive: Uuid,
    pub(crate) account: Uuid,
}

/// Stores `bytes` as a raw export for `account_ref` exactly as the receipt route does,
/// leaving the run at `stored` for the import worker.
pub(crate) async fn receive_export(
    pool: &sqlx::PgPool,
    blobs: &ratatoskr_chatgpt_archive::BlobStore,
    account_ref: &str,
    bytes: Vec<u8>,
) -> Received {
    use ratatoskr_chatgpt_archive::receipt::AcquisitionMode;
    use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
    use ratatoskr_chatgpt_archive::receipt::repository::{PublishRequest, ReceiptRepository as _};
    use sha2::Digest as _;

    let repository = PostgresReceiptRepository::new(pool.clone());
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    let length = u64::try_from(bytes.len()).expect("a test archive fits in u64");
    let raw = blobs
        .store(
            MEDIA_TYPE,
            futures_util::stream::iter([Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(
                bytes,
            ))]),
        )
        .await
        .expect("the raw archive is stored");
    let run_id = repository
        .create_run(account_ref, &AcquisitionMode::ConsumerExport, MEDIA_TYPE)
        .await
        .expect("a run is created");
    repository
        .record_hash(run_id, digest.clone(), length)
        .await
        .expect("the digest is recorded");
    let published = repository
        .publish_export(PublishRequest {
            run_id,
            account_external_ref: account_ref.to_owned(),
            mode: AcquisitionMode::ConsumerExport,
            blob_ref_json: serde_json::to_value(&raw).expect("a blob reference encodes"),
            sha256_hex: digest,
            byte_length: length,
            platform_operation: None,
        })
        .await
        .expect("the export is published");
    let account_id: Uuid =
        sqlx::query_scalar("SELECT id FROM chatgpt_archive.accounts WHERE external_ref = $1")
            .bind(account_ref)
            .fetch_one(pool)
            .await
            .expect("the receipt created the account");
    Received {
        export: published.export_id,
        archive: published.ai_archive_id,
        account: account_id,
    }
}

/// Runs the import worker until it has nothing left to do.
pub(crate) async fn drain(worker: &ratatoskr_chatgpt_archive::InitialImportWorker) {
    for _ in 0..32 {
        if worker
            .process_pending_once()
            .await
            .expect("the import worker pass succeeds")
            == 0
        {
            return;
        }
    }
    panic!("the import worker did not settle within 32 passes");
}

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_chatgpt_archive::BlobStore;
use ratatoskr_identifiers::BlobRef;

pub(crate) async fn blob(blobs: &BlobStore, bytes: &'static [u8]) -> BlobRef {
    blobs
        .store(
            "application/zip",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from_static(bytes))]),
        )
        .await
        .expect("a blob is stored")
}

pub(crate) async fn insert_account(database: &DisposableDatabase, label: &str) -> Uuid {
    let tenant = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref)
         VALUES ($1, 'personal', $2)",
    )
    .bind(tenant)
    .bind(format!("{label}-{tenant}"))
    .execute(database.pool())
    .await
    .expect("the account is inserted");
    tenant
}

pub(crate) async fn insert_export(
    database: &DisposableDatabase,
    tenant: Uuid,
    archive: Uuid,
    raw: &BlobRef,
) -> Uuid {
    let export = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.exports
         (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length)
         VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)",
    )
    .bind(export)
    .bind(archive)
    .bind(tenant)
    .bind(serde_json::to_value(raw).expect("a blob reference encodes"))
    .bind(raw.digest.hex.as_str())
    .bind(i64::try_from(raw.length_bytes).expect("a small length"))
    .execute(database.pool())
    .await
    .expect("the export is inserted");
    export
}
