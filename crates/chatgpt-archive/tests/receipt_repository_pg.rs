//! Contract tests for the `PostgreSQL` receipt repository.
//!
//! They skip unless `CHATGPT_TEST_DATABASE_URL` is set; CI always sets it.
//! These prove the SQL adapter against a real server; the receiver logic
//! above the seam is proven by `receipt_receiver.rs` through the fake.

// Test bodies fail through `panic!`; assertions are the contract.
#![allow(clippy::panic, reason = "test failures report through panics")]

use secrecy::SecretString;
use uuid::Uuid;

use ratatoskr_chatgpt_archive::receipt::AcquisitionMode;
use ratatoskr_chatgpt_archive::receipt::pg::PostgresReceiptRepository;
use ratatoskr_chatgpt_archive::receipt::repository::{ReceiptRepository as _, RepositoryError};
use ratatoskr_chatgpt_archive::receipt::state::ImportState;

const MEDIA_TYPE: &str = "application/zip";

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
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, 'consumer_export', '{}', $3, 5)")
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
    let export_id = repository
        .publish_export(
            run_id,
            &account_ref,
            &AcquisitionMode::ConsumerExport,
            blob_ref,
            digest.clone(),
            77,
        )
        .await?;

    let stored = repository.load_run(run_id).await?.expect("run exists");
    assert_eq!(stored.state, ImportState::Stored);
    assert_eq!(stored.export_id, Some(export_id));

    // A second publish of the same tenant digest names the existing row.
    let second = repository
        .publish_export(
            run_id,
            &account_ref,
            &AcquisitionMode::ConsumerExport,
            serde_json::json!({}),
            digest,
            77,
        )
        .await;
    match second {
        Err(RepositoryError::DuplicateExisting { existing_export_id }) => {
            assert_eq!(existing_export_id, export_id);
        }
        other => panic!("expected an explicit duplicate outcome, got {other:?}"),
    }
    Ok(())
}
