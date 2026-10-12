//! The announced-subject ledger records the owner each fact named, in the transaction that
//! enqueues the fact (XR-021 CONTRACTS.md R5-06).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test failures report through panics"
)]

mod common;

use std::sync::Arc;

use common::{DisposableDatabase, archive_limits, drain, receive_export, single_message_zip};
use ratatoskr_chatgpt_archive::{BlobStore, InitialImportWorker, ParserRegistry};
use uuid::Uuid;

const USER_A: &str = "018f0000-0000-7000-8000-0000000000a1";
const USER_B: &str = "018f0000-0000-7000-8000-0000000000b2";

fn worker(
    database: &DisposableDatabase,
    blobs: &BlobStore,
    mapping: Vec<(Uuid, String)>,
) -> InitialImportWorker {
    InitialImportWorker::new(
        database.pool().clone(),
        blobs.clone(),
        Arc::new(ParserRegistry::runtime().expect("the runtime registry builds")),
        archive_limits(),
    )
    .with_platform_users(mapping)
}

fn mapped(user: &str, account_ref: &str) -> Vec<(Uuid, String)> {
    vec![(
        Uuid::parse_str(user).expect("a literal UUID"),
        account_ref.to_owned(),
    )]
}

/// `(kind, subject id, archive, owner)` for every ledger row, in a stable order.
async fn ledger(database: &DisposableDatabase) -> Vec<(String, String, Uuid, String)> {
    sqlx::query_as(
        "SELECT subject_kind, subject_id, ai_archive_id, owner_ref
         FROM chatgpt_archive.announced_subjects ORDER BY subject_kind, subject_id, owner_ref",
    )
    .fetch_all(database.pool())
    .await
    .expect("the ledger is readable")
}

async fn fact_aggregate(database: &DisposableDatabase, event_type: &str) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT aggregate_id FROM chatgpt_archive.outbox_events
         WHERE event_type = $1 ORDER BY id",
    )
    .bind(event_type)
    .fetch_all(database.pool())
    .await
    .expect("the outbox is readable")
}

#[tokio::test]
async fn an_import_under_a_mapping_records_the_archive_each_conversation_and_each_project() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    let received =
        receive_export(database.pool(), &blobs, &account_ref, common::project_zip()).await;

    drain(&worker(&database, &blobs, mapped(USER_A, &account_ref))).await;

    let project = fact_aggregate(&database, "ai_archive.project.added.v1").await[0];
    let conversation = fact_aggregate(&database, "ai_archive.conversation.added.v1").await[0];
    let owner = format!("user:{USER_A}");
    let mut expected = vec![
        (
            "archive".to_owned(),
            received.archive.to_string(),
            received.archive,
            owner.clone(),
        ),
        (
            "conversation".to_owned(),
            conversation.to_string(),
            received.archive,
            owner.clone(),
        ),
        (
            "project".to_owned(),
            project.to_string(),
            received.archive,
            owner,
        ),
    ];
    expected.sort();
    assert_eq!(
        ledger(&database).await,
        expected,
        "one row for the archive, the conversation and the project, each with the mapped owner"
    );
    database.discard().await;
}

#[tokio::test]
async fn a_changed_digest_adds_no_row_for_an_unchanged_subject_or_for_the_same_owner() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    let importer = worker(&database, &blobs, mapped(USER_A, &account_ref));
    receive_export(
        database.pool(),
        &blobs,
        &account_ref,
        single_message_zip("conversation-1", "original"),
    )
    .await;
    drain(&importer).await;
    let after_first = ledger(&database).await;
    assert_eq!(
        after_first.len(),
        2,
        "one archive and one conversation after the first import"
    );

    receive_export(
        database.pool(),
        &blobs,
        &account_ref,
        single_message_zip("conversation-1", "rewritten"),
    )
    .await;
    drain(&importer).await;

    let after_second = ledger(&database).await;
    assert_eq!(
        after_second
            .iter()
            .filter(|row| row.0 == "conversation")
            .count(),
        1,
        "the updated conversation keeps its one row for the same owner"
    );
    assert_eq!(
        after_second.iter().filter(|row| row.0 == "archive").count(),
        2,
        "the second archive is a new subject"
    );
    database.discard().await;
}

#[tokio::test]
async fn an_account_without_a_mapping_emits_no_fact_and_writes_no_ledger_row() {
    let database = DisposableDatabase::create().await;
    let root = tempfile::tempdir().expect("blob root");
    let blobs = BlobStore::new(root.path()).expect("blob store");
    let account_ref = format!("acc-{}", Uuid::now_v7());
    receive_export(database.pool(), &blobs, &account_ref, common::project_zip()).await;

    drain(&worker(
        &database,
        &blobs,
        mapped(USER_B, "another-account"),
    ))
    .await;

    assert!(
        fact_aggregate(&database, "ai_archive.archive.imported.v1")
            .await
            .is_empty(),
        "an unmapped account announces nothing"
    );
    assert!(
        ledger(&database).await.is_empty(),
        "a subject that was never announced has no ledger row"
    );
    database.discard().await;
}

#[tokio::test]
async fn the_ledger_has_no_foreign_key_to_exports() {
    let database = DisposableDatabase::create().await;
    let references_exports: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint
         WHERE contype = 'f'
           AND conrelid = 'chatgpt_archive.announced_subjects'::regclass
           AND confrelid = 'chatgpt_archive.exports'::regclass",
    )
    .fetch_one(database.pool())
    .await
    .expect("the catalog is readable");
    assert_eq!(
        references_exports, 0,
        "deleting an export must not remove the record of what was announced"
    );
    database.discard().await;
}
