//! Deleting an archive must not tell Knowledge to drop a record that a retained archive still
//! holds (XR-021 CONTRACTS.md R3-06).
//!
//! `ChatGPT` exports are cumulative and a record is announced only under the first export that saw
//! it. The tombstone of that export's archive is the whole import for Knowledge, so the deletion
//! announces every survivor again under the earliest retained export that observed it, after the
//! tombstone and in the same transaction. Verified only against the documented synthetic schema.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test scaffolding: a failed setup step must fail the test loudly"
)]

mod common;

use std::sync::Arc;

use common::{DisposableDatabase, archive_limits, drain, receive_export, zip_of};
use ratatoskr_ai_archive_contracts::{
    AiArchiveProvenance, AiConversationAdded, AiConversationUpdated, AiProjectAdded,
    AiProjectUpdated,
};
use ratatoskr_chatgpt_archive::privacy_deletion::{
    FinalizationFault, PrivacyDeletionError, PrivacyDeletionScope, PrivacyDeletionService,
};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use ratatoskr_event_envelope::EventEnvelope;
use uuid::Uuid;

const PLATFORM_USER: &str = "018f0000-0000-7000-8000-0000000000c1";
const TOMBSTONED: &str = "ai_archive.subject.tombstoned.v1";
const HEAD: &str = "ai_archive.archive.imported.v1";
const ACCOUNT: &str = "acc-survivors";

/// One synthetic conversation with a single text message; its content never changes.
fn conversation(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": format!("Title of {id}"),
        "mapping": {
            "node": {
                "id": "node",
                "parent": null,
                "message": {
                    "id": format!("message-{id}"),
                    "author": { "role": "user" },
                    "content": { "parts": [format!("Text of {id}")] }
                }
            }
        }
    })
}

/// A cumulative export holding `conversations` and, when asked, the one project.
fn export(conversations: &[&str], with_project: bool) -> Vec<u8> {
    let documents: Vec<serde_json::Value> =
        conversations.iter().map(|id| conversation(id)).collect();
    let conversations_json = serde_json::Value::Array(documents).to_string();
    let projects_json = serde_json::json!([{
        "id": "project-one",
        "title": "Synthetic project",
        "description": "Invented fixture only",
        "instructions": [],
        "conversation_ids": [],
        "asset_ids": []
    }])
    .to_string();
    if with_project {
        zip_of(&[
            ("conversations.json", conversations_json.as_bytes()),
            ("projects.json", projects_json.as_bytes()),
        ])
    } else {
        zip_of(&[("conversations.json", conversations_json.as_bytes())])
    }
}

/// Three cumulative exports of one account: E1 holds X, A, B and the project; E2 holds A, B, C and
/// the project; E3 holds B, C, D. Each is imported and its facts are queued before the next.
struct Imported {
    database: DisposableDatabase,
    blobs: BlobStore,
    root: tempfile::TempDir,
    tenant: Uuid,
    /// `(export_id, ai_archive_id)` in receipt order.
    exports: [(Uuid, Uuid); 3],
}

async fn import_three_exports() -> Imported {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let worker = InitialImportWorker::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
        archive_limits(),
    )
    .with_platform_users(mapping());
    let mut exports = Vec::new();
    let mut tenant = Uuid::nil();
    for (conversations, with_project) in [
        (&["conv-x", "conv-a", "conv-b"][..], true),
        (&["conv-a", "conv-b", "conv-c"][..], true),
        (&["conv-b", "conv-c", "conv-d"][..], false),
    ] {
        let received = receive_export(
            database.pool(),
            &blobs,
            ACCOUNT,
            export(conversations, with_project),
        )
        .await;
        drain(&worker).await;
        tenant = received.account;
        exports.push((received.export, received.archive));
    }
    Imported {
        database,
        blobs,
        root,
        tenant,
        exports: <[(Uuid, Uuid); 3]>::try_from(exports).expect("three exports were received"),
    }
}

fn mapping() -> Vec<(Uuid, String)> {
    vec![(
        Uuid::parse_str(PLATFORM_USER).expect("a literal UUID"),
        ACCOUNT.to_owned(),
    )]
}

impl Imported {
    fn service(&self) -> PrivacyDeletionService {
        PrivacyDeletionService::new(self.database.pool().clone(), self.blobs.clone())
    }

    async fn plan(&self, scope: PrivacyDeletionScope) -> Uuid {
        let request = Uuid::now_v7();
        self.service()
            .plan(self.tenant, request, scope)
            .await
            .expect("planning succeeds")
            .expect("the owned scope plans");
        request
    }

    async fn delete_archive(&self, archive: Uuid) {
        let request = self
            .plan(PrivacyDeletionScope::Archive {
                ai_archive_id: archive,
            })
            .await;
        self.service()
            .execute(request)
            .await
            .expect("the deletion completes");
    }

    /// Every `ai_archive.*` fact in the order the pump publishes it.
    async fn facts(&self) -> Vec<EventEnvelope> {
        let rows: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT payload FROM chatgpt_archive.outbox_events
             WHERE event_type LIKE 'ai_archive.%' ORDER BY id",
        )
        .fetch_all(self.database.pool())
        .await
        .expect("the archive facts are readable");
        rows.into_iter()
            .map(|raw| serde_json::from_value(raw).expect("a complete envelope"))
            .collect()
    }

    async fn conversation_id(&self, external: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM chatgpt_archive.conversations WHERE external_id = $1")
            .bind(external)
            .fetch_one(self.database.pool())
            .await
            .expect("the conversation exists")
    }

    /// How many import heads each export's archive has in the outbox, in receipt order.
    async fn head_rows(&self) -> [i64; 3] {
        let mut counts = [0; 3];
        for (count, (_, archive)) in counts.iter_mut().zip(self.exports) {
            *count = sqlx::query_scalar(
                "SELECT count(*) FROM chatgpt_archive.outbox_events
                 WHERE event_type = $1 AND aggregate_id = $2",
            )
            .bind(HEAD)
            .bind(archive)
            .fetch_one(self.database.pool())
            .await
            .expect("the head rows are countable");
        }
        counts
    }

    async fn finish(self) {
        drop(self.root);
        self.database.discard().await;
    }
}

/// The facts queued after the last tombstone, which are the ones that follow the deletion.
fn after_the_tombstones(facts: &[EventEnvelope]) -> Vec<EventEnvelope> {
    let last = facts
        .iter()
        .rposition(|fact| fact.event_type.to_wire() == TOMBSTONED)
        .expect("the deletion queued a tombstone");
    facts
        .get(last + 1..)
        .expect("the tombstone is inside the list")
        .to_vec()
}

#[derive(serde::Deserialize)]
struct Stamped {
    import_provenance: AiArchiveProvenance,
}

fn provenance_of(fact: &EventEnvelope) -> AiArchiveProvenance {
    serde_json::from_value::<Stamped>(serde_json::Value::Object(fact.payload.clone()))
        .expect("the payload names its provenance")
        .import_provenance
}

fn types(facts: &[EventEnvelope]) -> Vec<String> {
    facts.iter().map(|fact| fact.event_type.to_wire()).collect()
}

/// The conversation identity a conversation fact is about.
fn conversation_of(fact: &EventEnvelope) -> Uuid {
    fact.aggregate_id
        .to_wire()
        .strip_prefix("ai_conversation:")
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("a conversation aggregate")
}

#[tokio::test]
async fn deleting_the_oldest_export_reannounces_its_survivors() {
    let imported = import_three_exports().await;
    let [(_, deleted), (_, earliest_retained), _] = imported.exports;
    let (a, b, x) = (
        imported.conversation_id("conv-a").await,
        imported.conversation_id("conv-b").await,
        imported.conversation_id("conv-x").await,
    );
    let first_facts = imported.facts().await;
    let retained_digest: String = sqlx::query_scalar(
        "SELECT sha256_hex FROM chatgpt_archive.exports WHERE ai_archive_id = $1",
    )
    .bind(earliest_retained)
    .fetch_one(imported.database.pool())
    .await
    .expect("the retained export is readable");

    imported.delete_archive(deleted).await;

    let facts = imported.facts().await;
    let follow_up = after_the_tombstones(&facts);
    assert_eq!(
        types(&follow_up),
        [
            "ai_archive.project.updated.v1",
            "ai_archive.conversation.updated.v1",
            "ai_archive.conversation.updated.v1",
        ],
        "the project and conversations A and B were announced only under the deleted archive; \
         X is gone with it and is not announced again"
    );
    let announced_again: Vec<Uuid> = follow_up[1..].iter().map(conversation_of).collect();
    assert_eq!(announced_again, [a, b], "in provider id order, never X");
    assert!(!announced_again.contains(&x));
    let tombstones: Vec<&EventEnvelope> = facts
        .iter()
        .filter(|fact| fact.event_type.to_wire() == TOMBSTONED)
        .collect();
    assert_eq!(
        tombstones.len(),
        2,
        "the archive and the conversation only E1 held"
    );
    for fact in &follow_up {
        let provenance = provenance_of(fact);
        assert_eq!(
            provenance.ai_archive_id.0, earliest_retained,
            "the earliest retained observing export, not the latest"
        );
        assert_eq!(
            provenance.source_export.digest.hex.as_str(),
            retained_digest,
            "the evidence is the retained raw export"
        );
        let tombstoned_at = tombstones
            .iter()
            .map(|tombstone| tombstone.occurred_at)
            .max()
            .expect("a tombstone exists");
        assert!(
            fact.occurred_at > tombstoned_at,
            "the repair is stamped after the tombstone so a consumer applies it afterwards"
        );
    }
    // The survivor is the record the archive still holds, unchanged.
    let project_now = follow_up[0]
        .payload_as::<AiProjectUpdated>()
        .expect("the project decodes");
    project_now
        .validate()
        .expect("the project satisfies its invariants");
    let project_then = first_facts
        .iter()
        .find(|fact| fact.event_type.to_wire() == "ai_archive.project.added.v1")
        .expect("the import announced the project")
        .payload_as::<AiProjectAdded>()
        .expect("the project decodes");
    assert_eq!(project_now.project, project_then.project);
    for (fact, id) in follow_up[1..].iter().zip([a, b]) {
        let now = fact
            .payload_as::<AiConversationUpdated>()
            .expect("the conversation decodes");
        now.validate()
            .expect("the conversation satisfies its invariants");
        let then = first_facts
            .iter()
            .filter(|fact| fact.event_type.to_wire() == "ai_archive.conversation.added.v1")
            .map(|fact| {
                fact.payload_as::<AiConversationAdded>()
                    .expect("the conversation decodes")
            })
            .find(|added| added.conversation.ai_conversation_id.0 == id)
            .expect("the import announced the conversation");
        assert_eq!(now.conversation, then.conversation);
    }
    imported.finish().await;
}

#[tokio::test]
async fn a_survivor_already_named_under_a_retained_archive_is_not_reannounced() {
    let imported = import_three_exports().await;
    // E2 announced C, and E3 observed it too: deleting E2 re-announces C under E3. A and B were
    // announced under E1, which stays, so they are not announced again.
    let [_, (_, deleted), (_, earliest_retained)] = imported.exports;
    let c = imported.conversation_id("conv-c").await;

    imported.delete_archive(deleted).await;

    let follow_up = after_the_tombstones(&imported.facts().await);
    assert_eq!(types(&follow_up), ["ai_archive.conversation.updated.v1"]);
    assert_eq!(conversation_of(&follow_up[0]), c);
    assert_eq!(
        provenance_of(&follow_up[0]).ai_archive_id.0,
        earliest_retained
    );
    imported.finish().await;
}

#[tokio::test]
async fn deleting_every_export_announces_no_survivor() {
    let imported = import_three_exports().await;
    let request = imported.plan(PrivacyDeletionScope::Tenant).await;
    imported
        .service()
        .execute(request)
        .await
        .expect("the deletion completes");

    let facts = imported.facts().await;
    let last = facts
        .iter()
        .rposition(|fact| fact.event_type.to_wire() == TOMBSTONED)
        .expect("the deletion queued tombstones");
    assert_eq!(last + 1, facts.len(), "nothing follows the last tombstone");
    assert_eq!(imported.head_rows().await, [0, 0, 0]);
    imported.finish().await;
}

#[tokio::test]
async fn the_deleted_exports_head_facts_leave_the_outbox() {
    let imported = import_three_exports().await;
    imported.delete_archive(imported.exports[0].1).await;

    assert_eq!(
        imported.head_rows().await,
        [0, 1, 1],
        "the deleted export's head names a source export whose bytes are erased; the retained \
         exports keep theirs"
    );
    imported.finish().await;
}

#[tokio::test]
async fn the_tombstone_and_the_repair_commit_together() {
    let imported = import_three_exports().await;
    let archive = imported.exports[0].1;
    let before = imported.facts().await.len();
    let request = imported
        .plan(PrivacyDeletionScope::Archive {
            ai_archive_id: archive,
        })
        .await;

    let outcome = imported
        .service()
        .execute_with_fault(request, FinalizationFault::AfterTombstones)
        .await;

    assert!(matches!(
        outcome,
        Err(PrivacyDeletionError::InjectedFinalization)
    ));
    let facts = imported.facts().await;
    assert_eq!(
        facts.len(),
        before,
        "neither the tombstone nor the repair is present after the failure"
    );
    assert!(
        facts
            .iter()
            .all(|fact| fact.event_type.to_wire() != TOMBSTONED),
        "no tombstone survived the rollback"
    );

    imported
        .service()
        .execute(request)
        .await
        .expect("re-execution converges");

    let follow_up = after_the_tombstones(&imported.facts().await);
    assert_eq!(
        types(&follow_up),
        [
            "ai_archive.project.updated.v1",
            "ai_archive.conversation.updated.v1",
            "ai_archive.conversation.updated.v1",
        ]
    );
    imported.finish().await;
}
