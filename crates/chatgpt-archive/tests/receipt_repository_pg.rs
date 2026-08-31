//! Contract tests for the `PostgreSQL` receipt repository.
//!
//! They skip unless `CHATGPT_TEST_DATABASE_URL` is set; CI always sets it.
//! These prove the SQL adapter against a real server; the receiver logic
//! above the seam is proven by `receipt_receiver.rs` through the fake.

// Test bodies fail through `panic!`; assertions are the contract.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

use secrecy::SecretString;
use sha2::Digest as _;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::OnceLock;
use uuid::Uuid;

use ratatoskr_chatgpt_archive::receipt::AcquisitionMode;
use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
use ratatoskr_chatgpt_archive::receipt::repository::{
    PlatformOperation, PublishRequest, ReceiptRepository as _, RepositoryError,
};
use ratatoskr_chatgpt_archive::receipt::state::ImportState;

const MEDIA_TYPE: &str = "application/zip";

async fn worker_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn synthetic_zip() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file(
            "conversations.json",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("synthetic archive entry starts");
    writer
        .write_all(include_bytes!("fixtures/synthetic_conversations.json"))
        .expect("synthetic archive entry writes");
    writer
        .finish()
        .expect("synthetic archive closes")
        .into_inner()
}

fn minimal_known_zip() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file(
            "conversations.json",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("minimal archive entry starts");
    writer
        .write_all(br#"[{"id":"conversation-minimal","mapping":{}}]"#)
        .expect("minimal archive entry writes");
    writer
        .finish()
        .expect("minimal archive closes")
        .into_inner()
}

fn test_url() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "the test harness reads the database URL the runner exports"
    )]
    let url = std::env::var("CHATGPT_TEST_DATABASE_URL").ok();
    url.filter(|url| !url.trim().is_empty())
}

async fn connected(
    url: &str,
) -> Result<(sqlx::PgPool, PostgresReceiptRepository), Box<dyn std::error::Error>> {
    let storage = ratatoskr_chatgpt_archive::config::StorageConfig {
        blob_root: None,
        database_url: Some(SecretString::from(url.to_owned())),
        receipt_staging_root: None,
    };
    let limits = ratatoskr_chatgpt_archive::config::Limits {
        database_connections: 2,
        database_acquire_timeout_ms: 5_000,
        shutdown_timeout_ms: 5_000,
        max_archive_bytes: 17_179_869_184,
        max_archive_entries: 10_000,
        max_archive_entry_bytes: 2_147_483_648,
        max_archive_decompressed_bytes: 34_359_738_368,
        max_archive_compression_ratio: 100,
    };
    let database =
        ratatoskr_chatgpt_archive::persistence::Database::connect(&storage, &limits).await?;
    database.apply_schema().await?;
    let pool = database.pool().clone();
    Ok((pool.clone(), PostgresReceiptRepository::new(pool)))
}

async fn drive_until_operation_report(
    worker: &ratatoskr_chatgpt_archive::InitialImportWorker,
    pool: &sqlx::PgPool,
    operation_id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..32 {
        let reported: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.outbox_events
             WHERE event_type='platform.operation.reported.v1' AND aggregate_id=$1)",
        )
        .bind(operation_id)
        .fetch_one(pool)
        .await?;
        if reported {
            return Ok(());
        }
        if worker.process_pending_once().await? == 0 {
            break;
        }
    }
    Err("the bounded worker drain did not report the target operation".into())
}

/// A run round-trips create -> load -> hash -> advance with its evidence.
#[tokio::test]
async fn run_roundtrips_through_create_load_and_advance() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (_pool, repository) = connected(&url).await?;

    let run_id = repository
        .create_run(
            "acc-roundtrip",
            &AcquisitionMode::ConsumerExport,
            MEDIA_TYPE,
        )
        .await?;

    let fresh = repository.load_run(run_id).await?.expect("run exists");
    assert_eq!(fresh.state, ImportState::Received);
    assert_eq!(fresh.account_external_ref, "acc-roundtrip");
    assert_eq!(fresh.media_type, MEDIA_TYPE);

    repository.record_hash(run_id, "a".repeat(64), 42).await?;
    let hashed = repository.load_run(run_id).await?.expect("run exists");
    assert_eq!(hashed.state, ImportState::Hashed);
    assert_eq!(hashed.sha256_hex.as_deref(), Some("a".repeat(64).as_str()));
    assert_eq!(hashed.byte_length, Some(42));

    repository
        .mark_run(run_id, &ImportState::Hashed, ImportState::Stored)
        .await?;
    let stored = repository.load_run(run_id).await?.expect("run exists");
    assert_eq!(stored.state, ImportState::Stored);
    Ok(())
}

/// A guarded transition accepts exactly one winner and refuses stale
/// sources afterwards.
#[tokio::test]
async fn guarded_transition_accepts_once_then_refuses_stale_source()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (_pool, repository) = connected(&url).await?;
    let run_id = repository
        .create_run("acc-guard", &AcquisitionMode::ConsumerExport, MEDIA_TYPE)
        .await?;
    repository.record_hash(run_id, "b".repeat(64), 10).await?;

    let left = repository.mark_run(run_id, &ImportState::Hashed, ImportState::Stored);
    let right = repository.mark_run(run_id, &ImportState::Hashed, ImportState::Failed);
    let (left, right) = tokio::join!(left, right);
    let outcomes = [left.is_ok(), right.is_ok()];
    assert!(
        outcomes.iter().filter(|ok| **ok).count() == 1,
        "exactly one concurrent transition wins: {outcomes:?}"
    );
    Ok(())
}

/// Raw receipt stays non-terminal, while a recreated worker imports real
/// fixture records and reports every operation bound to the immutable bytes.
#[tokio::test]
async fn restart_worker_reports_actual_counts_for_original_and_duplicate_operations()
-> Result<(), Box<dyn std::error::Error>> {
    let _guard = worker_test_guard().await;
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (pool, repository) = connected(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = ratatoskr_chatgpt_archive::BlobStore::new(root.path())?;
    let bytes = synthetic_zip();
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    let raw = blobs
        .store(
            MEDIA_TYPE,
            futures_util::stream::iter([Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(
                bytes.clone(),
            ))]),
        )
        .await?;
    let account_ref = format!("acc-worker-{}", Uuid::now_v7());
    let first_operation = Uuid::now_v7();
    let run_id = repository
        .create_run(&account_ref, &AcquisitionMode::ConsumerExport, MEDIA_TYPE)
        .await?;
    repository
        .record_hash(run_id, digest.clone(), bytes.len() as u64)
        .await?;
    let published = repository
        .publish_export(PublishRequest {
            run_id,
            account_external_ref: account_ref,
            mode: AcquisitionMode::ConsumerExport,
            blob_ref_json: serde_json::to_value(&raw)?,
            sha256_hex: digest,
            byte_length: bytes.len() as u64,
            platform_operation: Some(PlatformOperation {
                operation_id: first_operation,
            }),
        })
        .await?;
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events
         WHERE event_type = 'platform.operation.reported.v1' AND aggregate_id = $1",
    )
    .bind(first_operation)
    .fetch_one(&pool)
    .await?;
    assert_eq!(before, 0, "raw persistence must not be terminal");

    let worker = ratatoskr_chatgpt_archive::InitialImportWorker::new(
        pool.clone(),
        blobs.clone(),
        Arc::new(ratatoskr_chatgpt_archive::ParserRegistry::runtime()?),
        ratatoskr_chatgpt_archive::ArchiveLimits {
            max_entries: 32,
            max_compressed_bytes: 1_048_576,
            max_entry_bytes: 1_048_576,
            max_decompressed_bytes: 2_097_152,
            max_compression_ratio: 100,
        },
    );
    drive_until_operation_report(&worker, &pool, first_operation).await?;
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM chatgpt_archive.outbox_events
         WHERE event_type = 'platform.operation.reported.v1' AND aggregate_id = $1",
    )
    .bind(first_operation)
    .fetch_one(&pool)
    .await?;
    assert_eq!(payload["producer"], "ratatoskr-chatgpt");
    assert_eq!(payload["event_type"], "platform.operation.reported.v1");
    assert!(payload["event_id"].as_str().is_some());
    let summary = &payload["payload"]["results"][0]["ai_archive_import_summary"];
    assert_eq!(summary["conversation_count"], 2);
    assert_eq!(summary["message_count"], 3);

    let duplicate_operation = Uuid::now_v7();
    repository
        .bind_platform_operation(
            published.export_id,
            PlatformOperation {
                operation_id: duplicate_operation,
            },
        )
        .await?;
    drive_until_operation_report(&worker, &pool, duplicate_operation).await?;
    let reported: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events
         WHERE event_type = 'platform.operation.reported.v1'
           AND aggregate_id = ANY($1)",
    )
    .bind(vec![first_operation, duplicate_operation])
    .fetch_one(&pool)
    .await?;
    assert_eq!(reported, 2, "each Platform operation gets one result");
    Ok(())
}

/// A durable archive that can never parse reaches one safe terminal failure
/// instead of monopolizing the restart worker forever.
#[tokio::test]
async fn permanent_import_failure_is_terminal_and_reported_once()
-> Result<(), Box<dyn std::error::Error>> {
    let _guard = worker_test_guard().await;
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (pool, repository) = connected(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = ratatoskr_chatgpt_archive::BlobStore::new(root.path())?;
    let bytes = b"not a zip archive".to_vec();
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    let raw = blobs
        .store(
            MEDIA_TYPE,
            futures_util::stream::iter([Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(
                bytes.clone(),
            ))]),
        )
        .await?;
    let operation_id = Uuid::now_v7();
    let account_ref = format!("acc-failed-{}", Uuid::now_v7());
    let run_id = repository
        .create_run(&account_ref, &AcquisitionMode::ConsumerExport, MEDIA_TYPE)
        .await?;
    repository
        .record_hash(run_id, digest.clone(), bytes.len() as u64)
        .await?;
    repository
        .publish_export(PublishRequest {
            run_id,
            account_external_ref: account_ref,
            mode: AcquisitionMode::ConsumerExport,
            blob_ref_json: serde_json::to_value(&raw)?,
            sha256_hex: digest,
            byte_length: bytes.len() as u64,
            platform_operation: Some(PlatformOperation { operation_id }),
        })
        .await?;

    let worker = ratatoskr_chatgpt_archive::InitialImportWorker::new(
        pool.clone(),
        blobs,
        Arc::new(ratatoskr_chatgpt_archive::ParserRegistry::runtime()?),
        ratatoskr_chatgpt_archive::ArchiveLimits {
            max_entries: 32,
            max_compressed_bytes: 1_048_576,
            max_entry_bytes: 1_048_576,
            max_decompressed_bytes: 2_097_152,
            max_compression_ratio: 100,
        },
    );
    drive_until_operation_report(&worker, &pool, operation_id).await?;
    let state: String =
        sqlx::query_scalar("SELECT state FROM chatgpt_archive.import_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, "failed");
    let envelope: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM chatgpt_archive.outbox_events
         WHERE event_type = 'platform.operation.reported.v1' AND aggregate_id = $1",
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(envelope["payload"]["status"], "failed");
    assert_eq!(envelope["payload"]["error"]["retryable"], false);
    assert_eq!(
        envelope["payload"]["error"]["code"],
        "chatgpt.archive.invalid"
    );
    let report_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events
         WHERE event_type = 'platform.operation.reported.v1' AND aggregate_id = $1",
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(report_count, 1);
    Ok(())
}

#[tokio::test]
async fn completeness_counts_unobserved_archive_categories()
-> Result<(), Box<dyn std::error::Error>> {
    let _guard = worker_test_guard().await;
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (pool, repository) = connected(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = ratatoskr_chatgpt_archive::BlobStore::new(root.path())?;
    let bytes = minimal_known_zip();
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    let raw = blobs
        .store(
            MEDIA_TYPE,
            futures_util::stream::iter([Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(
                bytes.clone(),
            ))]),
        )
        .await?;
    let operation_id = Uuid::now_v7();
    let account_ref = format!("acc-completeness-{}", Uuid::now_v7());
    let run_id = repository
        .create_run(&account_ref, &AcquisitionMode::ConsumerExport, MEDIA_TYPE)
        .await?;
    repository
        .record_hash(run_id, digest.clone(), bytes.len() as u64)
        .await?;
    repository
        .publish_export(PublishRequest {
            run_id,
            account_external_ref: account_ref,
            mode: AcquisitionMode::ConsumerExport,
            blob_ref_json: serde_json::to_value(&raw)?,
            sha256_hex: digest,
            byte_length: bytes.len() as u64,
            platform_operation: Some(PlatformOperation { operation_id }),
        })
        .await?;
    let worker = ratatoskr_chatgpt_archive::InitialImportWorker::new(
        pool.clone(),
        blobs,
        Arc::new(ratatoskr_chatgpt_archive::ParserRegistry::runtime()?),
        ratatoskr_chatgpt_archive::ArchiveLimits {
            max_entries: 32,
            max_compressed_bytes: 1_048_576,
            max_entry_bytes: 1_048_576,
            max_decompressed_bytes: 2_097_152,
            max_compression_ratio: 100,
        },
    );
    drive_until_operation_report(&worker, &pool, operation_id).await?;
    let envelope: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM chatgpt_archive.outbox_events
         WHERE event_type='platform.operation.reported.v1' AND aggregate_id=$1",
    )
    .bind(operation_id)
    .fetch_one(&pool)
    .await?;
    let summary = &envelope["payload"]["results"][0]["ai_archive_import_summary"];
    assert_eq!(summary["completeness"], "structurally_partial");
    assert_eq!(summary["gap_count"], 3);
    Ok(())
}

/// Digest lookup scopes strictly by tenant account reference.
#[tokio::test]
async fn duplicate_lookup_scopes_by_account() -> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (pool, repository) = connected(&url).await?;

    // Seed two accounts and an export under the first directly.
    let account_a = Uuid::now_v7();
    let account_b = Uuid::now_v7();
    let owner_ref = format!("acc-scope-a-{}", Uuid::now_v7());
    let other_ref = format!("acc-scope-b-{}", Uuid::now_v7());
    for (id, reference) in [(account_a, &owner_ref), (account_b, &other_ref)] {
        sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
            .bind(id)
            .bind(reference)
            .execute(&pool)
            .await?;
    }
    let digest = "c".repeat(64);
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', '{}', $4, 5)")
        .bind(Uuid::now_v7())
        .bind(Uuid::now_v7())
        .bind(account_a)
        .bind(&digest)
        .execute(&pool)
        .await?;

    let mine = repository
        .find_export_by_digest(&owner_ref, &digest)
        .await?;
    assert!(mine.is_some(), "the owner finds its own export");
    let theirs = repository
        .find_export_by_digest(&other_ref, &digest)
        .await?;
    assert!(theirs.is_none(), "another tenant does not see it");
    Ok(())
}

/// Publishing records the export, links the run, and loses the duplicate
/// race explicitly.
#[tokio::test]
async fn record_export_persists_reference_digest_and_link() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let (_pool, repository) = connected(&url).await?;
    let account_ref = format!("acc-publish-{}", Uuid::now_v7());
    let run_id = repository
        .create_run(&account_ref, &AcquisitionMode::ConsumerExport, MEDIA_TYPE)
        .await?;
    let digest = "d".repeat(64);
    repository.record_hash(run_id, digest.clone(), 77).await?;

    let blob_ref = serde_json::json!({
        "owner_service": "ratatoskr-chatgpt",
        "digest": { "algorithm": "sha256", "hex": digest },
        "media_type": MEDIA_TYPE,
        "length_bytes": 77
    });
    let export = repository
        .publish_export(PublishRequest {
            run_id,
            account_external_ref: account_ref.clone(),
            mode: AcquisitionMode::ConsumerExport,
            blob_ref_json: blob_ref,
            sha256_hex: digest.clone(),
            byte_length: 77,
            platform_operation: None,
        })
        .await?;

    let stored = repository.load_run(run_id).await?.expect("run exists");
    assert_eq!(stored.state, ImportState::Stored);
    assert_eq!(stored.export_id, Some(export.export_id));

    // A second publish of the same tenant digest names the existing row.
    let second = repository
        .publish_export(PublishRequest {
            run_id,
            account_external_ref: account_ref,
            mode: AcquisitionMode::ConsumerExport,
            blob_ref_json: serde_json::json!({}),
            sha256_hex: digest,
            byte_length: 77,
            platform_operation: None,
        })
        .await;
    match second {
        Err(RepositoryError::DuplicateExisting { existing_export_id }) => {
            assert_eq!(existing_export_id, export.export_id);
        }
        other => panic!("expected an explicit duplicate outcome, got {other:?}"),
    }
    Ok(())
}
