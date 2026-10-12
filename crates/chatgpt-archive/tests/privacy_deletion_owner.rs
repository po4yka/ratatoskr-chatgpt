//! A deletion addresses every tombstone to the owner its subject was announced under, read from
//! the announced-subject ledger and never from the live mapping or from outbox envelopes
//! (XR-021 CONTRACTS.md R5-06).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::{DisposableDatabase, Received, archive_limits, drain, receive_export, zip_of};
use ratatoskr_ai_archive_contracts::{AiArchiveTombstone, AiArchiveTombstoneSubject};
use ratatoskr_chatgpt_archive::privacy_deletion::{
    DeletionReport, PrivacyDeletionScope, PrivacyDeletionService,
};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use ratatoskr_event_envelope::EventEnvelope;
use uuid::Uuid;

const ACCOUNT: &str = "acc-owner-ledger";
const USER_A: &str = "018f0000-0000-7000-8000-0000000000a1";
const USER_B: &str = "018f0000-0000-7000-8000-0000000000b2";
const USER_C: &str = "018f0000-0000-7000-8000-0000000000c3";

/// A cumulative export holding the listed `(conversation id, text)` pairs.
fn export(conversations: &[(&str, &str)]) -> Vec<u8> {
    let documents: Vec<serde_json::Value> = conversations
        .iter()
        .map(|(id, text)| {
            serde_json::json!({
                "id": id,
                "title": format!("Title of {id}"),
                "mapping": {"node": {
                    "id": "node",
                    "parent": null,
                    "message": {
                        "id": format!("message-{id}"),
                        "author": {"role": "user"},
                        "content": {"parts": [text]}
                    }
                }}
            })
        })
        .collect();
    let body = serde_json::Value::Array(documents).to_string();
    zip_of(&[("conversations.json", body.as_bytes())])
}

struct Fixture {
    database: DisposableDatabase,
    blobs: BlobStore,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().expect("blob root");
        Self {
            database: DisposableDatabase::create().await,
            blobs: BlobStore::new(root.path()).expect("blob store"),
            _root: root,
        }
    }

    /// An import worker whose mapping names `user` for the account, or no mapping at all.
    fn importer(&self, user: Option<&str>) -> InitialImportWorker {
        let mapping = user
            .map(|user| {
                vec![(
                    Uuid::parse_str(user).expect("a literal UUID"),
                    ACCOUNT.to_owned(),
                )]
            })
            .unwrap_or_default();
        InitialImportWorker::new(
            self.database.pool().clone(),
            self.blobs.clone(),
            Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
            archive_limits(),
        )
        .with_platform_users(mapping)
    }

    /// Receives and imports one export while the mapping names `user`.
    async fn import(&self, user: Option<&str>, conversations: &[(&str, &str)]) -> Received {
        let received = receive_export(
            self.database.pool(),
            &self.blobs,
            ACCOUNT,
            export(conversations),
        )
        .await;
        drain(&self.importer(user)).await;
        received
    }

    fn service(&self) -> PrivacyDeletionService {
        PrivacyDeletionService::new(self.database.pool().clone(), self.blobs.clone())
    }

    async fn delete(&self, tenant: Uuid, scope: PrivacyDeletionScope) -> DeletionReport {
        let service = self.service();
        let request = Uuid::now_v7();
        service
            .plan(tenant, request, scope)
            .await
            .expect("planning succeeds")
            .expect("the owned scope plans");
        service
            .execute(request)
            .await
            .expect("the deletion completes")
    }

    async fn conversation(&self, external_id: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM chatgpt_archive.conversations WHERE external_id = $1")
            .bind(external_id)
            .fetch_one(self.database.pool())
            .await
            .expect("the conversation was projected")
    }

    /// `(subject, owner, archive named by the tombstone)` of every tombstone, sorted.
    async fn tombstones(&self) -> Vec<(String, String, Uuid)> {
        let rows: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT payload FROM chatgpt_archive.outbox_events
             WHERE event_type = 'ai_archive.subject.tombstoned.v1' ORDER BY id",
        )
        .fetch_all(self.database.pool())
        .await
        .expect("the outbox is readable");
        let mut tombstones: Vec<(String, String, Uuid)> = rows
            .into_iter()
            .map(|stored| {
                let envelope: EventEnvelope =
                    serde_json::from_value(stored).expect("a complete envelope");
                let tombstone: AiArchiveTombstone =
                    envelope.payload_as().expect("a tombstone payload");
                assert_eq!(
                    envelope.tenant_id,
                    Some(tombstone.owner),
                    "the envelope tenant is the payload owner"
                );
                let subject = match &tombstone.subject {
                    AiArchiveTombstoneSubject::Archive => {
                        format!("archive:{}", tombstone.ai_archive_id.0)
                    }
                    AiArchiveTombstoneSubject::Conversation { ai_conversation_id } => {
                        format!("conversation:{}", ai_conversation_id.0)
                    }
                    AiArchiveTombstoneSubject::Project { ai_project_id } => {
                        format!("project:{}", ai_project_id.0)
                    }
                    AiArchiveTombstoneSubject::Artifact { .. } => {
                        panic!("ChatGPT has no artifacts")
                    }
                };
                (
                    subject,
                    tombstone.owner.to_string(),
                    tombstone.ai_archive_id.0,
                )
            })
            .collect();
        tombstones.sort();
        tombstones
    }

    async fn ledger_subjects(&self) -> BTreeSet<(String, String, String)> {
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT subject_kind, subject_id, owner_ref
             FROM chatgpt_archive.announced_subjects",
        )
        .fetch_all(self.database.pool())
        .await
        .expect("the ledger is readable")
        .into_iter()
        .collect()
    }
}

fn owner(user: &str) -> String {
    format!("user:{user}")
}

#[tokio::test]
async fn a_deletion_tombstones_to_the_owner_the_archive_was_announced_under() {
    // Every scope runs on a fresh account. The deletion service has no mapping to consult, which
    // is what a mapping that no longer lists the account looks like to it.
    for name in ["archive", "conversation", "tenant"] {
        let fixture = Fixture::new().await;
        let received = fixture
            .import(Some(USER_A), &[("conv-a", "a"), ("conv-b", "b")])
            .await;
        let scope = match name {
            "archive" => PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
            "conversation" => PrivacyDeletionScope::Conversation {
                conversation_id: fixture.conversation("conv-a").await,
            },
            _ => PrivacyDeletionScope::Tenant,
        };

        let report = fixture.delete(received.account, scope).await;

        let tombstones = fixture.tombstones().await;
        assert!(!tombstones.is_empty(), "{name}: a tombstone must be queued");
        for (subject, tombstone_owner, _) in &tombstones {
            assert_eq!(
                tombstone_owner,
                &owner(USER_A),
                "{name}: {subject} names the owner it was announced under"
            );
        }
        assert_eq!(
            report.totals.get("downstream_tombstone_unbound"),
            None,
            "{name}: no tombstone is reported as unbound"
        );
        fixture.database.discard().await;
    }
}

#[tokio::test]
async fn a_tenant_deletion_spanning_two_owners_sends_one_tombstone_to_each_owner() {
    let fixture = Fixture::new().await;
    let first = fixture
        .import(Some(USER_A), &[("conv-a", "first"), ("conv-b", "b")])
        .await;
    // The mapping changed between the imports: the second archive is announced under user B and
    // changes conv-a, so conv-a is announced under both users.
    let second = fixture.import(Some(USER_B), &[("conv-a", "second")]).await;
    let conv_a = fixture.conversation("conv-a").await;
    let conv_b = fixture.conversation("conv-b").await;

    fixture
        .delete(first.account, PrivacyDeletionScope::Tenant)
        .await;

    let mut expected = vec![
        (
            format!("archive:{}", first.archive),
            owner(USER_A),
            first.archive,
        ),
        (
            format!("archive:{}", second.archive),
            owner(USER_B),
            second.archive,
        ),
        (
            format!("conversation:{conv_a}"),
            owner(USER_A),
            first.archive,
        ),
        (
            format!("conversation:{conv_a}"),
            owner(USER_B),
            second.archive,
        ),
        (
            format!("conversation:{conv_b}"),
            owner(USER_A),
            first.archive,
        ),
    ];
    expected.sort();
    assert_eq!(
        fixture.tombstones().await,
        expected,
        "one tombstone per subject and owner, each naming the archive it was announced under"
    );
    assert!(
        fixture
            .tombstones()
            .await
            .iter()
            .all(|(_, tombstone_owner, _)| tombstone_owner != &owner(USER_C)),
        "no tombstone goes to a third user"
    );
    fixture.database.discard().await;
}

#[tokio::test]
async fn an_account_that_never_announced_completes_without_a_tombstone_or_a_gap() {
    let fixture = Fixture::new().await;
    let received = fixture.import(None, &[("conv-a", "a")]).await;

    let report = fixture
        .delete(
            received.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: received.archive,
            },
        )
        .await;

    assert!(
        fixture.tombstones().await.is_empty(),
        "nothing was announced, so there is nothing for Knowledge to retract"
    );
    assert_eq!(
        report.totals.get("downstream_tombstone_unbound"),
        None,
        "the report claims no gap for an account that announced nothing"
    );
    fixture.database.discard().await;
}

#[tokio::test]
async fn a_re_announced_survivor_carries_the_owner_of_the_retained_archive() {
    let fixture = Fixture::new().await;
    let first = fixture.import(Some(USER_A), &[("conv-a", "a")]).await;
    let second = fixture
        .import(Some(USER_B), &[("conv-a", "a"), ("conv-b", "b")])
        .await;
    let conv_a = fixture.conversation("conv-a").await;

    fixture
        .delete(
            first.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await;

    assert_eq!(
        fixture.tombstones().await,
        vec![(
            format!("archive:{}", first.archive),
            owner(USER_A),
            first.archive
        )],
        "only the deleted archive is tombstoned, to the owner it was announced under"
    );
    let survivors: Vec<(String, String)> = sqlx::query_as(
        "SELECT payload #>> '{payload,conversation,owner}', payload #>> '{payload,import_provenance,ai_archive_id}'
         FROM chatgpt_archive.outbox_events
         WHERE event_type = 'ai_archive.conversation.updated.v1' AND aggregate_id = $1",
    )
    .bind(conv_a)
    .fetch_all(fixture.database.pool())
    .await
    .expect("the outbox is readable");
    assert_eq!(
        survivors,
        vec![(owner(USER_B), second.archive.to_string())],
        "the survivor is announced again under the retained archive and its owner"
    );
    fixture.database.discard().await;
}

#[tokio::test]
async fn the_ledger_rows_of_a_tombstoned_subject_are_gone_and_no_other_row_is_touched() {
    let fixture = Fixture::new().await;
    let first = fixture.import(Some(USER_A), &[("conv-a", "a")]).await;
    let second = fixture
        .import(Some(USER_B), &[("conv-a", "a"), ("conv-b", "b")])
        .await;
    let conv_a = fixture.conversation("conv-a").await;
    let conv_b = fixture.conversation("conv-b").await;
    let before = fixture.ledger_subjects().await;

    fixture
        .delete(
            first.account,
            PrivacyDeletionScope::Archive {
                ai_archive_id: first.archive,
            },
        )
        .await;

    let after = fixture.ledger_subjects().await;
    let tombstoned = (
        "archive".to_owned(),
        first.archive.to_string(),
        owner(USER_A),
    );
    assert!(before.contains(&tombstoned), "the archive was announced");
    assert!(
        !after.contains(&tombstoned),
        "the tombstoned subject is gone"
    );
    let untouched: BTreeSet<_> = before
        .iter()
        .filter(|row| **row != tombstoned)
        .cloned()
        .collect();
    assert!(
        untouched.is_subset(&after),
        "no other subject's rows are touched: {untouched:?} not within {after:?}"
    );
    assert!(
        after.contains(&(
            "archive".to_owned(),
            second.archive.to_string(),
            owner(USER_B)
        )),
        "the retained archive stays announced"
    );
    assert!(
        after.contains(&("conversation".to_owned(), conv_b.to_string(), owner(USER_B))),
        "an untouched conversation stays announced"
    );
    assert!(
        after.contains(&("conversation".to_owned(), conv_a.to_string(), owner(USER_A))),
        "a retained conversation keeps the row it was announced with"
    );
    fixture.database.discard().await;
}

#[test]
fn the_outbox_scraping_owner_resolution_no_longer_exists() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        !source.join("privacy_deletion/owner.rs").exists(),
        "privacy_deletion/owner.rs is deleted"
    );
    let mut pending = vec![source];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("the source tree is readable") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text = std::fs::read_to_string(&path).expect("a source file is readable");
                assert!(
                    !text.contains("FACT_OWNER"),
                    "{} still scrapes owners out of outbox envelopes",
                    path.display()
                );
            }
        }
    }
}
