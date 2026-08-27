//! Parser migration report and execution contracts.

#![allow(clippy::expect_used, reason = "synthetic ZIP construction")]

use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_chatgpt_archive::config::{Limits, StorageConfig};
use ratatoskr_chatgpt_archive::parser_migration::{
    ParserMigrationEngine, ParserMigrationEntry, ParserMigrationEntryStatus, ParserMigrationReport,
    ParserMigrationStatus,
};
use ratatoskr_chatgpt_archive::reparse::ReparseEngine;
use ratatoskr_chatgpt_archive::{
    AcquisitionMode, ArchiveLimits, BlobStore, Database, ParsedConversation, ParsedConversations,
    ParserExecutionError, ParserExecutionInput, ParserExecutor, ParserId, ParserRegistration,
    ParserRegistry,
};
use secrecy::SecretString;
use uuid::Uuid;

#[test]
fn migration_report_classifies_each_archive_once_and_derives_totals() {
    let operation = Uuid::now_v7();
    let tenant = Uuid::now_v7();
    let ids: Vec<_> = (0..6).map(|_| Uuid::now_v7()).collect();
    let entries = vec![
        entry(ids[5], ParserMigrationEntryStatus::PrivacyBlocked),
        entry(ids[2], ParserMigrationEntryStatus::Unsupported),
        entry(ids[0], ParserMigrationEntryStatus::Eligible),
        entry(ids[4], ParserMigrationEntryStatus::FailedInspection),
        entry(ids[1], ParserMigrationEntryStatus::AlreadyCurrent),
        entry(ids[3], ParserMigrationEntryStatus::RawMissing),
    ];
    let reverse = entries.iter().cloned().rev().collect();
    let parser = ParserId {
        name: "chatgpt-export".to_owned(),
        version: "2.0".to_owned(),
    };
    let forward = ParserMigrationReport::planned(operation, tenant, parser.clone(), entries);
    let backward = ParserMigrationReport::planned(operation, tenant, parser, reverse);
    assert_eq!(
        forward, backward,
        "database result order must not affect JSON"
    );
    assert!(
        forward
            .entries
            .windows(2)
            .all(|pair| pair[0].archive_id < pair[1].archive_id),
        "entries must be sorted by stable archive identity"
    );
    assert_eq!(forward.entries.len(), ids.len());
    for key in [
        "eligible",
        "already_current",
        "unsupported",
        "raw_missing",
        "privacy_blocked",
        "failed_inspection",
    ] {
        assert_eq!(forward.totals.get(key), Some(&1), "wrong total for {key}");
    }
    assert_eq!(
        forward.totals.values().sum::<usize>(),
        forward.entries.len(),
        "totals must be reduced only from entries"
    );
}

fn entry(archive_id: Uuid, status: ParserMigrationEntryStatus) -> ParserMigrationEntry {
    ParserMigrationEntry { archive_id, status }
}

#[derive(Debug)]
struct MigrationParser;

impl ParserExecutor for MigrationParser {
    fn execute(
        &self,
        _input: ParserExecutionInput<'_>,
    ) -> Result<ParsedConversations, ParserExecutionError> {
        Ok(ParsedConversations {
            schema_id: "chatgpt.synthetic.migration".to_owned(),
            parser: ParserId {
                name: "chatgpt-test".to_owned(),
                version: "2.0".to_owned(),
            },
            conversations: vec![ParsedConversation {
                external_id: "conversation-1".to_owned(),
                title: None,
                created_at_epoch_seconds: None,
                updated_at_epoch_seconds: None,
                provider_metadata: serde_json::json!({"shape":"2.0"}),
                messages: Vec::new(),
            }],
            projects: Vec::new(),
            canvas_documents: Vec::new(),
            assets: Vec::new(),
            raw_records: Vec::new(),
        })
    }
}

#[tokio::test]
async fn migration_apply_reports_partial_when_one_archive_fails()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let tenant_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("migration-{tenant_id}")).execute(database.pool()).await?;
    let first = seed_archive(&database, &blobs, tenant_id, b"first").await?;
    let second = seed_archive(&database, &blobs, tenant_id, b"second").await?;
    let mut registry = ParserRegistry::default();
    registry.register_compiled(
        ParserRegistration {
            id: ParserId {
                name: "chatgpt-test".to_owned(),
                version: "1.0".to_owned(),
            },
            modes: vec![AcquisitionMode::ConsumerExport],
            required_signals: BTreeSet::from(["conversations.json".to_owned()]),
        },
        Arc::new(MigrationParser),
    )?;
    registry.register_compiled(
        ParserRegistration {
            id: ParserId {
                name: "chatgpt-test".to_owned(),
                version: "2.0".to_owned(),
            },
            modes: vec![AcquisitionMode::ConsumerExport],
            required_signals: BTreeSet::from(["conversations.json".to_owned()]),
        },
        Arc::new(MigrationParser),
    )?;
    let reparse = ReparseEngine::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(registry),
        archive_limits(),
    );
    let engine = ParserMigrationEngine::new(reparse);
    let plan = engine
        .plan(
            Uuid::now_v7(),
            tenant_id,
            ParserId {
                name: "chatgpt-test".to_owned(),
                version: "2.0".to_owned(),
            },
        )
        .await?;
    assert_eq!(plan.report.totals.get("eligible"), Some(&2));
    blobs.erase(&second.raw).await?;
    let report = engine.apply(&plan).await?;
    assert_eq!(report.status, ParserMigrationStatus::Partial);
    assert_eq!(report.totals.get("applied"), Some(&1));
    assert_eq!(report.totals.get("failed"), Some(&1));
    let first_runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.reparse_runs WHERE export_id = $1",
    )
    .bind(first.export_id)
    .fetch_one(database.pool())
    .await?;
    assert_eq!(first_runs, 1, "successful archive must remain applied");
    Ok(())
}

struct SeededArchive {
    export_id: Uuid,
    raw: ratatoskr_identifiers::BlobRef,
}

async fn seed_archive(
    database: &Database,
    blobs: &BlobStore,
    tenant_id: Uuid,
    marker: &[u8],
) -> Result<SeededArchive, Box<dyn std::error::Error>> {
    let raw = blobs
        .store(
            "application/zip",
            stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from(zip_bytes(marker)))]),
        )
        .await?;
    let archive_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(archive_id).bind(tenant_id).bind(serde_json::to_value(&raw)?)
        .bind(raw.digest.hex.as_str()).bind(i64::try_from(raw.length_bytes)?)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.import_runs (id, export_id, parser_name, parser_version, schema_id, state) VALUES ($1, $2, 'chatgpt-test', '1.0', 'chatgpt.synthetic.migration', 'completed')")
        .bind(Uuid::now_v7()).bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, 'conversation-1')")
        .bind(conversation_id).bind(tenant_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)")
        .bind(export_id).bind(conversation_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.revisions (id, entity_table, entity_id, revision_number, observed_in, payload) VALUES ($1, 'conversations', $2, 1, $3, $4)")
        .bind(Uuid::now_v7()).bind(conversation_id).bind(export_id)
        .bind(serde_json::json!({"digest":"old-digest"})).execute(database.pool()).await?;
    Ok(SeededArchive { export_id, raw })
}

fn zip_bytes(marker: &[u8]) -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file(
            "conversations.json",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("synthetic entry starts");
    writer.write_all(marker).expect("synthetic entry writes");
    writer
        .finish()
        .expect("synthetic zip finishes")
        .into_inner()
}

fn test_url() -> Option<String> {
    #[allow(clippy::disallowed_methods, reason = "integration database URL")]
    std::env::var("CHATGPT_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
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

fn archive_limits() -> ArchiveLimits {
    ArchiveLimits {
        max_entries: 32,
        max_compressed_bytes: 1_048_576,
        max_entry_bytes: 1_048_576,
        max_decompressed_bytes: 2_097_152,
        max_compression_ratio: 100,
    }
}
