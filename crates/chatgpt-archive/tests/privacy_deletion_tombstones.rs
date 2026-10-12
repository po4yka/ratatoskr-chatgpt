//! Deletion tombstones are complete contract envelopes owned by the Platform user
//! (XR-021 CONTRACTS.md section S07).

// Test bodies fail through `expect`/`panic!`; assertions are the contract.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::sync::Arc;

use common::{
    DisposableDatabase, archive_limits, blob, drain, insert_account, insert_export, project_zip,
    receive_export, zip_of,
};
use ratatoskr_ai_archive_contracts::{AiArchiveTombstone, AiArchiveTombstoneSubject};
use ratatoskr_chatgpt_archive::privacy_deletion::{PrivacyDeletionScope, PrivacyDeletionService};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
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
async fn an_account_that_announced_nothing_completes_without_a_tombstone_or_an_unbound_item() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let seeded = seed(&database, &blobs, b"unmapped raw").await;
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = plan_archive(&service, &seeded).await;

    let report = service
        .execute(request)
        .await
        .expect("an account with nothing announced never blocks the deletion");

    assert!(
        tombstone_rows(&database).await.is_empty(),
        "nothing was announced, so there is nothing for Knowledge to retract"
    );
    assert_eq!(
        report.totals.get("downstream_tombstone_unbound"),
        None,
        "the report claims no gap for an account that announced nothing"
    );
    database.discard().await;
}

#[tokio::test]
async fn an_account_with_a_mapping_and_no_outbox_fact_falls_back_to_the_mapping() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let seeded = seed(&database, &blobs, b"fallback raw").await;
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
    for (_, stored) in rows {
        assert_eq!(owner_of(&stored), format!("user:{PLATFORM_USER}"));
    }
    assert_eq!(report.totals.get("downstream_tombstone_unbound"), None);
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

// ---------------------------------------------------------------------------------------------
// The owner is the one the archive was announced under (XR-021 CONTRACTS.md R4-04).
// ---------------------------------------------------------------------------------------------

const ACCOUNT: &str = "acc-announced";
const ANNOUNCED_USER: &str = "018f0000-0000-7000-8000-0000000000a1";
const OTHER_USER: &str = "018f0000-0000-7000-8000-0000000000b2";

fn user(uuid: &str) -> Uuid {
    Uuid::parse_str(uuid).expect("a literal UUID")
}

fn owner_of(stored: &serde_json::Value) -> String {
    let envelope: EventEnvelope =
        serde_json::from_value(stored.clone()).expect("the stored body is an envelope");
    let tombstone: AiArchiveTombstone = envelope.payload_as().expect("a tombstone payload");
    tombstone.owner.to_string()
}

/// An export imported and announced while the mapping named `ANNOUNCED_USER`.
struct Announced {
    database: DisposableDatabase,
    blobs: BlobStore,
    _root: tempfile::TempDir,
    tenant: Uuid,
    archive: Uuid,
    conversation: Uuid,
}

impl Announced {
    /// Imports the synthetic project archive through the real worker under `mapping`.
    async fn import(mapping: Vec<(Uuid, String)>) -> Self {
        let database = DisposableDatabase::create().await;
        let root = tempfile::tempdir().expect("blob root");
        let blobs = BlobStore::new(root.path()).expect("blob store");
        let worker = InitialImportWorker::new(
            database.pool().clone(),
            blobs.clone(),
            Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
            archive_limits(),
        )
        .with_platform_users(mapping);
        let received = receive_export(database.pool(), &blobs, ACCOUNT, project_zip()).await;
        drain(&worker).await;
        let conversation: Uuid = sqlx::query_scalar(
            "SELECT id FROM chatgpt_archive.conversations WHERE account_id = $1",
        )
        .bind(received.account)
        .fetch_one(database.pool())
        .await
        .expect("the import projected one conversation");
        Self {
            database,
            blobs,
            _root: root,
            tenant: received.account,
            archive: received.archive,
            conversation,
        }
    }

    async fn announced_under(user_id: &str) -> Self {
        Self::import(vec![(user(user_id), ACCOUNT.to_owned())]).await
    }

    fn service(&self, mapping: Vec<(Uuid, String)>) -> PrivacyDeletionService {
        PrivacyDeletionService::new(self.database.pool().clone(), self.blobs.clone())
            .with_platform_users(mapping)
    }

    /// The deletion scope a case names: `archive`, `conversation` or `tenant`.
    fn scope(&self, name: &str) -> PrivacyDeletionScope {
        match name {
            "archive" => PrivacyDeletionScope::Archive {
                ai_archive_id: self.archive,
            },
            "conversation" => PrivacyDeletionScope::Conversation {
                conversation_id: self.conversation,
            },
            "tenant" => PrivacyDeletionScope::Tenant,
            other => panic!("unknown scope {other}"),
        }
    }

    async fn delete(
        &self,
        service: &PrivacyDeletionService,
        scope: PrivacyDeletionScope,
    ) -> ratatoskr_chatgpt_archive::privacy_deletion::DeletionReport {
        let request = Uuid::now_v7();
        service
            .plan(self.tenant, request, scope)
            .await
            .expect("planning succeeds")
            .expect("the owned scope plans");
        service
            .execute(request)
            .await
            .expect("the deletion completes")
    }
}

/// Deletes the announced archive in every scope under `replacement` and asserts that each
/// tombstone names `ANNOUNCED_USER` and that no item was left unbound.
async fn assert_announced_owner_wins(replacement: impl Fn() -> Vec<(Uuid, String)>) {
    for name in ["archive", "conversation", "tenant"] {
        let announced = Announced::announced_under(ANNOUNCED_USER).await;
        let report = announced
            .delete(&announced.service(replacement()), announced.scope(name))
            .await;

        let rows = tombstone_rows(&announced.database).await;
        let queued = report.totals.get("downstream_tombstone").copied();
        assert!(!rows.is_empty(), "{name}: a tombstone must be queued");
        assert_eq!(
            Some(u64::try_from(rows.len()).expect("a small count")),
            queued,
            "{name}: every planned tombstone is queued"
        );
        for (_, stored) in &rows {
            assert_eq!(
                owner_of(stored),
                format!("user:{ANNOUNCED_USER}"),
                "{name}: the owner the archive was announced under"
            );
        }
        assert_eq!(
            report.totals.get("downstream_tombstone_unbound"),
            None,
            "{name}: no tombstone is reported as unbound"
        );
        announced.database.discard().await;
    }
}

#[tokio::test]
async fn a_deletion_tombstones_to_the_owner_the_archive_was_announced_under_when_the_mapping_changed()
 {
    // The mapping no longer contains the account at all.
    assert_announced_owner_wins(Vec::new).await;
}

#[tokio::test]
async fn a_deletion_uses_the_announced_owner_even_when_the_mapping_names_another_user() {
    assert_announced_owner_wins(|| vec![(user(OTHER_USER), ACCOUNT.to_owned())]).await;
}

#[tokio::test]
async fn an_announced_account_with_no_resolvable_owner_still_reports_unbound() {
    let announced = Announced::announced_under(ANNOUNCED_USER).await;
    // Every announced fact loses its owner: the typed payload owners and the envelope tenant.
    sqlx::query(
        "UPDATE chatgpt_archive.outbox_events
         SET payload = (payload - 'tenant_id')
             #- '{payload,owner}'
             #- '{payload,import_provenance,owner}'
             #- '{payload,conversation,owner}'
         WHERE event_type LIKE 'ai_archive.%'",
    )
    .execute(announced.database.pool())
    .await
    .expect("the owners are stripped");

    let report = announced
        .delete(
            &announced.service(Vec::new()),
            PrivacyDeletionScope::Archive {
                ai_archive_id: announced.archive,
            },
        )
        .await;

    assert!(tombstone_rows(&announced.database).await.is_empty());
    assert_eq!(
        report.totals.get("downstream_tombstone_unbound"),
        Some(&3),
        "the skip stays visible when facts were announced and no owner can be found"
    );
    announced.database.discard().await;
}

/// A cumulative export holding the listed conversations, each with one text message.
fn export_of(conversations: &[&str]) -> Vec<u8> {
    let documents: Vec<serde_json::Value> = conversations
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "title": format!("Title of {id}"),
                "mapping": {"node": {
                    "id": "node",
                    "parent": null,
                    "message": {
                        "id": format!("message-{id}"),
                        "author": {"role": "user"},
                        "content": {"parts": [format!("Text of {id}")]}
                    }
                }}
            })
        })
        .collect();
    let body = serde_json::Value::Array(documents).to_string();
    zip_of(&[("conversations.json", body.as_bytes())])
}

#[tokio::test]
async fn survivors_are_re_announced_under_the_announced_owner() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let worker = InitialImportWorker::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
        archive_limits(),
    )
    .with_platform_users(vec![(user(ANNOUNCED_USER), ACCOUNT.to_owned())]);
    // Two cumulative exports: only the first announces conversation `conv-a`.
    let first = receive_export(database.pool(), &blobs, ACCOUNT, export_of(&["conv-a"])).await;
    drain(&worker).await;
    receive_export(
        database.pool(),
        &blobs,
        ACCOUNT,
        export_of(&["conv-a", "conv-b"]),
    )
    .await;
    drain(&worker).await;

    // The mapping changed after the import: the account is no longer in it.
    let service = PrivacyDeletionService::new(database.pool().clone(), blobs.clone());
    let request = Uuid::now_v7();
    service
        .plan(
            first.account,
            request,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await
        .expect("planning succeeds")
        .expect("the owned archive plans");
    let report = service
        .execute(request)
        .await
        .expect("the deletion completes");

    assert_eq!(report.totals.get("downstream_tombstone_unbound"), None);
    for (_, stored) in tombstone_rows(&database).await {
        assert_eq!(owner_of(&stored), format!("user:{ANNOUNCED_USER}"));
    }
    let owners: Vec<String> = sqlx::query_scalar(
        "SELECT payload #>> '{payload,conversation,owner}'
         FROM chatgpt_archive.outbox_events
         WHERE event_type = 'ai_archive.conversation.updated.v1'
           AND created_at >= (SELECT min(created_at) FROM chatgpt_archive.outbox_events
                              WHERE event_type = 'ai_archive.subject.tombstoned.v1')
         ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(
        owners,
        vec![format!("user:{ANNOUNCED_USER}")],
        "the survivor conv-a is announced again under the owner it was announced under"
    );
    database.discard().await;
}
