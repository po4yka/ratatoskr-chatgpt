//! `PostgreSQL` 17 integration tests for inventory-first privacy deletion.

use std::collections::BTreeMap;

use bytes::Bytes;
use futures_util::stream;
use ratatoskr_chatgpt_archive::config::{Limits, StorageConfig};
use ratatoskr_chatgpt_archive::privacy_deletion::{
    DeletionAction, FinalizationFault, PrivacyDeletionScope, PrivacyDeletionService,
};
use ratatoskr_chatgpt_archive::{BlobStore, Database};
use secrecy::SecretString;
use uuid::Uuid;

fn test_url() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "the integration harness reads its PostgreSQL URL"
    )]
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
        max_archive_bytes: 17_179_869_184,
        max_archive_entries: 10_000,
        max_archive_entry_bytes: 2_147_483_648,
        max_archive_decompressed_bytes: 34_359_738_368,
        max_archive_compression_ratio: 100,
    };
    let database = Database::connect(&storage, &limits).await?;
    database.apply_schema().await?;
    Ok(database)
}

fn bytes(
    value: &'static [u8],
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Unpin + Send + 'static {
    stream::iter([Ok(Bytes::from_static(value))])
}

async fn insert_export_row(
    database: &Database,
    tenant_id: Uuid,
    archive_id: Uuid,
    reference: &ratatoskr_identifiers::BlobRef,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let export_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(archive_id).bind(tenant_id).bind(serde_json::to_value(reference)?)
        .bind(reference.digest.hex.as_str()).bind(i64::try_from(reference.length_bytes)?)
        .execute(database.pool()).await?;
    Ok(export_id)
}

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "complete seeded inventory assertion")]
async fn deletion_inventory_enumerates_complete_scope() -> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw = blobs
        .store("application/zip", bytes(b"complete raw archive"))
        .await?;
    let extracted = blobs
        .store("application/json", bytes(b"extracted evidence"))
        .await?;
    let part_blob = blobs
        .store("application/octet-stream", bytes(b"content part bytes"))
        .await?;
    let asset_blob = blobs
        .store("application/octet-stream", bytes(b"asset bytes"))
        .await?;

    let tenant_id = Uuid::now_v7();
    let export_id = Uuid::now_v7();
    let archive_id = Uuid::now_v7();
    let run_id = Uuid::now_v7();
    let report_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let conversation_id = Uuid::now_v7();
    let parent_id = Uuid::now_v7();
    let child_id = Uuid::now_v7();
    let relation_id = Uuid::now_v7();
    let part_id = Uuid::now_v7();
    let asset_id = Uuid::now_v7();
    let revision_id = Uuid::now_v7();
    let raw_record_id = Uuid::now_v7();
    let artifact_id = Uuid::now_v7();

    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("inventory-{tenant_id}")).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(archive_id).bind(tenant_id).bind(serde_json::to_value(&raw)?)
        .bind(raw.digest.hex.as_str()).bind(i64::try_from(raw.length_bytes)?)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.import_runs (id, export_id, state) VALUES ($1, $2, 'completed')")
        .bind(run_id).bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.completeness_reports (id, import_run_id, status) VALUES ($1, $2, 'structurally_partial')")
        .bind(report_id).bind(run_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.projects (id, account_id, external_id, title, first_seen_export, last_seen_export) VALUES ($1, $2, $3, 'private title', $4, $4)")
        .bind(project_id).bind(tenant_id).bind(format!("project-{project_id}")).bind(export_id)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, project_id, account_id, external_id, title, first_seen_export, last_seen_export) VALUES ($1, $2, $3, $4, 'private title', $5, $5)")
        .bind(conversation_id).bind(project_id).bind(tenant_id).bind(format!("conversation-{conversation_id}"))
        .bind(export_id).execute(database.pool()).await?;
    for (message_id, external_id, parent) in [
        (parent_id, "parent", None),
        (child_id, "child", Some(parent_id)),
    ] {
        sqlx::query("INSERT INTO chatgpt_archive.messages (id, conversation_id, external_id, parent_message_id, role) VALUES ($1, $2, $3, $4, 'user')")
            .bind(message_id).bind(conversation_id).bind(external_id).bind(parent)
            .execute(database.pool()).await?;
    }
    sqlx::query("INSERT INTO chatgpt_archive.message_relations (id, from_message_id, to_message_id, relation_kind, observed_in_export) VALUES ($1, $2, $3, 'continues', $4)")
        .bind(relation_id).bind(child_id).bind(parent_id).bind(export_id)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.content_parts (id, message_id, ordinal, part_kind, payload, blob_ref) VALUES ($1, $2, 0, 'file', $3, $4)")
        .bind(part_id).bind(child_id).bind(serde_json::json!({"text":"private"}))
        .bind(serde_json::to_value(&part_blob)?).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.assets (id, external_id, asset_kind, blob_ref, locally_backed_up, observed_in) VALUES ($1, $2, 'uploaded_file', $3, TRUE, $4)")
        .bind(asset_id).bind(format!("asset-{asset_id}")).bind(serde_json::to_value(&asset_blob)?)
        .bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.revisions (id, entity_table, entity_id, revision_number, observed_in, payload) VALUES ($1, 'conversations', $2, 1, $3, $4)")
        .bind(revision_id).bind(conversation_id).bind(export_id)
        .bind(serde_json::json!({"private":"content"})).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.raw_records (id, export_id, record_path, payload) VALUES ($1, $2, '/private', $3)")
        .bind(raw_record_id).bind(export_id).bind(serde_json::json!({"private":"content"}))
        .execute(database.pool()).await?;
    for (kind, entity) in [
        ("project", project_id),
        ("conversation", conversation_id),
        ("message", parent_id),
        ("asset", asset_id),
    ] {
        sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, $2, $3)")
            .bind(export_id).bind(kind).bind(entity).execute(database.pool()).await?;
    }
    sqlx::query("INSERT INTO chatgpt_archive.extracted_artifacts (id, export_id, artifact_ordinal, artifact_kind, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, 0, 'entry', $3, $4, $5)")
        .bind(artifact_id).bind(export_id).bind(serde_json::to_value(&extracted)?)
        .bind(extracted.digest.hex.as_str()).bind(i64::try_from(extracted.length_bytes)?)
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.inbox_events (source, event_type, event_key, tenant_id, export_id, payload) VALUES ('fixture', 'fixture.received.v1', $1, $2, $3, $4)")
        .bind(format!("inbox-{tenant_id}")).bind(tenant_id).bind(export_id).bind(serde_json::json!({"private":"content"}))
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.outbox_events (event_type, aggregate_id, tenant_id, export_id, payload) VALUES ('ai_archive.conversation.added.v1', $1, $2, $3, $4)")
        .bind(conversation_id).bind(tenant_id).bind(export_id).bind(serde_json::json!({"private":"content"}))
        .execute(database.pool()).await?;

    let request_id = Uuid::now_v7();
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs);
    let plan = service
        .plan(
            tenant_id,
            request_id,
            PrivacyDeletionScope::Archive {
                ai_archive_id: archive_id,
            },
        )
        .await?
        .expect("owned archive must produce a plan");

    let expected = BTreeMap::from([
        ("asset".to_owned(), 1),
        ("asset_blob".to_owned(), 1),
        ("completeness_report".to_owned(), 1),
        ("content_part".to_owned(), 1),
        ("content_part_blob".to_owned(), 1),
        ("conversation".to_owned(), 1),
        ("downstream_tombstone".to_owned(), 3),
        ("export".to_owned(), 1),
        ("export_observation".to_owned(), 4),
        ("extracted_artifact".to_owned(), 1),
        ("extracted_artifact_blob".to_owned(), 1),
        ("import_run".to_owned(), 1),
        ("inbox_event".to_owned(), 1),
        ("message".to_owned(), 2),
        ("message_relation".to_owned(), 1),
        ("outbox_event".to_owned(), 1),
        ("project".to_owned(), 1),
        ("raw_archive_blob".to_owned(), 1),
        ("raw_record".to_owned(), 1),
        ("revision".to_owned(), 1),
    ]);
    assert_eq!(plan.totals, expected, "every seeded category is enumerated");
    let total_items = plan.totals.values().try_fold(
        0_usize,
        |total, count| -> Result<usize, std::num::TryFromIntError> {
            Ok(total + usize::try_from(*count)?)
        },
    )?;
    assert_eq!(
        plan.items.len(),
        total_items,
        "totals must be derived from itemized actions"
    );
    let persisted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.privacy_deletion_items WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_one(database.pool())
    .await?;
    assert_eq!(usize::try_from(persisted)?, plan.items.len());
    Ok(())
}

#[tokio::test]
async fn deletion_scope_does_not_disclose_cross_tenant_subjects()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw = blobs
        .store("application/zip", bytes(b"foreign scope raw"))
        .await?;
    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    for tenant in [tenant_a, tenant_b] {
        sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
            .bind(tenant).bind(format!("scope-{tenant}")).execute(database.pool()).await?;
    }
    let export_id = Uuid::now_v7();
    let foreign_archive = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.exports (id, ai_archive_id, account_id, acquisition_mode, blob_ref, sha256_hex, byte_length) VALUES ($1, $2, $3, 'consumer_export', $4, $5, $6)")
        .bind(export_id).bind(foreign_archive).bind(tenant_b).bind(serde_json::to_value(&raw)?)
        .bind(raw.digest.hex.as_str()).bind(i64::try_from(raw.length_bytes)?)
        .execute(database.pool()).await?;

    let service = PrivacyDeletionService::new(database.pool().clone(), blobs);
    let colliding_request = Uuid::now_v7();
    service
        .plan(
            tenant_b,
            colliding_request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: foreign_archive,
            },
        )
        .await?
        .expect("owner can plan its archive");

    let foreign = service
        .plan(
            tenant_a,
            colliding_request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: foreign_archive,
            },
        )
        .await;
    let unknown = service
        .plan(
            tenant_a,
            Uuid::now_v7(),
            PrivacyDeletionScope::Archive {
                ai_archive_id: Uuid::now_v7(),
            },
        )
        .await;
    assert!(
        matches!(foreign, Ok(None)) && matches!(unknown, Ok(None)),
        "foreign and unknown subjects must share one non-disclosing result: foreign={foreign:?}, unknown={unknown:?}"
    );
    let tenant_a_requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.privacy_deletion_requests WHERE tenant_id = $1",
    )
    .bind(tenant_a)
    .fetch_one(database.pool())
    .await?;
    assert_eq!(
        tenant_a_requests, 0,
        "refused scopes must create no request"
    );
    Ok(())
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "conversation-scope provenance and collateral assertion"
)]
async fn conversation_plan_includes_containing_archives_and_only_unprovenanced_collateral()
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
        .bind(tenant_id).bind(format!("conversation-scope-{tenant_id}"))
        .execute(database.pool()).await?;

    let first_raw = blobs
        .store("application/zip", bytes(b"first containing raw"))
        .await?;
    let second_raw = blobs
        .store("application/zip", bytes(b"second containing raw"))
        .await?;
    let retained_raw = blobs
        .store("application/zip", bytes(b"retained raw"))
        .await?;
    let first_export = insert_export_row(&database, tenant_id, Uuid::now_v7(), &first_raw).await?;
    let second_export =
        insert_export_row(&database, tenant_id, Uuid::now_v7(), &second_raw).await?;
    let retained_export =
        insert_export_row(&database, tenant_id, Uuid::now_v7(), &retained_raw).await?;

    let target = Uuid::now_v7();
    let retained_sibling = Uuid::now_v7();
    let lost_sibling = Uuid::now_v7();
    for conversation in [target, retained_sibling, lost_sibling] {
        sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, $3)")
            .bind(conversation).bind(tenant_id).bind(format!("conversation-{conversation}"))
            .execute(database.pool()).await?;
    }
    let target_message = Uuid::now_v7();
    let retained_message = Uuid::now_v7();
    let lost_message = Uuid::now_v7();
    for (message, conversation) in [
        (target_message, target),
        (retained_message, retained_sibling),
        (lost_message, lost_sibling),
    ] {
        sqlx::query("INSERT INTO chatgpt_archive.messages (id, conversation_id, external_id, role) VALUES ($1, $2, $3, 'user')")
            .bind(message).bind(conversation).bind(format!("message-{message}"))
            .execute(database.pool()).await?;
    }
    for (export, kind, entity) in [
        (first_export, "conversation", target),
        (first_export, "message", target_message),
        (first_export, "conversation", retained_sibling),
        (first_export, "message", retained_message),
        (second_export, "conversation", target),
        (second_export, "message", target_message),
        (second_export, "conversation", lost_sibling),
        (second_export, "message", lost_message),
        (retained_export, "conversation", retained_sibling),
        (retained_export, "message", retained_message),
    ] {
        sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, $2, $3)")
            .bind(export).bind(kind).bind(entity).execute(database.pool()).await?;
    }
    let retained_part = Uuid::now_v7();
    let retained_part_blob = blobs
        .store("application/octet-stream", bytes(b"retained sibling bytes"))
        .await?;
    sqlx::query("INSERT INTO chatgpt_archive.content_parts (id, message_id, ordinal, part_kind, payload, blob_ref) VALUES ($1, $2, 0, 'file', '{}', $3)")
        .bind(retained_part).bind(retained_message).bind(serde_json::to_value(&retained_part_blob)?)
        .execute(database.pool()).await?;

    let service = PrivacyDeletionService::new(database.pool().clone(), blobs);
    let plan = service
        .plan(
            tenant_id,
            Uuid::now_v7(),
            PrivacyDeletionScope::Conversation {
                conversation_id: target,
            },
        )
        .await?
        .expect("owned conversation plans");
    let action = |category: &str, id: Uuid| {
        plan.items
            .iter()
            .find(|item| item.category == category && item.opaque_id == id.to_string())
            .map(|item| item.action)
    };
    assert_eq!(action("export", first_export), Some(DeletionAction::Remove));
    assert_eq!(
        action("export", second_export),
        Some(DeletionAction::Remove)
    );
    assert_eq!(action("export", retained_export), None);
    assert_eq!(action("conversation", target), Some(DeletionAction::Remove));
    assert_eq!(
        action("conversation", retained_sibling),
        Some(DeletionAction::RetainEvidenced)
    );
    assert_eq!(
        action("conversation", lost_sibling),
        Some(DeletionAction::Remove)
    );
    assert_eq!(
        action("content_part", retained_part),
        Some(DeletionAction::RetainEvidenced)
    );
    assert_eq!(
        action("content_part_blob", retained_part),
        Some(DeletionAction::RetainShared),
        "bytes backing retained normalized evidence must not be erased"
    );
    Ok(())
}

#[tokio::test]
async fn deletion_finalization_is_atomic_with_audit_and_tombstones()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw = blobs.store("application/zip", bytes(b"atomic raw")).await?;
    let tenant_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("atomic-{tenant_id}"))
        .execute(database.pool()).await?;
    let archive_id = Uuid::now_v7();
    let export_id = insert_export_row(&database, tenant_id, archive_id, &raw).await?;
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, $3)")
        .bind(conversation_id).bind(tenant_id).bind(format!("atomic-{conversation_id}"))
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)")
        .bind(export_id).bind(conversation_id).execute(database.pool()).await?;
    let raw_record_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.raw_records (id, export_id, record_path, payload) VALUES ($1, $2, '/atomic', '{}')")
        .bind(raw_record_id).bind(export_id).execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.outbox_events (event_type, aggregate_id, tenant_id, export_id, payload) VALUES ('ai_archive.conversation.added.v1', $1, $2, $3, '{}')")
        .bind(conversation_id).bind(tenant_id).bind(export_id).execute(database.pool()).await?;

    let request_id = Uuid::now_v7();
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs);
    service
        .plan(
            tenant_id,
            request_id,
            PrivacyDeletionScope::Archive {
                ai_archive_id: archive_id,
            },
        )
        .await?
        .expect("owned archive plans");
    let outcome = service
        .execute_with_fault(request_id, FinalizationFault::AfterFirstRemoval)
        .await;
    assert!(outcome.is_err(), "the injected finalization must fail");

    let raw_present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.raw_records WHERE id = $1)",
    )
    .bind(raw_record_id)
    .fetch_one(database.pool())
    .await?;
    let conversation_present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chatgpt_archive.conversations WHERE id = $1)",
    )
    .bind(conversation_id)
    .fetch_one(database.pool())
    .await?;
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.privacy_deletion_audits WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_one(database.pool())
    .await?;
    let tombstones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chatgpt_archive.outbox_events
         WHERE deduplication_key LIKE $1",
    )
    .bind(format!("privacy-delete:{request_id}:%"))
    .fetch_one(database.pool())
    .await?;
    let status: String = sqlx::query_scalar(
        "SELECT status FROM chatgpt_archive.privacy_deletion_requests WHERE id = $1",
    )
    .bind(request_id)
    .fetch_one(database.pool())
    .await?;
    assert_eq!(
        (
            raw_present,
            conversation_present,
            audits,
            tombstones,
            status.as_str()
        ),
        (true, true, 0, 0, "planned"),
        "row removal, audit, tombstones, and terminal state must share one transaction"
    );
    Ok(())
}

#[tokio::test]
async fn completed_deletion_replay_returns_original_report()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let raw = blobs.store("application/zip", bytes(b"replay raw")).await?;
    let tenant_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
        .bind(tenant_id).bind(format!("replay-{tenant_id}"))
        .execute(database.pool()).await?;
    let archive_id = Uuid::now_v7();
    let export_id = insert_export_row(&database, tenant_id, archive_id, &raw).await?;
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO chatgpt_archive.conversations (id, account_id, external_id) VALUES ($1, $2, $3)")
        .bind(conversation_id).bind(tenant_id).bind(format!("replay-{conversation_id}"))
        .execute(database.pool()).await?;
    sqlx::query("INSERT INTO chatgpt_archive.export_entity_observations (export_id, entity_kind, entity_id) VALUES ($1, 'conversation', $2)")
        .bind(export_id).bind(conversation_id).execute(database.pool()).await?;

    let request_id = Uuid::now_v7();
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs);
    service
        .plan(
            tenant_id,
            request_id,
            PrivacyDeletionScope::Archive {
                ai_archive_id: archive_id,
            },
        )
        .await?
        .expect("owned archive plans");
    let first = service.execute(request_id).await?;
    let before: (i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM chatgpt_archive.privacy_deletion_audits WHERE request_id = $1),
           (SELECT count(*) FROM chatgpt_archive.outbox_events WHERE deduplication_key LIKE $2)",
    )
    .bind(request_id)
    .bind(format!("privacy-delete:{request_id}:%"))
    .fetch_one(database.pool())
    .await?;
    let replay = service.execute(request_id).await;
    assert_eq!(
        replay.as_ref().ok(),
        Some(&first),
        "completed replay must return its durable original report: {replay:?}"
    );
    let after: (i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM chatgpt_archive.privacy_deletion_audits WHERE request_id = $1),
           (SELECT count(*) FROM chatgpt_archive.outbox_events WHERE deduplication_key LIKE $2)",
    )
    .bind(request_id)
    .bind(format!("privacy-delete:{request_id}:%"))
    .fetch_one(database.pool())
    .await?;
    assert_eq!(before, after, "replay must add no audit or tombstone");
    Ok(())
}

#[tokio::test]
async fn tenant_deletion_retains_blob_referenced_by_another_tenant()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(url) = test_url() else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };
    let database = database(&url).await?;
    let root = tempfile::tempdir()?;
    let blobs = BlobStore::new(root.path())?;
    let shared = blobs
        .store("application/zip", bytes(b"byte-identical tenant archives"))
        .await?;
    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    for tenant in [tenant_a, tenant_b] {
        sqlx::query("INSERT INTO chatgpt_archive.accounts (id, external_kind, external_ref) VALUES ($1, 'personal', $2)")
            .bind(tenant).bind(format!("shared-{tenant}"))
            .execute(database.pool()).await?;
    }
    let export_a = insert_export_row(&database, tenant_a, Uuid::now_v7(), &shared).await?;
    let _export_b = insert_export_row(&database, tenant_b, Uuid::now_v7(), &shared).await?;

    let request_id = Uuid::now_v7();
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let plan = service
        .plan(tenant_a, request_id, PrivacyDeletionScope::Tenant)
        .await?
        .expect("owned tenant plans");
    service.execute(request_id).await?;

    blobs
        .verify(&shared)
        .await
        .expect("the surviving tenant must retain readable shared bytes");
    let raw_action = plan
        .items
        .iter()
        .find(|item| item.category == "raw_archive_blob" && item.opaque_id == export_a.to_string())
        .map(|item| item.action);
    assert_eq!(
        raw_action,
        Some(DeletionAction::RetainShared),
        "the inventory must report retained-shared instead of erased"
    );
    let tenant_a_exports: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.exports WHERE account_id = $1")
            .bind(tenant_a)
            .fetch_one(database.pool())
            .await?;
    let surviving_tenant_exports: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.exports WHERE account_id = $1")
            .bind(tenant_b)
            .fetch_one(database.pool())
            .await?;
    assert_eq!((tenant_a_exports, surviving_tenant_exports), (0, 1));
    Ok(())
}
