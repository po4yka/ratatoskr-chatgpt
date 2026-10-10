//! Deletion tombstones are complete contract envelopes owned by the Platform user
//! (XR-021 CONTRACTS.md section S07).

// Test bodies fail through `expect`/`panic!`; assertions are the contract.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use common::{DisposableDatabase, blob, insert_account, insert_export};
use ratatoskr_ai_archive_contracts::{AiArchiveTombstone, AiArchiveTombstoneSubject};
use ratatoskr_chatgpt_archive::BlobStore;
use ratatoskr_chatgpt_archive::privacy_deletion::{PrivacyDeletionScope, PrivacyDeletionService};
use ratatoskr_event_envelope::EventEnvelope;
use uuid::Uuid;

const PLATFORM_USER: &str = "018f0000-0000-7000-8000-000000000005";

async fn account_reference(database: &DisposableDatabase, tenant: Uuid) -> String {
    sqlx::query_scalar("SELECT external_ref FROM chatgpt_archive.accounts WHERE id = $1")
        .bind(tenant)
        .fetch_one(database.pool())
        .await
        .expect("the account is readable")
}

fn mapped(
    database: &DisposableDatabase,
    blobs: &BlobStore,
    account_ref: &str,
) -> PrivacyDeletionService {
    PrivacyDeletionService::new(database.pool().clone(), blobs.clone()).with_platform_users(vec![(
        Uuid::parse_str(PLATFORM_USER).expect("a literal UUID"),
        account_ref.to_owned(),
    )])
}

/// One archive holding one conversation and one project, both observed only there.
struct Seeded {
    tenant: Uuid,
    archive: Uuid,
    conversation: Uuid,
    project: Uuid,
}

async fn seed(database: &DisposableDatabase, blobs: &BlobStore, bytes: &'static [u8]) -> Seeded {
    let tenant = insert_account(database, "tombstone").await;
    let raw = blob(blobs, bytes).await;
    let archive = Uuid::now_v7();
    let export = insert_export(database, tenant, archive, &raw).await;
    let conversation = Uuid::now_v7();
    let project = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chatgpt_archive.projects (id, account_id, external_id)
         VALUES ($1, $2, 'project')",
    )
    .bind(project)
    .bind(tenant)
    .execute(database.pool())
    .await
    .expect("the project is inserted");
    sqlx::query(
        "INSERT INTO chatgpt_archive.conversations (id, account_id, external_id)
         VALUES ($1, $2, 'conversation')",
    )
    .bind(conversation)
    .bind(tenant)
    .execute(database.pool())
    .await
    .expect("the conversation is inserted");
    for (kind, entity) in [("project", project), ("conversation", conversation)] {
        sqlx::query(
            "INSERT INTO chatgpt_archive.export_entity_observations
             (export_id, entity_kind, entity_id) VALUES ($1, $2, $3)",
        )
        .bind(export)
        .bind(kind)
        .bind(entity)
        .execute(database.pool())
        .await
        .expect("the observation is inserted");
    }
    Seeded {
        tenant,
        archive,
        conversation,
        project,
    }
}

async fn plan_archive(service: &PrivacyDeletionService, seeded: &Seeded) -> Uuid {
    let request = Uuid::now_v7();
    service
        .plan(
            seeded.tenant,
            request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: seeded.archive,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned archive plans");
    request
}

async fn tombstone_rows(database: &DisposableDatabase) -> Vec<(i64, serde_json::Value)> {
    sqlx::query_as(
        "SELECT id, payload FROM chatgpt_archive.outbox_events
         WHERE event_type = 'ai_archive.subject.tombstoned.v1' ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable")
}

#[tokio::test]
async fn tombstone_outbox_row_is_a_complete_event_envelope_with_platform_owner() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let seeded = seed(&database, &blobs, b"tombstone raw").await;
    let service = mapped(
        &database,
        &blobs,
        &account_reference(&database, seeded.tenant).await,
    );
    let request = plan_archive(&service, &seeded).await;

    let report = service
        .execute(request)
        .await
        .expect("the deletion completes");

    let rows = tombstone_rows(&database).await;
    assert_eq!(
        rows.len(),
        3,
        "one per deleted archive, conversation and project"
    );
    let owner = format!("user:{PLATFORM_USER}");
    for (_, stored) in rows {
        let envelope: EventEnvelope =
            serde_json::from_value(stored).expect("the stored body is a complete envelope");
        let tombstone: AiArchiveTombstone = envelope
            .payload_as()
            .expect("the payload is the tombstone contract");
        assert_eq!(tombstone.reason.as_str(), "user_requested");
        assert_eq!(
            tombstone.owner.to_string(),
            owner,
            "the Platform owner, not the account"
        );
        assert_eq!(
            envelope.tenant_id.map(|tenant| tenant.to_string()),
            Some(owner.clone())
        );
        assert_eq!(envelope.producer.as_str(), "ratatoskr-chatgpt");
        assert_eq!(tombstone.evidence_ref, report.evidence_ref);
        assert_eq!(envelope.occurred_at, tombstone.observed_at);
        let expected = match &tombstone.subject {
            AiArchiveTombstoneSubject::Archive => format!("ai_archive:{}", seeded.archive),
            AiArchiveTombstoneSubject::Conversation { ai_conversation_id } => {
                assert_eq!(ai_conversation_id.0, seeded.conversation);
                format!("ai_conversation:{}", seeded.conversation)
            }
            AiArchiveTombstoneSubject::Project { ai_project_id } => {
                assert_eq!(ai_project_id.0, seeded.project);
                format!("ai_project:{}", seeded.project)
            }
            AiArchiveTombstoneSubject::Artifact { .. } => panic!("ChatGPT has no artifacts"),
        };
        assert_eq!(envelope.aggregate_id.to_wire(), expected);
    }
    blobs
        .verify(&report.evidence_ref)
        .await
        .expect("the evidence blob exists");
    let columns: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT tenant_id, export_id FROM chatgpt_archive.outbox_events
         WHERE event_type = 'ai_archive.subject.tombstoned.v1' LIMIT 1",
    )
    .fetch_one(database.pool())
    .await
    .expect("the row is readable");
    assert_eq!(
        columns,
        (seeded.tenant, None),
        "the row belongs to the archive account and survives the export cascade"
    );
    database.discard().await;
}

#[tokio::test]
async fn unmapped_account_emits_no_tombstone_and_reports_unbound_count() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let seeded = seed(&database, &blobs, b"unmapped raw").await;
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = plan_archive(&service, &seeded).await;

    let report = service
        .execute(request)
        .await
        .expect("an unmapped account never blocks the deletion");

    assert!(
        tombstone_rows(&database).await.is_empty(),
        "a tombstone with no Platform owner would name a tenant Knowledge cannot match"
    );
    assert_eq!(
        report.totals.get("downstream_tombstone_unbound"),
        Some(&3),
        "the skip is visible in the completion report"
    );
    database.discard().await;
}

#[tokio::test]
async fn tenant_deletion_keeps_unpublished_tombstones_of_earlier_requests() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let first = seed(&database, &blobs, b"first raw").await;
    let account_ref = account_reference(&database, first.tenant).await;
    let service = mapped(&database, &blobs, &account_ref);
    let first_request = plan_archive(&service, &first).await;
    service
        .execute(first_request)
        .await
        .expect("the first deletion completes");
    let earlier = tombstone_rows(&database).await;
    assert_eq!(earlier.len(), 3);

    // A second archive for the same account, then everything is deleted.
    let raw = blob(&blobs, b"second raw").await;
    insert_export(&database, first.tenant, Uuid::now_v7(), &raw).await;
    let tenant_request = Uuid::now_v7();
    service
        .plan(first.tenant, tenant_request, PrivacyDeletionScope::Tenant)
        .await
        .expect("planning succeeds")
        .expect("the owned tenant plans");
    service
        .execute(tenant_request)
        .await
        .expect("the tenant deletion completes");

    let kept: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chatgpt_archive.outbox_events
         WHERE deduplication_key LIKE $1 AND published_at IS NULL ORDER BY id",
    )
    .bind(format!("privacy-delete:{first_request}:%"))
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(
        kept,
        earlier.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "an unpublished tombstone is the only record that Knowledge must delete; tenant erasure must not take it"
    );
    database.discard().await;
}
