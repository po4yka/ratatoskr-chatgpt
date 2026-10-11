//! A survivor of a privacy deletion carries only what a retained export held (XR-021
//! CONTRACTS.md R3-06 and S07).
//!
//! Cumulative exports share one row per project and conversation, so the row holds the newest
//! export's text. When the export that wrote that text is deleted, the row, its content-part
//! revisions, its history and the fact announced for the survivor must all go back to a version
//! a retained export presented. Verified only against the documented synthetic schema.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test scaffolding: a failed setup step must fail the test loudly"
)]

mod common;

use std::sync::Arc;

use common::{DisposableDatabase, archive_limits, drain, receive_export, zip_of};
use ratatoskr_chatgpt_archive::privacy_deletion::{PrivacyDeletionScope, PrivacyDeletionService};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use ratatoskr_event_envelope::EventEnvelope;
use uuid::Uuid;

const PLATFORM_USER: &str = "018f0000-0000-7000-8000-0000000000c3";
const TOMBSTONED: &str = "ai_archive.subject.tombstoned.v1";
const ACCOUNT: &str = "acc-survivor-content";

/// What one export presents of the one conversation.
#[derive(Clone, Copy)]
struct Conversation<'a> {
    title: &'a str,
    /// The text of the first message.
    text: &'a str,
    /// The text of a second message that follows the first, when the export still holds it.
    second: Option<&'a str>,
}

/// What one export presents of the one project.
#[derive(Clone, Copy)]
struct Project<'a> {
    title: &'a str,
    description: &'a str,
}

fn conversation_json(conversation: Conversation<'_>) -> serde_json::Value {
    let first = serde_json::json!({
        "id": "node-1",
        "parent": null,
        "message": {
            "id": "message-1",
            "author": { "role": "user" },
            "content": { "parts": [conversation.text] }
        }
    });
    let mut mapping = serde_json::Map::new();
    mapping.insert("node-1".to_owned(), first);
    if let Some(second) = conversation.second {
        mapping.insert(
            "node-2".to_owned(),
            serde_json::json!({
                "id": "node-2",
                "parent": "node-1",
                "message": {
                    "id": "message-2",
                    "author": { "role": "assistant" },
                    "content": { "parts": [second] }
                }
            }),
        );
    }
    serde_json::json!({ "id": "conv-a", "title": conversation.title, "mapping": mapping })
}

/// An export of the conversation. `pad` names a second conversation only this export holds, so two
/// exports that present the same content are still different bytes.
fn export(conversation: Conversation<'_>, project: Option<Project<'_>>, pad: usize) -> Vec<u8> {
    let padding = serde_json::json!({
        "id": format!("conv-pad-{pad}"),
        "title": "Padding",
        "mapping": {}
    });
    let conversations =
        serde_json::Value::Array(vec![conversation_json(conversation), padding]).to_string();
    match project {
        None => zip_of(&[("conversations.json", conversations.as_bytes())]),
        Some(project) => {
            let projects = serde_json::json!([{
                "id": "project-one",
                "title": project.title,
                "description": project.description,
                "instructions": [],
                "conversation_ids": [],
                "asset_ids": []
            }])
            .to_string();
            zip_of(&[
                ("conversations.json", conversations.as_bytes()),
                ("projects.json", projects.as_bytes()),
            ])
        }
    }
}

struct Account {
    database: DisposableDatabase,
    blobs: BlobStore,
    root: tempfile::TempDir,
    worker: InitialImportWorker,
    tenant: Uuid,
    imports: usize,
}

async fn account() -> Account {
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
    Account {
        database,
        blobs,
        root,
        worker,
        tenant: Uuid::nil(),
        imports: 0,
    }
}

fn mapping() -> Vec<(Uuid, String)> {
    vec![(
        Uuid::parse_str(PLATFORM_USER).expect("a literal UUID"),
        ACCOUNT.to_owned(),
    )]
}

impl Account {
    /// Receives and imports one export of the conversation; returns its archive identity.
    async fn import(
        &mut self,
        conversation: Conversation<'_>,
        project: Option<Project<'_>>,
    ) -> Uuid {
        self.imports += 1;
        let bytes = export(conversation, project, self.imports);
        let received = receive_export(self.database.pool(), &self.blobs, ACCOUNT, bytes).await;
        drain(&self.worker).await;
        self.tenant = received.account;
        received.archive
    }

    async fn delete_archive(&self, archive: Uuid) {
        let service = PrivacyDeletionService::new(self.database.pool().clone(), self.blobs.clone())
            .with_platform_users(mapping());
        let request = Uuid::now_v7();
        service
            .plan(
                self.tenant,
                request,
                PrivacyDeletionScope::Archive {
                    ai_archive_id: archive,
                },
            )
            .await
            .expect("planning succeeds")
            .expect("the owned scope plans");
        service
            .execute(request)
            .await
            .expect("the deletion completes");
    }

    /// How many stored values contain `needle`: content parts, headers, projects, the history,
    /// the per-export versions and every queued fact.
    async fn occurrences(&self, needle: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM chatgpt_archive.content_parts WHERE payload::text LIKE $1)
                  + (SELECT count(*) FROM chatgpt_archive.conversations WHERE title LIKE $1)
                  + (SELECT count(*) FROM chatgpt_archive.projects
                     WHERE title LIKE $1 OR description LIKE $1 OR instructions LIKE $1)
                  + (SELECT count(*) FROM chatgpt_archive.revisions WHERE payload::text LIKE $1)
                  + (SELECT count(*) FROM chatgpt_archive.entity_versions
                     WHERE snapshot::text LIKE $1)
                  + (SELECT count(*) FROM chatgpt_archive.outbox_events WHERE payload::text LIKE $1)",
        )
        .bind(format!("%{needle}%"))
        .fetch_one(self.database.pool())
        .await
        .expect("the stored values are countable")
    }

    async fn conversation_title(&self) -> String {
        sqlx::query_scalar(
            "SELECT title FROM chatgpt_archive.conversations WHERE external_id = 'conv-a'",
        )
        .fetch_one(self.database.pool())
        .await
        .expect("the conversation exists")
    }

    /// The text of the current content-part revision of every message, by provider message id.
    async fn current_texts(&self) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT m.external_id, p.payload #>> '{}' FROM chatgpt_archive.messages m
             JOIN chatgpt_archive.content_parts p ON p.message_id = m.id
             WHERE p.revision = (SELECT max(revision) FROM chatgpt_archive.content_parts
                                 WHERE message_id = m.id)
             ORDER BY m.external_id",
        )
        .fetch_all(self.database.pool())
        .await
        .expect("the content parts are readable")
    }

    async fn project(&self) -> (String, String) {
        sqlx::query_as("SELECT title, description FROM chatgpt_archive.projects")
            .fetch_one(self.database.pool())
            .await
            .expect("the project exists")
    }

    /// The facts queued after the last tombstone, which are the ones that follow the deletion.
    async fn follow_up(&self) -> Vec<EventEnvelope> {
        let rows: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT payload FROM chatgpt_archive.outbox_events
             WHERE event_type LIKE 'ai_archive.%' ORDER BY id",
        )
        .fetch_all(self.database.pool())
        .await
        .expect("the archive facts are readable");
        let facts: Vec<EventEnvelope> = rows
            .into_iter()
            .map(|raw| serde_json::from_value(raw).expect("a complete envelope"))
            .collect();
        let last = facts
            .iter()
            .rposition(|fact| fact.event_type.to_wire() == TOMBSTONED)
            .expect("the deletion queued a tombstone");
        facts
            .get(last + 1..)
            .expect("the tombstone is inside the list")
            .to_vec()
    }

    async fn finish(self) {
        drop(self.root);
        self.database.discard().await;
    }
}

fn archive_of(fact: &EventEnvelope) -> Uuid {
    let archive = fact
        .payload
        .get("import_provenance")
        .and_then(|provenance| provenance.get("ai_archive_id"))
        .and_then(serde_json::Value::as_str)
        .expect("the fact names its archive");
    // The wire form of an archive identity may carry a type prefix.
    Uuid::parse_str(archive.rsplit(':').next().expect("an identity")).expect("a UUID")
}

fn text_of(fact: &EventEnvelope) -> String {
    serde_json::Value::Object(fact.payload.clone()).to_string()
}

fn conversation(title: &'static str, text: &'static str) -> Conversation<'static> {
    Conversation {
        title,
        text,
        second: None,
    }
}

#[tokio::test]
async fn a_conversation_edited_by_the_deleted_export_is_restored_to_the_retained_text() {
    let mut account = account().await;
    let first = account
        .import(conversation("Original title", "ORIGINAL-TEXT"), None)
        .await;
    let second = account
        .import(conversation("Edited title", "EDITED-TEXT"), None)
        .await;
    assert!(account.occurrences("EDITED-TEXT").await > 0);

    account.delete_archive(second).await;

    assert_eq!(
        account.occurrences("EDITED").await,
        0,
        "nothing the deleted export wrote survives in a row, a revision, a version or a fact"
    );
    assert_eq!(account.conversation_title().await, "Original title");
    assert_eq!(
        account.current_texts().await,
        [("message-1".to_owned(), "ORIGINAL-TEXT".to_owned())]
    );
    let follow_up = account.follow_up().await;
    assert_eq!(follow_up.len(), 1, "the survivor is announced again");
    assert_eq!(
        follow_up[0].event_type.to_wire(),
        "ai_archive.conversation.updated.v1"
    );
    assert!(text_of(&follow_up[0]).contains("ORIGINAL-TEXT"));
    assert_eq!(archive_of(&follow_up[0]), first);
    account.finish().await;
}

#[tokio::test]
async fn a_project_edited_by_the_deleted_export_is_restored_to_the_retained_title() {
    let mut account = account().await;
    let first = account
        .import(
            conversation("Title", "Text"),
            Some(Project {
                title: "ORIGINAL-PROJECT",
                description: "ORIGINAL-DESCRIPTION",
            }),
        )
        .await;
    let second = account
        .import(
            conversation("Title", "Text"),
            Some(Project {
                title: "EDITED-PROJECT",
                description: "EDITED-DESCRIPTION",
            }),
        )
        .await;
    assert!(account.occurrences("EDITED-PROJECT").await > 0);

    account.delete_archive(second).await;

    assert_eq!(
        account.occurrences("EDITED").await,
        0,
        "no edited project title or description survives"
    );
    assert_eq!(
        account.project().await,
        (
            "ORIGINAL-PROJECT".to_owned(),
            "ORIGINAL-DESCRIPTION".to_owned()
        )
    );
    let follow_up = account.follow_up().await;
    let project = follow_up
        .iter()
        .find(|fact| fact.event_type.to_wire() == "ai_archive.project.updated.v1")
        .expect("the project survivor is announced again");
    assert!(text_of(project).contains("ORIGINAL-PROJECT"));
    assert_eq!(archive_of(project), first);
    account.finish().await;
}

#[tokio::test]
async fn an_intermediate_version_does_not_survive_between_two_retained_ones() {
    let mut account = account().await;
    account
        .import(conversation("First title", "FIRST-TEXT"), None)
        .await;
    let middle = account
        .import(conversation("Second title", "SECOND-TEXT"), None)
        .await;
    account
        .import(conversation("Third title", "THIRD-TEXT"), None)
        .await;

    account.delete_archive(middle).await;

    assert_eq!(account.occurrences("SECOND").await, 0);
    assert_eq!(account.conversation_title().await, "Third title");
    assert_eq!(
        account.current_texts().await,
        [("message-1".to_owned(), "THIRD-TEXT".to_owned())],
        "the newest retained version stays current"
    );
    let revisions: Vec<String> = sqlx::query_scalar(
        "SELECT p.payload #>> '{}' FROM chatgpt_archive.content_parts p
         ORDER BY p.revision",
    )
    .fetch_all(account.database.pool())
    .await
    .expect("the revisions are readable");
    assert_eq!(
        revisions,
        ["FIRST-TEXT", "THIRD-TEXT"],
        "the history keeps the two retained versions in order"
    );
    for fact in account.follow_up().await {
        assert!(!text_of(&fact).contains("SECOND"));
    }
    account.finish().await;
}

#[tokio::test]
async fn deleting_the_newest_export_falls_back_to_the_newest_retained_version() {
    let mut account = account().await;
    account
        .import(conversation("First title", "FIRST-TEXT"), None)
        .await;
    let middle = account
        .import(conversation("Second title", "SECOND-TEXT"), None)
        .await;
    let newest = account
        .import(conversation("Third title", "THIRD-TEXT"), None)
        .await;

    account.delete_archive(newest).await;

    assert_eq!(account.occurrences("THIRD").await, 0);
    assert_eq!(account.conversation_title().await, "Second title");
    assert_eq!(
        account.current_texts().await,
        [("message-1".to_owned(), "SECOND-TEXT".to_owned())],
        "the second export's version, not the first's"
    );
    let follow_up = account.follow_up().await;
    assert_eq!(follow_up.len(), 1);
    assert!(text_of(&follow_up[0]).contains("SECOND-TEXT"));
    assert_eq!(
        archive_of(&follow_up[0]),
        middle,
        "the evidence is the export that holds this version"
    );
    account.finish().await;
}

#[tokio::test]
async fn a_retained_export_that_repeats_the_edit_keeps_it_and_is_named_as_its_evidence() {
    let mut account = account().await;
    account
        .import(conversation("Original title", "ORIGINAL-TEXT"), None)
        .await;
    let edited = account
        .import(conversation("Edited title", "EDITED-TEXT"), None)
        .await;
    let repeated = account
        .import(conversation("Edited title", "EDITED-TEXT"), None)
        .await;

    account.delete_archive(edited).await;

    assert_eq!(
        account.current_texts().await,
        [("message-1".to_owned(), "EDITED-TEXT".to_owned())],
        "a retained export holds the edit, so it stays"
    );
    let follow_up = account.follow_up().await;
    assert_eq!(follow_up.len(), 1);
    assert!(text_of(&follow_up[0]).contains("EDITED-TEXT"));
    assert_eq!(
        archive_of(&follow_up[0]),
        repeated,
        "the retained export that holds the edit, not the earlier one that holds the original"
    );
    account.finish().await;
}

#[tokio::test]
async fn a_message_only_the_deleted_export_held_goes_even_after_unchanged_reads() {
    let mut account = account().await;
    let first = account
        .import(
            Conversation {
                title: "Title",
                text: "KEPT-TEXT",
                second: Some("DROPPED-TEXT"),
            },
            None,
        )
        .await;
    account
        .import(conversation("Title", "KEPT-TEXT"), None)
        .await;
    account
        .import(conversation("Title", "KEPT-TEXT"), None)
        .await;

    account.delete_archive(first).await;

    assert_eq!(
        account.occurrences("DROPPED").await,
        0,
        "the later exports never held the second message, so reading the conversation unchanged \
         did not make them evidence of it"
    );
    assert_eq!(
        account.current_texts().await,
        [("message-1".to_owned(), "KEPT-TEXT".to_owned())]
    );
    account.finish().await;
}

#[tokio::test]
async fn deleting_every_export_leaves_no_version_snapshot() {
    let mut account = account().await;
    account
        .import(conversation("First title", "FIRST-TEXT"), None)
        .await;
    account
        .import(
            conversation("Second title", "SECOND-TEXT"),
            Some(Project {
                title: "PROJECT-TITLE",
                description: "PROJECT-DESCRIPTION",
            }),
        )
        .await;
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.entity_versions")
        .fetch_one(account.database.pool())
        .await
        .expect("the versions are countable");
    assert!(
        versions >= 4,
        "two conversation versions and a project, got {versions}"
    );

    let service =
        PrivacyDeletionService::new(account.database.pool().clone(), account.blobs.clone())
            .with_platform_users(mapping());
    let request = Uuid::now_v7();
    service
        .plan(account.tenant, request, PrivacyDeletionScope::Tenant)
        .await
        .expect("planning succeeds")
        .expect("the owned scope plans");
    service
        .execute(request)
        .await
        .expect("the deletion completes");

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM chatgpt_archive.entity_versions")
        .fetch_one(account.database.pool())
        .await
        .expect("the versions are countable");
    assert_eq!(remaining, 0, "no snapshot of a deleted record survives");
    assert_eq!(account.occurrences("TEXT").await, 0);
    account.finish().await;
}
