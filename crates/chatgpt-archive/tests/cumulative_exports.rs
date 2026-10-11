//! Cumulative exports of one account share one row per conversation (XR-021 CONTRACTS.md R3-06,
//! the identity every export-scoped fact, observation and deletion decision stands on).
//!
//! ChatGPT exports repeat what earlier exports held. A conversation is one record of the account
//! however many exports contain it: each export observes it, and a later export that reads the
//! same content announces nothing, because the fact it would announce already exists.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test scaffolding: a failed setup step must fail the test loudly"
)]

mod common;

use std::sync::Arc;

use common::{DisposableDatabase, archive_limits, drain, receive_export, zip_of};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use uuid::Uuid;

const PLATFORM_USER: &str = "018f0000-0000-7000-8000-0000000000c2";
const ACCOUNT: &str = "acc-cumulative";

fn conversation(id: &str, text: &str) -> serde_json::Value {
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
                    "content": { "parts": [text] }
                }
            }
        }
    })
}

fn export(conversations: &[(&str, &str)]) -> Vec<u8> {
    let documents: Vec<serde_json::Value> = conversations
        .iter()
        .map(|(id, text)| conversation(id, text))
        .collect();
    zip_of(&[(
        "conversations.json",
        serde_json::Value::Array(documents).to_string().as_bytes(),
    )])
}

struct Account {
    database: DisposableDatabase,
    blobs: BlobStore,
    root: tempfile::TempDir,
    worker: InitialImportWorker,
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
    .with_platform_users(vec![(
        Uuid::parse_str(PLATFORM_USER).expect("a literal UUID"),
        ACCOUNT.to_owned(),
    )]);
    Account {
        database,
        blobs,
        root,
        worker,
    }
}

impl Account {
    async fn import(&self, conversations: &[(&str, &str)]) {
        receive_export(
            self.database.pool(),
            &self.blobs,
            ACCOUNT,
            export(conversations),
        )
        .await;
        drain(&self.worker).await;
    }

    async fn count(&self, query: &str) -> i64 {
        sqlx::query_scalar(query)
            .fetch_one(self.database.pool())
            .await
            .expect("the count is readable")
    }

    async fn finish(self) {
        drop(self.root);
        self.database.discard().await;
    }
}

#[tokio::test]
async fn a_conversation_in_two_exports_is_one_row_observed_by_both() {
    let account = account().await;
    account
        .import(&[("conv-a", "Text of a"), ("conv-b", "Text of b")])
        .await;
    account
        .import(&[
            ("conv-a", "Text of a"),
            ("conv-b", "Text of b"),
            ("conv-c", "Text of c"),
        ])
        .await;

    assert_eq!(
        account
            .count("SELECT count(*) FROM chatgpt_archive.conversations")
            .await,
        3,
        "one row per provider conversation id, not one per export"
    );
    assert_eq!(
        account
            .count(
                "SELECT count(*) FROM chatgpt_archive.export_entity_observations
                 WHERE entity_kind = 'conversation'"
            )
            .await,
        5,
        "each export observes every conversation it holds"
    );
    assert_eq!(
        account
            .count(
                "SELECT count(*) FROM chatgpt_archive.outbox_events
                 WHERE event_type = 'ai_archive.conversation.added.v1'"
            )
            .await,
        3,
        "a conversation the second export reads unchanged is not announced again"
    );
    assert_eq!(
        account
            .count("SELECT count(*) FROM chatgpt_archive.messages")
            .await,
        3,
        "the messages of a shared conversation are not duplicated"
    );
    account.finish().await;
}

#[tokio::test]
async fn a_changed_conversation_keeps_its_identity_and_gains_a_revision() {
    let account = account().await;
    account.import(&[("conv-a", "First text")]).await;
    account.import(&[("conv-a", "Edited text")]).await;

    assert_eq!(
        account
            .count("SELECT count(*) FROM chatgpt_archive.conversations")
            .await,
        1
    );
    assert_eq!(
        account
            .count(
                "SELECT count(*) FROM chatgpt_archive.revisions
                 WHERE entity_table = 'conversations'"
            )
            .await,
        2,
        "the edit is a second revision of the same conversation"
    );
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM chatgpt_archive.outbox_events
         WHERE event_type LIKE 'ai_archive.conversation.%' ORDER BY id",
    )
    .fetch_all(account.database.pool())
    .await
    .expect("the facts are readable");
    assert_eq!(
        kinds,
        [
            "ai_archive.conversation.added.v1",
            "ai_archive.conversation.updated.v1"
        ],
        "the same conversation id is announced as added and then as updated"
    );
    account.finish().await;
}
