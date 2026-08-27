//! Reparse planning and apply integration contracts.

#![allow(clippy::expect_used, reason = "synthetic ZIP construction")]

use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_chatgpt_archive::config::{Limits, StorageConfig};
use ratatoskr_chatgpt_archive::reparse::{ReparseChangeKind, ReparseEngine};
use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveLimits, BlobStore, Database, ParsedConversation, ParsedConversations,
    ParserExecutionError, ParserExecutionInput, ParserExecutor, ParserId, ParserRegistration,
    ParserRegistry,
};
use secrecy::SecretString;
use uuid::Uuid;

fn test_url() -> Option<String> {
    #[allow(clippy::disallowed_methods, reason = "integration database URL")]
    let url = std::env::var("CHATGPT_TEST_DATABASE_URL").ok();
    url.filter(|url| !url.trim().is_empty())
}

async fn database(url: &str) -> Result<Database, Box<dyn std::error::Error>> {
    let storage = StorageConfig {
        blob_root: None,
        database_url: Some(SecretString::from(url.to_owned())),
        receipt_staging_root: None,
    };
    let limits = Limits {
        database_connections: 2,
        database_acquire_timeout_ms: 5_000,
        shutdown_timeout_ms: 5_000,
        max_archive_bytes: 1_048_576,
        max_archive_entries: 32,
        max_archive_entry_bytes: 1_048_576,
        max_archive_decompressed_bytes: 2_097_152,
        max_archive_compression_ratio: 100,
    };
    let database = Database::connect(&storage, &limits).await?;
    database.apply_schema().await?;
    Ok(database)
}

fn limits() -> ArchiveLimits {
    ArchiveLimits {
        max_entries: 32,
        max_compressed_bytes: 1_048_576,
        max_entry_bytes: 1_048_576,
        max_decompressed_bytes: 2_097_152,
        max_compression_ratio: 100,
    }
}

fn zip_bytes() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file(
            "conversations.json",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("synthetic entry starts");
    writer.write_all(b"[]").expect("synthetic entry writes");
    writer
        .finish()
        .expect("synthetic zip finishes")
        .into_inner()
}

fn conversation(version: &str) -> ParsedConversations {
    ParsedConversations {
        schema_id: "chatgpt.synthetic.reparse".to_owned(),
        parser: ParserId {
            name: "chatgpt-test".to_owned(),
            version: version.to_owned(),
        },
        conversations: vec![ParsedConversation {
            external_id: "conversation-1".to_owned(),
            title: None,
            created_at_epoch_seconds: None,
            updated_at_epoch_seconds: None,
            provider_metadata: serde_json::json!({"shape": version}),
            messages: Vec::new(),
        }],
        projects: Vec::new(),
        canvas_documents: Vec::new(),
        assets: Vec::new(),
        raw_records: Vec::new(),
    }
}

#[derive(Debug)]
struct FixedParser {
    version: &'static str,
}

impl ParserExecutor for FixedParser {
    fn execute(
        &self,
        _input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        Ok(conversation(self.version))
    }
}

#[derive(Debug)]
struct OmittingParser;

impl ParserExecutor for OmittingParser {
    fn execute(
        &self,
        _input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        let mut parsed = conversation("2.0");
        parsed.conversations.clear();
        Ok(parsed)
    }
}

fn registration(version: &str) -> ParserRegistration {
    ParserRegistration {
        id: ParserId {
            name: "chatgpt-test".to_owned(),
            version: version.to_owned(),
        },
        modes: vec![AcquisitionMode::ConsumerExport],
        required_signals: BTreeSet::from(["conversations.json".to_owned()]),
    }
}

fn count_files(path: &std::path::Path) -> usize {
    std::fs::read_dir(path).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| {
                if entry.path().is_dir() {
                    count_files(&entry.path())
                } else {
                    1
                }
            })
            .sum()
    })
}

async fn archive_row_counts(
    database: &Database,
    export_id: Uuid,
) -> Result<(i64, i64, i64, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT
          (SELECT count(*) FROM chatgpt_archive.reparse_runs WHERE export_id = $1),
          (SELECT count(*) FROM chatgpt_archive.revisions WHERE observed_in = $1),
          (SELECT count(*) FROM chatgpt_archive.extracted_artifacts WHERE export_id = $1),
          (SELECT count(*) FROM chatgpt_archive.outbox_events WHERE export_id = $1)",
    )
    .bind(export_id)
    .fetch_one(database.pool())
    .await
}

#[tokio::test]
async fn reparse_dry_run_matches_immediate_apply_without_writes()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw_bytes = zip_bytes();
    let raw = blobs
        .store(
            "application/zip",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from(raw_bytes))]),
        )
        .await?;
    let tenant_id = Uuid::now_v7();
    let archive_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("reparse-{tenant_id}")).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(archive_id).bind(tenant_id).bind(serde_json::to_value(&raw)?)
        .bind(raw.digest.hex.as_str()).bind(i64::try_from(raw.length_bytes)?)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.import_runs (id, export_id, parser_name, parser_version, schema_id, state) VALUES ($1, $2, 'chatgpt-test', '1.0', 'chatgpt.synthetic.reparse', 'completed')")
        .bind(Uuid::now_v7()).bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, 'conversation-1')")
        .bind(conversation_id).bind(tenant_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)")
        .bind(export_id).bind(conversation_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.revisions (id, entity_table, entity_id, revision_number, observed_in, payload) VALUES ($1, 'conversations', $2, 1, $3, $4)")
        .bind(Uuid::now_v7()).bind(conversation_id).bind(export_id)
        .bind(serde_json::json!({"digest":"old-digest"})).execute(database.pool()).await?;

    let mut registry = ParserRegistry::default();
    registry.register_compiled(
        registration("1.0"),
        Arc::new(FixedParser { version: "1.0" }),
    )?;
    registry.register_compiled(
        registration("2.0"),
        Arc::new(FixedParser { version: "2.0" }),
    )?;
    let engine = ReparseEngine::new(database.pool().clone(), blobs, Arc::new(registry), limits());
    let before_rows = archive_row_counts(&database, export_id).await?;
    let before_files = count_files(root.path());
    let plan = engine
        .plan(
            tenant_id,
            archive_id,
            ParserId {
                name: "chatgpt-test".to_owned(),
                version: "2.0".to_owned(),
            },
        )
        .await?;
    let after_dry_rows = archive_row_counts(&database, export_id).await?;
    assert_eq!(before_rows, after_dry_rows, "dry-run must write no row");
    assert_eq!(
        before_files,
        count_files(root.path()),
        "dry-run must write no blob"
    );
    let applied = engine.apply(&plan).await?;
    assert_eq!(
        plan.report, applied,
        "dry and immediate apply reports must match"
    );
    assert_eq!(
        applied
            .changes
            .iter()
            .filter(|change| change.kind == ReparseChangeKind::Changed)
            .count(),
        1,
        "newer parser must report the changed conversation"
    );
    Ok(())
}

#[tokio::test]
async fn reparse_apply_is_idempotent_for_same_fingerprints()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw = blobs
        .store(
            "application/zip",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from(zip_bytes()))]),
        )
        .await?;
    let tenant_id = Uuid::now_v7();
    let archive_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("idempotent-{tenant_id}")).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(archive_id).bind(tenant_id).bind(serde_json::to_value(&raw)?)
        .bind(raw.digest.hex.as_str()).bind(i64::try_from(raw.length_bytes)?)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.import_runs (id, export_id, parser_name, parser_version, schema_id, state) VALUES ($1, $2, 'chatgpt-test', '1.0', 'chatgpt.synthetic.reparse', 'completed')")
        .bind(Uuid::now_v7()).bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, 'conversation-1')")
        .bind(conversation_id).bind(tenant_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)")
        .bind(export_id).bind(conversation_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.revisions (id, entity_table, entity_id, revision_number, observed_in, payload) VALUES ($1, 'conversations', $2, 1, $3, $4)")
        .bind(Uuid::now_v7()).bind(conversation_id).bind(export_id)
        .bind(serde_json::json!({"digest":"old-digest"})).execute(database.pool()).await?;
    let mut registry = ParserRegistry::default();
    registry.register_compiled(
        registration("1.0"),
        Arc::new(FixedParser { version: "1.0" }),
    )?;
    registry.register_compiled(
        registration("2.0"),
        Arc::new(FixedParser { version: "2.0" }),
    )?;
    let engine = ReparseEngine::new(database.pool().clone(), blobs, Arc::new(registry), limits());
    let plan = engine
        .plan(
            tenant_id,
            archive_id,
            ParserId {
                name: "chatgpt-test".to_owned(),
                version: "2.0".to_owned(),
            },
        )
        .await?;
    let first = engine.apply(&plan).await?;
    let before: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
          (SELECT count(*) FROM chatgpt_archive.reparse_runs WHERE export_id = $1),
          (SELECT count(*) FROM chatgpt_archive.revisions WHERE entity_id = $2),
          (SELECT count(*) FROM chatgpt_archive.extracted_artifacts WHERE export_id = $1),
          (SELECT count(*) FROM chatgpt_archive.outbox_events WHERE export_id = $1)",
    )
    .bind(export_id)
    .bind(conversation_id)
    .fetch_one(database.pool())
    .await?;
    let replay = engine.apply(&plan).await;
    assert_eq!(
        replay.as_ref().ok(),
        Some(&first),
        "same immutable fingerprints must return the original report: {replay:?}"
    );
    let after: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
          (SELECT count(*) FROM chatgpt_archive.reparse_runs WHERE export_id = $1),
          (SELECT count(*) FROM chatgpt_archive.revisions WHERE entity_id = $2),
          (SELECT count(*) FROM chatgpt_archive.extracted_artifacts WHERE export_id = $1),
          (SELECT count(*) FROM chatgpt_archive.outbox_events WHERE export_id = $1)",
    )
    .bind(export_id)
    .bind(conversation_id)
    .fetch_one(database.pool())
    .await?;
    assert_eq!(before, after, "replay must create no evidence or event");
    Ok(())
}

#[tokio::test]
async fn reparse_omission_retains_existing_subject_with_warning()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw = blobs
        .store(
            "application/zip",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from(zip_bytes()))]),
        )
        .await?;
    let tenant_id = Uuid::now_v7();
    let archive_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("omission-{tenant_id}")).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(archive_id).bind(tenant_id).bind(serde_json::to_value(&raw)?)
        .bind(raw.digest.hex.as_str()).bind(i64::try_from(raw.length_bytes)?)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.import_runs (id, export_id, parser_name, parser_version, schema_id, state) VALUES ($1, $2, 'chatgpt-test', '1.0', 'chatgpt.synthetic.reparse', 'completed')")
        .bind(Uuid::now_v7()).bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, 'conversation-1')")
        .bind(conversation_id).bind(tenant_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)")
        .bind(export_id).bind(conversation_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.revisions (id, entity_table, entity_id, revision_number, observed_in, payload) VALUES ($1, 'conversations', $2, 1, $3, $4)")
        .bind(Uuid::now_v7()).bind(conversation_id).bind(export_id)
        .bind(serde_json::json!({"digest":"old-digest"})).execute(database.pool()).await?;
    let mut registry = ParserRegistry::default();
    registry.register_compiled(
        registration("1.0"),
        Arc::new(FixedParser { version: "1.0" }),
    )?;
    registry.register_compiled(registration("2.0"), Arc::new(OmittingParser))?;
    let engine = ReparseEngine::new(database.pool().clone(), blobs, Arc::new(registry), limits());
    let plan = engine
        .plan(
            tenant_id,
            archive_id,
            ParserId {
                name: "chatgpt-test".to_owned(),
                version: "2.0".to_owned(),
            },
        )
        .await?;
    assert!(
        plan.report.changes.iter().any(|change| {
            change.subject_id == "conversation-1"
                && change.kind == ReparseChangeKind::ProposedRemoval
        }),
        "an omission must be classified without becoming authoritative deletion"
    );
    assert!(
        plan.report
            .warnings
            .iter()
            .any(|warning| warning.code == "coverage_omission"),
        "coverage regression must be explicit"
    );
    assert!(
        !plan
            .report
            .event_subjects
            .iter()
            .any(|subject| subject.contains("tombstone")),
        "reparse must never predict a deletion event"
    );
    engine.apply(&plan).await?;
    let retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.conversations WHERE id = $1)",
    )
    .bind(conversation_id)
    .fetch_one(database.pool())
    .await?;
    let deletion_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events
         WHERE export_id = $1 AND event_type = 'ai_archive.subject.tombstoned.v1'",
    )
    .bind(export_id)
    .fetch_one(database.pool())
    .await?;
    assert!(
        retained,
        "omitted normalized evidence must remain available"
    );
    assert_eq!(deletion_events, 0, "omission must not emit a tombstone");
    Ok(())
}
